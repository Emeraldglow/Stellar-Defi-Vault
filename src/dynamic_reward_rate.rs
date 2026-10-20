//! Issue #510: algorithmic reward rate driven by pool utilization.
//!
//! Instead of a static admin-set `reward_rate_bps`, the rate can follow a
//! kinked curve over utilization (`total_deposited / target_tvl`), in the
//! spirit of lending-protocol interest-rate models. For a staking pool the
//! curve slopes *down*: an empty pool pays the most to attract deposits, and
//! the rate eases off as the pool fills past its target.
//!
//! ```text
//! rate (bps)
//!   max ┤●
//!       │  ╲            slope 1: max → target over 0‒100% utilization
//! target┤     ●
//!       │       ╲       slope 2: target → min over 100‒200% utilization
//!   min ┤         ●━━━━ flat beyond 200%
//!       └──┬──────┬──────┬──
//!          0%    100%   200%   utilization
//! ```
//!
//! While enabled, the rate is recomputed after every TVL change (stake,
//! deposit, batch deposit, unstake/withdraw, `add_yield`) and can also be
//! refreshed by anyone via `sync_dynamic_rate`. Manual `set_reward_rate_bps`
//! is rejected with `DynamicRateActive` so it can't be silently overwritten.
//! Disabling the mode leaves the last computed rate in place.
//!
//! The runway guard is not applied to algorithmic updates: they run inside
//! user deposits and withdrawals, which must never revert on it. Admins should
//! size `max_rate_bps` against the funded reward pool.
//!
//! Configuring the curve is gated on the `RateSetter` role (the admin holds
//! every role — see `access_roles`).
//!
//! # Storage
//!
//! `symbol_short!("dyn_rate")` -> `DynamicRateConfig` (instance)

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::access_roles::{self, Role};
use crate::balance;
use crate::errors::VaultAccessError;
use crate::events;
use crate::vault::{VaultContract, VaultContractClient, BOOST_BPS_BASE};

const CONFIG_KEY: Symbol = symbol_short!("dyn_rate");

/// Utilization, in bps, at which the rate reaches `min_rate_bps`.
const FLOOR_UTILIZATION_BPS: i128 = 2 * BOOST_BPS_BASE as i128;

/// Parameters of the utilization-driven reward-rate curve.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynamicRateConfig {
    /// Whether the algorithmic rate is active.
    pub enabled: bool,
    /// TVL (in stake-token units) that counts as 100% utilization.
    pub target_tvl: i128,
    /// Rate paid by an empty pool.
    pub max_rate_bps: u32,
    /// Rate paid at exactly `target_tvl`.
    pub target_rate_bps: u32,
    /// Floor reached at 2x `target_tvl` and held beyond.
    pub min_rate_bps: u32,
}

pub fn read_config(env: &Env) -> Option<DynamicRateConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

/// Whether the algorithmic rate is currently driving `reward_rate_bps`.
pub fn is_enabled(env: &Env) -> bool {
    read_config(env).map(|c| c.enabled).unwrap_or(false)
}

/// Pool utilization in bps (`10_000` = exactly at target). Saturates rather
/// than overflowing for extreme TVLs.
pub fn utilization_bps(total_deposited: i128, target_tvl: i128) -> i128 {
    if target_tvl <= 0 || total_deposited <= 0 {
        return 0;
    }
    total_deposited
        .checked_mul(BOOST_BPS_BASE as i128)
        .map(|v| v / target_tvl)
        .unwrap_or(i128::MAX)
}

/// Evaluates the curve at `total_deposited`.
pub fn compute_rate_bps(config: &DynamicRateConfig, total_deposited: i128) -> u32 {
    let base = BOOST_BPS_BASE as i128;
    let util = utilization_bps(total_deposited, config.target_tvl);
    let max = config.max_rate_bps as i128;
    let target = config.target_rate_bps as i128;
    let min = config.min_rate_bps as i128;

    let rate = if util <= base {
        // Slope 1: max → target.
        max - (max - target) * util / base
    } else {
        // Slope 2: target → min, clamped at 2x target.
        let excess = (util.min(FLOOR_UTILIZATION_BPS)) - base;
        target - (target - min) * excess / base
    };
    rate.clamp(min, max) as u32
}

/// Recomputes and stores the reward rate if the algorithmic mode is enabled.
/// A no-op otherwise, or when the rate is unchanged. Returns the rate in
/// effect afterwards.
pub(crate) fn sync(env: &Env) -> u32 {
    let current = balance::get_reward_rate_bps(env);
    let config = match read_config(env) {
        Some(c) if c.enabled => c,
        _ => return current,
    };
    let new_rate = compute_rate_bps(&config, balance::get_total_deposited(env));
    if new_rate != current {
        balance::set_reward_rate_bps(env, new_rate);
        events::rate_changed(env, current, new_rate);
    }
    new_rate
}

fn validate(config: &DynamicRateConfig) -> Result<(), VaultAccessError> {
    if config.target_tvl <= 0
        || config.min_rate_bps > config.target_rate_bps
        || config.target_rate_bps > config.max_rate_bps
    {
        return Err(VaultAccessError::InvalidDynamicRateConfig);
    }
    if config.max_rate_bps > balance::MAX_RATE_BPS {
        return Err(VaultAccessError::RateTooHigh);
    }
    Ok(())
}

#[contractimpl]
impl VaultContract {
    /// RateSetter: configure (and enable/disable) the utilization-driven
    /// reward rate. Requires `min <= target <= max <= MAX_RATE_BPS` and a
    /// positive `target_tvl`. When enabled, the new rate applies immediately.
    pub fn set_dynamic_rate_config(
        env: Env,
        caller: Address,
        config: DynamicRateConfig,
    ) -> Result<(), VaultAccessError> {
        access_roles::require_role(&env, Role::RateSetter, &caller)?;
        validate(&config)?;
        env.storage().instance().set(&CONFIG_KEY, &config);
        env.events().publish(
            (symbol_short!("dyn_cfg"), caller),
            (
                config.enabled,
                config.target_tvl,
                config.max_rate_bps,
                config.target_rate_bps,
                config.min_rate_bps,
            ),
        );
        sync(&env);
        Ok(())
    }

    /// Read-only: the configured curve, if any.
    pub fn get_dynamic_rate_config(env: Env) -> Option<DynamicRateConfig> {
        read_config(&env)
    }

    /// Read-only: current pool utilization in bps against `target_tvl`
    /// (`0` when no curve is configured).
    pub fn get_pool_utilization_bps(env: Env) -> i128 {
        read_config(&env)
            .map(|c| utilization_bps(balance::get_total_deposited(&env), c.target_tvl))
            .unwrap_or(0)
    }

    /// Read-only: the rate the curve yields at the current TVL, whether or
    /// not the mode is enabled (`None` when no curve is configured).
    pub fn preview_dynamic_rate(env: Env) -> Option<u32> {
        read_config(&env).map(|c| compute_rate_bps(&c, balance::get_total_deposited(&env)))
    }

    /// Permissionless: re-derive the reward rate from current utilization.
    /// Returns the rate in effect afterwards.
    pub fn sync_dynamic_rate(env: Env) -> u32 {
        sync(&env)
    }
}
