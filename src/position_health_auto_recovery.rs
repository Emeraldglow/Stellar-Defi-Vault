//! Position health auto-recovery (issue #459).
//!
//! Builds on issue #283 (position health alerts) and issue #229 / #261 (health
//! factor for stake-backed loans). Stakers pre-configure a protective action
//! that a keeper can execute on their behalf once their position health drops
//! below a chosen threshold — auto-claiming to reduce reward overhang,
//! auto-repaying part of a loan, or auto-unstaking part of the position to pull
//! the loan-to-value ratio back down.
//!
//! # Health measurement
//!
//! Health is the loan health factor in basis points, computed exactly the way
//! `position_health_alert()` (issue #377) computes it:
//! `max_borrowable * 10_000 / debt`, where `max_borrowable` is the borrower's
//! collateral value scaled by the configured `max_ltv_bps`. `10_000` means the
//! debt is exactly at the borrowing limit; lower is worse. A position with no
//! loan (or no debt) has unbounded health and can never trigger recovery.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs` and the other feature modules.
//!
//! - Per-user config: `(Symbol::new(env, "rec_cfg"), user)` -> `RecoveryConfig`
//! - Per-user last recovery ledger: `(Symbol::new(env, "rec_last"), user)` -> `u32`

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol};

use crate::balance;
use crate::errors::VaultCampaignError;
use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

/// One recovery per user per day (17280 ledgers) — prevents repeated triggering.
pub const RECOVERY_COOLDOWN_LEDGERS: u32 = LEDGERS_PER_DAY;

/// Keeper incentive: 0.5% of the recovery action value, in basis points.
pub const KEEPER_INCENTIVE_BPS: i128 = 50;

fn config_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "rec_cfg"), user.clone())
}

fn last_recovery_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "rec_last"), user.clone())
}

/// Protective action executed when a position's health drops below its trigger.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum RecoveryAction {
    /// Claim the user's accrued rewards to reduce reward overhang.
    AutoClaim,
    /// Repay up to `action_amount` of the user's loan from their collateral.
    AutoRepayLoan,
    /// Unstake `action_amount` shares; proceeds pay down the loan (reducing LTV)
    /// or, if no loan is open, are returned to the user.
    AutoUnstakePartial,
}

/// Per-user auto-recovery configuration.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryConfig {
    /// Health factor in bps at or below which recovery may be executed.
    pub trigger_health_bps: u32,
    /// Which protective action to run.
    pub action: RecoveryAction,
    /// Token amount the action operates on (shares for unstake, tokens for repay;
    /// ignored by `AutoClaim`).
    pub action_amount: i128,
    /// Whether the config is currently armed.
    pub active: bool,
}

pub fn get_recovery_config(env: &Env, user: &Address) -> Option<RecoveryConfig> {
    env.storage().persistent().get(&config_key(env, user))
}

fn set_recovery_config_raw(env: &Env, user: &Address, cfg: &RecoveryConfig) {
    env.storage().persistent().set(&config_key(env, user), cfg);
}

fn remove_recovery_config(env: &Env, user: &Address) {
    env.storage().persistent().remove(&config_key(env, user));
}

pub fn get_last_recovery_ledger(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&last_recovery_key(env, user))
        .unwrap_or(0)
}

fn set_last_recovery_ledger(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&last_recovery_key(env, user), &ledger);
}

/// Current loan health factor for `user`, in bps. `None` when the position has
/// no loan or no outstanding debt (health is then unbounded).
pub fn current_health_bps(env: &Env, user: &Address) -> Option<u32> {
    let loan_config = balance::get_loan_config(env)?;
    let loan = balance::get_loan(env, user)?;
    let debt = loan.principal.saturating_add(loan.interest_accrued);
    if debt <= 0 {
        return None;
    }
    let shares = balance::get_shares(env, user);
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    let collateral_value =
        balance::shares_to_amount(total_shares, total_deposited, shares).unwrap_or(0);
    let max_borrowable =
        collateral_value.saturating_mul(loan_config.max_ltv_bps as i128) / 10_000;
    let factor_bps = max_borrowable.saturating_mul(10_000) / debt;
    Some(factor_bps.clamp(0, u32::MAX as i128) as u32)
}

fn token_address(env: &Env) -> Result<Address, VaultCampaignError> {
    env.storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(VaultCampaignError::NotInitialized)
}

/// Reduce `loan` by `amount`, interest first then principal. Returns the loan to
/// storage, removing it entirely once fully repaid.
fn apply_loan_repayment(env: &Env, user: &Address, loan: &mut crate::storage::Loan, amount: i128) {
    let pay_interest = amount.min(loan.interest_accrued.max(0));
    loan.interest_accrued -= pay_interest;
    let pay_principal = (amount - pay_interest).min(loan.principal.max(0));
    loan.principal -= pay_principal;
    if loan.principal.saturating_add(loan.interest_accrued) <= 0 {
        balance::remove_loan(env, user);
    } else {
        balance::set_loan(env, user, loan);
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #459: arm an auto-recovery config for `user`.
    pub fn set_recovery_config(
        env: Env,
        user: Address,
        trigger_health_bps: u32,
        action: RecoveryAction,
        action_amount: i128,
    ) -> Result<(), VaultCampaignError> {
        user.require_auth();

        if trigger_health_bps == 0 {
            return Err(VaultCampaignError::InvalidRecoveryConfig);
        }
        if action_amount < 0 {
            return Err(VaultCampaignError::InvalidRecoveryConfig);
        }
        if matches!(
            action,
            RecoveryAction::AutoRepayLoan | RecoveryAction::AutoUnstakePartial
        ) && action_amount == 0
        {
            return Err(VaultCampaignError::InvalidRecoveryConfig);
        }

        set_recovery_config_raw(
            &env,
            &user,
            &RecoveryConfig {
                trigger_health_bps,
                action,
                action_amount,
                active: true,
            },
        );

        env.events().publish(
            (symbol_short!("rec_set"), user),
            (trigger_health_bps, action_amount, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Issue #459: user removes their auto-recovery config.
    pub fn cancel_recovery_config(env: Env, user: Address) -> Result<(), VaultCampaignError> {
        user.require_auth();
        if get_recovery_config(&env, &user).is_none() {
            return Err(VaultCampaignError::RecoveryNotConfigured);
        }
        remove_recovery_config(&env, &user);
        env.events().publish(
            (symbol_short!("rec_cncl"), user),
            env.ledger().sequence(),
        );
        Ok(())
    }

    /// Issue #459: read a user's auto-recovery config.
    pub fn get_recovery_config(env: Env, user: Address) -> Option<RecoveryConfig> {
        get_recovery_config(&env, &user)
    }

    /// Issue #459: current loan health factor (bps) for `user`, or `None` when
    /// the position carries no debt.
    pub fn position_health_bps(env: Env, user: Address) -> Option<u32> {
        current_health_bps(&env, &user)
    }

    /// Issue #459: a keeper checks `user`'s position health and, if it has
    /// dropped to or below the configured trigger, executes the configured
    /// recovery action. The keeper earns 0.5% of the recovery action value as
    /// an incentive, and one recovery is allowed per user per 17280 ledgers.
    ///
    /// Returns the recovery action value (tokens claimed / repaid / unstaked).
    pub fn check_and_recover(
        env: Env,
        keeper: Address,
        user: Address,
    ) -> Result<i128, VaultCampaignError> {
        keeper.require_auth();

        let cfg = get_recovery_config(&env, &user)
            .filter(|c| c.active)
            .ok_or(VaultCampaignError::RecoveryNotConfigured)?;

        let current_ledger = env.ledger().sequence();
        let last = get_last_recovery_ledger(&env, &user);
        if last != 0 && current_ledger < last.saturating_add(RECOVERY_COOLDOWN_LEDGERS) {
            return Err(VaultCampaignError::RecoveryOnCooldown);
        }

        let health = current_health_bps(&env, &user)
            .ok_or(VaultCampaignError::RecoveryNotTriggered)?;
        if health > cfg.trigger_health_bps {
            return Err(VaultCampaignError::RecoveryNotTriggered);
        }

        let token_addr = token_address(&env)?;
        let contract = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_addr);

        let action_value: i128 = match cfg.action {
            RecoveryAction::AutoClaim => {
                let accrued = balance::get_accrued_reward(&env, &user);
                if accrued <= 0 {
                    return Err(VaultCampaignError::RecoveryNotTriggered);
                }
                balance::set_accrued_reward(&env, &user, 0);
                let total_paid = balance::get_total_rewards_paid(&env);
                balance::set_total_rewards_paid(&env, total_paid.saturating_add(accrued));
                token_client.transfer(&contract, &user, &accrued);
                accrued
            }
            RecoveryAction::AutoRepayLoan => {
                let mut loan =
                    balance::get_loan(&env, &user).ok_or(VaultCampaignError::NoActiveLoan)?;
                let debt = loan.principal.saturating_add(loan.interest_accrued);
                if debt <= 0 {
                    return Err(VaultCampaignError::NoActiveLoan);
                }
                let target = cfg.action_amount.min(debt);

                // Repay from the borrower's own collateral: burn shares worth
                // `target` tokens; the tokens stay in the vault, so no external
                // transfer is needed. Debt drops, so health improves.
                let total_shares = balance::get_total_shares(&env);
                let total_deposited = balance::get_total_deposited(&env);
                let user_shares = balance::get_shares(&env, &user);
                let want_shares = balance::amount_to_shares(total_shares, total_deposited, target)
                    .ok_or(VaultCampaignError::ArithmeticError)?;
                let burn_shares = want_shares.min(user_shares);
                if burn_shares <= 0 {
                    return Err(VaultCampaignError::ArithmeticError);
                }
                let repaid = balance::shares_to_amount(total_shares, total_deposited, burn_shares)
                    .ok_or(VaultCampaignError::ArithmeticError)?;

                balance::set_shares(&env, &user, user_shares - burn_shares);
                balance::set_total_shares(&env, total_shares - burn_shares);
                balance::set_total_deposited(&env, total_deposited - repaid);
                apply_loan_repayment(&env, &user, &mut loan, repaid);
                repaid
            }
            RecoveryAction::AutoUnstakePartial => {
                let total_shares = balance::get_total_shares(&env);
                let total_deposited = balance::get_total_deposited(&env);
                let user_shares = balance::get_shares(&env, &user);
                let burn_shares = cfg.action_amount.min(user_shares);
                if burn_shares <= 0 {
                    return Err(VaultCampaignError::ZeroAmount);
                }
                let amount = balance::shares_to_amount(total_shares, total_deposited, burn_shares)
                    .ok_or(VaultCampaignError::ArithmeticError)?;

                balance::set_shares(&env, &user, user_shares - burn_shares);
                balance::set_total_shares(&env, total_shares - burn_shares);
                balance::set_total_deposited(&env, total_deposited - amount);

                // Proceeds pay down the loan (reducing loan-to-value); any
                // surplus beyond the debt is returned to the user.
                match balance::get_loan(&env, &user) {
                    Some(mut loan) => {
                        let debt = loan.principal.saturating_add(loan.interest_accrued).max(0);
                        let to_loan = amount.min(debt);
                        apply_loan_repayment(&env, &user, &mut loan, to_loan);
                        let surplus = amount - to_loan;
                        if surplus > 0 {
                            token_client.transfer(&contract, &user, &surplus);
                        }
                    }
                    None => {
                        token_client.transfer(&contract, &user, &amount);
                    }
                }
                amount
            }
        };

        // Keeper incentive: 0.5% of the recovery action value.
        let incentive = action_value.saturating_mul(KEEPER_INCENTIVE_BPS) / 10_000;
        if incentive > 0 {
            token_client.transfer(&contract, &keeper, &incentive);
        }

        set_last_recovery_ledger(&env, &user, current_ledger);

        env.events().publish(
            (symbol_short!("recovery"), user.clone()),
            (
                cfg.action,
                cfg.trigger_health_bps,
                health,
                keeper,
                current_ledger,
            ),
        );

        Ok(action_value)
    }
}
