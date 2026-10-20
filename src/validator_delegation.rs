//! Validator set delegation (issue #332).
//!
//! Lets a staker assign their staking weight to a Stellar validator node
//! address. The pool tracks total weight delegated to each validator,
//! enabling stake-weighted validator selection / on-chain proof of support.
//! Purely informational â€” delegation does not affect reward rate or staking
//! mechanics.
//!
//! # Storage
//!
//! Raw `Symbol`-keyed instance and persistent storage, matching
//! `balance.rs`, since `DataKey` is at Soroban's 50-variant cap.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol, Vec};

use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Most distinct validators `get_validator_weights` tracks at once.
pub const MAX_VALIDATORS: u32 = 20;

const DELEGATION_KEY: Symbol = symbol_short!("vd_dele");
const WEIGHTS_KEY: Symbol = symbol_short!("vd_wght");

fn get_delegation(env: &Env, user: &Address) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&(DELEGATION_KEY, user.clone()))
}

fn set_delegation(env: &Env, user: &Address, validator: &Address) {
    env.storage()
        .persistent()
        .set(&(DELEGATION_KEY, user.clone()), validator);
}

fn clear_delegation(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&(DELEGATION_KEY, user.clone()));
}

fn get_weights(env: &Env) -> Vec<(Address, i128)> {
    env.storage()
        .instance()
        .get(&WEIGHTS_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_weights(env: &Env, weights: &Vec<(Address, i128)>) {
    env.storage().instance().set(&WEIGHTS_KEY, weights);
}

/// Adds `delta` (which may be negative) to `validator`'s tracked weight,
/// creating a new entry if none exists yet. Removes the entry once its
/// weight settles back to zero.
fn adjust_weight(env: &Env, validator: &Address, delta: i128) -> Result<(), VaultError> {
    let mut weights = get_weights(env);
    let mut found = false;
    let mut new_weights: Vec<(Address, i128)> = Vec::new(env);

    for (addr, weight) in weights.iter() {
        if &addr == validator {
            found = true;
            let updated = weight.checked_add(delta).ok_or(VaultError::ArithmeticError)?;
            if updated > 0 {
                new_weights.push_back((addr, updated));
            }
        } else {
            new_weights.push_back((addr, weight));
        }
    }

    if !found {
        if new_weights.len() >= MAX_VALIDATORS {
            return Err(VaultError::BatchTooLarge);
        }
        if delta > 0 {
            new_weights.push_back((validator.clone(), delta));
        }
    }

    weights = new_weights;
    set_weights(env, &weights);
    Ok(())
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Assign the caller's current stake weight to `validator`. Replaces any
    /// existing delegation (the old validator's weight is removed first).
    pub fn delegate_to_validator(env: Env, user: Address, validator: Address) -> Result<(), VaultError> {
        user.require_auth();

        let position = balance::shares_to_amount(
            balance::get_total_shares(&env),
            balance::get_total_deposited(&env),
            balance::get_shares(&env, &user),
        )
        .unwrap_or(0);
        if position <= 0 {
            return Err(VaultError::PositionNotFound);
        }

        if let Some(old_validator) = get_delegation(&env, &user) {
            adjust_weight(&env, &old_validator, -position)?;
        }

        adjust_weight(&env, &validator, position)?;
        set_delegation(&env, &user, &validator);

        env.events().publish(
            (symbol_short!("vd_set"), user),
            (validator, position, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Remove the caller's validator delegation, zeroing their contribution
    /// to that validator's tracked weight.
    pub fn revoke_validator_delegation(env: Env, user: Address) -> Result<(), VaultError> {
        user.require_auth();

        let validator = get_delegation(&env, &user).ok_or(VaultError::PositionNotFound)?;
        let position = balance::shares_to_amount(
            balance::get_total_shares(&env),
            balance::get_total_deposited(&env),
            balance::get_shares(&env, &user),
        )
        .unwrap_or(0);

        adjust_weight(&env, &validator, -position)?;
        clear_delegation(&env, &user);

        env.events().publish(
            (symbol_short!("vd_rvk"), user),
            (validator, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only query: the validator `user` currently delegates to, if any.
    pub fn get_validator_delegation(env: Env, user: Address) -> Option<Address> {
        get_delegation(&env, &user)
    }

    /// Read-only query: total tracked weight per validator, descending by
    /// weight.
    pub fn get_validator_weights(env: Env) -> Vec<(Address, i128)> {
        let mut weights = get_weights(&env);
        // Simple descending insertion sort â€” bounded by MAX_VALIDATORS (20).
        let len = weights.len();
        for i in 1..len {
            let key = weights.get(i).unwrap();
            let mut j = i;
            while j > 0 && weights.get(j - 1).unwrap().1 < key.1 {
                let prev = weights.get(j - 1).unwrap();
                weights.set(j, prev);
                j -= 1;
            }
            weights.set(j, key);
        }
        weights
    }
}















