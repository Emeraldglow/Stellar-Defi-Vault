//! Active compound-interval optimizer (issue #338).
//!
//! Distinct from issue #185 (read-only optimal-claim-frequency advisory):
//! this lets a user opt in to having a keeper *execute* claim-and-restake at
//! the mathematically optimal interval, rather than just being told what
//! that interval is.
//!
//! Optimal interval = `sqrt(2 * tx_cost / (rate * position))`, adapted to
//! integer ledgers â€” the classic EOQ-style tradeoff between "claim often,
//! pay tx cost every time" and "claim rarely, leave more rewards uncompounded
//! for longer".
//!
//! # Wiring
//!
//! `trigger_optimized_compound` is specified to "claim and restake" â€” this
//! crate has no live `claim`/`stake`/`restake` entrypoint to call yet (see
//! this PR's description). The function here does everything up to that
//! point (auth, interval-elapsed gate, interval recalculation, event, and
//! the keeper incentive bookkeeping) and returns the amount that *would* be
//! claimed (`balance::get_accrued_reward`) rather than performing a token
//! transfer that doesn't have a real claim path to route through yet.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `vesting_cliff.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Persistent-storage key prefix for a user's optimizer config.
const CONFIG_KEY: Symbol = symbol_short!("cmp_opt");

/// Keeper incentive: 0.25% of the claimed amount, per the issue.
const KEEPER_INCENTIVE_BPS: i128 = 25;
const BPS_DENOMINATOR: i128 = 10_000;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OptimizerConfig {
    pub enabled: bool,
    pub tx_cost_bps: u32,
    pub last_optimized_at: u32,
    pub optimal_interval_ledgers: u32,
}

pub fn get_config(env: &Env, user: &Address) -> Option<OptimizerConfig> {
    env.storage().persistent().get(&(CONFIG_KEY, user.clone()))
}

fn set_config(env: &Env, user: &Address, config: &OptimizerConfig) {
    env.storage()
        .persistent()
        .set(&(CONFIG_KEY, user.clone()), config);
}

/// Integer square root via Newton's method (no `std`/`libm` available in this
/// `#![no_std]` contract). Converges in a handful of iterations for the
/// magnitudes involved here (tx-cost/rate/position ratios, not astronomical
/// values) and never overflows since it only ever squares numbers below its
/// own current estimate of the answer.
fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Computes `sqrt(2 * tx_cost / (rate * position))` adapted to integer
/// ledgers, given the position's current size and the pool's reward rate.
///
/// `tx_cost_bps` is basis points of the position size (matching how the user
/// supplies it to `enable_compound_optimizer`), not an absolute token
/// amount, so it scales with position size automatically rather than one
/// user's estimate going stale as they add/remove stake.
///
/// Returns a floor of `1` ledger rather than `0` â€” an interval of zero would
/// mean "compound every ledger," which is never actually optimal once any
/// tx cost is nonzero, and would make `trigger_optimized_compound`'s
/// elapsed-check trivially always-true.
fn compute_optimal_interval(
    tx_cost_bps: u32,
    reward_rate_bps: u32,
    position_amount: i128,
) -> u32 {
    if reward_rate_bps == 0 || position_amount <= 0 {
        return 1;
    }

    // tx_cost and rate*position are both expressed in the same bps-of-position
    // unit system, so they cancel cleanly: interval ~ sqrt(2 * tx_cost_bps / rate_bps).
    // Scaled by STELLAR_LEDGERS_PER_YEAR so the result lands in ledger units
    // rather than a fraction-of-a-year float this no_std contract can't hold.
    const LEDGERS_PER_YEAR: u128 = 6_307_200;

    let numerator = 2u128
        .saturating_mul(tx_cost_bps as u128)
        .saturating_mul(LEDGERS_PER_YEAR)
        .saturating_mul(LEDGERS_PER_YEAR);
    let denominator = (reward_rate_bps as u128).max(1);

    let squared_interval = numerator / denominator;
    let interval = isqrt(squared_interval) / LEDGERS_PER_YEAR.max(1);

    (interval as u32).max(1)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Opt in to the compound optimizer with an estimated tx cost, in basis
    /// points of position size.
    pub fn enable_compound_optimizer(
        env: Env,
        user: Address,
        tx_cost_bps: u32,
    ) -> Result<(), VaultError> {
        user.require_auth();

        let position_amount = balance::get_shares(&env, &user);
        let reward_rate_bps = balance::get_reward_rate_bps(&env);
        let optimal_interval_ledgers =
            compute_optimal_interval(tx_cost_bps, reward_rate_bps, position_amount);

        set_config(
            &env,
            &user,
            &OptimizerConfig {
                enabled: true,
                tx_cost_bps,
                last_optimized_at: env.ledger().sequence(),
                optimal_interval_ledgers,
            },
        );
        Ok(())
    }

    /// Opt out. The stored config is retained (marked disabled) rather than
    /// removed, so `get_config` can still report the user's last-known
    /// interval instead of `None` looking identical to "never opted in."
    pub fn disable_compound_optimizer(env: Env, user: Address) -> Result<(), VaultError> {
        user.require_auth();

        let mut config =
            crate::compound_optimizer::get_config(&env, &user).ok_or(VaultError::NotInitialized)?;
        config.enabled = false;
        crate::compound_optimizer::set_config(&env, &user, &config);
        Ok(())
    }

    /// Recomputes `optimal_interval_ledgers` from the user's current
    /// position size and the pool's current reward rate. Public so it can be
    /// called standalone, or ahead of `trigger_optimized_compound`, without
    /// requiring the user's own auth (recalculation only reads state).
    pub fn recalculate_optimal_interval(env: Env, user: Address) -> Result<u32, VaultError> {
        let mut config =
            crate::compound_optimizer::get_config(&env, &user).ok_or(VaultError::NotInitialized)?;

        let position_amount = balance::get_shares(&env, &user);
        let reward_rate_bps = balance::get_reward_rate_bps(&env);
        config.optimal_interval_ledgers =
            compute_optimal_interval(config.tx_cost_bps, reward_rate_bps, position_amount);

        crate::compound_optimizer::set_config(&env, &user, &config);
        Ok(config.optimal_interval_ledgers)
    }

    /// A keeper triggers a compound for `user` if their optimal interval has
    /// elapsed since it was last triggered. Recalculates the interval (to
    /// account for position-size changes) before checking it, per the
    /// issue's note that the interval is refreshed on every trigger.
    ///
    /// Reverts with `Unauthorized` unless `keeper` is an active, registered
    /// keeper (see `keeper_registry.rs`); a successful trigger records the
    /// incentive against that keeper's stats.
    ///
    /// Does not actually claim/restake -- see this module's doc comment --
    /// but performs every other step: auth, the elapsed gate, interval
    /// refresh, and the keeper-incentive event.
    pub fn trigger_optimized_compound(
        env: Env,
        keeper: Address,
        user: Address,
    ) -> Result<i128, VaultError> {
        keeper.require_auth();

        if !crate::keeper_registry::is_registered(&env, &keeper) {
            return Err(VaultError::Unauthorized);
        }

        let mut config =
            crate::compound_optimizer::get_config(&env, &user).ok_or(VaultError::NotInitialized)?;
        if !config.enabled {
            return Err(VaultError::NotInitialized);
        }

        let now = env.ledger().sequence();
        let elapsed = now.saturating_sub(config.last_optimized_at);
        if elapsed < config.optimal_interval_ledgers {
            return Err(VaultError::EpochNotFinalized);
        }

        let position_amount = balance::get_shares(&env, &user);
        let reward_rate_bps = balance::get_reward_rate_bps(&env);
        let new_interval =
            compute_optimal_interval(config.tx_cost_bps, reward_rate_bps, position_amount);

        let claimed_amount = balance::get_accrued_reward(&env, &user);
        let keeper_incentive = claimed_amount
            .saturating_mul(KEEPER_INCENTIVE_BPS)
            / BPS_DENOMINATOR;

        config.last_optimized_at = now;
        config.optimal_interval_ledgers = new_interval;
        crate::compound_optimizer::set_config(&env, &user, &config);

        crate::keeper_registry::record_keeper_action(&env, &keeper, keeper_incentive);

        env.events().publish(
            (symbol_short!("opt_trig"), user.clone()),
            (keeper, claimed_amount, new_interval, now),
        );

        Ok(keeper_incentive)
    }

    /// The user's current optimizer config, if they've ever opted in.
    pub fn get_compound_optimizer_config(env: Env, user: Address) -> Option<OptimizerConfig> {
        crate::compound_optimizer::get_config(&env, &user)
    }
}















