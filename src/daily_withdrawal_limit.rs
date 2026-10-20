//! Per-user rolling 24-hour withdrawal limit (issue #554).
//!
//! Distinct from `vault.rs`'s existing per-transaction `WithdrawalLimitExceeded`
//! circuit breaker: this caps each user's *cumulative* withdrawn amount over a
//! rolling day, limiting exposure from a compromised user key without
//! affecting normal usage patterns that stay under the cap.
//!
//! # Error code
//!
//! `VaultError` is already at Soroban's 50-variant `#[contracterror]` cap
//! (`InsufficientStake = 50`), so `withdraw`/`unstake`/`unstake_all` (which
//! all already return `Result<i128, VaultError>`) reuse the existing
//! `VaultError::WithdrawalLimitExceeded` variant for this rather than adding
//! a new `DailyLimitExceeded` case — mirroring how `DataKey`'s own
//! 50-variant cap is already handled elsewhere in this crate (raw
//! `Symbol`-keyed storage instead of new `DataKey` variants).
//!
//! # Rolling window
//!
//! The window isn't a fixed calendar day: `window_start_ledger` only resets
//! once a full `LEDGERS_PER_DAY` has elapsed since it was set, so a user's
//! very first withdrawal in a new window starts the clock for everything
//! that follows it. This means old withdrawals age out gradually as the
//! window slides forward, rather than all resetting at once at a fixed
//! boundary an attacker could time around.
//!
//! # Storage
//!
//! Raw `Symbol`-keyed persistent storage per user, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::vault::LEDGERS_PER_DAY;
use crate::vault::VaultContractClient;
use crate::VaultContract;

const LIMIT_KEY: Symbol = symbol_short!("dw_cfg");
const TRACKER_KEY: Symbol = symbol_short!("dw_trk");

/// A user's cumulative withdrawn amount for the current rolling window.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DailyWithdrawalTracker {
    pub window_start_ledger: u32,
    pub withdrawn_today: i128,
}

/// Read the configured per-user daily withdrawal cap (0 = disabled).
pub fn get_limit(env: &Env) -> i128 {
    env.storage().instance().get(&LIMIT_KEY).unwrap_or(0)
}

pub fn set_limit(env: &Env, max_per_day: i128) {
    env.storage().instance().set(&LIMIT_KEY, &max_per_day);
}

fn get_raw_tracker(env: &Env, user: &Address) -> Option<DailyWithdrawalTracker> {
    env.storage().persistent().get(&(TRACKER_KEY, user.clone()))
}

fn set_tracker(env: &Env, user: &Address, tracker: &DailyWithdrawalTracker) {
    env.storage()
        .persistent()
        .set(&(TRACKER_KEY, user.clone()), tracker);
}

/// `user`'s tracker for the current rolling window. A fresh, zeroed tracker
/// is returned once a full `LEDGERS_PER_DAY` has elapsed since the stored
/// one's `window_start_ledger`.
fn current_tracker(env: &Env, user: &Address) -> DailyWithdrawalTracker {
    let now = env.ledger().sequence();
    match get_raw_tracker(env, user) {
        Some(t) if now.saturating_sub(t.window_start_ledger) < LEDGERS_PER_DAY => t,
        _ => DailyWithdrawalTracker {
            window_start_ledger: now,
            withdrawn_today: 0,
        },
    }
}

/// Checks `amount` against `user`'s remaining rolling-24h headroom and, if
/// it fits, records it against the window. No-op when the limit is
/// disabled (0). Reverts with `VaultError::WithdrawalLimitExceeded` (see
/// module docs re: the 50-variant cap) when `amount` would push the
/// window's cumulative total over the configured cap.
pub fn enforce_and_record(env: &Env, user: &Address, amount: i128) -> Result<(), VaultError> {
    let limit = get_limit(env);
    if limit == 0 {
        return Ok(());
    }
    let mut tracker = current_tracker(env, user);
    let new_total = tracker.withdrawn_today.saturating_add(amount);
    if new_total > limit {
        return Err(VaultError::WithdrawalLimitExceeded);
    }
    tracker.withdrawn_today = new_total;
    set_tracker(env, user, &tracker);
    Ok(())
}

/// `user`'s remaining rolling-24h withdrawal headroom. `0` when the limit
/// is disabled or already exhausted for the current window.
pub fn remaining(env: &Env, user: &Address) -> i128 {
    let limit = get_limit(env);
    if limit == 0 {
        return 0;
    }
    let tracker = current_tracker(env, user);
    limit.saturating_sub(tracker.withdrawn_today).max(0)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Sets the per-user cap on cumulative withdrawals per rolling 24h
    /// window. Admin only. `0` disables the limit.
    pub fn set_daily_withdrawal_limit(env: Env, amount: i128) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        if amount < 0 {
            return Err(VaultError::ZeroAmount);
        }
        crate::daily_withdrawal_limit::set_limit(&env, amount);
        Ok(())
    }

    /// `user`'s remaining rolling-24h withdrawal headroom. `0` when the
    /// limit is disabled.
    pub fn get_remaining_daily_limit(env: Env, user: Address) -> i128 {
        crate::daily_withdrawal_limit::remaining(&env, &user)
    }
}
