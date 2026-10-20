//! Benchmark-linked reward rate.
//!
//! Lets the pool's reward rate track an external benchmark (e.g. a lending
//! market rate reported by another Soroban protocol) plus a configurable
//! spread, instead of only being set manually (`set_reward_rate_bps`) or
//! algorithmically from utilization (`dynamic_reward_rate.rs`). As with
//! that module, only one rate-driving mode should be enabled
//! administratively at a time — nothing here coordinates mutual exclusion
//! with it.
//!
//! # Flow
//! 1. Admin points the pool at a source contract and a spread with
//!    `set_rate_source`, and a `[floor_bps, ceiling_bps]` band with
//!    `set_rate_bounds` (defaults to `[0, DEFAULT_CEILING_BPS]` until set).
//! 2. Anyone — typically a keeper bot on a timer — calls `refresh_rate()`.
//!    It reads the source's current rate, adds the spread, clamps to the
//!    band, and (if the result differs from the current rate) writes it
//!    via `balance::set_reward_rate_bps` and emits `events::rate_changed`.
//! 3. Calls are throttled by an admin-configurable minimum ledger interval
//!    (`set_refresh_interval`) so a busy keeper can't force redundant
//!    updates.
//! 4. If the source call fails or doesn't return (wrong/unreachable
//!    contract), the pool's rate is left exactly as it was — `refresh_rate`
//!    still succeeds and returns the unchanged rate — and a
//!    `benchmark_source_failed` event is emitted so off-chain monitoring
//!    notices a stale or broken source.
//!
//! # Source interface (provisional)
//! `RateSourceInterface::get_rate` — a single `fn get_rate(env: Env) ->
//! i128` returning the benchmark's current annualized rate in basis
//! points. This is the simplest shape that fits every bps-rate-reporting
//! contract uniformly, but it hasn't been confirmed against a real
//! Soroban lending-market interface; flagging for maintainer input before
//! pointing this at a live source.
//!
//! # Storage
//! `DataKey`/`VaultExtError` both sit at Soroban's 50-variant cap, so this
//! uses raw `Symbol`-keyed instance storage and its own error type,
//! matching `ttl_management.rs`/`user_ttl.rs`.

use soroban_sdk::{contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::events;
use crate::vault::{VaultContract, VaultContractClient};

/// Client for the external rate source. Only the generated client is ever
/// called; the trait itself just declares its shape. See the module-level
/// "Source interface (provisional)" note.
#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RateSourceClient")]
pub trait RateSourceInterface {
    /// The source's current annualized rate, in basis points.
    fn get_rate(env: Env) -> i128;
}

const SOURCE_KEY: Symbol = symbol_short!("bmk_src");
const BOUNDS_KEY: Symbol = symbol_short!("bmk_bnd");
const INTERVAL_KEY: Symbol = symbol_short!("bmk_int");
const LAST_REFRESH_KEY: Symbol = symbol_short!("bmk_lrf");

/// Ceiling used until the admin configures one: 50 000 bps (500% APR),
/// matching the conventional cap used elsewhere in this crate.
pub const DEFAULT_CEILING_BPS: u32 = 50_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateSourceConfig {
    pub source: Address,
    /// Added to the benchmark's reported rate before clamping. May be
    /// negative (a discount to the benchmark).
    pub spread_bps: i128,
}

/// Errors for the benchmark-rate entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum BenchmarkRateError {
    NotInitialized = 1,
    Unauthorized = 2,
    /// `refresh_rate` called with no source configured yet.
    NoSourceConfigured = 3,
    /// `floor_bps > ceiling_bps`.
    InvalidBounds = 4,
    /// Called before `refresh_interval_ledgers` have elapsed since the
    /// last refresh.
    RefreshTooSoon = 5,
}

fn get_source_config(env: &Env) -> Option<RateSourceConfig> {
    env.storage().instance().get(&SOURCE_KEY)
}

fn set_source_config(env: &Env, config: &RateSourceConfig) {
    env.storage().instance().set(&SOURCE_KEY, config);
}

/// The configured `[floor_bps, ceiling_bps]` band, defaulting to
/// `[0, DEFAULT_CEILING_BPS]` until the admin sets one.
pub fn rate_bounds(env: &Env) -> (u32, u32) {
    env.storage()
        .instance()
        .get(&BOUNDS_KEY)
        .unwrap_or((0, DEFAULT_CEILING_BPS))
}

fn set_rate_bounds_storage(env: &Env, floor_bps: u32, ceiling_bps: u32) {
    env.storage()
        .instance()
        .set(&BOUNDS_KEY, &(floor_bps, ceiling_bps));
}

/// The configured minimum number of ledgers required between
/// `refresh_rate` calls. `0` means no throttle.
pub fn refresh_interval_ledgers(env: &Env) -> u32 {
    env.storage().instance().get(&INTERVAL_KEY).unwrap_or(0)
}

/// The ledger of the last `refresh_rate` call (successful or a handled
/// source failure), `0` if it has never been called.
pub fn last_refresh_ledger(env: &Env) -> u32 {
    env.storage().instance().get(&LAST_REFRESH_KEY).unwrap_or(0)
}

/// Adds `spread_bps` to `benchmark_bps` and clamps to `[floor_bps,
/// ceiling_bps]`. Saturates rather than overflowing for an extreme
/// benchmark/spread, and never returns a negative rate.
pub fn compute_target_rate(
    benchmark_bps: i128,
    spread_bps: i128,
    floor_bps: u32,
    ceiling_bps: u32,
) -> u32 {
    let raw = benchmark_bps.saturating_add(spread_bps);
    raw.clamp(floor_bps as i128, ceiling_bps as i128).max(0) as u32
}

#[contractimpl]
impl VaultContract {
    /// Admin: point the pool's reward rate at `source` (an external
    /// contract implementing `RateSourceInterface`) with the given
    /// `spread_bps` added to whatever it reports. Takes effect on the next
    /// `refresh_rate()` call — this doesn't itself refresh the rate.
    pub fn set_rate_source(
        env: Env,
        admin_addr: Address,
        source: Address,
        spread_bps: i128,
    ) -> Result<(), BenchmarkRateError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env).map_err(|_| BenchmarkRateError::NotInitialized)? {
            return Err(BenchmarkRateError::Unauthorized);
        }
        set_source_config(&env, &RateSourceConfig { source: source.clone(), spread_bps });
        env.events().publish(
            (symbol_short!("bmk_src"),),
            (source, spread_bps, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Admin: set the `[floor_bps, ceiling_bps]` band `refresh_rate()`
    /// clamps the benchmark-derived rate to.
    pub fn set_rate_bounds(
        env: Env,
        admin_addr: Address,
        floor_bps: u32,
        ceiling_bps: u32,
    ) -> Result<(), BenchmarkRateError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env).map_err(|_| BenchmarkRateError::NotInitialized)? {
            return Err(BenchmarkRateError::Unauthorized);
        }
        if ceiling_bps < floor_bps {
            return Err(BenchmarkRateError::InvalidBounds);
        }
        set_rate_bounds_storage(&env, floor_bps, ceiling_bps);
        env.events().publish(
            (symbol_short!("bmk_bnd"),),
            (floor_bps, ceiling_bps, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Admin: set the minimum number of ledgers required between
    /// `refresh_rate()` calls. `0` disables the throttle.
    pub fn set_refresh_interval(
        env: Env,
        admin_addr: Address,
        min_ledgers: u32,
    ) -> Result<(), BenchmarkRateError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env).map_err(|_| BenchmarkRateError::NotInitialized)? {
            return Err(BenchmarkRateError::Unauthorized);
        }
        env.storage().instance().set(&INTERVAL_KEY, &min_ledgers);
        Ok(())
    }

    /// Permissionless, keeper-callable: re-derive the reward rate from the
    /// configured source's current reading plus the configured spread,
    /// clamped to the configured bounds, and apply it if it changed.
    ///
    /// Rate-limited by `set_refresh_interval`. If the source call fails
    /// (unreachable contract, wrong interface, or any other error), the
    /// pool's rate is left exactly as it was: this still returns `Ok` with
    /// the unchanged rate, and emits `benchmark_source_failed` instead of
    /// `rate_changed`.
    pub fn refresh_rate(env: Env) -> Result<u32, BenchmarkRateError> {
        let config = get_source_config(&env).ok_or(BenchmarkRateError::NoSourceConfigured)?;

        let now = env.ledger().sequence();
        let interval = refresh_interval_ledgers(&env);
        if interval > 0 {
            let last = last_refresh_ledger(&env);
            if last > 0 && now.saturating_sub(last) < interval {
                return Err(BenchmarkRateError::RefreshTooSoon);
            }
        }

        let current_rate = balance::get_reward_rate_bps(&env);
        env.storage().instance().set(&LAST_REFRESH_KEY, &now);

        let client = RateSourceClient::new(&env, &config.source);
        let benchmark_bps: i128 = match client.try_get_rate() {
            Ok(Ok(value)) => value,
            _ => {
                env.events().publish(
                    (symbol_short!("bmk_fail"),),
                    (config.source, current_rate, now),
                );
                return Ok(current_rate);
            }
        };

        let (floor_bps, ceiling_bps) = rate_bounds(&env);
        let new_rate = compute_target_rate(benchmark_bps, config.spread_bps, floor_bps, ceiling_bps);

        if new_rate != current_rate {
            balance::set_reward_rate_bps(&env, new_rate);
            events::rate_changed(&env, current_rate, new_rate);
        }
        env.events().publish(
            (symbol_short!("bmk_sync"),),
            (benchmark_bps, new_rate, now),
        );

        Ok(new_rate)
    }

    /// Read-only: the configured rate source and spread, if any.
    pub fn get_rate_source(env: Env) -> Option<RateSourceConfig> {
        get_source_config(&env)
    }

    /// Read-only: the configured `[floor_bps, ceiling_bps]` band.
    pub fn get_rate_bounds(env: Env) -> (u32, u32) {
        rate_bounds(&env)
    }

    /// Read-only: the configured minimum number of ledgers between
    /// `refresh_rate()` calls. `0` means no throttle.
    pub fn get_refresh_interval(env: Env) -> u32 {
        refresh_interval_ledgers(&env)
    }

    /// Read-only: the ledger of the last `refresh_rate()` call, `0` if it
    /// has never been called.
    pub fn get_last_refresh_ledger(env: Env) -> u32 {
        last_refresh_ledger(&env)
    }
}
