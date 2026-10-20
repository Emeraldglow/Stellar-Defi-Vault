//! Sub-unit reward accumulator (issue #367).
//!
//! Soroban token transfers require amounts >= 1 stroop. Reward accrual â€”
//! `position * rate_bps * elapsed_ledgers / (10_000 * ledgers_per_year)` â€” is
//! only exact in real-number math; done directly in `i128` it truncates on
//! every accrual, and for small positions or short accrual windows that
//! truncated remainder can be a meaningful fraction of what's actually owed,
//! silently and permanently lost on every claim.
//!
//! This module accrues at a higher fixed-point precision
//! ([`SUB_UNIT_SCALE`] units per stroop) and carries the leftover fraction
//! forward per user instead of discarding it, so it eventually crosses the
//! 1-stroop line and gets paid once enough of it has accumulated.
//!
//! # Wiring
//!
//! Like `compound_optimizer.rs` and `epoch_reward_cap.rs`, this is a fully
//! self-contained accrual + claim path with its own checkpoint, rather than
//! a wrapper around `vault.rs`'s existing `claim()` â€” that flow's own reward
//! accrual (whatever feeds `AccruedReward`) is untouched, so there's no
//! double-accrual risk between this module and it. `claim_sub_unit_reward`
//! computes reward directly from the position size and the pool's reward
//! rate, exactly like `compound_optimizer::compute_optimal_interval` does.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Fixed-point scale for sub-stroop precision: one stroop = `SUB_UNIT_SCALE`
/// scaled units. The remainder carried between claims is always in
/// `[0, SUB_UNIT_SCALE)` scaled units, i.e. strictly less than one stroop.
pub const SUB_UNIT_SCALE: i128 = 1_000_000;

/// Ledgers per year at ~5s/ledger, matching `vault::STELLAR_LEDGERS_PER_YEAR`.
const LEDGERS_PER_YEAR: i128 = 6_307_200;

/// Persistent key prefix for a user's carried sub-stroop remainder. Keyed by
/// `(REMAINDER_KEY, user)`.
const REMAINDER_KEY: Symbol = symbol_short!("sua_rem");
/// Persistent key prefix for the ledger this module last accrued up to for a
/// user. Keyed by `(CHECKPOINT_KEY, user)`.
const CHECKPOINT_KEY: Symbol = symbol_short!("sua_ckp");

/// A user's current sub-unit accrual state, returned by
/// `get_sub_unit_status`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SubUnitAccrualStatus {
    pub remainder_scaled: i128,
    pub checkpoint_ledger: u32,
    pub pending_whole_stroops: i128,
}

fn get_remainder(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&(REMAINDER_KEY, user.clone()))
        .unwrap_or(0)
}

fn set_remainder(env: &Env, user: &Address, remainder: i128) {
    env.storage()
        .persistent()
        .set(&(REMAINDER_KEY, user.clone()), &remainder);
}

fn get_checkpoint(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&(CHECKPOINT_KEY, user.clone()))
}

fn set_checkpoint(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&(CHECKPOINT_KEY, user.clone()), &ledger);
}

/// Computes whole stroops payable and the new remainder, without writing
/// anything to storage. Used by both the read-only preview and the mutating
/// claim path so they can never disagree.
fn compute_accrual(env: &Env, user: &Address, now: u32) -> Result<(i128, i128), VaultError> {
    let last = get_checkpoint(env, user).unwrap_or(now);
    let elapsed = now.saturating_sub(last);
    let remainder = get_remainder(env, user);

    let position = balance::get_shares(env, user);
    let rate_bps = balance::get_reward_rate_bps(env) as i128;
    if elapsed == 0 || position <= 0 || rate_bps <= 0 {
        return Ok((remainder / SUB_UNIT_SCALE, remainder % SUB_UNIT_SCALE));
    }

    let scaled_reward = position
        .checked_mul(rate_bps)
        .and_then(|v| v.checked_mul(elapsed as i128))
        .and_then(|v| v.checked_mul(SUB_UNIT_SCALE))
        .and_then(|v| v.checked_div(10_000i128.checked_mul(LEDGERS_PER_YEAR)?))
        .ok_or(VaultError::ArithmeticError)?;

    let total_scaled = scaled_reward
        .checked_add(remainder)
        .ok_or(VaultError::ArithmeticError)?;

    Ok((total_scaled / SUB_UNIT_SCALE, total_scaled % SUB_UNIT_SCALE))
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Read-only preview of what `claim_sub_unit_reward` would do right now,
    /// without mutating any state: the carried remainder, the last accrual
    /// checkpoint, and the whole stroops that would be paid if claimed at
    /// the current ledger.
    pub fn get_sub_unit_status(env: Env, user: Address) -> Result<SubUnitAccrualStatus, VaultError> {
        let now = env.ledger().sequence();
        let (pending_whole_stroops, _) = compute_accrual(&env, &user, now)?;
        Ok(SubUnitAccrualStatus {
            remainder_scaled: get_remainder(&env, &user),
            checkpoint_ledger: get_checkpoint(&env, &user).unwrap_or(now),
            pending_whole_stroops,
        })
    }

    /// Accrue reward at sub-stroop precision since the user's last
    /// checkpoint and pay out the whole-stroop portion, carrying any
    /// leftover fraction forward. Returns the amount actually transferred
    /// (0 if the accrued whole-stroop amount is still zero).
    pub fn claim_sub_unit_reward(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let now = env.ledger().sequence();
        let (whole, new_remainder) = compute_accrual(&env, &user, now)?;

        set_checkpoint(&env, &user, now);
        set_remainder(&env, &user, new_remainder);

        if whole <= 0 {
            return Ok(0);
        }

        let pool_balance = balance::get_reward_pool_balance(&env);
        if pool_balance < whole {
            return Err(VaultError::InsufficientRewardPool);
        }
        balance::set_reward_pool_balance(&env, pool_balance - whole);

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&crate::storage::DataKey::Token)
            .ok_or(VaultError::NotInitialized)?;
        soroban_sdk::token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &user,
            &whole,
        );

        events::claimed(&env, &user, whole, now);
        Ok(whole)
    }
}















