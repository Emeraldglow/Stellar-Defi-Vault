//! Daily community tip (issue #458).
//!
//! Stakers nominate a short tip/announcement/insight and vote on it with
//! stake-weighted votes. The candidate with the most votes at the daily
//! boundary becomes the featured tip for the next 24 hours, visible to
//! anyone querying the pool.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw
//! `Symbol`-keyed storage, matching `balance.rs` and `content_curation.rs`.
//!
//! Storage keys:
//! - Next tip id: `symbol_short!("tip_nid")` -> `u32` (instance)
//! - Candidates for a day: `(symbol_short!("tip_day"), day)` -> `Vec<TipCandidate>` (persistent)
//! - Last submission day per user: `(symbol_short!("tip_usr"), user)` -> `u32` (persistent)
//! - Vote record: `(symbol_short!("tip_vot"), day, user)` -> `bool` (persistent)
//! - Featured tip for a day: `(symbol_short!("tip_ftd"), day)` -> `TipCandidate` (persistent)
//! - Latest featured tip: `symbol_short!("tip_flt")` -> `TipCandidate` (instance)

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

use crate::balance;
use crate::errors::VaultQuizError;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

/// Maximum characters allowed in a submitted tip's content.
pub const MAX_TIP_CONTENT_LEN: u32 = 140;
/// Maximum open candidates tracked per day.
pub const MAX_TIPS_PER_DAY: u32 = 20;

const NEXT_ID_KEY: Symbol = symbol_short!("tip_nid");
const CANDIDATES_KEY: Symbol = symbol_short!("tip_day");
const LAST_SUBMIT_KEY: Symbol = symbol_short!("tip_usr");
const VOTED_KEY: Symbol = symbol_short!("tip_vot");
const FEATURED_DAY_KEY: Symbol = symbol_short!("tip_ftd");
const FEATURED_LATEST_KEY: Symbol = symbol_short!("tip_flt");

/// A daily tip nomination and its accumulated stake-weighted votes.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TipCandidate {
    pub id: u32,
    pub author: Address,
    pub content: String,
    pub votes: i128,
    pub submitted_for_day: u32,
}

fn current_day(env: &Env) -> u32 {
    env.ledger().sequence() / LEDGERS_PER_DAY
}

fn next_id(env: &Env) -> u32 {
    let id: u32 = env.storage().instance().get(&NEXT_ID_KEY).unwrap_or(0);
    env.storage().instance().set(&NEXT_ID_KEY, &(id + 1));
    id
}

fn get_candidates(env: &Env, day: u32) -> Vec<TipCandidate> {
    env.storage()
        .persistent()
        .get(&(CANDIDATES_KEY, day))
        .unwrap_or(Vec::new(env))
}

fn set_candidates(env: &Env, day: u32, candidates: &Vec<TipCandidate>) {
    env.storage()
        .persistent()
        .set(&(CANDIDATES_KEY, day), candidates);
}

fn last_submit_day(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&(LAST_SUBMIT_KEY, user.clone()))
}

fn set_last_submit_day(env: &Env, user: &Address, day: u32) {
    env.storage()
        .persistent()
        .set(&(LAST_SUBMIT_KEY, user.clone()), &day);
}

fn has_voted(env: &Env, day: u32, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&(VOTED_KEY, day, user.clone()))
        .unwrap_or(false)
}

fn set_voted(env: &Env, day: u32, user: &Address) {
    env.storage()
        .persistent()
        .set(&(VOTED_KEY, day, user.clone()), &true);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #458: Submit a nomination for the daily featured tip. Content
    /// is capped at `MAX_TIP_CONTENT_LEN` characters, and each staker may
    /// submit at most one tip per day. Returns the new tip's id.
    pub fn submit_daily_tip(
        env: Env,
        user: Address,
        content: String,
    ) -> Result<u32, VaultQuizError> {
        user.require_auth();

        if content.len() > MAX_TIP_CONTENT_LEN {
            return Err(VaultQuizError::TipTooLong);
        }
        if balance::get_shares(&env, &user) == 0 {
            return Err(VaultQuizError::ZeroAmount);
        }

        let day = current_day(&env);
        if last_submit_day(&env, &user) == Some(day) {
            return Err(VaultQuizError::AlreadySubmittedToday);
        }

        let mut candidates = get_candidates(&env, day);
        if candidates.len() >= MAX_TIPS_PER_DAY {
            return Err(VaultQuizError::TooManyTipsToday);
        }

        let id = next_id(&env);
        let candidate = TipCandidate {
            id,
            author: user.clone(),
            content,
            votes: 0,
            submitted_for_day: day,
        };
        candidates.push_back(candidate);
        set_candidates(&env, day, &candidates);
        set_last_submit_day(&env, &user, day);

        env.events()
            .publish((symbol_short!("tip_sub"), user), (id, day));

        Ok(id)
    }

    /// Issue #458: Cast a stake-weighted vote for a tip candidate submitted
    /// for the current day. One vote per staker per day.
    pub fn vote_daily_tip(env: Env, voter: Address, tip_id: u32) -> Result<(), VaultQuizError> {
        voter.require_auth();

        let weight = balance::get_shares(&env, &voter);
        if weight == 0 {
            return Err(VaultQuizError::ZeroAmount);
        }

        let day = current_day(&env);
        if has_voted(&env, day, &voter) {
            return Err(VaultQuizError::AlreadyVotedToday);
        }

        let mut candidates = get_candidates(&env, day);
        let mut found = false;
        for i in 0..candidates.len() {
            let candidate = candidates.get(i).unwrap();
            if candidate.id == tip_id {
                let mut updated = candidate.clone();
                updated.votes = updated
                    .votes
                    .checked_add(weight)
                    .ok_or(VaultQuizError::ArithmeticError)?;
                candidates.set(i, updated);
                found = true;
                break;
            }
        }
        if !found {
            return Err(VaultQuizError::TipNotFound);
        }

        set_candidates(&env, day, &candidates);
        set_voted(&env, day, &voter);

        env.events()
            .publish((symbol_short!("tip_vote"), voter), (tip_id, weight, day));

        Ok(())
    }

    /// Issue #458: Finalize the featured tip for `day` — the candidate with
    /// the most stake-weighted votes becomes the featured tip. Callable by
    /// anyone once the day has candidates; re-running it for the same day
    /// simply recomputes the winner from current votes.
    pub fn finalize_daily_tip(env: Env, day: u32) -> Result<TipCandidate, VaultQuizError> {
        let candidates = get_candidates(&env, day);

        let mut winner: Option<TipCandidate> = None;
        for candidate in candidates.iter() {
            let is_better = match &winner {
                Some(current) => candidate.votes > current.votes,
                None => true,
            };
            if is_better {
                winner = Some(candidate);
            }
        }

        let winner = winner.ok_or(VaultQuizError::TipNotFound)?;

        env.storage()
            .persistent()
            .set(&(FEATURED_DAY_KEY, day), &winner);
        env.storage().instance().set(&FEATURED_LATEST_KEY, &winner);

        env.events().publish(
            (symbol_short!("tip_feat"), winner.author.clone()),
            (winner.id, winner.votes, day),
        );

        Ok(winner)
    }

    /// Issue #458: Read-only query for all tip candidates submitted for a
    /// given day.
    pub fn get_daily_tip_candidates(env: Env, day: u32) -> Vec<TipCandidate> {
        crate::daily_community_tip::get_candidates(&env, day)
    }

    /// Issue #458: Read-only query for the currently featured tip (the
    /// winner of the most recently finalized day), if any.
    pub fn get_featured_tip(env: Env) -> Option<TipCandidate> {
        env.storage().instance().get(&FEATURED_LATEST_KEY)
    }

    /// Issue #458: Read-only query for the featured tip of a specific day,
    /// if that day has been finalized.
    pub fn get_featured_tip_for_day(env: Env, day: u32) -> Option<TipCandidate> {
        env.storage().persistent().get(&(FEATURED_DAY_KEY, day))
    }
}
