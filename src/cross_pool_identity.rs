//! Cross-pool identity linking (issue #470).
//!
//! Allows a staker to register the same address across multiple pool contracts
//! deployed by the same operator, creating a unified cross-pool identity. The
//! registry aggregates their total staked value across all linked pools for
//! governance weight, reputation, and analytics purposes.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! persistent and instance storage, matching `balance.rs` and other feature modules.
//!
//! Storage keys:
//! - Per-user identity: `(symbol_short!("cp_id"), user: Address)` -> `CrossPoolIdentity`
//! - Governance weight config flag: `symbol_short!("cp_gov_w")` -> `bool`

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::balance;

const CP_IDENTITY_KEY: Symbol = symbol_short!("cp_id");
const CP_GOV_WEIGHT_KEY: Symbol = symbol_short!("cp_gov_w");

/// Maximum allowed linked pools per user.
pub const MAX_LINKED_POOLS: u32 = 10;

/// Stored cross-pool identity record per user.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CrossPoolIdentity {
    pub linked_pools: Vec<Address>,
    pub total_staked_all_pools: i128,
    pub last_synced_at: u32,
}

pub fn get_identity(env: &Env, user: &Address) -> Option<CrossPoolIdentity> {
    env.storage()
        .persistent()
        .get(&(CP_IDENTITY_KEY, user.clone()))
}

pub fn set_identity(env: &Env, user: &Address, identity: &CrossPoolIdentity) {
    env.storage()
        .persistent()
        .set(&(CP_IDENTITY_KEY, user.clone()), identity);
}

pub fn is_governance_weight_enabled(env: &Env) -> bool {
    env.storage().instance().get(&CP_GOV_WEIGHT_KEY).unwrap_or(false)
}

pub fn set_governance_weight_enabled(env: &Env, enabled: bool) {
    env.storage().instance().set(&CP_GOV_WEIGHT_KEY, &enabled);
}

/// Helper to get effective stake for voting weight: cross-pool total if enabled, else single pool.
pub fn get_effective_vote_weight(env: &Env, user: &Address) -> i128 {
    if is_governance_weight_enabled(env) {
        match get_identity(env, user) {
            Some(identity) => identity.total_staked_all_pools,
            None => balance::get_shares(env, user),
        }
    } else {
        balance::get_shares(env, user)
    }
}
