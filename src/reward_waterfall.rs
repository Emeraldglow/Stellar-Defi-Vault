//! Reward waterfall â€” priority order for paying multiple reward streams when
//! the reward pool can't cover all of them (issue #341).
//!
//! `BaseRate` is sourced from the existing accrual ledger
//! (`balance::get_accrued_reward` / `balance::set_accrued_reward`) â€” nothing
//! new is introduced there. `ValidatorBonus`, `CampaignBoost`,
//! `AnniversaryBonus`, and `ReferralBonus` have no dedicated per-user ledger
//! anywhere else in the contract yet (validator_rewards.rs tracks its own
//! balance privately and isn't reused here to avoid taking a dependency on
//! another module's internal storage), so this module owns a single credit
//! ledger for all four, populated via `credit_reward()`.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs` / `validator_rewards.rs`.
//!
//! # Known gap
//!
//! This module adds `claim_via_waterfall()` as a new, standalone claim path
//! rather than editing the pool's existing `claim()` â€” that entrypoint (and
//! most of `vault.rs` past `join_waitlist`) is currently missing from
//! `src/vault.rs` on `main` due to an unrelated bad merge (see PR
//! description). Once that's restored, `claim()` should call into
//! `calc_total_reward`/pay the waterfall instead of (or before) its own
//! reward math, so a single claim reflects every stream at once.

use soroban_sdk::{contractimpl, contracttype, symbol_short, vec, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// One reward stream a staker can be owed.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RewardType {
    BaseRate,
    ValidatorBonus,
    CampaignBoost,
    AnniversaryBonus,
    ReferralBonus,
}

pub const MAX_REWARD_WATERFALL_ENTRIES: u32 = 5;

/// Instance-storage key for the admin-configured priority order.
const WATERFALL_KEY: Symbol = symbol_short!("rw_order");

/// Persistent-storage key prefix for a user's credited (non-`BaseRate`)
/// reward-type balances. Keyed by `(REWARD_CREDIT_KEY, user, reward_type)`.
const REWARD_CREDIT_KEY: Symbol = symbol_short!("rw_cred");

fn default_waterfall(env: &Env) -> Vec<RewardType> {
    vec![
        env,
        RewardType::BaseRate,
        RewardType::ValidatorBonus,
        RewardType::CampaignBoost,
        RewardType::AnniversaryBonus,
        RewardType::ReferralBonus,
    ]
}

fn get_waterfall(env: &Env) -> Vec<RewardType> {
    env.storage()
        .instance()
        .get(&WATERFALL_KEY)
        .unwrap_or_else(|| default_waterfall(env))
}

fn get_credit(env: &Env, user: &Address, reward_type: RewardType) -> i128 {
    env.storage()
        .persistent()
        .get(&(REWARD_CREDIT_KEY, user.clone(), reward_type))
        .unwrap_or(0)
}

fn set_credit(env: &Env, user: &Address, reward_type: RewardType, amount: i128) {
    env.storage()
        .persistent()
        .set(&(REWARD_CREDIT_KEY, user.clone(), reward_type), &amount);
}

/// Pending amount owed for one reward type, before any payout.
fn pending_for(env: &Env, user: &Address, reward_type: RewardType) -> i128 {
    match reward_type {
        RewardType::BaseRate => balance::get_accrued_reward(env, user),
        _ => get_credit(env, user, reward_type),
    }
}

/// Reduce the pending balance for one reward type by `amount` after paying it.
fn debit(env: &Env, user: &Address, reward_type: RewardType, amount: i128) {
    match reward_type {
        RewardType::BaseRate => {
            let current = balance::get_accrued_reward(env, user);
            balance::set_accrued_reward(env, user, current.saturating_sub(amount));
        }
        _ => {
            let current = get_credit(env, user, reward_type);
            set_credit(env, user, reward_type, current.saturating_sub(amount));
        }
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Set the priority order rewards are paid in when the pool can't cover
    /// all of them. Admin only. Must list each `RewardType` at most once.
    pub fn set_reward_waterfall(env: Env, order: Vec<RewardType>) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if order.is_empty() {
            return Err(VaultError::ZeroAmount);
        }
        if order.len() > MAX_REWARD_WATERFALL_ENTRIES {
            return Err(VaultError::BatchTooLarge);
        }

        // Reject duplicates â€” a repeated type would silently swallow one of
        // the others out of the priority order.
        for i in 0..order.len() {
            for j in (i + 1)..order.len() {
                if order.get(i) == order.get(j) {
                    return Err(VaultError::InvalidRate);
                }
            }
        }

        env.storage().instance().set(&WATERFALL_KEY, &order);

        env.events()
            .publish((symbol_short!("rw_set"),), (order, env.ledger().sequence()));
        Ok(())
    }

    /// The current reward waterfall priority order (first = highest
    /// priority). Defaults to `[BaseRate, ValidatorBonus, CampaignBoost,
    /// AnniversaryBonus, ReferralBonus]` when unconfigured.
    pub fn get_reward_waterfall(env: Env) -> Vec<RewardType> {
        get_waterfall(&env)
    }

    /// Admin-only credit to a user's `ValidatorBonus` / `CampaignBoost` /
    /// `AnniversaryBonus` / `ReferralBonus` balance. `BaseRate` can't be
    /// credited here â€” it's driven entirely by the existing accrual ledger.
    pub fn credit_reward(
        env: Env,
        user: Address,
        reward_type: RewardType,
        amount: i128,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if reward_type == RewardType::BaseRate {
            return Err(VaultError::InvalidRate);
        }
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let current = get_credit(&env, &user, reward_type);
        let new_total = current
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticError)?;
        set_credit(&env, &user, reward_type, new_total);
        Ok(())
    }

    /// Pending amount per reward type for `user`, in `RewardType` enum order
    /// (not waterfall priority order).
    pub fn get_reward_breakdown(env: Env, user: Address) -> Vec<(RewardType, i128)> {
        let mut out = Vec::new(&env);
        for reward_type in [
            RewardType::BaseRate,
            RewardType::ValidatorBonus,
            RewardType::CampaignBoost,
            RewardType::AnniversaryBonus,
            RewardType::ReferralBonus,
        ] {
            out.push_back((reward_type, pending_for(&env, &user, reward_type)));
        }
        out
    }

    /// Sum of every reward type currently pending for `user`.
    pub fn calc_total_reward(env: Env, user: Address) -> i128 {
        Self::get_reward_breakdown(env, user)
            .iter()
            .fold(0i128, |acc, (_, amount)| acc.saturating_add(amount))
    }

    /// Claim every pending reward type, paying in waterfall priority order
    /// and stopping once the reward pool is exhausted. Returns the total
    /// amount actually paid.
    ///
    /// When the pool covers everything, all types are paid in full and no
    /// event beyond the transfer is needed. When it doesn't, this pays as
    /// many full reward-type amounts as fit, in priority order, and emits
    /// `partial_reward_paid` naming every type that was skipped entirely â€”
    /// it never pays a type partially, so a skipped type's balance is left
    /// untouched for the next claim.
    pub fn claim_via_waterfall(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let mut available = balance::get_reward_pool_balance(&env);
        if available <= 0 {
            return Err(VaultError::InsufficientRewardPool);
        }

        let order = get_waterfall(&env);
        let mut paid_total: i128 = 0;
        let mut skipped: Vec<RewardType> = Vec::new(&env);

        for reward_type in order.iter() {
            let owed = pending_for(&env, &user, reward_type);
            if owed <= 0 {
                continue;
            }
            if owed > available {
                skipped.push_back(reward_type);
                continue;
            }

            debit(&env, &user, reward_type, owed);
            available = available
                .checked_sub(owed)
                .ok_or(VaultError::ArithmeticError)?;
            paid_total = paid_total
                .checked_add(owed)
                .ok_or(VaultError::ArithmeticError)?;
        }

        if paid_total <= 0 {
            return Err(VaultError::InsufficientRewardPool);
        }

        let new_pool_balance = balance::get_reward_pool_balance(&env)
            .checked_sub(paid_total)
            .ok_or(VaultError::ArithmeticError)?;
        balance::set_reward_pool_balance(&env, new_pool_balance);

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&crate::storage::DataKey::Token)
            .ok_or(VaultError::NotInitialized)?;
        let token_client = soroban_sdk::token::Client::new(&env, &token_addr);
        token_client.transfer(&env.current_contract_address(), &user, &paid_total);

        if !skipped.is_empty() {
            env.events().publish(
                (symbol_short!("rw_part"), user.clone()),
                (paid_total, skipped, env.ledger().sequence()),
            );
        }

        env.events().publish(
            (symbol_short!("rw_claim"), user),
            (paid_total, env.ledger().sequence()),
        );
        Ok(paid_total)
    }
}















