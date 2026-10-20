//! Minimum reward-reserve ratio floor (issue #405).
//!
//! Enforces a hard floor under the reward token balance: it must always keep
//! at least `ratio_bps` of total outstanding (accrued-but-unclaimed) reward
//! obligations in reserve. A claim that would breach the floor is capped at
//! whatever headroom remains above it; the rest is queued rather than paid.
//!
//! # Wiring
//!
//! Like `epoch_reward_cap.rs` (issue #270, whose `claim_epoch_capped_reward`
//! this mirrors) and `compound_optimizer.rs`, this exposes its own capped
//! claim entrypoint (`claim_with_reserve_floor`) rather than editing the
//! existing `claim()` flow in `vault.rs`, keeping the floor opt-in and
//! additive. Per this issue's own notes, any amount deferred by the floor is
//! queued into the *same* `epoch_reward_cap::DeferredReward(Address)` bucket
//! epoch_reward_cap uses, claimable through the existing
//! `claim_deferred_reward()` entrypoint â€” whichever guardrail (epoch cap or
//! reserve floor) binds first, the overflow lands in one place.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! instance storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::epoch_reward_cap;
use crate::errors::VaultError;
use crate::events;
use crate::VaultContract;
use crate::vault::VaultContractClient;
use crate::vault::{ BOOST_BPS_BASE, MAX_GINI_STAKERS};

/// Instance-storage key for the configured floor ratio, in basis points.
/// `0` (the default) disables the floor entirely.
const RATIO_KEY: Symbol = symbol_short!("mrr_bps");

/// Sum of every active staker's accrued (unclaimed) reward, used as the
/// "total outstanding reward obligations" the floor ratio is measured
/// against.
///
/// Bounded by `MAX_GINI_STAKERS`, matching `get_reward_gini_coefficient`'s
/// scan bound (issue #275) â€” this is called from a claim path, so unlike
/// that admin-gated query it does not revert once the bound is hit; it is a
/// best-effort sum over the first `MAX_GINI_STAKERS` registered stakers.
fn total_pending_rewards(env: &Env) -> i128 {
    let all_stakers = balance::get_all_stakers(env);
    let mut total: i128 = 0;
    let scan_len = all_stakers.len().min(MAX_GINI_STAKERS);
    for i in 0..scan_len {
        let staker = all_stakers.get(i).unwrap();
        total = total.saturating_add(balance::get_accrued_reward(env, &staker));
    }
    total
}

fn get_ratio_bps(env: &Env) -> u32 {
    env.storage().instance().get(&RATIO_KEY).unwrap_or(0)
}

/// The minimum reward-token amount that must stay in reserve at all times,
/// given the current ratio and total outstanding obligations.
fn compute_minimum_reserve_amount(env: &Env) -> i128 {
    let ratio_bps = get_ratio_bps(env);
    if ratio_bps == 0 {
        return 0;
    }
    let obligations = total_pending_rewards(env);
    obligations.saturating_mul(ratio_bps as i128) / (BOOST_BPS_BASE as i128)
}

/// Reward-token amount currently available to pay out without breaching the
/// floor. Never negative.
fn compute_available_for_claim(env: &Env) -> i128 {
    let pool_balance = balance::get_reward_pool_balance(env);
    let floor = compute_minimum_reserve_amount(env);
    (pool_balance - floor).max(0)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Set the minimum reserve ratio, in basis points (e.g. `2000` = keep at
    /// least 20% of outstanding reward obligations in reserve at all times).
    /// `0` disables the floor. Admin only.
    pub fn set_minimum_reserve_ratio_bps(env: Env, ratio_bps: u32) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if ratio_bps > BOOST_BPS_BASE {
            return Err(VaultError::InvalidRate);
        }

        env.storage().instance().set(&RATIO_KEY, &ratio_bps);

        env.events().publish(
            (symbol_short!("mrr_set"),),
            (ratio_bps, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only query: the currently configured minimum reserve ratio, in
    /// basis points.
    pub fn get_minimum_reserve_ratio_bps(env: Env) -> u32 {
        get_ratio_bps(&env)
    }

    /// Read-only query: the reward-token amount that must stay in reserve
    /// right now, given the configured ratio and outstanding obligations.
    pub fn minimum_reserve_amount(env: Env) -> i128 {
        compute_minimum_reserve_amount(&env)
    }

    /// Read-only query: reward-token amount currently available to pay out
    /// without breaching the reserve floor.
    pub fn available_for_claim(env: Env) -> i128 {
        compute_available_for_claim(&env)
    }

    /// Read-only query: whether the reward pool currently holds at least the
    /// configured minimum reserve.
    pub fn is_reserve_floor_met(env: Env) -> bool {
        balance::get_reward_pool_balance(&env) >= compute_minimum_reserve_amount(&env)
    }

    /// Claim accrued rewards subject to the configured minimum reserve
    /// floor.
    ///
    /// If paying the full accrued balance would push the reward pool below
    /// the floor, only the available headroom is paid now; the remainder is
    /// queued as a `DeferredReward` (shared with `epoch_reward_cap.rs`,
    /// immediately claimable via `claim_deferred_reward` once the pool has
    /// enough reserve again). Returns the amount actually paid now.
    pub fn claim_with_reserve_floor(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let accrued = balance::get_accrued_reward(&env, &user);
        if accrued <= 0 {
            return Ok(0);
        }

        let ratio_bps = get_ratio_bps(&env);
        let payable = if ratio_bps == 0 {
            accrued
        } else {
            accrued.min(compute_available_for_claim(&env))
        };
        let deferred_amount = accrued - payable;

        // Settle the whole accrued balance here: `payable` is transferred
        // below, the rest moves into the deferred bucket.
        balance::set_accrued_reward(&env, &user, 0);

        if payable > 0 {
            let pool_balance = balance::get_reward_pool_balance(&env);
            if pool_balance < payable {
                return Err(VaultError::InsufficientRewardPool);
            }
            balance::set_reward_pool_balance(&env, pool_balance - payable);

            let token_addr: Address = env
                .storage()
                .instance()
                .get(&crate::storage::DataKey::Token)
                .ok_or(VaultError::NotInitialized)?;
            soroban_sdk::token::Client::new(&env, &token_addr).transfer(
                &env.current_contract_address(),
                &user,
                &payable,
            );

            events::claimed(&env, &user, payable, env.ledger().sequence());
        }

        if deferred_amount > 0 {
            // Immediately eligible: `claim_deferred_reward`'s own
            // `InsufficientRewardPool` check is what actually gates payout
            // once reserves recover, so there is no separate epoch-style
            // deadline to wait out here.
            let next_epoch_start = env.ledger().sequence();
            epoch_reward_cap::queue_deferred(&env, &user, deferred_amount, next_epoch_start);

            env.events().publish(
                (symbol_short!("mrr_trig"), user.clone()),
                (
                    accrued,
                    payable,
                    deferred_amount,
                    env.ledger().sequence(),
                ),
            );
        }

        Ok(payable)
    }
}









