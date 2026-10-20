//! Deterministic staking position fingerprint ("position DNA").
//!
//! Combines a staker's address, stake amount, `staked_at_ledger`, and this
//! contract's own address into a single 32-byte SHA256 digest. Useful for
//! off-chain indexing, NFT metadata, and cross-system position tracking
//! without depending on mutable on-chain fields directly.
//!
//! # Wiring
//!
//! There is no hook into the core `stake()` flow to auto-capture the DNA at
//! open time, so `capture_position_dna()` is exposed as a standalone,
//! idempotent entrypoint: it snapshots the *current* position once and never
//! overwrites an existing snapshot, approximating "recorded on first stake"
//! for a caller that invokes it right after opening a position.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! persistent storage, matching `balance.rs`.

use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{contractimpl, symbol_short, Address, Bytes, Env, Symbol};

use crate::balance;
use crate::errors::VaultError;
use crate::storage::DataKey;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Persistent-storage key prefix for a user's originally captured DNA.
const DNA_KEY: Symbol = symbol_short!("pos_dna");

/// `SHA256(user || staked_amount_be_bytes || staked_at_ledger_be_bytes ||
/// contract_address)`.
fn compute_dna(env: &Env, user: &Address, staked_amount: i128, staked_at_ledger: u32) -> Bytes {
    let mut preimage = Bytes::new(env);
    preimage.append(&user.clone().to_xdr(env));
    for byte in staked_amount.to_be_bytes().iter() {
        preimage.push_back(*byte);
    }
    for byte in staked_at_ledger.to_be_bytes().iter() {
        preimage.push_back(*byte);
    }
    preimage.append(&env.current_contract_address().to_xdr(env));

    env.crypto().sha256(&preimage).into()
}

fn staked_at_ledger_of(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::StakedAtLedger(user.clone()))
}

pub fn get_original(env: &Env, user: &Address) -> Option<Bytes> {
    env.storage().persistent().get(&(DNA_KEY, user.clone()))
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// The deterministic fingerprint for `user`'s current position.
    ///
    /// Reverts with `PositionNotFound` if the user has no active position.
    /// Read-only: no auth required, no state changes.
    pub fn get_position_dna(env: Env, user: Address) -> Result<Bytes, VaultError> {
        let staked_amount = balance::get_shares(&env, &user);
        if staked_amount <= 0 {
            return Err(VaultError::PositionNotFound);
        }
        let staked_at_ledger =
            crate::position_dna::staked_at_ledger_of(&env, &user).unwrap_or(0);

        Ok(crate::position_dna::compute_dna(
            &env,
            &user,
            staked_amount,
            staked_at_ledger,
        ))
    }

    /// Snapshot `user`'s current position DNA into persistent storage, if one
    /// has not already been captured. The stored value never changes after
    /// this, even if the position is later topped up or partially closed.
    pub fn capture_position_dna(env: Env, user: Address) -> Result<Bytes, VaultError> {
        if let Some(existing) = crate::position_dna::get_original(&env, &user) {
            return Ok(existing);
        }

        let dna = Self::get_position_dna(env.clone(), user.clone())?;
        env.storage()
            .persistent()
            .set(&(crate::position_dna::DNA_KEY, user), &dna);
        Ok(dna)
    }

    /// The originally captured DNA for `user`, if `capture_position_dna` has
    /// ever been called for them â€” unchanged even after the position changes.
    pub fn get_original_position_dna(env: Env, user: Address) -> Option<Bytes> {
        crate::position_dna::get_original(&env, &user)
    }
}















