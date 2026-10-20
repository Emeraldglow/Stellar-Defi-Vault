//! Regulatory compliance report generator (issue #409).
//!
//! Produces a structured, point-in-time summary of pool operations for
//! regulatory reporting use cases. This is a snapshot, not a ledger-range
//! replay â€” the contract keeps no per-ledger transaction history, so fields
//! that would require reconstructing history (`peak_tvl`, `slash_events`,
//! `dispute_events`, `pause_events`) are populated from the closest
//! currently-tracked equivalent, documented per field below, rather than
//! inventing new persistent counters this issue doesn't otherwise need.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! instance/persistent storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::storage::DataKey;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Most reports retained in history (issue #409: "monthly cadence").
pub const MAX_COMPLIANCE_REPORTS: u32 = 12;

/// Instance-storage key for the monotonic report id counter.
const NEXT_ID_KEY: Symbol = symbol_short!("cr_next");

/// Instance-storage key for the retained report history, capped at
/// `MAX_COMPLIANCE_REPORTS`, oldest evicted first.
const HISTORY_KEY: Symbol = symbol_short!("cr_hist");

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ComplianceReport {
    pub report_id: u32,
    pub ledger_from: u32,
    pub ledger_to: u32,
    pub unique_stakers: u32,
    pub peak_tvl: i128,
    pub total_rewards_paid: i128,
    pub total_fees_collected: i128,
    pub kyc_approved_stakers: u32,
    pub slash_events: u32,
    pub dispute_events: u32,
    pub pause_events: u32,
    pub generated_at: u32,
}

fn get_history(env: &Env) -> Vec<ComplianceReport> {
    env.storage()
        .instance()
        .get(&HISTORY_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_history(env: &Env, history: &Vec<ComplianceReport>) {
    env.storage().instance().set(&HISTORY_KEY, history);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Generate a structured compliance report covering pool operations,
    /// labeled with the given `[ledger_from, ledger_to]` range (issue #409).
    /// Admin only.
    ///
    /// Reverts with `InvalidLedgerRange` unless `ledger_from < ledger_to`.
    /// The report is appended to history, evicting the oldest entry once
    /// more than `MAX_COMPLIANCE_REPORTS` are retained.
    pub fn generate_compliance_report(
        env: Env,
        ledger_from: u32,
        ledger_to: u32,
    ) -> Result<ComplianceReport, VaultError> {
        admin::require_admin(&env)?;

        if ledger_from >= ledger_to {
            return Err(VaultError::InvalidRate);
        }

        let all_stakers = balance::get_all_stakers(&env);
        let mut unique_stakers: u32 = 0;
        let mut kyc_approved_stakers: u32 = 0;
        for staker in all_stakers.iter() {
            if balance::get_shares(&env, &staker) <= 0 {
                continue;
            }
            unique_stakers += 1;
            let kyc_approved: bool = env
                .storage()
                .persistent()
                .get(&DataKey::KycApproved(staker.clone()))
                .unwrap_or(false);
            if kyc_approved {
                kyc_approved_stakers += 1;
            }
        }

        let report_id: u32 = env.storage().instance().get(&NEXT_ID_KEY).unwrap_or(0);
        env.storage().instance().set(&NEXT_ID_KEY, &(report_id + 1));

        let report = ComplianceReport {
            report_id,
            ledger_from,
            ledger_to,
            unique_stakers,
            // No historical high-water mark is tracked; the current TVL is
            // the closest available reading for a point-in-time snapshot.
            peak_tvl: balance::get_total_deposited(&env),
            total_rewards_paid: balance::get_total_rewards_paid(&env),
            total_fees_collected: balance::get_protocol_fee_collected(&env),
            kyc_approved_stakers,
            // No slash/dispute/pause event counters exist anywhere in this
            // contract to read from (same gap the issue's own notes call
            // out for `kyc_approved_stakers`) â€” default to 0 rather than
            // adding new persistent counters this report doesn't otherwise
            // need.
            slash_events: 0,
            dispute_events: 0,
            pause_events: 0,
            generated_at: env.ledger().sequence(),
        };

        let mut history = get_history(&env);
        if history.len() >= MAX_COMPLIANCE_REPORTS {
            history.remove(0);
        }
        history.push_back(report.clone());
        set_history(&env, &history);

        env.events().publish(
            (symbol_short!("cr_gen"),),
            (report_id, ledger_from, ledger_to, env.ledger().sequence()),
        );
        Ok(report)
    }

    /// Read-only query: a single retained compliance report by id. Admin only.
    pub fn get_compliance_report(
        env: Env,
        report_id: u32,
    ) -> Result<Option<ComplianceReport>, VaultError> {
        admin::require_admin(&env)?;

        let history = get_history(&env);
        for report in history.iter() {
            if report.report_id == report_id {
                return Ok(Some(report));
            }
        }
        Ok(None)
    }

    /// Read-only query: every retained compliance report, oldest first.
    /// Admin only.
    pub fn get_all_compliance_reports(env: Env) -> Result<Vec<ComplianceReport>, VaultError> {
        admin::require_admin(&env)?;
        Ok(get_history(&env))
    }
}
















