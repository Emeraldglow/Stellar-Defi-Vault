//! Transfer cooldown ΓÇö a waiting period before the recipient of a
//! transferred position can unstake it (issue #340).
//!
//! Builds on issues #29 (`transfer_position`) and #97
//! (`transfer_position_with_rewards`). Without a cooldown, a transfer could
//! be used to route around the sender's lock-up or holding period entirely.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.
//!
//! # Known gap
//!
//! `transfer_position` and `transfer_position_with_rewards` are currently
//! missing from `src/vault.rs` on `main` (see PR description ΓÇö an unrelated
//! bad merge truncated most of that file). This module exposes
//! [`record_transfer_received`] and [`assert_transfer_cooldown_cleared`] as
//! free functions specifically so that, once those two entrypoints are
//! restored, they only need to add one call each:
//!
//! - the end of a successful transfer: `transfer_cooldown::record_transfer_received(&env, &recipient);`
//! - the top of `unstake`: `transfer_cooldown::assert_transfer_cooldown_cleared(&env, &user)?;`
//! - the top of a full unstake / re-stake path: `transfer_cooldown::clear_transfer_received(&env, &user);`
//!
//! Until then, [`check_transfer_cooldown`] (the same assertion, exposed as a
//! contract entrypoint) and [`get_transfer_cooldown_remaining`] are directly
//! callable and tested on their own.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance-storage key for the configured cooldown length, in ledgers.
/// `0` (the default) disables the check entirely.
const COOLDOWN_KEY: Symbol = symbol_short!("tc_cool");

/// Persistent-storage key for the ledger a user most recently received a
/// transferred position at. Keyed by `(RECEIVED_AT_KEY, user)`.
const RECEIVED_AT_KEY: Symbol = symbol_short!("tc_recv");

fn get_cooldown(env: &Env) -> u32 {
    env.storage().instance().get(&COOLDOWN_KEY).unwrap_or(0)
}

fn get_received_at(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&(RECEIVED_AT_KEY, user.clone()))
}

/// Record that `recipient` just received a transferred position at the
/// current ledger, starting their cooldown (if one is configured).
///
/// Intended to be called from `transfer_position` and
/// `transfer_position_with_rewards` ΓÇö see the module-level "Known gap" note.
pub fn record_transfer_received(env: &Env, recipient: &Address) {
    env.storage()
        .persistent()
        .set(&(RECEIVED_AT_KEY, recipient.clone()), &env.ledger().sequence());
}

/// Clear `user`'s transfer-cooldown marker. Intended to be called once a
/// user has fully unstaked and later opens a brand-new position by staking
/// directly (not via transfer) ΓÇö see the module-level "Known gap" note.
pub fn clear_transfer_received(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&(RECEIVED_AT_KEY, user.clone()));
}

/// Ledgers remaining before `user`'s transfer cooldown clears. `0` if no
/// cooldown is active (never received a transfer, or it already expired).
pub fn remaining(env: &Env, user: &Address) -> u32 {
    let cooldown = get_cooldown(env);
    if cooldown == 0 {
        return 0;
    }
    let Some(received_at) = get_received_at(env, user) else {
        return 0;
    };
    let elapsed = env.ledger().sequence().saturating_sub(received_at);
    cooldown.saturating_sub(elapsed)
}

/// Reverts with `TransferCooldownActive` if `user` received a transferred
/// position and is still inside the configured cooldown window.
///
/// Intended to be called at the top of `unstake` ΓÇö see the module-level
/// "Known gap" note.
pub fn assert_transfer_cooldown_cleared(env: &Env, user: &Address) -> Result<(), VaultError> {
    if remaining(env, user) > 0 {
        return Err(VaultError::UseCooldownFlow);
    }
    Ok(())
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Set the transfer cooldown length, in ledgers. `0` disables the check.
    /// Admin only.
    pub fn set_transfer_cooldown(env: Env, ledgers: u32) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        env.storage().instance().set(&COOLDOWN_KEY, &ledgers);

        env.events().publish(
            (symbol_short!("tc_set"),),
            (ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// The currently configured transfer cooldown length, in ledgers.
    pub fn get_transfer_cooldown(env: Env) -> u32 {
        get_cooldown(&env)
    }

    /// Ledgers remaining before `user`'s transfer cooldown clears.
    pub fn get_transfer_cooldown_remaining(env: Env, user: Address) -> u32 {
        remaining(&env, &user)
    }

    /// Reverts with `TransferCooldownActive` if `user` is still inside
    /// their transfer cooldown. Exposed directly since `unstake` doesn't
    /// currently exist on `main` to call this itself ΓÇö see the module-level
    /// "Known gap" note.
    pub fn check_transfer_cooldown(env: Env, user: Address) -> Result<(), VaultError> {
        assert_transfer_cooldown_cleared(&env, &user)
    }
}
















