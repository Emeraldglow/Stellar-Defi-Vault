//! Vote changing before a proposal closes.
//!
//! `vault.rs`'s governance `vote()` (issue #216) records only a per-user
//! "has voted" flag (`balance::has_voted`/`set_voted`) — not which side or
//! how much weight was cast — so there was no way to let a voter switch
//! sides without either double-counting or needing to re-derive their
//! weight from their *current* balance (which `vote()` deliberately does
//! not do: "Voting power is not adjusted retroactively if the caller stakes
//! more (or less) after voting"). This module adds that: it records
//! `(support, weight)` for every successful vote (`record_vote`, called
//! from `vote()`), and `change_vote` uses that frozen weight to move the
//! tally from one side to the other.
//!
//! # Why the recorded weight, not current balance
//! If `change_vote` re-derived weight from the caller's current stake, a
//! voter could change their vote after unstaking to shrink (or after
//! staking more to grow) the weight they originally cast with — silently
//! altering the tally by an amount nobody voted with. Reusing the exact
//! weight recorded at the original `vote()` call keeps a side switch
//! neutral: the tally's total (`votes_for + votes_against`) for this voter
//! never changes, only which side it's on.
//!
//! # Limit
//! Each user may change their vote on a given proposal at most
//! `max_vote_changes` times (admin-configurable via
//! `set_max_vote_changes`; defaults to `DEFAULT_MAX_VOTE_CHANGES`), to
//! bound how much a single voter can thrash the tally while a proposal is
//! open.
//!
//! # Storage
//! `DataKey`/`VaultError` both sit at Soroban's 50-variant cap, so this
//! uses raw `Symbol`-keyed storage and its own error type, matching
//! `ttl_management.rs`/`user_ttl.rs`.

use soroban_sdk::{contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::events;
use crate::vault::{VaultContract, VaultContractClient};

/// Persistent key prefix: `(RECORD_KEY, proposal_id, user)` -> `VoteRecord`.
const RECORD_KEY: Symbol = symbol_short!("vc_rec");
/// Persistent key prefix: `(COUNT_KEY, proposal_id, user)` -> `u32`.
const COUNT_KEY: Symbol = symbol_short!("vc_cnt");
/// Instance key: the admin-configured per-user, per-proposal change limit.
const MAX_CHANGES_KEY: Symbol = symbol_short!("vc_max");

/// Limit used until the admin configures one.
pub const DEFAULT_MAX_VOTE_CHANGES: u32 = 3;

/// What a user voted and how much weight they cast, frozen at vote time.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteRecord {
    pub support: bool,
    pub weight: i128,
}

/// Errors for the vote-changing entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VoteChangeError {
    NotInitialized = 1,
    Unauthorized = 2,
    ProposalNotFound = 3,
    /// The proposal has already ended or been enacted.
    ProposalClosed = 4,
    /// The caller has no recorded vote on this proposal to change.
    NotVoted = 5,
    /// `support` matches the already-recorded side; nothing to change.
    NoChange = 6,
    /// The admin-configured per-proposal change limit has been reached.
    ChangeLimitReached = 7,
    ArithmeticError = 8,
}

fn get_record(env: &Env, proposal_id: u32, user: &Address) -> Option<VoteRecord> {
    env.storage()
        .persistent()
        .get(&(RECORD_KEY, proposal_id, user.clone()))
}

/// Records `user`'s `(support, weight)` for `proposal_id`, overwriting any
/// prior record. Called by `vote()` on every successful vote, and by
/// `change_vote` to update the recorded side (the weight itself is carried
/// over unchanged).
pub fn record_vote(env: &Env, user: &Address, proposal_id: u32, support: bool, weight: i128) {
    env.storage().persistent().set(
        &(RECORD_KEY, proposal_id, user.clone()),
        &VoteRecord { support, weight },
    );
}

fn change_count(env: &Env, proposal_id: u32, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&(COUNT_KEY, proposal_id, user.clone()))
        .unwrap_or(0)
}

fn max_changes(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&MAX_CHANGES_KEY)
        .unwrap_or(DEFAULT_MAX_VOTE_CHANGES)
}

#[contractimpl]
impl VaultContract {
    /// Admin: set how many times a user may change their vote on a single
    /// proposal.
    pub fn set_max_vote_changes(
        env: Env,
        admin_addr: Address,
        limit: u32,
    ) -> Result<(), VoteChangeError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env).map_err(|_| VoteChangeError::NotInitialized)? {
            return Err(VoteChangeError::Unauthorized);
        }
        env.storage().instance().set(&MAX_CHANGES_KEY, &limit);
        Ok(())
    }

    /// Read-only: the configured per-proposal vote-change limit.
    pub fn get_max_vote_changes(env: Env) -> u32 {
        max_changes(&env)
    }

    /// Read-only: how many times `user` has changed their vote on
    /// `proposal_id` so far.
    pub fn get_vote_change_count(env: Env, proposal_id: u32, user: Address) -> u32 {
        change_count(&env, proposal_id, &user)
    }

    /// Read-only: `user`'s recorded `(support, weight)` on `proposal_id`,
    /// if they have voted.
    pub fn get_vote_record(env: Env, proposal_id: u32, user: Address) -> Option<VoteRecord> {
        get_record(&env, proposal_id, &user)
    }

    /// Change the caller's vote on an open proposal to `support`, moving
    /// their *originally recorded* weight — not their current balance —
    /// from one side of the tally to the other.
    ///
    /// Reverts with `ProposalNotFound` if the proposal doesn't exist,
    /// `ProposalClosed` if voting has ended or it was already enacted,
    /// `NotVoted` if the caller never voted on it, `NoChange` if `support`
    /// already matches their recorded side, `ChangeLimitReached` if they've
    /// already used up `get_max_vote_changes()` changes on this proposal,
    /// and `Unauthorized` if the caller has delegated their vote weight
    /// away (matching `vote()`'s own delegate check).
    pub fn change_vote(
        env: Env,
        user: Address,
        proposal_id: u32,
        support: bool,
    ) -> Result<(), VoteChangeError> {
        user.require_auth();

        if balance::get_vote_delegate(&env, &user).is_some() {
            return Err(VoteChangeError::Unauthorized);
        }

        let mut proposal =
            balance::get_proposal(&env, proposal_id).ok_or(VoteChangeError::ProposalNotFound)?;
        if proposal.enacted || env.ledger().sequence() > proposal.ends_at {
            return Err(VoteChangeError::ProposalClosed);
        }

        let record = get_record(&env, proposal_id, &user).ok_or(VoteChangeError::NotVoted)?;
        if record.support == support {
            return Err(VoteChangeError::NoChange);
        }

        let used = change_count(&env, proposal_id, &user);
        if used >= max_changes(&env) {
            return Err(VoteChangeError::ChangeLimitReached);
        }

        // Move the originally recorded weight from the old side to the new
        // one. The weight itself never changes — only which side of the
        // tally it counts toward — so this voter's contribution to
        // `votes_for + votes_against` is exactly the same before and after.
        if record.support {
            proposal.votes_for = proposal
                .votes_for
                .checked_sub(record.weight)
                .ok_or(VoteChangeError::ArithmeticError)?;
            proposal.votes_against = proposal
                .votes_against
                .checked_add(record.weight)
                .ok_or(VoteChangeError::ArithmeticError)?;
        } else {
            proposal.votes_against = proposal
                .votes_against
                .checked_sub(record.weight)
                .ok_or(VoteChangeError::ArithmeticError)?;
            proposal.votes_for = proposal
                .votes_for
                .checked_add(record.weight)
                .ok_or(VoteChangeError::ArithmeticError)?;
        }
        balance::set_proposal(&env, proposal_id, &proposal);

        record_vote(&env, &user, proposal_id, support, record.weight);
        let new_count = used + 1;
        env.storage()
            .persistent()
            .set(&(COUNT_KEY, proposal_id, user.clone()), &new_count);

        events::vote_changed(
            &env,
            &user,
            proposal_id,
            support,
            record.weight,
            new_count,
            env.ledger().sequence(),
        );
        Ok(())
    }
}
