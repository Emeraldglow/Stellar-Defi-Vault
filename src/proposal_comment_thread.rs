//! Stake-weighted discussion threads on governance proposals (issue #375).
//!
//! Builds on the governance proposal storage from issue #160/#216
//! (`GovernanceProposal`, `balance::get_proposal()`) — voting there happens
//! silently with no on-chain discussion. This module adds a comment thread
//! per proposal id, ranked by the commenter's staked shares rather than post
//! order, so a higher-conviction voice surfaces first.
//!
//! # Design
//!
//! `stake_weight` is snapshotted from the author's current shares at post
//! time, same convention `GovernanceProposal` voting already uses ("not
//! adjusted retroactively if the staker's position changes afterward").
//! Comments are kept in a single stake-sorted `Vec`, using the same bounded
//! insertion-ranking idiom `competitive_season::top_stakers()` uses for its
//! leaderboard: once a proposal's thread is at `MAX_COMMENTS_PER_PROPOSAL`,
//! a new comment displaces the current lowest-stake entry rather than
//! growing storage without bound.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, String, Symbol, Vec};

use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::storage::ProposalComment;
use crate::vault::{VaultContract, VaultContractClient};

const COMMENTS_KEY: Symbol = symbol_short!("prop_cmt");

/// Maximum comment length in characters.
pub const MAX_COMMENT_LENGTH: u32 = 500;
/// Maximum comments kept per proposal thread, highest-stake-first.
pub const MAX_COMMENTS_PER_PROPOSAL: u32 = 50;

fn get_comments(env: &Env, proposal_id: u32) -> Vec<ProposalComment> {
    env.storage()
        .persistent()
        .get(&(COMMENTS_KEY, proposal_id))
        .unwrap_or_else(|| Vec::new(env))
}

fn set_comments(env: &Env, proposal_id: u32, comments: &Vec<ProposalComment>) {
    env.storage()
        .persistent()
        .set(&(COMMENTS_KEY, proposal_id), comments);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Post a stake-weighted comment on a governance proposal's discussion
    /// thread (issue #375). `author` must sign; the proposal must exist.
    pub fn post_proposal_comment(
        env: Env,
        proposal_id: u32,
        author: Address,
        text: String,
    ) -> Result<(), VaultError> {
        author.require_auth();
        balance::get_proposal(&env, proposal_id).ok_or(VaultError::PositionNotFound)?;
        if text.len() > crate::proposal_comment_thread::MAX_COMMENT_LENGTH {
            return Err(VaultError::CommentTooLong);
        }

        let stake_weight = balance::get_shares(&env, &author);
        let comment = ProposalComment {
            author: author.clone(),
            text,
            stake_weight,
            posted_at: env.ledger().sequence(),
        };

        let mut comments = crate::proposal_comment_thread::get_comments(&env, proposal_id);
        let mut inserted = false;
        for i in 0..comments.len() {
            let existing = comments.get(i).unwrap();
            if stake_weight > existing.stake_weight {
                comments.insert(i, comment.clone());
                inserted = true;
                break;
            }
        }
        if !inserted {
            comments.push_back(comment.clone());
        }
        while comments.len() > crate::proposal_comment_thread::MAX_COMMENTS_PER_PROPOSAL {
            comments.pop_back();
        }
        crate::proposal_comment_thread::set_comments(&env, proposal_id, &comments);

        events::proposal_comment_posted(&env, &author, proposal_id, stake_weight, comment.posted_at);
        Ok(())
    }

    /// A proposal's comment thread, highest-stake-first (issue #375).
    pub fn proposal_comment_thread(env: Env, proposal_id: u32) -> Vec<ProposalComment> {
        crate::proposal_comment_thread::get_comments(&env, proposal_id)
    }
}
