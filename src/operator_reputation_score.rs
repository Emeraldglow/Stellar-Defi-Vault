//! Cross-pool operator reputation scoring (issue #442).
//!
//! A pool operator's admin address accumulates a reputation score based on
//! how well they manage all their pools: uptime, solvency maintenance,
//! governance participation, slash dispute outcomes, and community ratings.
//! The score is visible to prospective stakers evaluating new pools.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::vault::{VaultContract, LEDGERS_PER_DAY, STELLAR_LEDGERS_PER_YEAR};

const OPERATOR_REP_KEY: Symbol = symbol_short!("op_rep");

/// Raw reputation inputs accumulated per pool operator (admin address).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OperatorReputationData {
    /// Total ledgers the operator's pool has been continuously running.
    pub pool_uptime_ledgers: u32,
    /// Ledgers the pool has remained solvent.
    pub solvency_ledgers: u32,
    /// Percentage of governance proposals voted on, in basis points.
    pub governance_participation_bps: u32,
    /// Number of slash disputes the operator lost.
    pub slash_disputes_lost: u32,
    /// Community rating of the pool, in basis points (from Issue #195).
    pub pool_average_rating_bps: u32,
    /// Age of the pool in days (ledgers / LEDGERS_PER_DAY).
    pub pool_age_days: u32,
}

/// Computed reputation score breakdown returned by `compute_operator_score`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OperatorScore {
    pub admin: Address,
    pub uptime_score: u32,
    pub solvency_score: u32,
    pub governance_score: u32,
    pub dispute_score: u32,
    pub community_score: u32,
    pub total_score: u32,
    pub pools_operated: u32,
}

fn get_rep_data(env: &Env, admin: &Address) -> Option<OperatorReputationData> {
    env.storage()
        .persistent()
        .get(&(OPERATOR_REP_KEY, admin.clone()))
}

fn days_in_ledgers(ledgers: u32) -> u32 {
    ledgers / LEDGERS_PER_DAY
}

/// Component scores, each capped at 25. `total_score` is the sum capped at 100.
fn compute_scores(data: &OperatorReputationData) -> (u32, u32, u32, u32, u32) {
    // uptime_score: min(25, pool_uptime_ledgers / (LEDGERS_PER_YEAR / 25))
    let uptime_score = (data.pool_uptime_ledgers / (STELLAR_LEDGERS_PER_YEAR / 25)).min(25);

    // solvency_score: min(25, days_solvent * 25 / pool_age_days)
    let solvency_score = if data.pool_age_days > 0 && data.solvency_ledgers > 0 {
        let days_solvent = days_in_ledgers(data.solvency_ledgers);
        (days_solvent.saturating_mul(25) / data.pool_age_days).min(25)
    } else {
        0
    };

    // governance_score: min(25, governance_participation_bps / 400)
    let governance_score = (data.governance_participation_bps / 400).min(25);

    // dispute_score: min(25, 25 - slash_disputes_lost * 5), floor 0
    let dispute_score = (25u32.saturating_sub(data.slash_disputes_lost.saturating_mul(5))).min(25);

    // community_score: min(25, pool_average_rating_bps / 400)
    let community_score = (data.pool_average_rating_bps / 400).min(25);

    (
        uptime_score,
        solvency_score,
        governance_score,
        dispute_score,
        community_score,
    )
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Record or update an operator's raw reputation inputs. Admin only.
    /// `pool_uptime_ledgers` is the pool's continuous running time in ledgers;
    /// `solvency_ledgers` is how long it has remained solvent; the remaining
    /// fields are the governance participation rate (bps), slash disputes
    /// lost, community rating (bps), and pool age in days.
    pub fn record_operator_reputation(
        env: Env,
        admin: Address,
        operator: Address,
        data: OperatorReputationData,
    ) -> Result<(), VaultError> {
        admin.require_auth();
        admin::require_admin(&env)?;

        env.storage()
            .persistent()
            .set(&(OPERATOR_REP_KEY, operator.clone()), &data);
        Ok(())
    }

    /// Read-only query: the stored raw reputation inputs for `operator`, if any.
    pub fn get_operator_reputation(env: Env, operator: Address) -> Option<OperatorReputationData> {
        get_rep_data(&env, &operator)
    }

    /// Compute the reputation score for `admin`'s pools. Score is per-pool
    /// then averaged across all pools by the same admin; `pools_operated`
    /// defaults to 1 (pool registry from Issue #260 not yet wired).
    pub fn compute_operator_score(env: Env, operator: Address) -> OperatorScore {
        let (uptime_score, solvency_score, governance_score, dispute_score, community_score) =
            match get_rep_data(&env, &operator) {
                Some(data) => compute_scores(&data),
                None => (0, 0, 0, 0, 0),
            };

        let total_score =
            uptime_score + solvency_score + governance_score + dispute_score + community_score;
        let total_score = total_score.min(100);

        OperatorScore {
            admin: operator.clone(),
            uptime_score,
            solvency_score,
            governance_score,
            dispute_score,
            community_score,
            total_score,
            pools_operated: 1,
        }
    }
}
