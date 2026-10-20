//! Roadmap voting (issue #462).
//!
//! An advisory product-roadmap poll. The admin publishes planned features and
//! stakers distribute a fixed budget of 100 vote points across them each epoch
//! to signal priority. Unlike governance proposals (issue #160), nothing here
//! executes on-chain — the rankings are a signal for off-chain planning.
//!
//! Linear points (no quadratic weighting). Vote budgets reset every
//! `LEDGERS_PER_DAY * 30` (monthly): on the first vote of a new epoch a user's
//! previous allocations are rolled back off the item tallies and their
//! allocation record is cleared for a fresh 100-point budget.
//!
//! # Storage (`DataKey` is at Soroban's 50-variant cap — raw `Symbol` keys)
//!
//! - Items: `symbol_short!("rdmp_itm")` -> `Vec<RoadmapItem>`
//! - Next item id: `symbol_short!("rdmp_nid")` -> `u32`
//! - Per-user allocation: `(Symbol::new(env, "rdmp_alc"), user)` -> `Vec<(u32, u32)>`
//! - Per-user epoch stamp: `(Symbol::new(env, "rdmp_ep"), user)` -> `u32`

use soroban_sdk::{
    contractimpl, contracttype, symbol_short, Address, Bytes, Env, String, Symbol, Vec,
};

use crate::admin;
use crate::balance;
use crate::errors::VaultCampaignError;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

const ITEMS_KEY: Symbol = symbol_short!("rdmp_itm");
const NEXT_ID_KEY: Symbol = symbol_short!("rdmp_nid");

/// Maximum roadmap items that can exist at once.
pub const MAX_ROADMAP_ITEMS: u32 = 20;
/// Maximum title length, in characters.
pub const MAX_TITLE_LEN: u32 = 80;
/// Vote points each user may distribute per epoch.
pub const VOTE_POINTS_BUDGET: u32 = 100;
/// Length of a voting epoch: 30 days.
pub const ROADMAP_EPOCH_LEDGERS: u32 = LEDGERS_PER_DAY * 30;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RoadmapItem {
    pub id: u32,
    pub title: String,
    pub description_hash: Bytes,
    pub votes: i128,
    pub category: String,
}

fn allocation_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "rdmp_alc"), user.clone())
}

fn epoch_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "rdmp_ep"), user.clone())
}

/// The current monthly voting epoch index.
pub fn current_epoch(env: &Env) -> u32 {
    env.ledger().sequence() / ROADMAP_EPOCH_LEDGERS
}

pub fn get_items(env: &Env) -> Vec<RoadmapItem> {
    env.storage()
        .instance()
        .get(&ITEMS_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_items(env: &Env, items: &Vec<RoadmapItem>) {
    env.storage().instance().set(&ITEMS_KEY, items);
}

fn next_id(env: &Env) -> u32 {
    env.storage().instance().get(&NEXT_ID_KEY).unwrap_or(1)
}

fn set_next_id(env: &Env, id: u32) {
    env.storage().instance().set(&NEXT_ID_KEY, &id);
}

pub fn get_allocation(env: &Env, user: &Address) -> Vec<(u32, u32)> {
    env.storage()
        .persistent()
        .get(&allocation_key(env, user))
        .unwrap_or_else(|| Vec::new(env))
}

fn find_index(items: &Vec<RoadmapItem>, id: u32) -> Option<u32> {
    for i in 0..items.len() {
        if items.get(i).unwrap().id == id {
            return Some(i);
        }
    }
    None
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #462: admin adds a roadmap item. Max 20 items; title max 80 chars.
    /// Returns the new item id.
    pub fn add_roadmap_item(
        env: Env,
        admin_addr: Address,
        title: String,
        description_hash: Bytes,
        category: String,
    ) -> Result<u32, VaultCampaignError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        let mut items = get_items(&env);
        if items.len() >= MAX_ROADMAP_ITEMS {
            return Err(VaultCampaignError::TooManyRoadmapItems);
        }
        if title.len() > MAX_TITLE_LEN {
            return Err(VaultCampaignError::TitleTooLong);
        }

        let id = next_id(&env);
        set_next_id(&env, id + 1);
        items.push_back(RoadmapItem {
            id,
            title,
            description_hash,
            votes: 0,
            category,
        });
        set_items(&env, &items);

        env.events()
            .publish((symbol_short!("rdmp_add"),), (id, env.ledger().sequence()));
        Ok(id)
    }

    /// Issue #462: admin removes a roadmap item by id.
    pub fn remove_roadmap_item(
        env: Env,
        admin_addr: Address,
        id: u32,
    ) -> Result<(), VaultCampaignError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        let mut items = get_items(&env);
        let idx = find_index(&items, id).ok_or(VaultCampaignError::RoadmapItemNotFound)?;
        items.remove(idx);
        set_items(&env, &items);

        env.events()
            .publish((symbol_short!("rdmp_rm"),), (id, env.ledger().sequence()));
        Ok(())
    }

    /// Issue #462: a staker allocates `weight` of their 100 monthly vote points
    /// to `item_id`. Re-voting the same item replaces that item's allocation.
    /// On the first vote of a new epoch the caller's prior allocations are
    /// rolled back and their budget resets.
    pub fn vote_roadmap_item(
        env: Env,
        user: Address,
        item_id: u32,
        weight: u32,
    ) -> Result<(), VaultCampaignError> {
        user.require_auth();

        if weight > VOTE_POINTS_BUDGET {
            return Err(VaultCampaignError::InvalidVoteWeight);
        }
        if balance::get_shares(&env, &user) <= 0 {
            return Err(VaultCampaignError::PositionNotFound);
        }

        let mut items = get_items(&env);
        let idx = find_index(&items, item_id).ok_or(VaultCampaignError::RoadmapItemNotFound)?;

        let epoch = current_epoch(&env);
        let stored_epoch: Option<u32> = env.storage().persistent().get(&epoch_key(&env, &user));

        let mut allocation = get_allocation(&env, &user);
        if stored_epoch.map_or(false, |e| e != epoch) {
            // New epoch: roll the caller's previous allocations back off the
            // item tallies, then start from an empty allocation.
            for entry in allocation.iter() {
                let (prev_id, prev_weight) = entry;
                if let Some(i) = find_index(&items, prev_id) {
                    let mut it = items.get(i).unwrap();
                    it.votes = it.votes.saturating_sub(prev_weight as i128);
                    items.set(i, it);
                }
            }
            allocation = Vec::new(&env);
        }

        // Existing weight the caller already put on this item (this epoch).
        let mut existing_pos: Option<u32> = None;
        let mut old_weight: u32 = 0;
        for i in 0..allocation.len() {
            let (aid, aw) = allocation.get(i).unwrap();
            if aid == item_id {
                existing_pos = Some(i);
                old_weight = aw;
                break;
            }
        }

        let mut used: u32 = 0;
        for i in 0..allocation.len() {
            used += allocation.get(i).unwrap().1;
        }
        let new_used = used - old_weight + weight;
        if new_used > VOTE_POINTS_BUDGET {
            return Err(VaultCampaignError::VoteBudgetExceeded);
        }

        // Apply the delta to the item tally.
        let mut item = items.get(idx).unwrap();
        item.votes = item
            .votes
            .saturating_add(weight as i128)
            .saturating_sub(old_weight as i128);
        items.set(idx, item);
        set_items(&env, &items);

        // Record the caller's allocation for this item.
        match existing_pos {
            Some(i) => allocation.set(i, (item_id, weight)),
            None => allocation.push_back((item_id, weight)),
        }
        env.storage()
            .persistent()
            .set(&allocation_key(&env, &user), &allocation);
        env.storage()
            .persistent()
            .set(&epoch_key(&env, &user), &epoch);

        env.events().publish(
            (symbol_short!("rdmp_vot"), user),
            (allocation, new_used, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Issue #462: roadmap items sorted by total votes, descending.
    pub fn get_roadmap_rankings(env: Env) -> Vec<RoadmapItem> {
        let items = get_items(&env);
        let mut remaining = items.clone();
        let mut sorted: Vec<RoadmapItem> = Vec::new(&env);

        while remaining.len() > 0 {
            let mut best_i: u32 = 0;
            let mut best_votes: i128 = remaining.get(0).unwrap().votes;
            for i in 1..remaining.len() {
                let v = remaining.get(i).unwrap().votes;
                if v > best_votes {
                    best_votes = v;
                    best_i = i;
                }
            }
            sorted.push_back(remaining.get(best_i).unwrap());
            remaining.remove(best_i);
        }
        sorted
    }

    /// Issue #462: all roadmap items in insertion order.
    pub fn get_roadmap_items(env: Env) -> Vec<RoadmapItem> {
        get_items(&env)
    }

    /// Issue #462: a user's current-epoch allocation as `(item_id, points)`
    /// pairs. Returns an empty vec once the caller's stored epoch is stale.
    pub fn get_roadmap_vote_allocation(env: Env, user: Address) -> Vec<(u32, u32)> {
        let stored_epoch: Option<u32> = env.storage().persistent().get(&epoch_key(&env, &user));
        match stored_epoch {
            Some(e) if e == current_epoch(&env) => get_allocation(&env, &user),
            _ => Vec::new(&env),
        }
    }
}
