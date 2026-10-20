//! Replay protection for sensitive admin transactions (issue #374).
//!
//! A signed admin transaction, once broadcast, can theoretically be captured
//! and re-submitted by anyone. Stellar's own transaction envelope already
//! carries a sequence number that stops byte-for-byte replay of the *outer*
//! transaction, but that protection is opaque to this contract — it can't be
//! relied on by other tooling (relayers, multisig co-signers rebuilding a
//! transaction) that only sees the contract call's arguments. This module
//! adds an explicit, contract-level nonce so a captured invocation of a
//! sensitive admin action can't be re-executed even if resubmitted.
//!
//! # Design
//!
//! Each admin address has its own strictly-incrementing nonce, starting at 0.
//! `execute_admin_action_with_nonce()` requires the caller to supply exactly
//! the current expected value; any other value (including a previously
//! consumed one) is rejected with `VaultError::NonceMismatch`. On success the
//! nonce advances by one, so the same call can never execute twice.
//!
//! # Scope
//!
//! Mirrors `execute_action()`'s dispatcher (issue #195, timelocked admin
//! actions): only `AdminAction::SetRewardRate`, `SetLockPeriod`, `Pause`, and
//! `Unpause` are actually wired up here. Other `AdminAction` variants type-
//! check but revert with `VaultError::InvalidRate` — decoding and dispatching
//! every admin action generically is a much larger surface than this issue's
//! stated scope (replay protection), and the pattern of covering only the
//! "primary candidates" already has precedent in `execute_action()`.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Bytes, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::storage::{AdminAction, DataKey};
use crate::vault::{VaultContract, VaultContractClient};

const NONCE_KEY: Symbol = symbol_short!("adm_nonce");

fn get_nonce(env: &Env, admin: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&(NONCE_KEY, admin.clone()))
        .unwrap_or(0)
}

fn set_nonce(env: &Env, admin: &Address, nonce: u64) {
    env.storage()
        .persistent()
        .set(&(NONCE_KEY, admin.clone()), &nonce);
}

fn decode_u32(bytes: &Bytes) -> Option<u32> {
    if bytes.len() != 4 {
        return None;
    }
    let mut buf = [0u8; 4];
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = bytes.get(i as u32)?;
    }
    Some(u32::from_be_bytes(buf))
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// The nonce `admin` must supply to its next
    /// `execute_admin_action_with_nonce()` call. Starts at 0 for an address
    /// that has never used the nonce flow.
    pub fn admin_action_nonce(env: Env, admin: Address) -> u64 {
        crate::admin_action_nonce::get_nonce(&env, &admin)
    }

    /// Execute a sensitive admin action guarded by a strictly-incrementing
    /// per-admin nonce (issue #374). `admin` must be the current pool admin
    /// and must supply exactly the value `admin_action_nonce()` currently
    /// returns for it — a captured-and-resubmitted call carrying a
    /// previously-consumed nonce is rejected with `NonceMismatch` rather than
    /// re-executing the action.
    ///
    /// See the module doc for which `AdminAction` variants are actually
    /// dispatched; unsupported variants revert with `InvalidRate`.
    pub fn execute_admin_action_with_nonce(
        env: Env,
        admin: Address,
        action: AdminAction,
        params: Bytes,
        nonce: u64,
    ) -> Result<(), VaultError> {
        let stored_admin = admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        let expected = crate::admin_action_nonce::get_nonce(&env, &admin);
        if nonce != expected {
            return Err(VaultError::NonceMismatch);
        }

        match action {
            AdminAction::SetRewardRate => {
                let rate_bps = decode_u32(&params).ok_or(VaultError::InvalidRate)?;
                balance::set_reward_rate_bps(&env, rate_bps);
            }
            AdminAction::SetLockPeriod => {
                let ledgers = decode_u32(&params).ok_or(VaultError::InvalidRate)?;
                env.storage().instance().set(&DataKey::LockPeriod, &ledgers);
            }
            AdminAction::Pause => {
                env.storage().instance().set(&DataKey::Paused, &true);
            }
            AdminAction::Unpause => {
                env.storage().instance().set(&DataKey::Paused, &false);
            }
            _ => return Err(VaultError::InvalidRate),
        }

        crate::admin_action_nonce::set_nonce(&env, &admin, nonce + 1);
        events::admin_action_nonce_consumed(&env, &admin, nonce, env.ledger().sequence());
        Ok(())
    }
}
