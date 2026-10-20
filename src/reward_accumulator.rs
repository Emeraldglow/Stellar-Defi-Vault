//! Global reward-per-share accumulator with high-precision scaling (Issue #650).
//!
//! Replaces per-position sequential reward iteration with an O(1) global accumulator index.
//!
//! # Scaling Constant & Headroom Analysis
//! - Scale factor: `ACCUMULATOR_SCALE = 1_000_000_000_000` (1e12).
//! - Precision: Supports sub-stroop precision down to 1e-12 stroops, eliminating dust loss.
//! - Overflow Headroom Analysis:
//!   - In Soroban, numbers are represented as signed 128-bit integers (`i128::MAX` ≈ 1.7014 * 10^38).
//!   - Max Stellar token supply / TVL in stroops: 10^11 XLM = 10^18 stroops.
//!   - Max reward rate: 50,000 bps (500% APR).
//!   - Over 100 years, `elapsed_ledgers * rate_bps / (10_000 * LEDGERS_PER_YEAR) ≈ 500`.
//!   - Global accumulator index `A ≈ 500 * 10^12 = 5 * 10^14`.
//!   - Product when computing reward: `shares * delta_index <= 10^18 * (5 * 10^14) = 5 * 10^32`.
//!   - Headroom ratio: `i128::MAX / (5 * 10^32) > 3.4 * 10^5` (> 340,000x headroom).
//!   - Therefore, checked math will never overflow under all reasonable operating parameters.

use soroban_sdk::{symbol_short, Address, Env, Symbol};

use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::storage::{PositionKey, RewardKey, StorageKey};
use crate::vault::{BOOST_BPS_BASE, STELLAR_LEDGERS_PER_YEAR};

/// High-precision scaling constant: 1e12.
pub const ACCUMULATOR_SCALE: i128 = 1_000_000_000_000;

const GLOBAL_INDEX_KEY: Symbol = symbol_short!("g_rw_idx");
const LAST_LEDGER_KEY: Symbol = symbol_short!("g_rw_ldg");
const USER_CHECKPOINT_KEY: Symbol = symbol_short!("u_rw_ckp");

/// Initializes the global accumulator at contract deployment or initial setup.
pub fn init_accumulator(env: &Env) {
    let current_ledger = env.ledger().sequence();
    set_last_accumulator_ledger(env, current_ledger);
    if env.storage().instance().get::<_, i128>(&GLOBAL_INDEX_KEY).is_none() {
        set_global_reward_per_share(env, 0);
    }
}

/// Reads the global reward index (scaled by 1e12).
pub fn get_global_reward_per_share(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&StorageKey::Reward(RewardKey::GlobalRewardIndex))
        .or_else(|| env.storage().instance().get(&GLOBAL_INDEX_KEY))
        .unwrap_or(0)
}

/// Sets the global reward index.
pub fn set_global_reward_per_share(env: &Env, index: i128) {
    env.storage()
        .instance()
        .set(&StorageKey::Reward(RewardKey::GlobalRewardIndex), &index);
    env.storage().instance().set(&GLOBAL_INDEX_KEY, &index);
}

/// Reads the ledger at which the global accumulator was last updated.
pub fn get_last_accumulator_ledger(env: &Env) -> Option<u32> {
    env.storage()
        .instance()
        .get(&StorageKey::Reward(RewardKey::LastAccumulatorLedger))
        .or_else(|| env.storage().instance().get(&LAST_LEDGER_KEY))
}

/// Sets the ledger at which the global accumulator was last updated.
pub fn set_last_accumulator_ledger(env: &Env, ledger: u32) {
    env.storage()
        .instance()
        .set(&StorageKey::Reward(RewardKey::LastAccumulatorLedger), &ledger);
    env.storage().instance().set(&LAST_LEDGER_KEY, &ledger);
}

/// Reads a user's checkpointed accumulator index.
pub fn get_user_checkpoint(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&StorageKey::Position(PositionKey::RewardAccumulatorCheckpoint(user.clone())))
        .or_else(|| env.storage().persistent().get(&(USER_CHECKPOINT_KEY, user.clone())))
        .unwrap_or_else(|| get_global_reward_per_share(env))
}

/// Checkpoints a user's accumulator index to the current global index.
pub fn checkpoint_user(env: &Env, user: &Address) {
    let global_index = get_global_reward_per_share(env);
    env.storage()
        .persistent()
        .set(&StorageKey::Position(PositionKey::RewardAccumulatorCheckpoint(user.clone())), &global_index);
    env.storage()
        .persistent()
        .set(&(USER_CHECKPOINT_KEY, user.clone()), &global_index);
}

/// Updates the global accumulator based on elapsed ledgers and pool total shares.
/// Called on every stake, unstake, claim, and reward rate change.
pub fn update_global_accumulator(env: &Env) -> Result<i128, VaultError> {
    let current_ledger = env.ledger().sequence();
    let last_ledger = get_last_accumulator_ledger(env).unwrap_or(current_ledger);
    let mut index = get_global_reward_per_share(env);

    if current_ledger > last_ledger {
        let elapsed = current_ledger - last_ledger;
        let total_shares = balance::get_total_shares(env);
        let rate_bps = balance::get_reward_rate_bps(env) as i128;

        if total_shares > 0 && rate_bps > 0 {
            // Delta reward per share scaled by ACCUMULATOR_SCALE (1e12):
            // delta = (rate_bps * elapsed * ACCUMULATOR_SCALE) / (BOOST_BPS_BASE * STELLAR_LEDGERS_PER_YEAR)
            let numerator = rate_bps
                .checked_mul(elapsed as i128)
                .ok_or(VaultError::ArithmeticError)?
                .checked_mul(ACCUMULATOR_SCALE)
                .ok_or(VaultError::ArithmeticError)?;

            let denominator = (BOOST_BPS_BASE as i128)
                .checked_mul(STELLAR_LEDGERS_PER_YEAR as i128)
                .ok_or(VaultError::ArithmeticError)?;

            let delta_index = numerator
                .checked_div(denominator)
                .ok_or(VaultError::ArithmeticError)?;

            index = index
                .checked_add(delta_index)
                .ok_or(VaultError::ArithmeticError)?;

            set_global_reward_per_share(env, index);
            events::reward_accumulator_updated(env, index, current_ledger);
        }
        set_last_accumulator_ledger(env, current_ledger);
    }

    Ok(index)
}

/// Computes the pending reward accrued by a position since its last checkpoint.
pub fn compute_accumulated_reward(env: &Env, user: &Address, shares: i128) -> Result<i128, VaultError> {
    if shares <= 0 {
        return Ok(0);
    }
    let global_index = get_global_reward_per_share(env);
    let user_checkpoint = get_user_checkpoint(env, user);

    if global_index <= user_checkpoint {
        return Ok(0);
    }

    let delta = global_index
        .checked_sub(user_checkpoint)
        .ok_or(VaultError::ArithmeticError)?;

    let reward = shares
        .checked_mul(delta)
        .ok_or(VaultError::ArithmeticError)?
        .checked_div(ACCUMULATOR_SCALE)
        .ok_or(VaultError::ArithmeticError)?;

    Ok(reward)
}
