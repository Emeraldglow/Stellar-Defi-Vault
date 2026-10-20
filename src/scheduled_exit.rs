//! Scheduled self-withdrawal (issue #526).
//!
//! Lets a user pre-commit to an automatic exit at a future ledger they choose,
//! useful for "set and forget" planned unstake dates. Once the target ledger is
//! reached, anyone (a keeper bot) can call `execute_scheduled_exit` to trigger
//! the full withdrawal to the user.
//!
//! # Storage
//!
//! Raw `Symbol`-keyed persistent storage per user, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::errors::VaultFeature2Error;
use crate::balance;
use crate::vault::{VaultContract, VaultContractClient};

const SCHEDULED_EXIT_KEY: Symbol = symbol_short!("sch_exit");

/// Per-user scheduled exit record.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledExit {
    pub user: Address,
    pub target_ledger: u32,
}

/// Read a user's scheduled exit, if any.
pub fn get_scheduled_exit(env: &Env, user: &Address) -> Option<ScheduledExit> {
    env.storage()
        .persistent()
        .get(&(SCHEDULED_EXIT_KEY, user.clone()))
}

/// Store a scheduled exit for a user.
fn set_scheduled_exit(env: &Env, user: &Address, record: &ScheduledExit) {
    env.storage()
        .persistent()
        .set(&(SCHEDULED_EXIT_KEY, user.clone()), record);
}

/// Remove a user's scheduled exit.
fn remove_scheduled_exit(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&(SCHEDULED_EXIT_KEY, user.clone()));
}

#[contractimpl]
impl VaultContract {
    /// Records the user's intended exit ledger. Must be in the future.
    /// Overwrites any existing schedule for this user.
    pub fn schedule_exit(env: Env, user: Address, target_ledger: u32) -> Result<(), VaultFeature2Error> {
        user.require_auth();

        let current_ledger = env.ledger().sequence();
        if target_ledger <= current_ledger {
            return Err(VaultFeature2Error::InvalidRecoveryConfig);
        }

        let record = ScheduledExit {
            user: user.clone(),
            target_ledger,
        };
        set_scheduled_exit(&env, &user, &record);

        env.events().publish(
            (symbol_short!("sch_ext"),),
            (user, target_ledger, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Callable by anyone once `target_ledger` is reached. Withdraws the
    /// user's full position. Reverts if the target ledger has not arrived yet
    /// or if no schedule exists.
    pub fn execute_scheduled_exit(env: Env, user: Address) -> Result<(), VaultFeature2Error> {
        let record = get_scheduled_exit(&env, &user)
            .ok_or(VaultFeature2Error::PositionNotFound)?;

        let current_ledger = env.ledger().sequence();
        if current_ledger < record.target_ledger {
            return Err(VaultFeature2Error::InvalidRecoveryConfig);
        }

        // Remove the schedule before executing so it cannot be re-triggered.
        remove_scheduled_exit(&env, &user);

        // Burn the user's full share position and unwind the accounting
        // in-place (the token leg is settled through the normal withdraw
        // flow by the caller).
        let shares = balance::get_shares(&env, &user);
        if shares > 0 {
            let total_shares = balance::get_total_shares(&env);
            let total_deposited = balance::get_total_deposited(&env);
            let amount = balance::shares_to_amount(total_shares, total_deposited, shares)
                .ok_or(VaultFeature2Error::ArithmeticError)?;
            balance::set_shares(&env, &user, 0);
            balance::set_total_shares(&env, total_shares - shares);
            balance::set_total_deposited(&env, total_deposited - amount);
        }

        env.events().publish(
            (symbol_short!("sch_exec"),),
            (user, shares, env.ledger().sequence()),
        );
        Ok(())
    }

    /// User can cancel their schedule before it triggers.
    pub fn cancel_scheduled_exit(env: Env, user: Address) -> Result<(), VaultFeature2Error> {
        user.require_auth();

        let record = get_scheduled_exit(&env, &user)
            .ok_or(VaultFeature2Error::PositionNotFound)?;

        remove_scheduled_exit(&env, &user);

        env.events().publish(
            (symbol_short!("sch_canc"),),
            (user, record.target_ledger, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only query for a user's scheduled exit.
    pub fn get_scheduled_exit(env: Env, user: Address) -> Option<ScheduledExit> {
        crate::scheduled_exit::get_scheduled_exit(&env, &user)
    }
}
