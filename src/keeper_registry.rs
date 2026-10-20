//! Keeper registry.
//!
//! Several keeper-triggered flows (e.g. `compound_optimizer.rs`'s
//! `trigger_optimized_compound`) rely on an arbitrary caller-supplied
//! `keeper: Address` with no notion of approval or track record. This module
//! adds a formal registry: which keeper addresses are admin-approved, and
//! their historical trigger count / total earnings, so keeper-triggered
//! entrypoints can gate on `is_registered_keeper` instead of trusting any
//! caller.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::errors::VaultError;
use crate::events;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Persistent-storage key prefix for a single keeper's record.
const KEEPER_KEY: Symbol = symbol_short!("kpr_rec");
/// Instance-storage key for the list of every address ever registered.
const KEEPER_LIST_KEY: Symbol = symbol_short!("kpr_all");

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct KeeperRecord {
    pub address: Address,
    pub registered_at: u32,
    pub total_triggers: u32,
    pub total_earned: i128,
    pub active: bool,
}

fn get_record(env: &Env, keeper: &Address) -> Option<KeeperRecord> {
    env.storage()
        .persistent()
        .get(&(KEEPER_KEY, keeper.clone()))
}

fn set_record(env: &Env, record: &KeeperRecord) {
    env.storage()
        .persistent()
        .set(&(KEEPER_KEY, record.address.clone()), record);
}

fn get_keeper_list(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&KEEPER_LIST_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_keeper_list(env: &Env, list: &Vec<Address>) {
    env.storage().instance().set(&KEEPER_LIST_KEY, list);
}

/// Whether `keeper` is currently an approved, active keeper.
pub fn is_registered(env: &Env, keeper: &Address) -> bool {
    get_record(env, keeper)
        .map(|record| record.active)
        .unwrap_or(false)
}

/// Records a successful keeper-triggered action against `keeper`'s stats.
/// No-op if the keeper is not registered, so a caller cannot backfill a
/// track record for an address the admin never approved.
pub fn record_keeper_action(env: &Env, keeper: &Address, earned: i128) {
    if let Some(mut record) = get_record(env, keeper) {
        record.total_triggers = record.total_triggers.saturating_add(1);
        record.total_earned = record.total_earned.saturating_add(earned);
        set_record(env, &record);
    }
}

#[contractimpl]
impl VaultContract {
    /// Approve a keeper address. Admin only. Re-registering a previously
    /// deregistered keeper reactivates it and preserves its accumulated
    /// stats rather than resetting them.
    pub fn register_keeper(env: Env, keeper: Address) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let record = match crate::keeper_registry::get_record(&env, &keeper) {
            Some(mut existing) => {
                existing.active = true;
                existing
            }
            None => {
                let mut list = crate::keeper_registry::get_keeper_list(&env);
                list.push_back(keeper.clone());
                crate::keeper_registry::set_keeper_list(&env, &list);

                KeeperRecord {
                    address: keeper.clone(),
                    registered_at: env.ledger().sequence(),
                    total_triggers: 0,
                    total_earned: 0,
                    active: true,
                }
            }
        };
        crate::keeper_registry::set_record(&env, &record);
        events::keeper_registered(&env, &keeper, env.ledger().sequence());
        Ok(())
    }

    /// Deactivate a keeper. Admin only. The record and its stats are kept
    /// (marked inactive) rather than removed.
    pub fn deregister_keeper(env: Env, keeper: Address) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let mut record = crate::keeper_registry::get_record(&env, &keeper)
            .ok_or(VaultError::PositionNotFound)?;
        record.active = false;
        crate::keeper_registry::set_record(&env, &record);
        events::keeper_deregistered(&env, &keeper, env.ledger().sequence());
        Ok(())
    }

    /// Whether `keeper` is currently an approved, active keeper.
    pub fn is_registered_keeper(env: Env, keeper: Address) -> bool {
        crate::keeper_registry::is_registered(&env, &keeper)
    }

    /// A keeper's registry record, if it has ever been registered.
    pub fn get_keeper_record(env: Env, keeper: Address) -> Option<KeeperRecord> {
        crate::keeper_registry::get_record(&env, &keeper)
    }

    /// Every keeper record ever registered (including inactive ones). Admin
    /// only, since the full list is an operational detail rather than
    /// something a keeper or staker needs to query about others.
    pub fn get_all_keepers(env: Env) -> Result<Vec<KeeperRecord>, VaultError> {
        admin::require_admin(&env)?;

        let list = crate::keeper_registry::get_keeper_list(&env);
        let mut records = Vec::new(&env);
        for address in list.iter() {
            if let Some(record) = crate::keeper_registry::get_record(&env, &address) {
                records.push_back(record);
            }
        }
        Ok(records)
    }
}















