//! Stake-weighted content curation voting.
//!
//! Stakers submit off-chain content items (identified by hash) and vote to
//! approve or reject them. Vote weight equals the voter's staked amount.
//! Admin closes voting; an event is emitted when the outcome is decided.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this module uses raw
//! `Symbol`-keyed storage, matching `balance.rs` and `commitment.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::stake_quota;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Maximum number of open (not-yet-closed) content items.
pub const MAX_OPEN_ITEMS: u32 = 100;

/// Instance-storage key for the full content items list.
const ITEMS_KEY: Symbol = symbol_short!("cc_items");

/// Persistent-storage key prefix for per-user vote records.
/// Keyed by `(CC_VOTE_KEY, content_hash, voter)`.
const CC_VOTE_KEY: Symbol = symbol_short!("cc_vote");

/// A submitted content item tracked by the curation system.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ContentItem {
    pub content_hash: String,
    pub submitter: Address,
    pub votes_for: i128,
    pub votes_against: i128,
    pub submitted_at: u32,
    pub closed: bool,
}

/// Return the full content items list from instance storage.
fn get_items(env: &Env) -> Vec<ContentItem> {
    env.storage()
        .instance()
        .get(&ITEMS_KEY)
        .unwrap_or(Vec::new(env))
}

/// Persist the full content items list.
fn set_items(env: &Env, items: &Vec<ContentItem>) {
    env.storage().instance().set(&ITEMS_KEY, items);
}

/// Look up a single content item by hash.
fn get_item(env: &Env, content_hash: &String) -> Option<ContentItem> {
    let items = get_items(env);
    for item in items.iter() {
        if &item.content_hash == content_hash {
            return Some(item);
        }
    }
    None
}

/// Whether a user has already voted on the given content item.
fn has_voted(env: &Env, content_hash: &String, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&(CC_VOTE_KEY, content_hash.clone(), user.clone()))
        .unwrap_or(false)
}

/// Record that a user has voted on the given content item.
fn set_voted(env: &Env, content_hash: &String, user: &Address) {
    env.storage()
        .persistent()
        .set(&(CC_VOTE_KEY, content_hash.clone(), user.clone()), &true);
}

/// Resolve a user's current staked token amount from their shares.
///
/// Returns `None` if the user has no position or arithmetic overflows.
fn get_position_amount(env: &Env, user: &Address) -> Option<i128> {
    let shares = balance::get_shares(env, user);
    if shares == 0 {
        return None;
    }
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    balance::shares_to_amount(total_shares, total_deposited, shares)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Submit a new content item for curation voting.
    ///
    /// Any staker with an active position can submit. The content is
    /// identified by its hash â€” the contract never stores actual content.
    /// Reverts if 100 open items already exist.
    pub fn submit_content(
        env: Env,
        user: Address,
        content_hash: String,
    ) -> Result<(), VaultError> {
        user.require_auth();

        // Caller must have an active staking position.
        if balance::get_shares(&env, &user) == 0 {
            return Err(VaultError::PositionNotFound);
        }

        // Issue #339: consuming quota before the open-items check means a
        // caller can't grief the shared item cap without spending their own
        // quota, but still leaves the quota check itself first so an
        // exhausted caller gets `QuotaExhausted` rather than a confusing
        // `MaxPositionsReached` once items happen to be full too.
        stake_quota::consume_quota(&env, &user, 1)?;

        let mut items = get_items(&env);

        // Count open items.
        let mut open_count: u32 = 0;
        for item in items.iter() {
            if !item.closed {
                open_count += 1;
            }
        }
        if open_count >= MAX_OPEN_ITEMS {
            return Err(VaultError::MaxPositionsReached);
        }

        let entry = ContentItem {
            content_hash: content_hash.clone(),
            submitter: user.clone(),
            votes_for: 0,
            votes_against: 0,
            submitted_at: env.ledger().sequence(),
            closed: false,
        };
        items.push_back(entry);
        set_items(&env, &items);

        env.events().publish(
            (symbol_short!("cc_sub"), user),
            (content_hash, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Vote on a content item. Vote weight equals the voter's staked amount.
    ///
    /// `approve = true` counts toward acceptance; `false` counts toward
    /// rejection. One vote per address per content item â€” double voting is
    /// rejected (overwrite not allowed).
    pub fn vote_on_content(
        env: Env,
        user: Address,
        content_hash: String,
        approve: bool,
    ) -> Result<(), VaultError> {
        user.require_auth();

        // Must have an active position whose amount becomes vote weight.
        let weight = get_position_amount(&env, &user).ok_or(VaultError::PositionNotFound)?;

        // Reject double vote.
        if has_voted(&env, &content_hash, &user) {
            return Err(VaultError::TooManyStakers);
        }

        // Find the content item and verify it is still open.
        let mut items = get_items(&env);
        let mut found = false;
        for i in 0..items.len() {
            let item = items.get(i).unwrap();
            if item.content_hash == content_hash {
                if item.closed {
                    return Err(VaultError::NothingToWithdraw);
                }
                let mut updated = item.clone();
                if approve {
                    updated.votes_for = updated
                        .votes_for
                        .checked_add(weight)
                        .ok_or(VaultError::ArithmeticError)?;
                } else {
                    updated.votes_against = updated
                        .votes_against
                        .checked_add(weight)
                        .ok_or(VaultError::ArithmeticError)?;
                }
                items.set(i, updated);
                found = true;
                break;
            }
        }
        if !found {
            return Err(VaultError::PositionNotFound);
        }

        set_items(&env, &items);
        set_voted(&env, &content_hash, &user);

        env.events().publish(
            (symbol_short!("cc_vote"), user),
            (content_hash, approve, weight, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Close voting on a content item. Admin only.
    ///
    /// Emits `content_approved` event when `votes_for > votes_against`.
    pub fn close_content_vote(
        env: Env,
        content_hash: String,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let mut items = get_items(&env);
        let mut found = false;
        for i in 0..items.len() {
            let item = items.get(i).unwrap();
            if item.content_hash == content_hash {
                if item.closed {
                    return Err(VaultError::NothingToWithdraw);
                }
                let mut updated = item.clone();
                updated.closed = true;
                items.set(i, updated.clone());

                if updated.votes_for > updated.votes_against {
                    let admin = admin::get_admin(&env)?;
                    env.events().publish(
                        (symbol_short!("cc_apprv"), admin),
                        (
                            content_hash.clone(),
                            updated.votes_for,
                            updated.votes_against,
                            env.ledger().sequence(),
                        ),
                    );
                }
                found = true;
                break;
            }
        }
        if !found {
            return Err(VaultError::PositionNotFound);
        }

        set_items(&env, &items);
        Ok(())
    }

    /// Read-only query: return a single content item by hash, or `None`.
    pub fn get_content_item(env: Env, content_hash: String) -> Option<ContentItem> {
        get_item(&env, &content_hash)
    }

    /// Read-only query: return all content items (max 100).
    pub fn get_all_content_items(env: Env) -> Vec<ContentItem> {
        get_items(&env)
    }
}















