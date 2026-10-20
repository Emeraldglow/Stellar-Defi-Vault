//! Time-delayed admin recovery.
//!
//! A last-resort escape hatch for the case where the current admin key is
//! permanently lost: anyone may propose a replacement admin, which only takes
//! effect after a long, publicly-visible delay, and the (still-live) current
//! admin can cancel it at any time to prove the key is not actually lost.
//!
//! This is intentionally slow. For a live admin that wants to transfer control
//! promptly, use the ordinary admin-transfer path instead.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so the single active proposal is
//! kept under a raw `Symbol`-keyed instance entry (matching `admin_succession`
//! and the other feature modules).
//!
//! Storage key: `symbol_short!("adm_rec")` -> [`AdminRecoveryProposal`]

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultOpsError;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

const RECOVERY_KEY: Symbol = symbol_short!("adm_rec");

/// Fixed delay between proposing an admin recovery and being able to execute
/// it: 30 days in ledgers. Long enough that a live admin can always cancel.
pub const ADMIN_RECOVERY_DELAY_LEDGERS: u32 = 30 * LEDGERS_PER_DAY;

/// A pending admin-recovery proposal. Only one may be active at a time.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AdminRecoveryProposal {
    /// Address that would become admin once the delay elapses.
    pub new_admin: Address,
    /// Address that created the proposal (for the audit trail).
    pub proposed_by: Address,
    /// Ledger at which the proposal was created.
    pub proposed_at_ledger: u32,
    /// Ledger at or after which `execute_admin_recovery` may run.
    pub executable_at_ledger: u32,
}

fn get_proposal(env: &Env) -> Option<AdminRecoveryProposal> {
    env.storage().instance().get(&RECOVERY_KEY)
}

#[contractimpl]
impl VaultContract {
    /// Anyone may propose replacing the admin with `new_admin`. Starts the
    /// fixed [`ADMIN_RECOVERY_DELAY_LEDGERS`] delay. Reverts with
    /// `RecoveryAlreadyPending` when a proposal is already active, or
    /// `InvalidRecoveryConfig` when `new_admin` is already the admin.
    pub fn propose_admin_recovery(
        env: Env,
        proposer: Address,
        new_admin: Address,
    ) -> Result<(), VaultOpsError> {
        proposer.require_auth();

        let current_admin = admin::get_admin(&env)?;
        if new_admin == current_admin {
            return Err(VaultOpsError::InvalidRecoveryConfig);
        }
        if get_proposal(&env).is_some() {
            return Err(VaultOpsError::RecoveryAlreadyPending);
        }

        let now = env.ledger().sequence();
        let executable_at_ledger = now.saturating_add(ADMIN_RECOVERY_DELAY_LEDGERS);
        let proposal = AdminRecoveryProposal {
            new_admin: new_admin.clone(),
            proposed_by: proposer.clone(),
            proposed_at_ledger: now,
            executable_at_ledger,
        };
        env.storage().instance().set(&RECOVERY_KEY, &proposal);

        env.events()
            .publish((symbol_short!("adm_recp"),), (new_admin, executable_at_ledger));
        Ok(())
    }

    /// Anyone may execute an uncancelled recovery proposal once its delay has
    /// elapsed. Reverts with `RecoveryNotPending` when none is active, or
    /// `RecoveryDelayNotElapsed` before `executable_at_ledger`.
    pub fn execute_admin_recovery(env: Env) -> Result<(), VaultOpsError> {
        let proposal = get_proposal(&env).ok_or(VaultOpsError::RecoveryNotPending)?;
        if env.ledger().sequence() < proposal.executable_at_ledger {
            return Err(VaultOpsError::RecoveryDelayNotElapsed);
        }

        admin::set_admin(&env, &proposal.new_admin);
        env.storage().instance().remove(&RECOVERY_KEY);
        env.events()
            .publish((symbol_short!("adm_rece"),), proposal.new_admin);
        Ok(())
    }

    /// The current admin cancels a pending recovery proposal, proving the key
    /// is still live. Reverts with `RecoveryNotPending` when none is active,
    /// or `Unauthorized` when `admin_addr` is not the current admin.
    pub fn cancel_admin_recovery(env: Env, admin_addr: Address) -> Result<(), VaultOpsError> {
        admin_addr.require_auth();

        let current_admin = admin::get_admin(&env)?;
        if admin_addr != current_admin {
            return Err(VaultOpsError::Unauthorized);
        }
        if get_proposal(&env).is_none() {
            return Err(VaultOpsError::RecoveryNotPending);
        }

        env.storage().instance().remove(&RECOVERY_KEY);
        env.events()
            .publish((symbol_short!("adm_recc"),), admin_addr);
        Ok(())
    }

    /// Read-only: the active recovery proposal, if any.
    pub fn get_admin_recovery_proposal(env: Env) -> Option<AdminRecoveryProposal> {
        get_proposal(&env)
    }
}
