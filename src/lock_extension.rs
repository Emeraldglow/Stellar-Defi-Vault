//! Voluntary lock-period extension for a reward boost.
//!
//! Vote-escrow-style tokenomics: a user who voluntarily commits to a longer
//! lock than their position's current baseline receives a reward boost
//! proportional to how many extra ledgers they commit.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so per-user extension state is
//! kept under raw `Symbol`-keyed persistent storage, matching the pattern
//! already established in `balance.rs` / `price_oracle.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::storage::DataKey;
use crate::VaultContract;
use crate::vault::VaultContractClient;

const CONFIG_KEY: Symbol = symbol_short!("lockxcfg");
/// Per-user cumulative extra ledgers committed via `extend_lock_period()`.
const EXTRA_LOCK_KEY: Symbol = symbol_short!("lockxtra");
/// Per-user cumulative boost (bps) earned via `extend_lock_period()`.
const BOOST_KEY: Symbol = symbol_short!("lockboost");

/// Admin-set terms for the lock-extension boost.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LockExtensionConfig {
    pub max_extension_ledgers: u32,
    pub boost_per_10k_ledgers_bps: u32,
}

fn get_config(env: &Env) -> Option<LockExtensionConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Sets the lock-extension boost terms. Admin only.
    pub fn set_lock_extension_config(
        env: Env,
        admin: Address,
        max_extension_ledgers: u32,
        boost_per_10k_ledgers_bps: u32,
    ) -> Result<(), VaultError> {
        let stored_admin = crate::admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        if max_extension_ledgers == 0 {
            return Err(VaultError::ZeroAmount);
        }

        let config = LockExtensionConfig {
            max_extension_ledgers,
            boost_per_10k_ledgers_bps,
        };
        env.storage().instance().set(&CONFIG_KEY, &config);

        env.events().publish(
            (symbol_short!("lockxcfg"),),
            (max_extension_ledgers, boost_per_10k_ledgers_bps),
        );
        Ok(())
    }

    /// Read-only lookup of the current lock-extension config.
    pub fn get_lock_extension_config(env: Env) -> Option<LockExtensionConfig> {
        get_config(&env)
    }

    /// Voluntarily extends the caller's lock commitment by
    /// `additional_ledgers` (capped by `max_extension_ledgers` per call) and
    /// grants a proportional boost: `additional_ledgers / 10_000 *
    /// boost_per_10k_ledgers_bps`. Boost accumulates across repeated calls.
    /// Returns the caller's new total boost in bps.
    pub fn extend_lock_period(
        env: Env,
        user: Address,
        additional_ledgers: u32,
    ) -> Result<u32, VaultError> {
        user.require_auth();

        let config = get_config(&env).ok_or(VaultError::NotInitialized)?;
        if additional_ledgers == 0 || additional_ledgers > config.max_extension_ledgers {
            return Err(VaultError::ZeroAmount);
        }
        if !env
            .storage()
            .persistent()
            .has(&DataKey::StakedAtLedger(user.clone()))
        {
            return Err(VaultError::PositionNotFound);
        }

        let extra_key = (EXTRA_LOCK_KEY, user.clone());
        let current_extra: u32 = env.storage().persistent().get(&extra_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&extra_key, &(current_extra + additional_ledgers));

        let boost_gained = (additional_ledgers as u64)
            .checked_mul(config.boost_per_10k_ledgers_bps as u64)
            .and_then(|v| v.checked_div(10_000))
            .ok_or(VaultError::ArithmeticError)? as u32;

        let boost_key = (BOOST_KEY, user.clone());
        let current_boost: u32 = env.storage().persistent().get(&boost_key).unwrap_or(0);
        let new_boost = current_boost + boost_gained;
        env.storage().persistent().set(&boost_key, &new_boost);

        env.events().publish(
            (symbol_short!("lock_ext"), user),
            (additional_ledgers, new_boost),
        );
        Ok(new_boost)
    }

    /// Total extension boost (bps) accumulated by `user` via
    /// `extend_lock_period()`.
    pub fn get_lock_extension_boost(env: Env, user: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&(BOOST_KEY, user))
            .unwrap_or(0)
    }
}















