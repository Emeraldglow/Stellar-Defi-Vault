//! Proposal cancellation by the proposer.
//!
//! Pairs with an admin-configurable proposal bond: when `set_proposal_bond`
//! is set above zero, `create_proposal` (in `vault.rs`) pulls `bond_amount`
//! from the proposer — crediting what a fee-on-transfer token actually
//! delivered, via `transfer_safety::pull_tokens`, the same safeguard
//! `stake()` uses — and this module records who proposed it, when, and how
//! much was actually collected. The proposer alone may then withdraw their
//! own proposal with `cancel_proposal` while it is still early enough to do
//! so, getting the bond back.
//!
//! # Cancellation window
//! Allowed only while *both* hold:
//! - no vote has been cast yet (`votes_for == 0 && votes_against == 0` on
//!   the proposal), and
//! - if a cutoff is configured (`set_cancellation_cutoff`; `0`, the
//!   default, means no extra time limit beyond the no-votes-yet gate), the
//!   proposal was created no more than `cancellation_cutoff_ledgers` ago.
//!
//! Once either closes, the proposal can only run its normal course (vote,
//! then `enact_proposal`) — there is no cancellation after that point.
//!
//! # Effect
//! Cancelling marks the proposal so `vote()`, `batch_vote()`, and
//! `enact_proposal()` all refuse to touch it again, decrements the open-
//! proposal count (freeing the slot the same way enactment does), refunds
//! the bond actually collected (if any), and emits `proposal_cancelled`.
//! This is permanent — there is no "un-cancel".
//!
//! # Storage
//! `DataKey`/`VaultError` both sit at Soroban's 50-variant cap, so this
//! uses raw `Symbol`-keyed storage and its own error type, matching
//! `ttl_management.rs`/`user_ttl.rs`.

use soroban_sdk::{
    contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
};

use crate::admin;
use crate::balance;
use crate::events;
use crate::vault::{VaultContract, VaultContractClient};

const BOND_AMOUNT_KEY: Symbol = symbol_short!("pc_bond");
const CUTOFF_KEY: Symbol = symbol_short!("pc_cut");
/// Persistent key prefix: `(META_KEY, proposal_id)` -> `ProposalMeta`.
const META_KEY: Symbol = symbol_short!("pc_meta");
/// Persistent key prefix: `(CANCELLED_KEY, proposal_id)` -> `bool`.
const CANCELLED_KEY: Symbol = symbol_short!("pc_cncl");

/// Who proposed a governance proposal, when, and how much bond was
/// actually collected from them at creation time.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalMeta {
    pub proposer: Address,
    pub created_at: u32,
    pub bond_paid: i128,
}

/// Errors for the proposal-cancellation entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProposalCancelError {
    NotInitialized = 1,
    Unauthorized = 2,
    ProposalNotFound = 3,
    AlreadyEnacted = 4,
    AlreadyCancelled = 5,
    /// A vote has already been cast, or the configured cancellation cutoff
    /// has already passed.
    WindowClosed = 6,
    ArithmeticError = 7,
}

/// The admin-configured bond `create_proposal` pulls from the proposer.
/// `0` (the default) means no bond is required.
pub fn bond_amount(env: &Env) -> i128 {
    env.storage().instance().get(&BOND_AMOUNT_KEY).unwrap_or(0)
}

/// Crate-internal: overwrites the configured bond amount. Shared by
/// `set_proposal_bond` below and `set_proposal_requirements`
/// (`proposal_requirements.rs`), so both setters agree on the one
/// underlying bond value `create_proposal` actually pulls.
pub(crate) fn set_bond_amount(env: &Env, amount: i128) {
    env.storage().instance().set(&BOND_AMOUNT_KEY, &amount);
}

/// The admin-configured cancellation cutoff, in ledgers since creation.
/// `0` (the default) means no extra time limit beyond the no-votes-yet gate.
pub fn cancellation_cutoff_ledgers(env: &Env) -> u32 {
    env.storage().instance().get(&CUTOFF_KEY).unwrap_or(0)
}

fn get_meta(env: &Env, proposal_id: u32) -> Option<ProposalMeta> {
    env.storage().persistent().get(&(META_KEY, proposal_id))
}

/// Records who proposed `proposal_id`, when, and how much bond was
/// collected. Called once, by `create_proposal`.
pub fn record_proposal_created(
    env: &Env,
    proposal_id: u32,
    proposer: &Address,
    created_at: u32,
    bond_paid: i128,
) {
    env.storage().persistent().set(
        &(META_KEY, proposal_id),
        &ProposalMeta {
            proposer: proposer.clone(),
            created_at,
            bond_paid,
        },
    );
}

/// Whether `proposal_id` has been cancelled. Checked unconditionally by
/// `vote()`, `batch_vote()`, and `enact_proposal()`.
pub fn is_cancelled(env: &Env, proposal_id: u32) -> bool {
    env.storage()
        .persistent()
        .get(&(CANCELLED_KEY, proposal_id))
        .unwrap_or(false)
}

#[contractimpl]
impl VaultContract {
    /// Admin: set the bond `create_proposal` pulls from the proposer. `0`
    /// disables the requirement.
    pub fn set_proposal_bond(
        env: Env,
        admin_addr: Address,
        amount: i128,
    ) -> Result<(), ProposalCancelError> {
        admin_addr.require_auth();
        if admin_addr
            != admin::get_admin(&env).map_err(|_| ProposalCancelError::NotInitialized)?
        {
            return Err(ProposalCancelError::Unauthorized);
        }
        if amount < 0 {
            return Err(ProposalCancelError::ArithmeticError);
        }
        set_bond_amount(&env, amount);
        Ok(())
    }

    /// Read-only: the configured proposal bond. `0` means none required.
    pub fn get_proposal_bond(env: Env) -> i128 {
        bond_amount(&env)
    }

    /// Admin: set how many ledgers after creation a proposal may still be
    /// cancelled, even if no vote has been cast. `0` disables the extra
    /// time limit (cancellation then stays open until the first vote).
    pub fn set_cancellation_cutoff(
        env: Env,
        admin_addr: Address,
        cutoff_ledgers: u32,
    ) -> Result<(), ProposalCancelError> {
        admin_addr.require_auth();
        if admin_addr
            != admin::get_admin(&env).map_err(|_| ProposalCancelError::NotInitialized)?
        {
            return Err(ProposalCancelError::Unauthorized);
        }
        env.storage().instance().set(&CUTOFF_KEY, &cutoff_ledgers);
        Ok(())
    }

    /// Read-only: the configured cancellation cutoff, in ledgers.
    pub fn get_cancellation_cutoff(env: Env) -> u32 {
        cancellation_cutoff_ledgers(&env)
    }

    /// Read-only: the proposer, creation ledger, and bond collected for
    /// `proposal_id`, if it exists and was created after this module was
    /// wired in.
    pub fn get_proposal_meta(env: Env, proposal_id: u32) -> Option<ProposalMeta> {
        get_meta(&env, proposal_id)
    }

    /// Read-only: whether `proposal_id` has been cancelled.
    pub fn is_proposal_cancelled(env: Env, proposal_id: u32) -> bool {
        is_cancelled(&env, proposal_id)
    }

    /// The original proposer withdraws their own proposal, refunding the
    /// bond collected at creation (if any). Only allowed before the first
    /// vote is cast and before the configured cancellation cutoff (if any)
    /// elapses. Permanent — there is no "un-cancel".
    ///
    /// Reverts with `Unauthorized` if the caller is not the proposal's
    /// original proposer, `ProposalNotFound` if it doesn't exist (or
    /// predates this module), `AlreadyEnacted`/`AlreadyCancelled` if it's
    /// too late in either of those ways, and `WindowClosed` if a vote has
    /// already been cast or the cancellation cutoff has passed.
    pub fn cancel_proposal(
        env: Env,
        proposer: Address,
        proposal_id: u32,
    ) -> Result<(), ProposalCancelError> {
        proposer.require_auth();

        let proposal = balance::get_proposal(&env, proposal_id)
            .ok_or(ProposalCancelError::ProposalNotFound)?;
        if proposal.enacted {
            return Err(ProposalCancelError::AlreadyEnacted);
        }
        if is_cancelled(&env, proposal_id) {
            return Err(ProposalCancelError::AlreadyCancelled);
        }

        let meta = get_meta(&env, proposal_id).ok_or(ProposalCancelError::ProposalNotFound)?;
        if meta.proposer != proposer {
            return Err(ProposalCancelError::Unauthorized);
        }

        if proposal.votes_for != 0 || proposal.votes_against != 0 {
            return Err(ProposalCancelError::WindowClosed);
        }
        let cutoff = cancellation_cutoff_ledgers(&env);
        if cutoff > 0 {
            let now = env.ledger().sequence();
            if now > meta.created_at.saturating_add(cutoff) {
                return Err(ProposalCancelError::WindowClosed);
            }
        }

        env.storage()
            .persistent()
            .set(&(CANCELLED_KEY, proposal_id), &true);

        let open_count = balance::get_open_proposal_count(&env);
        if open_count > 0 {
            balance::set_open_proposal_count(&env, open_count - 1);
        }

        if meta.bond_paid > 0 {
            let token_addr =
                Self::token_address(&env).map_err(|_| ProposalCancelError::NotInitialized)?;
            token::Client::new(&env, &token_addr).transfer(
                &env.current_contract_address(),
                &proposer,
                &meta.bond_paid,
            );
        }

        events::proposal_cancelled(
            &env,
            &proposer,
            proposal_id,
            meta.bond_paid,
            env.ledger().sequence(),
        );
        Ok(())
    }
}
