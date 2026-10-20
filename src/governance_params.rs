//! Admin-configurable governance quorum and pass threshold (issue #712).
//!
//! `quorum_bps` is the minimum participation, as basis points of the pool's
//! total staked amount, that a proposal's `votes_for + votes_against` must
//! reach. `pass_threshold_bps` is the minimum share of the votes cast, in
//! basis points, that must be `votes_for`. Finalization checks participation
//! against quorum first, then support against the threshold; a threshold of
//! exactly `pass_threshold_bps` passes.
//!
//! The parameters in force when a proposal is created, and the total staked
//! amount quorum is measured against, are snapshotted for that proposal, so
//! changing them or moving the pool size mid-vote does not apply to it. A
//! proposal created while no parameters are configured keeps the original rule
//! (`votes_for > votes_against`, no quorum).
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::vault::{VaultContract, VaultContractClient};

/// Instance-storage key for the current governance parameters.
const PARAMS_KEY: Symbol = symbol_short!("gv_cfg");

/// Persistent-storage key prefix for a proposal's snapshot, keyed by
/// `(SNAPSHOT_KEY, proposal_id)`.
const SNAPSHOT_KEY: Symbol = symbol_short!("gv_snap");

/// Persistent-storage key prefix for the total deposited when a proposal was
/// created, keyed by `(SNAPSHOT_BASE_KEY, proposal_id)`.
const SNAPSHOT_BASE_KEY: Symbol = symbol_short!("gv_base");

const BPS_BASE: i128 = 10_000;

/// Quorum and pass threshold, both in basis points.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct GovernanceParams {
    pub quorum_bps: u32,
    pub pass_threshold_bps: u32,
}

fn get_params(env: &Env) -> Option<GovernanceParams> {
    env.storage().instance().get(&PARAMS_KEY)
}

/// Snapshot the current parameters and total deposited for a newly created
/// proposal. Does nothing when no parameters are configured.
pub fn snapshot(env: &Env, proposal_id: u32) {
    if let Some(params) = get_params(env) {
        env.storage()
            .persistent()
            .set(&(SNAPSHOT_KEY, proposal_id), &params);
        env.storage().persistent().set(
            &(SNAPSHOT_BASE_KEY, proposal_id),
            &balance::get_total_deposited(env),
        );
    }
}

/// Whether a proposal with these tallies passes under the parameters
/// snapshotted for it.
pub fn passes(
    env: &Env,
    proposal_id: u32,
    votes_for: i128,
    votes_against: i128,
) -> Result<bool, VaultError> {
    let Some(params) = env
        .storage()
        .persistent()
        .get::<_, GovernanceParams>(&(SNAPSHOT_KEY, proposal_id))
    else {
        return Ok(votes_for > votes_against);
    };

    let total_votes = votes_for
        .checked_add(votes_against)
        .ok_or(VaultError::ArithmeticError)?;
    if total_votes == 0 {
        return Ok(false);
    }

    // Quorum: total_votes / total_staked >= quorum_bps / 10_000.
    let participation = total_votes
        .checked_mul(BPS_BASE)
        .ok_or(VaultError::ArithmeticError)?;
    let total_staked = env
        .storage()
        .persistent()
        .get::<_, i128>(&(SNAPSHOT_BASE_KEY, proposal_id))
        .unwrap_or(0);
    let required = total_staked
        .checked_mul(params.quorum_bps as i128)
        .ok_or(VaultError::ArithmeticError)?;
    if participation < required {
        return Ok(false);
    }

    // Threshold: votes_for / total_votes >= pass_threshold_bps / 10_000.
    let support = votes_for
        .checked_mul(BPS_BASE)
        .ok_or(VaultError::ArithmeticError)?;
    let needed = total_votes
        .checked_mul(params.pass_threshold_bps as i128)
        .ok_or(VaultError::ArithmeticError)?;
    Ok(support >= needed)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Admin: set the quorum and pass threshold for proposals created from now
    /// on. Proposals already open keep the parameters snapshotted at creation.
    ///
    /// # Authorization
    /// The stored admin must sign.
    ///
    /// # Errors
    /// - `Unauthorized`: `admin` is not the stored admin.
    /// - `InvalidRate`: `quorum_bps` exceeds 10 000, or `pass_threshold_bps` is
    ///   zero or exceeds 10 000.
    ///
    /// # Events
    /// - Topic `("gov_par", admin)` with data
    ///   `(quorum_bps, pass_threshold_bps, ledger_sequence)`.
    pub fn set_governance_params(
        env: Env,
        admin: Address,
        quorum_bps: u32,
        pass_threshold_bps: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        if quorum_bps > BPS_BASE as u32
            || pass_threshold_bps == 0
            || pass_threshold_bps > BPS_BASE as u32
        {
            return Err(VaultError::InvalidRate);
        }
        env.storage().instance().set(
            &PARAMS_KEY,
            &GovernanceParams {
                quorum_bps,
                pass_threshold_bps,
            },
        );
        env.events().publish(
            (symbol_short!("gov_par"), admin),
            (quorum_bps, pass_threshold_bps, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: the governance parameters applied to new proposals. Both
    /// fields are zero until `set_governance_params` is called, in which case
    /// proposals use the original simple-majority rule.
    pub fn get_governance_params(env: Env) -> GovernanceParams {
        get_params(&env).unwrap_or(GovernanceParams {
            quorum_bps: 0,
            pass_threshold_bps: 0,
        })
    }
}
