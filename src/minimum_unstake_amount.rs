//! Admin-configurable minimum unstake amount (issue #441).
//!
//! Prevents tiny dust withdrawals that waste ledger space and create
//! accounting noise. Users must unstake at least the configured minimum per
//! transaction, or unstake their full remaining balance — a full position
//! exit is always allowed so users can never be trapped.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{symbol_short, Env, Symbol};

use crate::errors::VaultError;

const MIN_UNSTAKE_KEY: Symbol = symbol_short!("mn_unstk");

/// Enforce the minimum-unstake rule for an unstake of `amount` against the
/// caller's full position. Full position exit is always allowed.
///
/// Returns `Ok(())` when the amount is above the configured minimum, is a
/// full position exit, or the minimum is disabled (0).
pub fn enforce_min_unstake(
    env: &Env,
    amount: i128,
    position_amount: i128,
) -> Result<(), VaultError> {
    let min = get_min_unstake_amount(env);
    if min > 0 && amount < min && amount != position_amount {
        return Err(VaultError::BelowMinimumStake);
    }
    Ok(())
}

/// Read the configured minimum unstake amount (0 = disabled).
pub fn get_min_unstake_amount(env: &Env) -> i128 {
    env.storage().instance().get(&MIN_UNSTAKE_KEY).unwrap_or(0)
}

pub fn set_min_unstake_amount(env: &Env, amount: i128) {
    env.storage().instance().set(&MIN_UNSTAKE_KEY, &amount);
}
