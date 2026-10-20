//! Partial position freeze (issue #337).
//!
//! Distinct from a whole-position freeze â€” this locks only a specific token
//! amount within a user's stake, leaving the remainder freely withdrawable.
//! Rewards keep accruing on the full position; only the frozen portion is
//! meant to be blocked from unstaking.
//!
//! # Storage
//!
//! Raw `Symbol`-keyed persistent storage, matching `balance.rs`, since
//! `DataKey` is at Soroban's 50-variant cap.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

const FROZEN_KEY: Symbol = symbol_short!("pf_frzn");

fn get_position_amount(env: &Env, user: &Address) -> Option<i128> {
    let shares = balance::get_shares(env, user);
    if shares == 0 {
        return None;
    }
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    balance::shares_to_amount(total_shares, total_deposited, shares)
}

fn get_frozen(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&(FROZEN_KEY, user.clone()))
        .unwrap_or(0)
}

fn set_frozen(env: &Env, user: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&(FROZEN_KEY, user.clone()), &amount);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Freeze `amount` of `user`'s position. Admin only. The frozen portion
    /// is tracked separately from the unfrozen, freely-available remainder â€”
    /// see `get_available_amount`. Reverts with `WithdrawalLimitExceeded` if
    /// the total frozen amount would exceed the user's current position.
    pub fn partial_freeze(env: Env, user: Address, amount: i128) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let position = get_position_amount(&env, &user).ok_or(VaultError::PositionNotFound)?;
        let current_frozen = get_frozen(&env, &user);
        let new_frozen = current_frozen
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticError)?;

        if new_frozen > position {
            return Err(VaultError::WithdrawalLimitExceeded);
        }

        set_frozen(&env, &user, new_frozen);

        env.events().publish(
            (symbol_short!("pf_frz"), user),
            (new_frozen, position, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Release `amount` of a user's frozen balance. Admin only.
    pub fn partial_unfreeze(env: Env, user: Address, amount: i128) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let current_frozen = get_frozen(&env, &user);
        let new_frozen = current_frozen
            .checked_sub(amount)
            .filter(|v| *v >= 0)
            .ok_or(VaultError::ArithmeticError)?;

        set_frozen(&env, &user, new_frozen);

        env.events().publish(
            (symbol_short!("pf_unfrz"), user),
            (new_frozen, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only query: the amount currently frozen within `user`'s position.
    pub fn get_frozen_amount(env: Env, user: Address) -> i128 {
        get_frozen(&env, &user)
    }

    /// Read-only query: the portion of `user`'s position that is not frozen
    /// and so freely available to unstake. Zero if the user has no position.
    pub fn get_available_amount(env: Env, user: Address) -> i128 {
        let position = get_position_amount(&env, &user).unwrap_or(0);
        let frozen = get_frozen(&env, &user);
        (position - frozen).max(0)
    }
}















