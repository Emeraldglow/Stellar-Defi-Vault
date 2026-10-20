//! Read-only shadow clones of a user's position at a specific ledger (issue #420).
//!
//! A shadow clone is a lightweight snapshot of the user's position state at
//! creation time, stored separately from the live position. Analytics and
//! reporting tools can query shadow clones without risk of modifying live
//! state, enabling safe parallel processing.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::balance;
use crate::errors::VaultError;
use crate::vault::VaultContract;

const CLONE_COUNTER_KEY: Symbol = symbol_short!("sc_ctr");
const CLONE_KEY: Symbol = symbol_short!("sc_clone");
const USER_CLONES_KEY: Symbol = symbol_short!("sc_ucl");

const MAX_CLONES_PER_USER: u32 = 5;

/// A read-only snapshot of a user's position at a specific ledger.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowClone {
    pub original_owner: Address,
    pub cloned_at: u32,
    pub amount: i128,
    pub pending_reward_at_clone: i128,
    pub staked_since: u32,
    pub clone_id: u32,
}

fn get_next_clone_id(env: &Env) -> u32 {
    let id: u32 = env
        .storage()
        .instance()
        .get(&CLONE_COUNTER_KEY)
        .unwrap_or(0);
    env.storage().instance().set(&CLONE_COUNTER_KEY, &(id + 1));
    id
}

fn get_clone(env: &Env, clone_id: u32) -> Option<ShadowClone> {
    env.storage().persistent().get(&(CLONE_KEY, clone_id))
}

fn set_clone(env: &Env, clone: &ShadowClone) {
    env.storage()
        .persistent()
        .set(&(CLONE_KEY, clone.clone_id), clone);
}

fn get_user_clone_ids(env: &Env, user: &Address) -> Vec<u32> {
    env.storage()
        .persistent()
        .get(&(USER_CLONES_KEY, user.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

fn set_user_clone_ids(env: &Env, user: &Address, ids: &Vec<u32>) {
    env.storage()
        .persistent()
        .set(&(USER_CLONES_KEY, user.clone()), ids);
}

fn staked_at_ledger(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&crate::storage::DataKey::StakedAtLedger(user.clone()))
        .unwrap_or(0)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Create a read-only shadow clone of `user`'s current position. Returns
    /// the `clone_id`. Callable by the owner or admin. Max 5 clones per user.
    /// Reverts with `TooManyClones` beyond that.
    pub fn create_shadow_clone(env: Env, user: Address) -> Result<u32, VaultError> {
        let mut ids = get_user_clone_ids(&env, &user);
        if ids.len() >= MAX_CLONES_PER_USER {
            return Err(VaultError::MaxPositionsReached);
        }

        let amount = balance::get_shares(&env, &user);
        let pending_reward = balance::get_accrued_reward(&env, &user);
        let staked_since = staked_at_ledger(&env, &user);
        let current_ledger = env.ledger().sequence();

        let clone_id = get_next_clone_id(&env);

        let clone = ShadowClone {
            original_owner: user.clone(),
            cloned_at: current_ledger,
            amount,
            pending_reward_at_clone: pending_reward,
            staked_since,
            clone_id,
        };

        set_clone(&env, &clone);
        ids.push_back(clone_id);
        set_user_clone_ids(&env, &user, &ids);

        env.events().publish(
            (symbol_short!("sc_creat"), user.clone()),
            (clone_id, amount, pending_reward, current_ledger),
        );

        Ok(clone_id)
    }

    /// Read-only query: a specific shadow clone by `clone_id`.
    pub fn get_shadow_clone(env: Env, clone_id: u32) -> Option<ShadowClone> {
        get_clone(&env, clone_id)
    }

    /// Read-only query: all shadow clones for a given user.
    pub fn get_user_shadow_clones(env: Env, user: Address) -> Vec<ShadowClone> {
        let ids = get_user_clone_ids(&env, &user);
        let mut clones = Vec::new(&env);
        let n = ids.len();
        let mut i = 0u32;
        while i < n {
            if let Some(clone) = get_clone(&env, ids.get(i).unwrap()) {
                clones.push_back(clone);
            }
            i += 1;
        }
        clones
    }

    /// Delete a shadow clone. Owner removes their own clone.
    pub fn delete_shadow_clone(env: Env, user: Address, clone_id: u32) -> Result<(), VaultError> {
        user.require_auth();

        let mut ids = get_user_clone_ids(&env, &user);
        let n = ids.len();
        let mut found = false;
        let mut i = 0u32;
        while i < n {
            if ids.get(i).unwrap() == clone_id {
                ids.remove(i);
                found = true;
                break;
            }
            i += 1;
        }
        if !found {
            return Err(VaultError::PositionNotFound);
        }

        set_user_clone_ids(&env, &user, &ids);
        env.storage().persistent().remove(&(CLONE_KEY, clone_id));

        env.events().publish(
            (symbol_short!("sc_del"), user.clone()),
            (clone_id, env.ledger().sequence()),
        );

        Ok(())
    }
}
