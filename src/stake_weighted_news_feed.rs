//! Stake-weighted news feed (issue #451).
//!
//! Community curated announcements where stakers vote to promote/suppress.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Bytes, Env, String, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::vault::VaultContract;

const NEWS_ITEMS_KEY: Symbol = symbol_short!("news_itm");
const NEWS_COUNT_KEY: Symbol = symbol_short!("news_cnt");
const NEWS_VOTE_KEY: Symbol = symbol_short!("news_vt");

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct NewsItem {
    pub id: u32,
    pub author: Address,
    pub title: String,
    pub content_hash: Bytes,
    pub votes_promote: i128,
    pub votes_suppress: i128,
    pub promoted: bool,
    pub submitted_at: u32,
}

fn get_news_items(env: &Env) -> Vec<NewsItem> {
    env.storage()
        .instance()
        .get(&NEWS_ITEMS_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_news_items(env: &Env, items: &Vec<NewsItem>) {
    env.storage().instance().set(&NEWS_ITEMS_KEY, items);
}

fn get_news_count(env: &Env) -> u32 {
    env.storage().instance().get(&NEWS_COUNT_KEY).unwrap_or(0)
}

fn set_news_count(env: &Env, count: u32) {
    env.storage().instance().set(&NEWS_COUNT_KEY, &count);
}

fn has_voted(env: &Env, item_id: u32, voter: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&(NEWS_VOTE_KEY, item_id, voter.clone()))
        .unwrap_or(false)
}

fn set_voted(env: &Env, item_id: u32, voter: &Address) {
    env.storage()
        .persistent()
        .set(&(NEWS_VOTE_KEY, item_id, voter.clone()), &true);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Any staker can submit; title max 80 chars
    pub fn submit_news_item(
        env: Env,
        user: Address,
        title: String,
        content_hash: Bytes,
    ) -> Result<u32, VaultError> {
        user.require_auth();
        if balance::get_shares(&env, &user) == 0 {
            return Err(VaultError::PositionNotFound);
        }
        if title.len() > 80 {
            return Err(VaultError::DescriptionTooLong);
        }
        let mut items = get_news_items(&env);
        // Enforce max 50 total items (promoted + pending). Suppressed items are not stored, so we just cap at 50.
        if items.len() >= 50 {
            // Drop oldest promoted == false ? For now drop oldest overall
            // Spec says oldest suppressed dropped first, but suppressed are removed, so just remove oldest pending.
            // We'll remove oldest non-promoted first, else oldest.
            let mut oldest_idx: Option<u32> = None;
            for i in 0..items.len() {
                if !items.get(i).unwrap().promoted {
                    oldest_idx = Some(i);
                    break;
                }
            }
            let idx = oldest_idx.unwrap_or(0);
            items.remove(idx);
        }
        let id = get_news_count(&env);
        let item = NewsItem {
            id,
            author: user.clone(),
            title: title.clone(),
            content_hash: content_hash.clone(),
            votes_promote: 0,
            votes_suppress: 0,
            promoted: false,
            submitted_at: env.ledger().sequence(),
        };
        items.push_back(item);
        set_news_items(&env, &items);
        set_news_count(&env, id + 1);
        env.events().publish(
            (symbol_short!("news_sub"), user),
            (id, title, env.ledger().sequence()),
        );
        Ok(id)
    }

    /// Stake-weighted vote; one per address per item
    pub fn vote_news_item(
        env: Env,
        user: Address,
        item_id: u32,
        promote: bool,
    ) -> Result<(), VaultError> {
        user.require_auth();
        let weight = balance::get_shares(&env, &user);
        if weight == 0 {
            return Err(VaultError::PositionNotFound);
        }
        if has_voted(&env, item_id, &user) {
            return Err(VaultError::TooManyStakers);
        }
        let mut items = get_news_items(&env);
        let mut found_idx: Option<u32> = None;
        for i in 0..items.len() {
            if items.get(i).unwrap().id == item_id {
                found_idx = Some(i);
                break;
            }
        }
        let idx = found_idx.ok_or(VaultError::PositionNotFound)?;
        let mut item = items.get(idx).unwrap();
        if item.promoted {
            // Already promoted, no further voting? Allow but no effect
        }
        if promote {
            item.votes_promote = item.votes_promote.saturating_add(weight);
        } else {
            item.votes_suppress = item.votes_suppress.saturating_add(weight);
        }
        // Auto-promotes when votes_promote > votes_suppress *2
        if !item.promoted && item.votes_promote > item.votes_suppress * 2 {
            item.promoted = true;
            env.events().publish(
                (symbol_short!("news_prom"),),
                (item_id, item.author.clone(), item.votes_promote, env.ledger().sequence()),
            );
        }
        items.set(idx, item);
        set_news_items(&env, &items);
        set_voted(&env, item_id, &user);
        env.events().publish(
            (symbol_short!("news_vt"), user),
            (item_id, promote, weight, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Only promoted items, newest first, max 20
    pub fn get_promoted_feed(env: Env) -> Vec<NewsItem> {
        let items = get_news_items(&env);
        // Collect promoted
        let mut promoted = Vec::new(&env);
        for it in items.iter() {
            if it.promoted {
                promoted.push_back(it);
            }
        }
        // Sort newest first: reverse order (items stored oldest first)
        let mut reversed = Vec::new(&env);
        for i in 0..promoted.len() {
            let idx = promoted.len() - 1 - i;
            reversed.push_back(promoted.get(idx).unwrap());
            if reversed.len() >= 20 {
                break;
            }
        }
        reversed
    }

    /// Items awaiting promotion vote
    pub fn get_pending_items(env: Env) -> Vec<NewsItem> {
        let items = get_news_items(&env);
        let mut pending = Vec::new(&env);
        for it in items.iter() {
            if !it.promoted {
                pending.push_back(it);
            }
        }
        pending
    }

    /// Admin override to suppress any item (permanently removed)
    pub fn suppress_item(env: Env, admin: Address, item_id: u32) -> Result<(), VaultError> {
        admin.require_auth();
        admin::require_admin(&env)?;
        let mut items = get_news_items(&env);
        let mut found_idx: Option<u32> = None;
        for i in 0..items.len() {
            if items.get(i).unwrap().id == item_id {
                found_idx = Some(i);
                break;
            }
        }
        let idx = found_idx.ok_or(VaultError::PositionNotFound)?;
        items.remove(idx);
        set_news_items(&env, &items);
        env.events().publish(
            (symbol_short!("news_sup"), admin),
            (item_id, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Get all news items (for debugging)
    pub fn get_all_news_items(env: Env) -> Vec<NewsItem> {
        get_news_items(&env)
    }
}
