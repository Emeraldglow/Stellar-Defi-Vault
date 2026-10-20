//! Unified position health alert (issue #377).
//!
//! Stakers may be unaware their position needs attention until it's too
//! late — a lock or an admin-configured max stake duration (issue #232)
//! elapsing, a stake-backed loan (issue #261) drifting toward liquidation,
//! or the epoch reward cap (issue's own `epoch_reward_cap.rs`) about to
//! truncate their next claim. This module checks all four conditions in one
//! call and emits a single structured event when any of them fire, instead
//! of requiring a frontend to poll several unrelated getters.
//!
//! Read-only: no storage is written. `needs_attention` is the OR of the four
//! flags so a caller can branch on it alone.

use soroban_sdk::{contractimpl, Address, Env};

use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::storage::{DataKey, PositionHealthAlert};
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

/// How far ahead of an expiry/unlock ledger the alert fires.
const ALERT_WINDOW_LEDGERS: u32 = LEDGERS_PER_DAY * 3;
/// Loan health factor (in bps, 10000 = 100%) below which a loan is flagged at risk.
const LOAN_HEALTH_WARNING_BPS: u32 = 12_000;

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Check every attention-worthy condition for `user`'s position and
    /// return a structured report (issue #377). Emits `position_health_alert`
    /// if any condition is true. Errors with `PositionNotFound` if the user
    /// has no active stake.
    pub fn position_health_alert(env: Env, user: Address) -> Result<PositionHealthAlert, VaultError> {
        let staked_at: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StakedAtLedger(user.clone()))
            .ok_or(VaultError::PositionNotFound)?;
        let current_ledger = env.ledger().sequence();

        // Approaching expiry (issue #232's max stake duration).
        let max_duration = balance::get_max_stake_duration(&env);
        let (approaching_expiry, ledgers_until_expiry) = if max_duration > 0 {
            let expires_at = staked_at.saturating_add(max_duration);
            let remaining = expires_at.saturating_sub(current_ledger);
            (
                remaining > 0 && remaining <= ALERT_WINDOW_LEDGERS,
                Some(remaining),
            )
        } else {
            (false, None)
        };

        // Lock period ending.
        let lock_period: u32 = env
            .storage()
            .instance()
            .get(&DataKey::LockPeriod)
            .unwrap_or(0);
        let (lock_ending_soon, ledgers_until_unlock) = if lock_period > 0 {
            let unlocks_at = staked_at.saturating_add(lock_period);
            let remaining = unlocks_at.saturating_sub(current_ledger);
            (
                remaining > 0 && remaining <= ALERT_WINDOW_LEDGERS,
                Some(remaining),
            )
        } else {
            (false, None)
        };

        // Loan health factor (issue #261's stake-backed loans).
        let (loan_at_risk, loan_health_factor_bps) =
            match (balance::get_loan_config(&env), balance::get_loan(&env, &user)) {
                (Some(loan_config), Some(loan)) => {
                    let debt = loan.principal.saturating_add(loan.interest_accrued);
                    if debt <= 0 {
                        (false, None)
                    } else {
                        let shares = balance::get_shares(&env, &user);
                        let total_shares = balance::get_total_shares(&env);
                        let total_deposited = balance::get_total_deposited(&env);
                        let collateral_value =
                            balance::shares_to_amount(total_shares, total_deposited, shares)
                                .unwrap_or(0);
                        let max_borrowable = collateral_value
                            .saturating_mul(loan_config.max_ltv_bps as i128)
                            / 10_000;
                        let factor_bps = max_borrowable.saturating_mul(10_000) / debt;
                        let factor_bps_u32 = factor_bps.clamp(0, u32::MAX as i128) as u32;
                        (factor_bps_u32 < LOAN_HEALTH_WARNING_BPS, Some(factor_bps_u32))
                    }
                }
                _ => (false, None),
            };

        // Rewards about to be capped (epoch_reward_cap.rs).
        let rewards_near_cap = match crate::vault::VaultContract::get_epoch_cap_remaining(env.clone())
        {
            Ok(remaining) => {
                let accrued = balance::get_accrued_reward(&env, &user);
                accrued > 0 && remaining < accrued
            }
            Err(_) => false,
        };

        let needs_attention =
            approaching_expiry || lock_ending_soon || loan_at_risk || rewards_near_cap;

        if needs_attention {
            events::position_health_alert(
                &env,
                &user,
                approaching_expiry,
                lock_ending_soon,
                loan_at_risk,
                rewards_near_cap,
                current_ledger,
            );
        }

        Ok(PositionHealthAlert {
            user,
            needs_attention,
            approaching_expiry,
            ledgers_until_expiry,
            lock_ending_soon,
            ledgers_until_unlock,
            loan_at_risk,
            loan_health_factor_bps,
            rewards_near_cap,
        })
    }
}
