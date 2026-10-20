//! Stake-gated IPFS content-hash storage (issue #439).
//!
//! Active stakers above a configurable threshold can pin IPFS content hashes
//! on-chain, linked to their address, as a lightweight file registry where
//! staking is the access control mechanism. The contract does not interact
//! with IPFS itself — it only stores the CID string on-chain.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

use crate::balance;
use crate::errors::VaultError;
use crate::vault::VaultContract;

const IPFS_CONFIG_KEY: Symbol = symbol_short!("ipfscfg");
const IPFS_RECORDS_KEY: Symbol = symbol_short!("ipfsrec");

/// Maximum length of an IPFS CID v1 string.
const MAX_HASH_LEN: u32 = 64;
/// Maximum length of a stored description.
const MAX_DESC_LEN: u32 = 100;

/// A single pinned IPFS content hash.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct IPFSRecord {
    pub hash: String,
    pub description: String,
    pub pinned_at: u32,
}

/// Admin-configured storage gate parameters.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct IPFSStorageConfig {
    pub min_stake: i128,
    pub max_hashes_per_user: u32,
}

fn get_config(env: &Env) -> Option<IPFSStorageConfig> {
    env.storage().instance().get(&IPFS_CONFIG_KEY)
}

fn get_user_records(env: &Env, user: &Address) -> Vec<IPFSRecord> {
    env.storage()
        .persistent()
        .get(&(IPFS_RECORDS_KEY, user.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

fn set_user_records(env: &Env, user: &Address, records: &Vec<IPFSRecord>) {
    env.storage()
        .persistent()
        .set(&(IPFS_RECORDS_KEY, user.clone()), records);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Configure the stake-gated IPFS storage service. Admin only.
    ///
    /// `min_stake` is the minimum staked position amount required to pin
    /// hashes; `max_hashes_per_user` caps how many hashes one address can pin.
    pub fn set_ipfs_storage_config(
        env: Env,
        admin: Address,
        min_stake: i128,
        max_hashes_per_user: u32,
    ) -> Result<(), VaultError> {
        admin.require_auth();
        crate::admin::require_admin(&env)?;

        if min_stake < 0 {
            return Err(VaultError::InvalidRate);
        }
        if max_hashes_per_user == 0 {
            return Err(VaultError::InvalidRate);
        }

        env.storage().instance().set(
            &IPFS_CONFIG_KEY,
            &IPFSStorageConfig {
                min_stake,
                max_hashes_per_user,
            },
        );
        Ok(())
    }

    /// Pin an IPFS content hash to the caller's address.
    ///
    /// Requires the caller's staked position amount to be at least the
    /// configured `min_stake`; otherwise reverts with
    /// `InsufficientStakeForStorage`. Hash must be at most 64 characters
    /// (IPFS CID v1 length) and description at most 100 characters.
    pub fn pin_ipfs_hash(
        env: Env,
        user: Address,
        hash: String,
        description: String,
    ) -> Result<(), VaultError> {
        user.require_auth();

        let config = get_config(&env).ok_or(VaultError::NotInitialized)?;

        if hash.len() == 0 || hash.len() > MAX_HASH_LEN {
            return Err(VaultError::InvalidRate);
        }
        if description.len() > MAX_DESC_LEN {
            return Err(VaultError::DescriptionTooLong);
        }

        let position_amount = balance::get_shares(&env, &user);
        if position_amount < config.min_stake {
            return Err(VaultError::InsufficientStakeForStorage);
        }

        let mut records = get_user_records(&env, &user);

        let n = records.len();
        let mut i = 0u32;
        while i < n {
            if records.get(i).unwrap().hash == hash {
                return Err(VaultError::InvalidAddress);
            }
            i += 1;
        }

        if records.len() >= config.max_hashes_per_user {
            return Err(VaultError::MaxPositionsReached);
        }

        records.push_back(IPFSRecord {
            hash: hash.clone(),
            description,
            pinned_at: env.ledger().sequence(),
        });
        set_user_records(&env, &user, &records);

        env.events().publish(
            (symbol_short!("hash_pin"), user.clone()),
            (hash, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Remove a previously pinned IPFS hash from the caller's address.
    pub fn unpin_ipfs_hash(env: Env, user: Address, hash: String) -> Result<(), VaultError> {
        user.require_auth();

        let mut records = get_user_records(&env, &user);
        let n = records.len();
        let mut i = 0u32;
        while i < n {
            if records.get(i).unwrap().hash == hash {
                records.remove(i);
                set_user_records(&env, &user, &records);
                env.events().publish(
                    (symbol_short!("hash_unp"), user.clone()),
                    (hash, env.ledger().sequence()),
                );
                return Ok(());
            }
            i += 1;
        }

        Err(VaultError::PositionNotFound)
    }

    /// Read-only query: all IPFS hashes pinned by `user`.
    pub fn get_ipfs_hashes(env: Env, user: Address) -> Vec<IPFSRecord> {
        get_user_records(&env, &user)
    }

    /// Read-only query: the configured storage gate parameters, if any.
    pub fn get_ipfs_storage_config(env: Env) -> Option<IPFSStorageConfig> {
        get_config(&env)
    }
}
