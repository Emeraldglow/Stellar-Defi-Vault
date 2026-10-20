//! Peer mutual insurance pool (issue #366).
//!
//! Distinct from `insurance.rs` (issue #289, a single external guarantor's
//! reserve covering admin misbehavior) and from the `InsuranceProduct` /
//! `InsurancePolicy` admin-run principal-protection product in `storage.rs`
//! (issue #259, funded and priced by the admin). This is a peer fund: opted-in
//! stakers redirect a small share of their own reward claims into a shared
//! pot, and members vote â€” stake-weighted, like `slash_dispute.rs` â€” on
//! whether a claimed loss event is real before it pays out of that pot.
//!
//! # Wiring
//!
//! Like `epoch_reward_cap.rs`, this exposes its own contribution-aware claim
//! entrypoint (`claim_reward_with_mutual_contribution`) rather than editing
//! `vault.rs`'s existing `claim()`.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::balance;
use crate::admin;
use crate::errors::VaultError;
use crate::events;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance key: pool configuration.
const CONFIG_KEY: Symbol = symbol_short!("mi_cfg");
/// Instance key: the shared fund balance.
const FUND_KEY: Symbol = symbol_short!("mi_fund");
/// Instance key: next loss-event id to assign.
const NEXT_EVENT_KEY: Symbol = symbol_short!("mi_next");
/// Persistent key prefix: membership flag. Keyed by `(MEMBER_KEY, user)`.
const MEMBER_KEY: Symbol = symbol_short!("mi_memb");
/// Persistent key prefix: a loss event. Keyed by `(EVENT_KEY, event_id)`.
const EVENT_KEY: Symbol = symbol_short!("mi_evt");
/// Persistent key prefix: a cast vote. Keyed by `(VOTED_KEY, event_id, voter)`.
const VOTED_KEY: Symbol = symbol_short!("mi_vted");

/// Admin-configured mutual pool terms (issue #366).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MutualInsuranceConfig {
    pub contribution_bps: u32,
    pub voting_period_ledgers: u32,
    pub active: bool,
}

/// A member-filed loss event awaiting (or having completed) a governance
/// vote (issue #366).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MutualLossEvent {
    pub claimant: Address,
    pub requested_amount: i128,
    pub votes_for: i128,
    pub votes_against: i128,
    pub voting_ends_at: u32,
    pub resolved: bool,
    pub approved: bool,
    pub payout_amount: i128,
}

fn get_config(env: &Env) -> Option<MutualInsuranceConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

fn set_config(env: &Env, config: &MutualInsuranceConfig) {
    env.storage().instance().set(&CONFIG_KEY, config);
}

fn get_fund(env: &Env) -> i128 {
    env.storage().instance().get(&FUND_KEY).unwrap_or(0)
}

fn set_fund(env: &Env, amount: i128) {
    env.storage().instance().set(&FUND_KEY, &amount);
}

fn is_member(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&(MEMBER_KEY, user.clone()))
        .unwrap_or(false)
}

fn get_event(env: &Env, event_id: u32) -> Option<MutualLossEvent> {
    env.storage().persistent().get(&(EVENT_KEY, event_id))
}

fn set_event(env: &Env, event_id: u32, event: &MutualLossEvent) {
    env.storage()
        .persistent()
        .set(&(EVENT_KEY, event_id), event);
}

/// User's current staked token amount, used as governance vote weight.
/// Mirrors `slash_dispute::position_amount`.
fn position_amount(env: &Env, user: &Address) -> Option<i128> {
    let shares = balance::get_shares(env, user);
    if shares == 0 {
        return None;
    }
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    balance::shares_to_amount(total_shares, total_deposited, shares)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Enable (or reconfigure) the mutual insurance pool. Admin only. Does
    /// not touch the accumulated fund balance.
    pub fn enable_mutual_insurance(
        env: Env,
        contribution_bps: u32,
        voting_period_ledgers: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if contribution_bps > 10_000 {
            return Err(VaultError::InvalidRate);
        }
        if voting_period_ledgers == 0 {
            return Err(VaultError::ZeroAmount);
        }

        crate::mutual_insurance_pool::set_config(
            &env,
            &MutualInsuranceConfig {
                contribution_bps,
                voting_period_ledgers,
                active: true,
            },
        );

        env.events().publish(
            (symbol_short!("mi_en"),),
            (contribution_bps, voting_period_ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Disable the mutual pool. Admin only. Membership and the fund balance
    /// are left untouched; `active` gates new contributions and new loss
    /// filings only.
    pub fn disable_mutual_insurance(env: Env) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let mut config = crate::mutual_insurance_pool::get_config(&env)
            .ok_or(VaultError::MutualInsuranceNotActive)?;
        config.active = false;
        crate::mutual_insurance_pool::set_config(&env, &config);
        Ok(())
    }

    /// Current mutual pool configuration, if ever set.
    pub fn get_mutual_insurance_config(env: Env) -> Option<MutualInsuranceConfig> {
        crate::mutual_insurance_pool::get_config(&env)
    }

    /// The shared fund's current balance.
    pub fn get_mutual_fund_balance(env: Env) -> i128 {
        crate::mutual_insurance_pool::get_fund(&env)
    }

    /// Opt in to the mutual pool. Membership is required to file a loss
    /// claim and to have contributions redirected on claim.
    pub fn join_mutual_insurance(env: Env, user: Address) -> Result<(), VaultError> {
        user.require_auth();

        let config = crate::mutual_insurance_pool::get_config(&env)
            .ok_or(VaultError::MutualInsuranceNotActive)?;
        if !config.active {
            return Err(VaultError::MutualInsuranceNotActive);
        }

        env.storage()
            .persistent()
            .set(&(MEMBER_KEY, user.clone()), &true);
        env.events()
            .publish((symbol_short!("mi_join"), user), env.ledger().sequence());
        Ok(())
    }

    /// Opt out of the mutual pool. Past contributions remain in the shared
    /// fund â€” it is a mutual pot, not a personal account.
    pub fn leave_mutual_insurance(env: Env, user: Address) -> Result<(), VaultError> {
        user.require_auth();

        env.storage()
            .persistent()
            .set(&(MEMBER_KEY, user.clone()), &false);
        env.events()
            .publish((symbol_short!("mi_leav"), user), env.ledger().sequence());
        Ok(())
    }

    /// Whether `user` is currently an opted-in member.
    pub fn is_mutual_member(env: Env, user: Address) -> bool {
        crate::mutual_insurance_pool::is_member(&env, &user)
    }

    /// Claim accrued rewards, redirecting `contribution_bps` of the accrued
    /// amount into the shared mutual fund instead of paying it out. Only
    /// callable by members. Returns the amount actually transferred to
    /// `user` (accrued minus the contribution).
    pub fn claim_reward_with_mutual_contribution(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        if !crate::mutual_insurance_pool::is_member(&env, &user) {
            return Err(VaultError::NotAMutualMember);
        }
        let config = crate::mutual_insurance_pool::get_config(&env)
            .ok_or(VaultError::MutualInsuranceNotActive)?;
        if !config.active {
            return Err(VaultError::MutualInsuranceNotActive);
        }

        let accrued = balance::get_accrued_reward(&env, &user);
        if accrued <= 0 {
            return Ok(0);
        }

        let pool_balance = balance::get_reward_pool_balance(&env);
        if pool_balance < accrued {
            return Err(VaultError::InsufficientRewardPool);
        }

        let contribution = accrued
            .checked_mul(config.contribution_bps as i128)
            .and_then(|v| v.checked_div(10_000))
            .ok_or(VaultError::ArithmeticError)?;
        let payout = accrued
            .checked_sub(contribution)
            .ok_or(VaultError::ArithmeticError)?;

        balance::set_accrued_reward(&env, &user, 0);
        balance::set_reward_pool_balance(&env, pool_balance - accrued);

        let fund = crate::mutual_insurance_pool::get_fund(&env)
            .checked_add(contribution)
            .ok_or(VaultError::ArithmeticError)?;
        crate::mutual_insurance_pool::set_fund(&env, fund);

        if payout > 0 {
            let token_addr: Address = env
                .storage()
                .instance()
                .get(&crate::storage::DataKey::Token)
                .ok_or(VaultError::NotInitialized)?;
            soroban_sdk::token::Client::new(&env, &token_addr).transfer(
                &env.current_contract_address(),
                &user,
                &payout,
            );
            events::claimed(&env, &user, payout, env.ledger().sequence());
        }

        env.events().publish(
            (symbol_short!("mi_contr"), user),
            (contribution, fund, env.ledger().sequence()),
        );
        Ok(payout)
    }

    /// File a claim that `claimant` suffered a verified principal loss,
    /// opening a stake-weighted vote. Members only.
    pub fn file_mutual_loss_claim(
        env: Env,
        claimant: Address,
        requested_amount: i128,
    ) -> Result<u32, VaultError> {
        claimant.require_auth();

        if !crate::mutual_insurance_pool::is_member(&env, &claimant) {
            return Err(VaultError::NotAMutualMember);
        }
        if requested_amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        let config = crate::mutual_insurance_pool::get_config(&env)
            .ok_or(VaultError::MutualInsuranceNotActive)?;
        if !config.active {
            return Err(VaultError::MutualInsuranceNotActive);
        }

        let event_id: u32 = env.storage().instance().get(&NEXT_EVENT_KEY).unwrap_or(0);
        env.storage()
            .instance()
            .set(&NEXT_EVENT_KEY, &(event_id + 1));

        let voting_ends_at = env
            .ledger()
            .sequence()
            .saturating_add(config.voting_period_ledgers);
        let event = MutualLossEvent {
            claimant: claimant.clone(),
            requested_amount,
            votes_for: 0,
            votes_against: 0,
            voting_ends_at,
            resolved: false,
            approved: false,
            payout_amount: 0,
        };
        crate::mutual_insurance_pool::set_event(&env, event_id, &event);

        env.events().publish(
            (symbol_short!("mi_file"), claimant),
            (event_id, requested_amount, voting_ends_at),
        );
        Ok(event_id)
    }

    /// Vote for or against an open loss event. Vote weight equals the
    /// voter's current staked amount. One vote per address per event.
    pub fn vote_on_mutual_loss(
        env: Env,
        voter: Address,
        event_id: u32,
        approve: bool,
    ) -> Result<(), VaultError> {
        voter.require_auth();

        let mut event = crate::mutual_insurance_pool::get_event(&env, event_id)
            .ok_or(VaultError::LossEventNotFound)?;
        if event.resolved {
            return Err(VaultError::LossEventAlreadyResolved);
        }
        if env.ledger().sequence() > event.voting_ends_at {
            return Err(VaultError::DisputeWindowClosed);
        }

        let weight = position_amount(&env, &voter).ok_or(VaultError::AlreadyVotedOrNoWeight)?;

        let voted_key = (VOTED_KEY, event_id, voter.clone());
        if env.storage().persistent().has(&voted_key) {
            return Err(VaultError::AlreadyVotedOrNoWeight);
        }
        env.storage().persistent().set(&voted_key, &true);

        if approve {
            event.votes_for = event
                .votes_for
                .checked_add(weight)
                .ok_or(VaultError::ArithmeticError)?;
        } else {
            event.votes_against = event
                .votes_against
                .checked_add(weight)
                .ok_or(VaultError::ArithmeticError)?;
        }
        crate::mutual_insurance_pool::set_event(&env, event_id, &event);

        env.events().publish(
            (symbol_short!("mi_vote"), voter),
            (event_id, approve, weight, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Resolve a loss event after its voting deadline. Anyone may call this
    /// â€” it just tallies already-cast votes. Approved
    /// (`votes_for > votes_against` and `votes_for > 0`) events pay out
    /// `min(requested_amount, fund_balance)` from the shared fund to the
    /// claimant. Returns the amount actually paid (0 if rejected).
    pub fn resolve_mutual_loss_claim(env: Env, event_id: u32) -> Result<i128, VaultError> {
        let mut event = crate::mutual_insurance_pool::get_event(&env, event_id)
            .ok_or(VaultError::LossEventNotFound)?;
        if event.resolved {
            return Err(VaultError::LossEventAlreadyResolved);
        }
        if env.ledger().sequence() <= event.voting_ends_at {
            return Err(VaultError::DisputeWindowClosed);
        }

        let approved = event.votes_for > event.votes_against && event.votes_for > 0;
        let mut payout = 0i128;

        if approved {
            let fund = crate::mutual_insurance_pool::get_fund(&env);
            payout = event.requested_amount.min(fund);
            if payout > 0 {
                crate::mutual_insurance_pool::set_fund(&env, fund - payout);

                let token_addr: Address = env
                    .storage()
                    .instance()
                    .get(&crate::storage::DataKey::Token)
                    .ok_or(VaultError::NotInitialized)?;
                soroban_sdk::token::Client::new(&env, &token_addr).transfer(
                    &env.current_contract_address(),
                    &event.claimant,
                    &payout,
                );
            }
        }

        event.resolved = true;
        event.approved = approved;
        event.payout_amount = payout;
        crate::mutual_insurance_pool::set_event(&env, event_id, &event);

        env.events().publish(
            (symbol_short!("mi_res"),),
            (
                event_id,
                approved,
                payout,
                event.votes_for,
                event.votes_against,
                env.ledger().sequence(),
            ),
        );
        Ok(payout)
    }

    /// Read-only query: a loss event's current state.
    pub fn get_mutual_loss_event(env: Env, event_id: u32) -> Option<MutualLossEvent> {
        crate::mutual_insurance_pool::get_event(&env, event_id)
    }
}















