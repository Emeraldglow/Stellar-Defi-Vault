//! Minimum stake and bond requirement for creating governance proposals.
//!
//! Extends `proposal_cancel.rs`'s bond (the two share one underlying
//! `bond_amount` value — `proposal_cancel::set_bond_amount`) with two more
//! pieces: a minimum stake a proposer must hold to call `create_proposal`
//! at all, and a quorum threshold that decides what happens to the
//! escrowed bond once a proposal finishes its vote.
//!
//! # Minimum stake
//! `create_proposal` already rejects a zero-share caller
//! (`PositionNotFound`). `set_proposal_requirements` adds a configurable
//! floor on top of that: the proposer's staked amount (token-denominated,
//! the same conversion `vote()` uses for weight) must be at least
//! `min_stake_amount` (`0`, the default, means no floor beyond holding
//! *some* position). This is unrelated to `balance::get_min_stake`, which
//! floors a *deposit* amount, not a proposer's standing balance.
//!
//! # Quorum and the bond's fate
//! `enact_proposal` already decides whether a proposal *passed*
//! (`votes_for > votes_against`). This module adds a second, independent
//! question, evaluated at the same time via `settle_bond`: did it reach
//! *quorum* — total participation (`votes_for + votes_against`) at or
//! above `quorum_bps` of the pool's total shares (`0`, the default, means
//! no quorum requirement; every proposal reaches it)?
//!
//! The bond's fate turns on quorum alone, deliberately decoupled from
//! whether the proposal passed or failed:
//! - Reached quorum → refunded to the proposer in full, win or lose. A
//!   proposal that drew real participation and simply lost on the merits
//!   is not a spam proposal, and slashing rules are meant to stay
//!   conservative enough that an honest minority position is never
//!   punished for losing.
//! - Did not reach quorum → slashed to the slash treasury
//!   (`balance::get_slash_treasury`, falling back to the admin if no
//!   treasury is configured). Nobody engaged with it enough to tell
//!   whether it was serious, which is exactly the spam signal the bond
//!   exists to price in.
//!
//! A proposal the proposer cancelled (`proposal_cancel::cancel_proposal`)
//! never reaches `settle_bond`: it is refunded immediately on
//! cancellation, and `enact_proposal` refuses to touch a cancelled
//! proposal at all, so there is no double-refund/slash risk.
//!
//! # Storage
//! `DataKey`/`VaultError` both sit at Soroban's 50-variant cap, so this
//! uses raw `Symbol`-keyed storage and its own error type, matching
//! `ttl_management.rs`/`user_ttl.rs`.

use soroban_sdk::{contracterror, contractimpl, symbol_short, token, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::events;
use crate::vault::proposal_cancel;
use crate::vault::{VaultContract, VaultContractClient};

const MIN_STAKE_KEY: Symbol = symbol_short!("pr_minst");
const QUORUM_BPS_KEY: Symbol = symbol_short!("pr_qbps");
/// Persistent key prefix: `(SETTLED_KEY, proposal_id)` -> `bool`.
const SETTLED_KEY: Symbol = symbol_short!("pr_settl");

/// Errors for the proposal-requirements entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProposalRequirementsError {
    NotInitialized = 1,
    Unauthorized = 2,
    InvalidRequirements = 3,
}

/// The admin-configured minimum staked amount a proposer must hold.
pub fn min_stake_amount(env: &Env) -> i128 {
    env.storage().instance().get(&MIN_STAKE_KEY).unwrap_or(0)
}

/// The admin-configured quorum, in basis points of total pool shares.
pub fn quorum_bps(env: &Env) -> u32 {
    env.storage().instance().get(&QUORUM_BPS_KEY).unwrap_or(0)
}

fn is_settled(env: &Env, proposal_id: u32) -> bool {
    env.storage()
        .persistent()
        .get(&(SETTLED_KEY, proposal_id))
        .unwrap_or(false)
}

fn mark_settled(env: &Env, proposal_id: u32) {
    env.storage()
        .persistent()
        .set(&(SETTLED_KEY, proposal_id), &true);
}

/// Whether `total_votes` meets `quorum_bps_value` against `total_shares`.
/// Always true when no quorum is configured (`quorum_bps_value == 0`).
pub fn meets_quorum(total_votes: i128, total_shares: i128, quorum_bps_value: u32) -> bool {
    if quorum_bps_value == 0 {
        return true;
    }
    if total_shares <= 0 {
        return false;
    }
    match total_votes.checked_mul(10_000) {
        Some(scaled) => scaled >= (quorum_bps_value as i128).saturating_mul(total_shares),
        // Only reachable with an astronomically large `total_votes`; such a
        // proposal has obviously drawn more than enough participation.
        None => true,
    }
}

/// Settles the escrowed bond for `proposal_id`, called once from
/// `enact_proposal` after its final `votes_for`/`votes_against` are known.
/// Refunds the proposer if quorum was reached, otherwise transfers the
/// bond to the slash treasury. A no-op if no bond was ever collected for
/// this proposal, or if it has already been settled.
pub fn settle_bond(env: &Env, proposal_id: u32, votes_for: i128, votes_against: i128) {
    if is_settled(env, proposal_id) {
        return;
    }
    mark_settled(env, proposal_id);

    let meta = match VaultContract::get_proposal_meta(env.clone(), proposal_id) {
        Some(m) if m.bond_paid > 0 => m,
        _ => return,
    };

    let total_votes = votes_for.saturating_add(votes_against);
    let total_shares = balance::get_total_shares(env);
    let reached_quorum = meets_quorum(total_votes, total_shares, quorum_bps(env));

    let token_addr = match VaultContract::token_address(env) {
        Ok(addr) => addr,
        Err(_) => return,
    };
    let client = token::Client::new(env, &token_addr);
    let now = env.ledger().sequence();

    if reached_quorum {
        client.transfer(&env.current_contract_address(), &meta.proposer, &meta.bond_paid);
        events::proposal_bond_refunded(env, &meta.proposer, proposal_id, meta.bond_paid, now);
    } else {
        let destination = balance::get_slash_treasury(env).or_else(|| admin::get_admin(env).ok());
        if let Some(destination) = destination {
            client.transfer(&env.current_contract_address(), &destination, &meta.bond_paid);
        }
        events::proposal_bond_slashed(env, &meta.proposer, proposal_id, meta.bond_paid, now);
    }
}

#[contractimpl]
impl VaultContract {
    /// Admin: set the minimum staked amount a proposer must hold to call
    /// `create_proposal`, and the bond it pulls from them (shared with
    /// `proposal_cancel::set_proposal_bond`). `min_stake == 0` disables the
    /// stake floor; `bond_amount == 0` disables the bond.
    pub fn set_proposal_requirements(
        env: Env,
        admin_addr: Address,
        min_stake: i128,
        bond_amount: i128,
    ) -> Result<(), ProposalRequirementsError> {
        admin_addr.require_auth();
        if admin_addr
            != admin::get_admin(&env).map_err(|_| ProposalRequirementsError::NotInitialized)?
        {
            return Err(ProposalRequirementsError::Unauthorized);
        }
        if min_stake < 0 || bond_amount < 0 {
            return Err(ProposalRequirementsError::InvalidRequirements);
        }
        env.storage().instance().set(&MIN_STAKE_KEY, &min_stake);
        proposal_cancel::set_bond_amount(&env, bond_amount);
        Ok(())
    }

    /// Admin: set the quorum (basis points of total pool shares) a
    /// proposal's total votes must reach for its bond to be refunded
    /// rather than slashed. `0` disables the quorum requirement (every
    /// proposal's bond is refunded).
    pub fn set_proposal_quorum_bps(
        env: Env,
        admin_addr: Address,
        bps: u32,
    ) -> Result<(), ProposalRequirementsError> {
        admin_addr.require_auth();
        if admin_addr
            != admin::get_admin(&env).map_err(|_| ProposalRequirementsError::NotInitialized)?
        {
            return Err(ProposalRequirementsError::Unauthorized);
        }
        if bps > 10_000 {
            return Err(ProposalRequirementsError::InvalidRequirements);
        }
        env.storage().instance().set(&QUORUM_BPS_KEY, &bps);
        Ok(())
    }

    /// Read-only: the configured minimum stake to create a proposal.
    pub fn get_min_proposal_stake(env: Env) -> i128 {
        min_stake_amount(&env)
    }

    /// Read-only: the configured quorum, in basis points of total shares.
    pub fn get_proposal_quorum_bps(env: Env) -> u32 {
        quorum_bps(&env)
    }

    /// Read-only: whether `proposal_id`'s bond has already been settled
    /// (refunded or slashed).
    pub fn is_proposal_bond_settled(env: Env, proposal_id: u32) -> bool {
        is_settled(&env, proposal_id)
    }
}
