//! Vault extensions for issues #701, #702, #703, #704.
//!
//! - Issue #701: percentage-based emission reduction schedule. The schedule
//!   itself (`set_halving_schedule`, `get_halving_config`, `next_halving_at`)
//!   lives next to the original halving feature (#231) in `vault.rs` and
//!   `balance.rs`; this module adds `get_next_reduction`,
//!   `get_halving_reduction_bps` and `disable_halving_schedule`.
//! - Issue #702: per-address reward cap per epoch (`set_user_epoch_reward_cap`,
//!   `get_user_epoch_rewards`).
//! - Issue #703: temporary fee holiday (`start_fee_holiday`,
//!   `end_fee_holiday`, `get_fee_holiday`).
//! - Issue #704: operator-funded insurance reserve
//!   (`set_insurance_fee_share_bps`, `use_insurance_reserve`,
//!   `get_insurance_reserve`).
//!
//! # Rate precedence (issue #701)
//!
//! Reward accrual starts from the stored base rate. That stored rate is what
//! `set_reward_rate_bps`, a settled `set_rate_ramp` (`complete_rate_ramp` /
//! `cancel_rate_ramp`) and the dynamic reward curve (`sync_dynamic_rate`) all
//! write to, so the order is: the base rate is chosen by those controls first,
//! then (bootstrap window aside) the reduction schedule is applied on top of
//! it for every elapsed interval, and the schedule's floor is applied last.
//! `get_next_reduction` therefore reports the reduced stored rate, not the
//! interpolated value `get_current_rate` shows during an unsettled ramp.
//! Disabling the schedule makes the static base rate apply again.
//!
//! # Per-user epoch cap (issue #702)
//!
//! The cap bounds the reward one address can be credited for a single epoch
//! of the epoch distribution mode (`set_epoch_mode`). It is enforced where an
//! epoch's reward is computed (`epoch_reward`), so it applies identically to
//! `claim_epoch_rewards` and the views, and each epoch is capped on its own,
//! which is what resets it at rollover. Only the capped amount is ever debited
//! from the reward pool; the excess simply stays there and is not
//! redistributed. The cap is per address, so it is only a speed bump: a holder
//! can split a position across several wallets to earn more than the cap.
//!
//! # Fee holiday (issue #703)
//!
//! While a holiday is active every unstake fee is 0 (and the per-token deposit
//! fee override reads as 0). Expiry is evaluated lazily, as with scheduled
//! unpause: the stored end ledger is compared with the current ledger on each
//! fee calculation and the `fee_holiday_ended` event is emitted by the first
//! fee calculation (or `end_fee_holiday`) that sees it has passed.
//!
//! # Insurance reserve (issue #704)
//!
//! A configurable share of every collected unstake fee is credited to a
//! reserve counter instead of the reward pool, the revenue-share pool or the
//! community treasury. The fee never belongs to user principal (it has already
//! been taken out of `total_deposited`), so the reserve cannot mix with staked
//! funds. It is distinct from `insurance.rs` (per-position policies) and the
//! mutual insurance pool: it is funded only by the operator's fee income.
//! `use_insurance_reserve` moves at most half of the reserve (rounded up) into
//! the reward pool per call, to cover a shortfall.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap (see `storage.rs`), so everything
//! here uses raw `Symbol`-keyed storage, matching the other feature modules.

use soroban_sdk::{contractimpl, symbol_short, Address, BytesN, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::storage::{DataKey, EpochState};
use crate::vault::{VaultContract, VaultContractClient, BOOST_BPS_BASE};

const USER_EPOCH_CAP_KEY: Symbol = symbol_short!("uep_cap");
const FEE_HOLIDAY_KEY: Symbol = symbol_short!("fee_hol");
const INSURANCE_BPS_KEY: Symbol = symbol_short!("ins_bps");
const INSURANCE_RESERVE_KEY: Symbol = symbol_short!("ins_rsv");

/// Fixed-point scale used by `finalize_epoch` for the per-epoch reward factor.
const EPOCH_REWARD_SCALE: i128 = 1_000_000_000_000;

/// Largest share of the reserve (in bps, rounded up) one
/// `use_insurance_reserve` call may release.
const INSURANCE_USE_CAP_BPS: i128 = 5_000;

// ── Issue #702: per-user epoch reward cap ────────────────────────────────────

fn get_user_epoch_cap(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&USER_EPOCH_CAP_KEY)
        .unwrap_or(0)
}

/// Clamp one epoch's `reward` for a single address to the configured cap.
/// A cap of `0` means disabled.
pub fn apply_user_epoch_cap(env: &Env, reward: i128) -> i128 {
    let cap = get_user_epoch_cap(env);
    if cap > 0 && reward > cap {
        cap
    } else {
        reward
    }
}

// ── Issue #703: fee holiday ──────────────────────────────────────────────────

fn get_fee_holiday_end(env: &Env) -> Option<u32> {
    env.storage().instance().get(&FEE_HOLIDAY_KEY)
}

/// Whether a fee holiday is active at the current ledger. Read-only.
pub fn is_fee_holiday_active(env: &Env) -> bool {
    match get_fee_holiday_end(env) {
        Some(ends_at) => env.ledger().sequence() < ends_at,
        None => false,
    }
}

/// Whether a fee holiday is active, clearing an expired one first and emitting
/// `fee_holiday_ended` for it (lazy expiry). For state-changing fee paths.
pub fn settle_fee_holiday(env: &Env) -> bool {
    match get_fee_holiday_end(env) {
        Some(ends_at) if env.ledger().sequence() < ends_at => true,
        Some(ends_at) => {
            env.storage().instance().remove(&FEE_HOLIDAY_KEY);
            events::fee_holiday_ended(env, ends_at, env.ledger().sequence());
            false
        }
        None => false,
    }
}

// ── Issue #704: insurance reserve ────────────────────────────────────────────

fn get_insurance_share_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&INSURANCE_BPS_KEY)
        .unwrap_or(0)
}

fn get_reserve(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&INSURANCE_RESERVE_KEY)
        .unwrap_or(0)
}

/// Credit the configured share of `fee` to the insurance reserve and return
/// the amount taken, so the caller routes only `fee - taken` onwards.
pub fn take_insurance_share(env: &Env, fee: i128) -> Result<i128, VaultError> {
    let share_bps = get_insurance_share_bps(env);
    if share_bps == 0 || fee <= 0 {
        return Ok(0);
    }
    let share = fee
        .checked_mul(share_bps as i128)
        .and_then(|v| v.checked_div(BOOST_BPS_BASE as i128))
        .ok_or(VaultError::ArithmeticError)?;
    if share > 0 {
        let reserve = get_reserve(env)
            .checked_add(share)
            .ok_or(VaultError::ArithmeticError)?;
        env.storage().instance().set(&INSURANCE_RESERVE_KEY, &reserve);
    }
    Ok(share)
}

#[contractimpl]
impl VaultContract {
    // ── Issue #701: emission reduction schedule ──────────────────────────────

    /// The next scheduled rate reduction as `(at_ledger, new_rate_bps)`, or
    /// `None` when no schedule is configured (issue #701). `new_rate_bps` is
    /// the stored base rate after the reduction, never below the floor.
    pub fn get_next_reduction(env: Env) -> Option<(u32, u32)> {
        let at_ledger = balance::next_halving_at(&env)?;
        let base_rate_bps = balance::get_reward_rate_bps(&env);
        let new_rate = balance::halving_adjusted_rate(&env, base_rate_bps, at_ledger);
        Some((at_ledger, u32::try_from(new_rate).unwrap_or(u32::MAX)))
    }

    /// The per-interval reduction of the schedule, in bps (issue #701).
    pub fn get_halving_reduction_bps(env: Env) -> u32 {
        balance::get_halving_reduction_bps(&env)
    }

    /// Admin: remove the reduction schedule so the static base rate applies
    /// again (issue #701). Fails with `NoCampaignActive` if none is set.
    pub fn disable_halving_schedule(env: Env, admin: Address) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        if balance::get_halving_config(&env).is_none() {
            return Err(VaultError::NoCampaignActive);
        }
        balance::clear_halving_config(&env);
        Ok(())
    }

    // ── Issue #702: per-user epoch reward cap ────────────────────────────────

    /// Admin: cap the reward a single address can earn per epoch; `0`
    /// disables the cap (issue #702).
    pub fn set_user_epoch_reward_cap(
        env: Env,
        admin: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        if amount < 0 {
            return Err(VaultError::InvalidRewardAmount);
        }
        env.storage().instance().set(&USER_EPOCH_CAP_KEY, &amount);
        Ok(())
    }

    /// The configured per-address epoch reward cap (`0` = disabled).
    pub fn get_user_epoch_reward_cap(env: Env) -> i128 {
        get_user_epoch_cap(&env)
    }

    /// The reward `user` would receive for the current epoch at their current
    /// stake, with the per-address cap applied (issue #702). Uses the same
    /// arithmetic as `finalize_epoch`/`epoch_reward`; `0` outside epoch mode.
    pub fn get_user_epoch_rewards(env: Env, user: Address) -> i128 {
        let state: EpochState = match env.storage().instance().get(&DataKey::CurrentEpoch) {
            Some(state) => state,
            None => return 0,
        };
        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let shares = balance::get_shares(&env, &user);
        if total_shares <= 0 || total_deposited <= 0 || shares <= 0 {
            return 0;
        }
        let staked = balance::shares_to_amount(total_shares, total_deposited, shares).unwrap_or(0);
        let reward_factor = state
            .reward_pool
            .checked_mul(EPOCH_REWARD_SCALE)
            .and_then(|v| v.checked_div(total_deposited))
            .unwrap_or(0);
        let reward = staked
            .checked_mul(reward_factor)
            .and_then(|v| v.checked_div(EPOCH_REWARD_SCALE))
            .unwrap_or(0);
        apply_user_epoch_cap(&env, reward)
    }

    // ── Issue #703: fee holiday ──────────────────────────────────────────────

    /// Admin: waive unstake (and per-token deposit) fees until ledger
    /// `ends_at_ledger` (issue #703). Replaces any holiday already running.
    /// Fails with `InvalidRate` if `ends_at_ledger` is not in the future.
    pub fn start_fee_holiday(
        env: Env,
        admin: Address,
        ends_at_ledger: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        let now = env.ledger().sequence();
        if ends_at_ledger <= now {
            return Err(VaultError::InvalidRate);
        }
        env.storage().instance().set(&FEE_HOLIDAY_KEY, &ends_at_ledger);
        events::fee_holiday_started(&env, ends_at_ledger, now);
        Ok(())
    }

    /// Admin: end the fee holiday early (issue #703). Fails with
    /// `NoCampaignActive` if none was started.
    pub fn end_fee_holiday(env: Env, admin: Address) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        let ends_at = get_fee_holiday_end(&env).ok_or(VaultError::NoCampaignActive)?;
        env.storage().instance().remove(&FEE_HOLIDAY_KEY);
        events::fee_holiday_ended(&env, ends_at, env.ledger().sequence());
        Ok(())
    }

    /// The ledger the active fee holiday ends at, or `None` when no holiday
    /// is active (an expired one reads as `None`) (issue #703).
    pub fn get_fee_holiday(env: Env) -> Option<u32> {
        get_fee_holiday_end(&env).filter(|ends_at| env.ledger().sequence() < *ends_at)
    }

    // ── Issue #704: insurance reserve ────────────────────────────────────────

    /// Admin: set the share of each collected unstake fee, in bps, that is
    /// routed to the insurance reserve; `0` disables (issue #704).
    pub fn set_insurance_fee_share_bps(
        env: Env,
        admin: Address,
        bps: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        if bps > BOOST_BPS_BASE {
            return Err(VaultError::InvalidRate);
        }
        env.storage().instance().set(&INSURANCE_BPS_KEY, &bps);
        Ok(())
    }

    /// The configured insurance share of collected fees, in bps.
    pub fn get_insurance_fee_share_bps(env: Env) -> u32 {
        get_insurance_share_bps(&env)
    }

    /// The insurance reserve balance, tracked separately from the reward pool,
    /// the treasury and user principal (issue #704).
    pub fn get_insurance_reserve(env: Env) -> i128 {
        get_reserve(&env)
    }

    /// Admin: release `amount` from the insurance reserve into the reward pool
    /// to cover a shortfall, recording `reason_hash` in the emitted
    /// `insurance_reserve_used` event (issue #704). One call may release at
    /// most half of the reserve (rounded up); a larger `amount` fails with
    /// `WithdrawalLimitExceeded`.
    pub fn use_insurance_reserve(
        env: Env,
        admin: Address,
        amount: i128,
        reason_hash: BytesN<32>,
    ) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        let reserve = get_reserve(&env);
        let per_call_cap = reserve
            .checked_mul(INSURANCE_USE_CAP_BPS)
            .and_then(|v| v.checked_add(BOOST_BPS_BASE as i128 - 1))
            .and_then(|v| v.checked_div(BOOST_BPS_BASE as i128))
            .ok_or(VaultError::ArithmeticError)?;
        if amount > per_call_cap {
            return Err(VaultError::WithdrawalLimitExceeded);
        }

        env.storage()
            .instance()
            .set(&INSURANCE_RESERVE_KEY, &(reserve - amount));
        let pool = balance::get_reward_pool_balance(&env)
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticError)?;
        balance::set_reward_pool_balance(&env, pool);

        events::insurance_reserve_used(&env, &admin, amount, &reason_hash, env.ledger().sequence());
        Ok(())
    }
}
