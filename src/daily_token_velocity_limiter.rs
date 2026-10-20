//! Daily token velocity limiter (issue #411).
//!
//! Distinct from `epoch_reward_cap.rs` (issue #270's per-user epoch cap):
//! this caps total reward token outflow across ALL stakers in any 24-hour
//! window, so a coordinated mass-claim event can't dump the whole reward
//! pool onto the market in a single day. Any amount that would push the
//! day over its limit is queued as a `DeferredReward` for the caller and
//! becomes payable once the day rolls over (or immediately, if headroom
//! opens up sooner).
//!
//! # Wiring
//!
//! Like `epoch_reward_cap.rs`, this exposes its own capped claim entrypoint
//! (`claim_with_daily_velocity_limit`) rather than editing `vault.rs`'s
//! existing `claim()`. `unstake`'s auto-claim isn't touched by this module
//! at all, so it naturally bypasses the daily limit as required â€” users can
//! always exit.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw
//! `Symbol`-keyed storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::VaultContract;
use crate::vault::{LEDGERS_PER_DAY};
use crate::vault::VaultContractClient;

const LIMIT_KEY: Symbol = symbol_short!("dv_cfg");
const TRACKER_KEY: Symbol = symbol_short!("dv_trk");
const DEFERRED_KEY: Symbol = symbol_short!("dv_dfr");

/// Pool-wide reward outflow tracked for the current day (issue #411).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DailyVelocityTracker {
    pub day_start_ledger: u32,
    pub outflow_today: i128,
}

/// A staker's reward amount deferred past the daily limit, payable once
/// headroom is available.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeferredReward {
    pub amount: i128,
}

fn get_limit(env: &Env) -> i128 {
    env.storage().instance().get(&LIMIT_KEY).unwrap_or(0)
}

fn set_limit(env: &Env, max_per_day: i128) {
    env.storage().instance().set(&LIMIT_KEY, &max_per_day);
}

fn get_raw_tracker(env: &Env) -> Option<DailyVelocityTracker> {
    env.storage().instance().get(&TRACKER_KEY)
}

fn set_tracker(env: &Env, tracker: &DailyVelocityTracker) {
    env.storage().instance().set(&TRACKER_KEY, tracker);
}

fn get_deferred(env: &Env, user: &Address) -> Option<DeferredReward> {
    env.storage().persistent().get(&(DEFERRED_KEY, user.clone()))
}

fn set_deferred(env: &Env, user: &Address, deferred: &DeferredReward) {
    env.storage()
        .persistent()
        .set(&(DEFERRED_KEY, user.clone()), deferred);
}

fn remove_deferred(env: &Env, user: &Address) {
    env.storage().persistent().remove(&(DEFERRED_KEY, user.clone()));
}

/// The current day's tracker. Day boundary is `current_ledger /
/// LEDGERS_PER_DAY`; a fresh, zeroed tracker is returned once the stored
/// one belongs to an earlier day.
fn current_tracker(env: &Env) -> DailyVelocityTracker {
    let now = env.ledger().sequence();
    let day_start = (now / LEDGERS_PER_DAY) * LEDGERS_PER_DAY;
    match get_raw_tracker(env) {
        Some(t) if t.day_start_ledger / LEDGERS_PER_DAY == now / LEDGERS_PER_DAY => t,
        _ => DailyVelocityTracker {
            day_start_ledger: day_start,
            outflow_today: 0,
        },
    }
}

fn transfer_reward(env: &Env, user: &Address, amount: i128) -> Result<(), VaultError> {
    let pool_balance = balance::get_reward_pool_balance(env);
    if pool_balance < amount {
        return Err(VaultError::InsufficientRewardPool);
    }
    balance::set_reward_pool_balance(env, pool_balance - amount);

    let token_addr: Address = env
        .storage()
        .instance()
        .get(&crate::storage::DataKey::Token)
        .ok_or(VaultError::NotInitialized)?;
    soroban_sdk::token::Client::new(env, &token_addr).transfer(
        &env.current_contract_address(),
        user,
        &amount,
    );
    Ok(())
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Sets the pool-wide cap on reward token outflow per rolling day.
    /// Admin only. `0` disables the limit.
    pub fn set_daily_velocity_limit(env: Env, max_per_day: i128) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        if max_per_day < 0 {
            return Err(VaultError::ZeroAmount);
        }
        crate::daily_token_velocity_limiter::set_limit(&env, max_per_day);
        Ok(())
    }

    /// Current daily velocity stats as `(limit, used_today, remaining)`.
    /// All zero when the limit is disabled.
    pub fn get_daily_velocity_stats(env: Env) -> (i128, i128, i128) {
        let limit = crate::daily_token_velocity_limiter::get_limit(&env);
        if limit == 0 {
            return (0, 0, 0);
        }
        let tracker = crate::daily_token_velocity_limiter::current_tracker(&env);
        let remaining = limit.saturating_sub(tracker.outflow_today).max(0);
        (limit, tracker.outflow_today, remaining)
    }

    /// A user's currently queued deferred reward amount, `0` if none.
    pub fn get_deferred_daily_reward(env: Env, user: Address) -> i128 {
        crate::daily_token_velocity_limiter::get_deferred(&env, &user)
            .map(|d| d.amount)
            .unwrap_or(0)
    }

    /// Claims accrued rewards subject to the pool-wide daily velocity
    /// limit. If the payout would exceed the day's remaining headroom, only
    /// the remaining headroom is paid now and the rest is queued in
    /// `DeferredReward` for `user`. Returns the amount actually paid now.
    pub fn claim_with_daily_velocity_limit(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let accrued = balance::get_accrued_reward(&env, &user);
        if accrued <= 0 {
            return Ok(0);
        }

        let limit = crate::daily_token_velocity_limiter::get_limit(&env);
        if limit == 0 {
            balance::set_accrued_reward(&env, &user, 0);
            crate::daily_token_velocity_limiter::transfer_reward(&env, &user, accrued)?;
            events::claimed(&env, &user, accrued, env.ledger().sequence());
            return Ok(accrued);
        }

        let mut tracker = crate::daily_token_velocity_limiter::current_tracker(&env);
        let remaining = limit.saturating_sub(tracker.outflow_today).max(0);
        let payable = accrued.min(remaining);
        let deferred_amount = accrued - payable;

        balance::set_accrued_reward(&env, &user, 0);

        if payable > 0 {
            crate::daily_token_velocity_limiter::transfer_reward(&env, &user, payable)?;
            tracker.outflow_today = tracker.outflow_today.saturating_add(payable);
            events::claimed(&env, &user, payable, env.ledger().sequence());
        }
        crate::daily_token_velocity_limiter::set_tracker(&env, &tracker);

        if deferred_amount > 0 {
            let existing = crate::daily_token_velocity_limiter::get_deferred(&env, &user)
                .map(|d| d.amount)
                .unwrap_or(0);
            crate::daily_token_velocity_limiter::set_deferred(
                &env,
                &user,
                &crate::daily_token_velocity_limiter::DeferredReward {
                    amount: existing.saturating_add(deferred_amount),
                },
            );
            env.events().publish(
                (symbol_short!("dv_hit"),),
                (tracker.outflow_today, limit, env.ledger().sequence()),
            );
        }

        Ok(payable)
    }

    /// Collects a previously queued deferred daily reward, up to whatever
    /// headroom is available under the current day's limit. Any amount
    /// still over the limit stays queued for a later call. Returns the
    /// amount actually paid.
    pub fn claim_deferred_daily_reward(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let deferred = crate::daily_token_velocity_limiter::get_deferred(&env, &user)
            .ok_or(VaultError::NothingToWithdraw)?;

        let limit = crate::daily_token_velocity_limiter::get_limit(&env);
        let mut tracker = crate::daily_token_velocity_limiter::current_tracker(&env);
        let remaining = if limit == 0 {
            deferred.amount
        } else {
            limit.saturating_sub(tracker.outflow_today).max(0)
        };
        let payable = deferred.amount.min(remaining);
        if payable <= 0 {
            return Ok(0);
        }

        let leftover = deferred.amount - payable;
        if leftover > 0 {
            crate::daily_token_velocity_limiter::set_deferred(
                &env,
                &user,
                &crate::daily_token_velocity_limiter::DeferredReward { amount: leftover },
            );
        } else {
            crate::daily_token_velocity_limiter::remove_deferred(&env, &user);
        }

        crate::daily_token_velocity_limiter::transfer_reward(&env, &user, payable)?;
        if limit != 0 {
            tracker.outflow_today = tracker.outflow_today.saturating_add(payable);
            crate::daily_token_velocity_limiter::set_tracker(&env, &tracker);
        }
        events::claimed(&env, &user, payable, env.ledger().sequence());

        Ok(payable)
    }
}















