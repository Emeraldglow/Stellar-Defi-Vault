//! Guardian role with pause-only emergency power (issue #521).
//!
//! A guardian address, set by the admin, can trigger a pause unilaterally in
//! an emergency without holding any other admin capability — narrowing the
//! blast radius if that specific key is ever compromised, versus handing out
//! full admin for the same purpose. The guardian can only pause; resuming
//! (`unpause`, already admin-only in `vault.rs`) is deliberately left out of
//! its power.
//!
//! Every error case this needs already exists on `VaultError`
//! (`Unauthorized`, `NotInitialized`, `ContractStopped`), so this returns
//! `VaultError` directly rather than introducing a new error enum.
//!
//! # Wiring
//!
//! `vault.rs`'s own `pause()` is admin-only (`admin::require_admin`) and its
//! stopped-check helper (`require_not_stopped`) is private to that module, so
//! `guardian_pause` re-implements the same effect here: set `DataKey::Paused`
//! and record a `PauseInfo`, exactly as `pause()` does, after checking the
//! caller against the configured guardian and the `DataKey::Stopped` flag
//! directly. This mirrors how `synchronized_pause.rs` sets `DataKey::Paused`
//! from outside `vault.rs`.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so the guardian address is
//! kept under a raw `Symbol`-keyed instance entry, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, String, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::storage::{DataKey, PauseInfo, PauseReason};
use crate::vault::VaultContract;

const GUARDIAN_KEY: Symbol = symbol_short!("guardian");

fn get_guardian(env: &Env) -> Option<Address> {
    env.storage().instance().get(&GUARDIAN_KEY)
}

#[contractimpl]
impl VaultContract {
    /// Set (or replace) the guardian address. Admin only.
    pub fn set_guardian(env: Env, guardian: Address) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        env.storage().instance().set(&GUARDIAN_KEY, &guardian);
        env.events().publish(
            (symbol_short!("grd_set"),),
            (guardian, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Remove the guardian role entirely. Admin only.
    pub fn remove_guardian(env: Env) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        env.storage().instance().remove(&GUARDIAN_KEY);
        env.events()
            .publish((symbol_short!("grd_rm"),), env.ledger().sequence());
        Ok(())
    }

    /// The currently configured guardian, if any.
    pub fn get_guardian(env: Env) -> Option<Address> {
        crate::guardian_pause::get_guardian(&env)
    }

    /// Whether `user` is the currently configured guardian.
    pub fn is_guardian(env: Env, user: Address) -> bool {
        crate::guardian_pause::get_guardian(&env) == Some(user)
    }

    /// Pause the pool as the guardian, without holding any other admin
    /// capability. Reverts with `Unauthorized` if the caller is not the
    /// configured guardian, and with `ContractStopped` if the pool has been
    /// permanently stopped (matching `pause()`'s own behavior).
    pub fn guardian_pause(
        env: Env,
        guardian: Address,
        message: String,
    ) -> Result<(), VaultError> {
        guardian.require_auth();

        let configured = crate::guardian_pause::get_guardian(&env).ok_or(VaultError::NotInitialized)?;
        if configured != guardian {
            return Err(VaultError::Unauthorized);
        }
        if env
            .storage()
            .instance()
            .get(&DataKey::Stopped)
            .unwrap_or(false)
        {
            return Err(VaultError::ContractStopped);
        }
        if message.len() > 200 {
            return Err(VaultError::DescriptionTooLong);
        }

        env.storage().instance().set(&DataKey::Paused, &true);
        let current_ledger = env.ledger().sequence();
        balance::set_pause_info(
            &env,
            &PauseInfo {
                reason: PauseReason::SecurityIncident,
                message: message.clone(),
                paused_at: current_ledger,
            },
        );

        env.events().publish(
            (symbol_short!("grd_paus"), guardian),
            (message, current_ledger),
        );
        Ok(())
    }
}
