//! Time-locked admin proposal announcements (issue #455).
//!
//! Distinct from the generic admin-action timelock (issue #137). Here the
//! admin publicly announces an intended configuration change well in
//! advance of executing it, giving stakers time to review the change (and
//! exit their position if they disagree) before it takes effect. This is
//! transparency tooling, not a second-key requirement — the same admin can
//! still execute once the delay has elapsed.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw
//! `Symbol`-keyed instance storage, matching `balance.rs` and
//! `vault_extensions_463_466.rs`'s parameter change log.
//!
//! Storage keys:
//! - Next proposal id: `symbol_short!("acp_nid")` -> `u32`
//! - Proposals list (capped at `MAX_ADMIN_PROPOSALS`): `symbol_short!("acp_lst")` -> `Vec<AdminProposal>`

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

use crate::admin;
use crate::errors::VaultQuizError;
use crate::vault::{VaultContract, VaultContractClient};

/// Maximum proposals retained; oldest are dropped once exceeded.
pub const MAX_ADMIN_PROPOSALS: u32 = 50;

const NEXT_ID_KEY: Symbol = symbol_short!("acp_nid");
const PROPOSALS_KEY: Symbol = symbol_short!("acp_lst");

/// A publicly announced, time-locked configuration change.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AdminProposal {
    pub id: u32,
    pub change_type: String,
    pub new_value: i128,
    pub announced_at: u32,
    pub executes_at: u32,
    pub cancelled: bool,
    pub executed: bool,
}

fn next_id(env: &Env) -> u32 {
    let id: u32 = env.storage().instance().get(&NEXT_ID_KEY).unwrap_or(0);
    env.storage().instance().set(&NEXT_ID_KEY, &(id + 1));
    id
}

pub fn get_proposals(env: &Env) -> Vec<AdminProposal> {
    env.storage()
        .instance()
        .get(&PROPOSALS_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_proposals(env: &Env, proposals: &Vec<AdminProposal>) {
    env.storage().instance().set(&PROPOSALS_KEY, proposals);
}

/// Marks every open (neither executed nor cancelled) proposal as cancelled,
/// returning how many were cancelled. Called by `admin::set_admin` on an
/// admin change: an announcement made by the previous admin must not be
/// executable by, or inherited by, the new one.
pub(crate) fn cancel_open_proposals(env: &Env) -> u32 {
    let mut proposals = get_proposals(env);
    let mut cancelled: u32 = 0;
    for i in 0..proposals.len() {
        let mut proposal = proposals.get(i).unwrap();
        if !proposal.executed && !proposal.cancelled {
            proposal.cancelled = true;
            proposals.set(i, proposal);
            cancelled += 1;
        }
    }
    if cancelled > 0 {
        set_proposals(env, &proposals);
    }
    cancelled
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #455: Admin publicly announces an intended configuration
    /// change, executable only after `delay_ledgers` have passed. Returns
    /// the new proposal's id.
    pub fn announce_config_change(
        env: Env,
        admin_addr: Address,
        change_type: String,
        new_value: i128,
        delay_ledgers: u32,
    ) -> Result<u32, VaultQuizError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        let mut proposals = get_proposals(&env);
        while proposals.len() >= MAX_ADMIN_PROPOSALS {
            proposals.remove(0);
        }

        let now = env.ledger().sequence();
        let id = next_id(&env);
        let proposal = AdminProposal {
            id,
            change_type: change_type.clone(),
            new_value,
            announced_at: now,
            executes_at: now.saturating_add(delay_ledgers),
            cancelled: false,
            executed: false,
        };
        proposals.push_back(proposal);
        set_proposals(&env, &proposals);

        env.events().publish(
            (symbol_short!("acp_ann"), admin_addr),
            (id, change_type, new_value, now, delay_ledgers),
        );

        Ok(id)
    }

    /// Issue #455: Admin cancels a previously announced proposal before it
    /// executes.
    pub fn cancel_config_change(
        env: Env,
        admin_addr: Address,
        proposal_id: u32,
    ) -> Result<(), VaultQuizError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        let mut proposals = get_proposals(&env);
        for i in 0..proposals.len() {
            let proposal = proposals.get(i).unwrap();
            if proposal.id == proposal_id {
                if proposal.executed {
                    return Err(VaultQuizError::AdminProposalAlreadyExecuted);
                }
                if proposal.cancelled {
                    return Err(VaultQuizError::AdminProposalAlreadyCancelled);
                }
                let mut updated = proposal.clone();
                updated.cancelled = true;
                proposals.set(i, updated);
                set_proposals(&env, &proposals);

                env.events()
                    .publish((symbol_short!("acp_cncl"), admin_addr), proposal_id);
                return Ok(());
            }
        }
        Err(VaultQuizError::AdminProposalNotFound)
    }

    /// Issue #455: Admin marks an announced proposal as executed, once its
    /// `executes_at` ledger has been reached. The contract does not itself
    /// interpret `change_type` / `new_value` — this records that the
    /// publicly announced change has now taken effect.
    pub fn execute_config_change(
        env: Env,
        admin_addr: Address,
        proposal_id: u32,
    ) -> Result<(), VaultQuizError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        let mut proposals = get_proposals(&env);
        for i in 0..proposals.len() {
            let proposal = proposals.get(i).unwrap();
            if proposal.id == proposal_id {
                if proposal.cancelled {
                    return Err(VaultQuizError::AdminProposalAlreadyCancelled);
                }
                if proposal.executed {
                    return Err(VaultQuizError::AdminProposalAlreadyExecuted);
                }
                if env.ledger().sequence() < proposal.executes_at {
                    return Err(VaultQuizError::AdminProposalNotYetExecutable);
                }
                let mut updated = proposal.clone();
                updated.executed = true;
                proposals.set(i, updated.clone());
                set_proposals(&env, &proposals);

                env.events().publish(
                    (symbol_short!("acp_exec"), admin_addr),
                    (proposal_id, updated.change_type, updated.new_value),
                );
                return Ok(());
            }
        }
        Err(VaultQuizError::AdminProposalNotFound)
    }

    /// Issue #455: Read-only query for a single announced proposal.
    pub fn get_admin_proposal(env: Env, proposal_id: u32) -> Option<AdminProposal> {
        for proposal in crate::time_locked_admin_proposal::get_proposals(&env).iter() {
            if proposal.id == proposal_id {
                return Some(proposal);
            }
        }
        None
    }

    /// Issue #455: Read-only query for all announced proposals (max
    /// `MAX_ADMIN_PROPOSALS`), oldest first.
    pub fn get_all_admin_proposals(env: Env) -> Vec<AdminProposal> {
        crate::time_locked_admin_proposal::get_proposals(&env)
    }
}
