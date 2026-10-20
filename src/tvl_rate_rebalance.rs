//! Automated pool-level TVL-tiered rate rebalancing (issue #333).
//!
//! Distinct from issue #182 (per-user tier rebalancing) and issue #233 (TVL
//! smoothing, which keeps emission constant): this steps the *base*
//! `reward_rate_bps` discretely as total pool TVL crosses admin-configured
//! thresholds â€” higher TVL triggers a lower rate to preserve reward-token
//! sustainability as more capital chases the same emission budget.
//!
//! # Wiring
//!
//! The issue specifies `check_and_rebalance_rate()` runs automatically
//! "inside `stake` and `unstake` after position update" â€” this crate has no
//! live `stake`/`unstake` entrypoint to call it from yet (see this PR's
//! description). `check_and_rebalance_rate` is implemented as a public
//! function ready to be called from wherever that lands, and is also exposed
//! directly so it can be triggered standalone (e.g. by a keeper) in the
//! meantime.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `vesting_cliff.rs`. Thresholds are stored as a single
//! `Vec` under one key rather than per-threshold entries â€” the issue caps
//! the list at 5, so there's no per-entry storage-growth concern that would
//! motivate splitting it.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance-storage key for the configured thresholds, ascending by TVL.
const THRESHOLDS_KEY: Symbol = symbol_short!("tvl_thr");

/// Maximum number of thresholds, per the issue's acceptance criteria.
pub const MAX_THRESHOLDS: u32 = 5;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TVLRateThreshold {
    pub tvl_threshold: i128,
    pub reward_rate_bps: i128,
}

/// The configured thresholds, ascending by TVL. Empty when none are set.
pub fn get_thresholds(env: &Env) -> Vec<TVLRateThreshold> {
    env.storage()
        .instance()
        .get(&THRESHOLDS_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

/// The threshold whose `tvl_threshold` is the highest one at or below
/// `current_tvl` â€” i.e. the tier `current_tvl` currently sits in. `None`
/// when no thresholds are configured, or `current_tvl` is below every
/// configured threshold (rate stays at the base `reward_rate_bps`).
///
/// Thresholds are stored ascending, so the active one is the last entry
/// whose `tvl_threshold <= current_tvl`.
fn active_threshold_for(thresholds: &Vec<TVLRateThreshold>, current_tvl: i128) -> Option<TVLRateThreshold> {
    let mut active: Option<TVLRateThreshold> = None;
    for t in thresholds.iter() {
        if t.tvl_threshold <= current_tvl {
            active = Some(t);
        } else {
            break;
        }
    }
    active
}

/// The chokepoint the stake/unstake path is meant to call after a position
/// update: reads current TVL, applies the matching threshold's rate if it
/// differs from the currently-set rate, and emits `rate_rebalanced` only
/// when a change actually happens.
///
/// When no thresholds are configured, this is a no-op and the base
/// `reward_rate_bps` is left exactly as an admin last set it via
/// `set_reward_rate_bps` â€” introducing this feature never silently changes
/// an existing pool's rate.
pub fn check_and_rebalance(env: &Env) {
    let thresholds = get_thresholds(env);
    if thresholds.is_empty() {
        return;
    }

    let current_tvl = balance::get_total_deposited(env);
    let target_rate_bps = match active_threshold_for(&thresholds, current_tvl) {
        Some(t) => t.reward_rate_bps,
        // Below every configured threshold: rate reverts to whatever the
        // admin's base rate was before any threshold applied. We don't have
        // a separately-stored "base" rate distinct from the live one, so â€”
        // matching how `set_reward_rate_bps` is the single source of truth
        // elsewhere â€” the lowest-TVL state simply leaves the current rate
        // untouched rather than guessing at an implicit "tier 0" rate the
        // issue doesn't specify.
        None => return,
    };

    let current_rate_bps = balance::get_reward_rate_bps(env) as i128;
    if target_rate_bps == current_rate_bps {
        return;
    }

    balance::set_reward_rate_bps(env, target_rate_bps as u32);

    env.events().publish(
        (symbol_short!("rate_rbl"),),
        (
            current_rate_bps,
            target_rate_bps,
            current_tvl,
            env.ledger().sequence(),
        ),
    );
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Set the TVL-tiered rate thresholds. Admin only. Max 5, must be
    /// strictly ascending by `tvl_threshold`.
    pub fn set_tvl_rate_thresholds(
        env: Env,
        thresholds: Vec<TVLRateThreshold>,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if thresholds.len() > MAX_THRESHOLDS {
            return Err(VaultError::TooManyBoostTiers);
        }

        let mut last_tvl: Option<i128> = None;
        for t in thresholds.iter() {
            if let Some(prev) = last_tvl {
                if t.tvl_threshold <= prev {
                    return Err(VaultError::InvalidBoostSchedule);
                }
            }
            last_tvl = Some(t.tvl_threshold);
        }

        env.storage().instance().set(&THRESHOLDS_KEY, &thresholds);
        Ok(())
    }

    /// The configured TVL-tiered rate thresholds, ascending by TVL.
    pub fn get_tvl_rate_thresholds(env: Env) -> Vec<TVLRateThreshold> {
        get_thresholds(&env)
    }

    /// Reads current TVL and applies the matching threshold's rate if it
    /// differs from the rate currently in effect. Callable directly (e.g.
    /// by a keeper) since no live `stake`/`unstake` path exists yet to call
    /// it automatically â€” see this module's doc comment.
    pub fn check_and_rebalance_rate(env: Env) {
        check_and_rebalance(&env);
    }

    /// The threshold currently active for the pool's TVL, if any.
    pub fn get_active_tvl_threshold(env: Env) -> Option<TVLRateThreshold> {
        let thresholds = get_thresholds(&env);
        let current_tvl = balance::get_total_deposited(&env);
        active_threshold_for(&thresholds, current_tvl)
    }
}















