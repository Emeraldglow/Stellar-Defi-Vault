// Issue #536 — configurable inactivity-based reward decay
//
// Add an optional mechanic where a position's reward rate gradually decays
// if the user hasn't interacted with the contract (deposited, claimed) in a
// long time, encouraging active engagement over passive forgotten deposits.

use crate::admin;
use crate::errors::VaultError;
use soroban_sdk::{symbol_short, Address, Env};

/// Admin-only: enable/configure inactivity decay.
/// `threshold_ledgers` — number of ledgers of inactivity before decay begins.
/// `decay_bps_per_period` — decay applied per period (in basis points, e.g. 500 = 5%).
pub fn set_config(
    env: &Env,
    admin_addr: &Address,
    threshold_ledgers: u32,
    decay_bps_per_period: u32,
) -> Result<(), VaultError> {
    admin_addr.require_auth();
    admin::require_admin(env)?;

    env.storage()
        .instance()
        .set(&symbol_short!("ina_th"), &threshold_ledgers);
    env.storage()
        .instance()
        .set(&symbol_short!("ina_dc"), &decay_bps_per_period);

    env.events().publish(
        (symbol_short!("ina_cfg"), admin_addr),
        (threshold_ledgers, decay_bps_per_period, env.ledger().sequence()),
    );

    Ok(())
}

/// Read-only: get the inactivity threshold in ledgers.
pub fn get_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("ina_th"))
        .unwrap_or(0) // disabled by default
}

/// Read-only: get the decay rate in bps per period.
pub fn get_decay_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("ina_dc"))
        .unwrap_or(0)
}

/// Read-only: get the last interaction ledger for a user.
pub fn get_last_interaction(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&(symbol_short!("ina_li"), user.clone()))
        .unwrap_or(0)
}

/// Record a user interaction (deposit, claim, etc.) — resets the inactivity clock.
pub fn record_interaction(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .set(&(symbol_short!("ina_li"), user.clone()), &env.ledger().sequence());
}

/// Calculate the decay multiplier for a user based on inactivity.
/// Returns a multiplier in bps (e.g. 10000 = 1x, 9500 = 0.95x).
/// Returns 10000 if decay is disabled or user is active.
pub fn get_decay_multiplier_bps(env: &Env, user: &Address) -> u32 {
    let threshold = get_threshold(env);
    if threshold == 0 {
        return 10_000; // decay disabled
    }

    let last_interaction = get_last_interaction(env, user);
    if last_interaction == 0 {
        return 10_000; // no interaction recorded, treat as active
    }

    let current = env.ledger().sequence();
    let inactive_ledgers = current.saturating_sub(last_interaction);

    if inactive_ledgers <= threshold {
        return 10_000; // within threshold, no decay
    }

    let decay_bps = get_decay_bps(env);
    if decay_bps == 0 {
        return 10_000;
    }

    let periods_inactive = (inactive_ledgers - threshold) / threshold; // periods of inactivity
    let decay_factor = 10_000u32.saturating_sub(decay_bps.saturating_mul(periods_inactive));

    // Floor at 1000 (10%) to prevent complete reward elimination
    decay_factor.max(1_000)
}
