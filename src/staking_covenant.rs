//! Staking covenant (issue #413).
//!
//! Before staking, a user acknowledges and commits to a set of pool terms â€”
//! published by the admin as a hash â€” creating an auditable on-chain record
//! of agreement. When the admin publishes a new terms version, existing
//! stakers keep their positions untouched but must re-sign before their
//! next stake top-up.
//!
//! # Wiring
//!
//! Like `epoch_reward_cap.rs`, this exposes its own opt-in entrypoint
//! (`stake_with_covenant`) rather than editing `vault.rs`'s existing
//! `stake()`, keeping the covenant requirement additive.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw
//! `Symbol`-keyed storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Bytes, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance key: `(terms_hash, terms_version)`.
const TERMS_KEY: Symbol = symbol_short!("cov_trm");
/// Persistent key prefix: `(RECORD_KEY, user) -> CovenantRecord`.
const RECORD_KEY: Symbol = symbol_short!("cov_rec");

/// A staker's signed commitment to a specific terms version (issue #413).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CovenantRecord {
    pub terms_version: u32,
    pub signed_at: u32,
}

fn get_terms(env: &Env) -> Option<(Bytes, u32)> {
    env.storage().instance().get(&TERMS_KEY)
}

fn set_terms(env: &Env, terms_hash: &Bytes, version: u32) {
    env.storage()
        .instance()
        .set(&TERMS_KEY, &(terms_hash.clone(), version));
}

fn get_record(env: &Env, user: &Address) -> Option<CovenantRecord> {
    env.storage().persistent().get(&(RECORD_KEY, user.clone()))
}

fn set_record(env: &Env, user: &Address, record: &CovenantRecord) {
    env.storage()
        .persistent()
        .set(&(RECORD_KEY, user.clone()), record);
}

/// Whether `user` has signed the currently published terms version. `false`
/// if no terms have been published yet, or the user's signature is for an
/// older version.
fn signed_current(env: &Env, user: &Address) -> bool {
    match (get_terms(env), get_record(env, user)) {
        (Some((_, version)), Some(record)) => record.terms_version == version,
        _ => false,
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Publishes the pool terms as a hash and version. Admin only. Existing
    /// stakers' positions are untouched, but their `CovenantRecord` is no
    /// longer current, so `stake_with_covenant` will require a fresh
    /// signature on their next top-up.
    pub fn set_pool_terms(env: Env, terms_hash: Bytes, version: u32) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        crate::staking_covenant::set_terms(&env, &terms_hash, version);
        env.events()
            .publish((symbol_short!("cov_set"),), (version, env.ledger().sequence()));
        Ok(())
    }

    /// The currently published pool terms as `(terms_hash, version)`, or
    /// `None` if none have been published yet.
    pub fn get_pool_terms(env: Env) -> Option<(Bytes, u32)> {
        crate::staking_covenant::get_terms(&env)
    }

    /// Signs the currently published pool terms for the caller. Reverts
    /// with `TermsMismatch` if `terms_hash` does not match the currently
    /// published hash, or `NotInitialized` if no terms have been published.
    pub fn sign_covenant(env: Env, user: Address, terms_hash: Bytes) -> Result<(), VaultError> {
        user.require_auth();

        let (current_hash, version) =
            crate::staking_covenant::get_terms(&env).ok_or(VaultError::NotInitialized)?;
        if terms_hash != current_hash {
            return Err(VaultError::InvalidRate);
        }

        crate::staking_covenant::set_record(
            &env,
            &user,
            &CovenantRecord {
                terms_version: version,
                signed_at: env.ledger().sequence(),
            },
        );

        env.events().publish(
            (symbol_short!("cov_sig"), user),
            (version, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Whether the caller has signed the currently published terms.
    pub fn is_covenant_signed(env: Env, user: Address) -> bool {
        crate::staking_covenant::signed_current(&env, &user)
    }

    /// Stakes `amount` for `user` (same behavior as `stake()`), but first
    /// requires an up-to-date signed covenant. Reverts with
    /// `CovenantRequired` if the caller has not signed the current terms
    /// version.
    pub fn stake_with_covenant(env: Env, user: Address, amount: i128) -> Result<i128, VaultError> {
        // Only gate once terms have actually been published â€” before that
        // there is nothing for a staker to sign.
        if crate::staking_covenant::get_terms(&env).is_some()
            && !crate::staking_covenant::signed_current(&env, &user)
        {
            return Err(VaultError::InvalidRate);
        }
        Self::stake(env, user, amount)
    }
}

















