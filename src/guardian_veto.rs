//! Guardian veto over enacted governance proposals.
//!
//! `vault.rs`'s stake-weighted governance proposals already let a
//! sufficiently large staker veto a passed proposal
//! (`veto_proposal`/`get_proposal_veto_status`, issue #241). This adds a
//! second, narrower safeguard: the single `guardian` address
//! (`guardian_pause.rs`) gets a limited window — the proposal's *execution
//! delay* — to veto a proposal that passed its vote, before
//! `enact_proposal` can apply it.
//!
//! # Execution delay
//! `enact_proposal` already requires `now > proposal.ends_at`. For a
//! *passed* proposal (`votes_for > votes_against`) it now also requires
//! `now >= ends_at + execution_delay_ledgers` (admin-configurable via
//! `set_execution_delay`; see `enact_proposal` in `vault.rs` for the actual
//! gate). A failed proposal needs no delay — there is nothing for a veto
//! to block.
//!
//! # Guardian veto
//! `guardian_veto_proposal(guardian, proposal_id, reason_hash)` — named
//! distinctly from the existing `veto_proposal` entrypoint to avoid a name
//! collision with issue #241's stake-weighted veto, which this sits
//! alongside rather than replacing — may only be called:
//! - by the currently configured guardian (`VaultContract::is_guardian`),
//! - on a proposal that passed its vote, strictly after its voting period
//!   ends and strictly before its execution delay elapses,
//! - while the proposal is not already enacted or already guardian-vetoed,
//! - while the admin-configured veto-use budget is not exhausted.
//!
//! A veto is permanent: there is no "un-veto" entrypoint, and
//! `enact_proposal` checks `is_vetoed` unconditionally. It is publicly
//! recorded — the guardian's address, the `reason_hash`, and the ledger are
//! kept in storage (`get_guardian_veto`) and emitted as the
//! `proposal_vetoed` event (`events::guardian_proposal_vetoed`).
//!
//! # Veto budget
//! Per the issue's own concern about concentrating trust in one key, the
//! admin can cap how many proposals the guardian may ever veto in total
//! (`set_guardian_veto_limit`; `0` = unlimited, the default). The count is
//! global rather than tied to a specific guardian identity — rotating the
//! guardian via `set_guardian`/`remove_guardian` does not by itself reset
//! it; the admin can reset it explicitly with `reset_guardian_veto_count`
//! after a rotation if that is the intent.
//!
//! # Storage
//! `DataKey`/`VaultError` both sit at Soroban's 50-variant cap, so this
//! uses raw `Symbol`-keyed storage and its own error type, matching
//! `ttl_management.rs`/`user_ttl.rs`.

use soroban_sdk::{
    contracterror, contractimpl, contracttype, symbol_short, Address, BytesN, Env, Symbol,
};

use crate::admin;
use crate::balance;
use crate::events;
use crate::vault::{VaultContract, VaultContractClient};

const DELAY_KEY: Symbol = symbol_short!("gv_delay");
const LIMIT_KEY: Symbol = symbol_short!("gv_limit");
const COUNT_KEY: Symbol = symbol_short!("gv_count");
/// Persistent key prefix: `(VETO_KEY, proposal_id)` -> `GuardianVetoRecord`.
const VETO_KEY: Symbol = symbol_short!("gv_veto");

/// Execution delay used until the admin configures one: ~1 day of ledgers
/// at 5s/ledger. Until it is raised above `0`, the guardian veto window has
/// zero width and `guardian_veto_proposal` can never succeed — the admin
/// must opt in to a real delay for this safeguard to do anything.
pub const DEFAULT_EXECUTION_DELAY_LEDGERS: u32 = 17_280;

/// A permanent, publicly readable record of a guardian veto.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianVetoRecord {
    pub guardian: Address,
    pub reason_hash: BytesN<32>,
    pub ledger: u32,
}

/// Errors for the guardian-veto entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GuardianVetoError {
    NotInitialized = 1,
    Unauthorized = 2,
    ProposalNotFound = 3,
    /// The proposal failed its vote (`votes_for <= votes_against`) — there
    /// is nothing for a veto to block.
    ProposalNotPassed = 4,
    /// Called before the proposal's voting period has ended, or on/after
    /// its execution delay has already elapsed.
    OutsideVetoWindow = 5,
    /// The proposal was already enacted.
    AlreadyEnacted = 6,
    /// The proposal already carries a guardian veto.
    AlreadyVetoed = 7,
    /// The admin-configured veto-use budget has been spent.
    VetoLimitReached = 8,
}

/// The configured execution delay, in ledgers.
pub fn execution_delay_ledgers(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DELAY_KEY)
        .unwrap_or(DEFAULT_EXECUTION_DELAY_LEDGERS)
}

/// The configured limit on total guardian vetoes. `0` means unlimited.
pub fn guardian_veto_limit(env: &Env) -> u32 {
    env.storage().instance().get(&LIMIT_KEY).unwrap_or(0)
}

/// How many proposals the guardian has vetoed so far.
pub fn guardian_veto_count(env: &Env) -> u32 {
    env.storage().instance().get(&COUNT_KEY).unwrap_or(0)
}

fn get_veto_record(env: &Env, proposal_id: u32) -> Option<GuardianVetoRecord> {
    env.storage().persistent().get(&(VETO_KEY, proposal_id))
}

/// Whether `proposal_id` currently carries a guardian veto. Checked
/// unconditionally by `enact_proposal` — there is no path that clears it
/// once set.
pub fn is_vetoed(env: &Env, proposal_id: u32) -> bool {
    get_veto_record(env, proposal_id).is_some()
}

#[contractimpl]
impl VaultContract {
    /// Admin: set the number of ledgers a *passed* proposal must wait after
    /// its voting period ends before `enact_proposal` can apply it — the
    /// guardian's veto window.
    pub fn set_execution_delay(
        env: Env,
        admin_addr: Address,
        delay_ledgers: u32,
    ) -> Result<(), GuardianVetoError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env).map_err(|_| GuardianVetoError::NotInitialized)? {
            return Err(GuardianVetoError::Unauthorized);
        }
        env.storage().instance().set(&DELAY_KEY, &delay_ledgers);
        Ok(())
    }

    /// Read-only: the configured execution delay, in ledgers.
    pub fn get_execution_delay(env: Env) -> u32 {
        execution_delay_ledgers(&env)
    }

    /// Admin: cap how many proposals the guardian may ever veto in total.
    /// `0` means unlimited (the default).
    pub fn set_guardian_veto_limit(
        env: Env,
        admin_addr: Address,
        max_vetoes: u32,
    ) -> Result<(), GuardianVetoError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env).map_err(|_| GuardianVetoError::NotInitialized)? {
            return Err(GuardianVetoError::Unauthorized);
        }
        env.storage().instance().set(&LIMIT_KEY, &max_vetoes);
        Ok(())
    }

    /// Admin: reset the guardian's used-veto count to zero — e.g. after
    /// rotating to a new guardian address via `set_guardian`.
    pub fn reset_guardian_veto_count(
        env: Env,
        admin_addr: Address,
    ) -> Result<(), GuardianVetoError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env).map_err(|_| GuardianVetoError::NotInitialized)? {
            return Err(GuardianVetoError::Unauthorized);
        }
        env.storage().instance().set(&COUNT_KEY, &0u32);
        Ok(())
    }

    /// Read-only: `(vetoes_used, limit)`; `limit == 0` means unlimited.
    pub fn get_guardian_veto_usage(env: Env) -> (u32, u32) {
        (guardian_veto_count(&env), guardian_veto_limit(&env))
    }

    /// The guardian vetoes a proposal that has passed its vote, during its
    /// execution delay — strictly before `enact_proposal` could otherwise
    /// apply it. Permanent and publicly recorded; see the module
    /// documentation for the exact window and preconditions.
    pub fn guardian_veto_proposal(
        env: Env,
        guardian: Address,
        proposal_id: u32,
        reason_hash: BytesN<32>,
    ) -> Result<(), GuardianVetoError> {
        guardian.require_auth();
        if !Self::is_guardian(env.clone(), guardian.clone()) {
            return Err(GuardianVetoError::Unauthorized);
        }

        let limit = guardian_veto_limit(&env);
        let used = guardian_veto_count(&env);
        if limit > 0 && used >= limit {
            return Err(GuardianVetoError::VetoLimitReached);
        }

        let proposal =
            balance::get_proposal(&env, proposal_id).ok_or(GuardianVetoError::ProposalNotFound)?;
        if proposal.enacted {
            return Err(GuardianVetoError::AlreadyEnacted);
        }
        if is_vetoed(&env, proposal_id) {
            return Err(GuardianVetoError::AlreadyVetoed);
        }
        if proposal.votes_for <= proposal.votes_against {
            return Err(GuardianVetoError::ProposalNotPassed);
        }

        let now = env.ledger().sequence();
        let executable_at = proposal
            .ends_at
            .saturating_add(execution_delay_ledgers(&env));
        if now <= proposal.ends_at || now >= executable_at {
            return Err(GuardianVetoError::OutsideVetoWindow);
        }

        env.storage().persistent().set(
            &(VETO_KEY, proposal_id),
            &GuardianVetoRecord {
                guardian: guardian.clone(),
                reason_hash: reason_hash.clone(),
                ledger: now,
            },
        );
        env.storage().instance().set(&COUNT_KEY, &(used + 1));

        events::guardian_proposal_vetoed(&env, &guardian, proposal_id, &reason_hash, now);
        Ok(())
    }

    /// Read-only: the guardian-veto record for `proposal_id`, if any.
    pub fn get_guardian_veto(env: Env, proposal_id: u32) -> Option<GuardianVetoRecord> {
        get_veto_record(&env, proposal_id)
    }
}
