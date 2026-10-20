//! Admin succession plan.
//!
//! If the admin loses access to its key, the pool would otherwise become
//! permanently unmanageable. This module lets the admin designate a fallback
//! "heir" address that can claim admin rights once the current admin has
//! gone silent â€” performed no admin action â€” for a configurable number of
//! ledgers.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so the plan is kept under a
//! raw `Symbol`-keyed instance entry, matching the pattern already
//! established in `balance.rs` / `price_oracle.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

const PLAN_KEY: Symbol = symbol_short!("succ_pln");

/// A designated heir and the inactivity window after which they may claim
/// admin rights via `claim_succession()`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SuccessionPlan {
    pub heir: Address,
    pub inactivity_threshold_ledgers: u32,
    pub last_admin_action_at: u32,
}

fn get_plan(env: &Env) -> Option<SuccessionPlan> {
    env.storage().instance().get(&PLAN_KEY)
}

/// Refreshes the stored admin activity clock, if a succession plan exists.
/// Intended to be called from admin-gated entrypoints so a live admin's
/// succession plan never becomes claimable while they are still active.
pub fn touch_admin_activity(env: &Env) {
    if let Some(mut plan) = get_plan(env) {
        plan.last_admin_action_at = env.ledger().sequence();
        env.storage().instance().set(&PLAN_KEY, &plan);
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Designates `heir` as the address that may claim admin rights if the
    /// current admin performs no admin action for
    /// `inactivity_threshold_ledgers` ledgers. Admin only.
    pub fn set_succession_plan(
        env: Env,
        admin: Address,
        heir: Address,
        inactivity_threshold_ledgers: u32,
    ) -> Result<(), VaultError> {
        let stored_admin = crate::admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        if inactivity_threshold_ledgers == 0 {
            return Err(VaultError::ZeroAmount);
        }

        let plan = SuccessionPlan {
            heir: heir.clone(),
            inactivity_threshold_ledgers,
            last_admin_action_at: env.ledger().sequence(),
        };
        env.storage().instance().set(&PLAN_KEY, &plan);

        env.events().publish((symbol_short!("succ_set"),), heir);
        Ok(())
    }

    /// Read-only lookup of the current succession plan, if any.
    pub fn get_succession_plan(env: Env) -> Option<SuccessionPlan> {
        get_plan(&env)
    }

    /// Callable by the designated heir once the admin has been inactive for
    /// longer than the configured threshold. Transfers admin rights to the
    /// heir and clears the succession plan.
    pub fn claim_succession(env: Env, heir: Address) -> Result<(), VaultError> {
        heir.require_auth();

        let plan = get_plan(&env).ok_or(VaultError::NotInitialized)?;
        if plan.heir != heir {
            return Err(VaultError::Unauthorized);
        }

        let current_ledger = env.ledger().sequence();
        let elapsed = current_ledger.saturating_sub(plan.last_admin_action_at);
        if elapsed <= plan.inactivity_threshold_ledgers {
            return Err(VaultError::Unauthorized);
        }

        admin::set_admin(&env, &heir);
        env.storage().instance().remove(&PLAN_KEY);

        env.events().publish((symbol_short!("succ_clm"),), heir);
        Ok(())
    }
}
















