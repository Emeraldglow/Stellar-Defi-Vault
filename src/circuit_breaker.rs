//! Single-transaction withdrawal circuit breaker (issue acceptance criteria).
//!
//! If any single withdrawal exceeds a configured fraction of total pool value
//! (expressed in basis points), the breaker automatically pauses withdrawals
//! and reverts the triggering transaction — guarding against exploit-driven
//! mass drains.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap; a raw `Symbol`-keyed instance
//! entry (`cb_bps`) stores the threshold.
//!
//! # Behavior
//!
//! When the threshold is 0 (default), the circuit breaker is disabled and
//! every `check` call is a no-op. Non-zero values are checked on every call
//! to `do_unstake` in `vault.rs`. A triggered breaker sets `DataKey::Paused`
//! to `true` before returning `Err(VaultError::VaultPaused)`, so all
//! subsequent withdrawal attempts also fail until an admin calls `unpause`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env};

use crate::{
    admin,
    balance,
    errors::VaultError,
    storage::DataKey,
    VaultContract,
};

const CB_BPS_KEY: soroban_sdk::Symbol = symbol_short!("cb_bps");

/// Read the configured circuit-breaker threshold in basis points (0 = disabled).
pub fn get_threshold_bps(env: &Env) -> u32 {
    env.storage().instance().get(&CB_BPS_KEY).unwrap_or(0)
}

fn store_threshold_bps(env: &Env, bps: u32) {
    env.storage().instance().set(&CB_BPS_KEY, &bps);
}

/// Check whether `amount` would trigger the circuit breaker.
///
/// If the threshold is set and `amount > total_deposited * threshold_bps /
/// 10_000`, this function pauses the vault (sets `DataKey::Paused = true`)
/// and returns `Err(VaultError::VaultPaused)`, reverting the caller.
/// Returns `Ok(())` when disabled (threshold = 0) or the amount is within
/// the allowed fraction.
pub fn check(env: &Env, amount: i128) -> Result<(), VaultError> {
    let bps = get_threshold_bps(env);
    if bps == 0 {
        return Ok(());
    }
    let total_deposited = balance::get_total_deposited(env);
    if total_deposited <= 0 {
        return Ok(());
    }
    // threshold_amount = total_deposited * bps / 10_000
    let threshold = total_deposited
        .checked_mul(bps as i128)
        .and_then(|v| v.checked_div(10_000))
        .unwrap_or(i128::MAX);
    if amount > threshold {
        // Auto-pause: set the vault-wide pause flag, then revert the trigger tx.
        env.storage().instance().set(&DataKey::Paused, &true);
        return Err(VaultError::VaultPaused);
    }
    Ok(())
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Admin: configure the single-transaction withdrawal circuit-breaker
    /// threshold. `bps` is the maximum allowed fraction of total pool value
    /// per single withdrawal (e.g. 2000 = 20%). Pass `0` to disable.
    ///
    /// Reverts with `Unauthorized` when `admin` is not the stored admin, and
    /// with `InvalidRate` when `bps` exceeds 10 000 (100%).
    pub fn set_circuit_breaker_threshold_bps(
        env: Env,
        admin: Address,
        bps: u32,
    ) -> Result<(), VaultError> {
        admin.require_auth();
        if admin != crate::admin::get_admin(&env)? {
            return Err(VaultError::Unauthorized);
        }
        if bps > 10_000 {
            return Err(VaultError::InvalidRate);
        }
        store_threshold_bps(&env, bps);
        env.events().publish(
            (symbol_short!("cb_set"),),
            bps,
        );
        Ok(())
    }

    /// Read-only: the configured circuit-breaker threshold in basis points.
    /// Returns 0 when the breaker is disabled.
    pub fn get_circuit_breaker_threshold_bps(env: Env) -> u32 {
        get_threshold_bps(&env)
    }
}
