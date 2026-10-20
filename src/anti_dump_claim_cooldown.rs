//! Anti-dump claim cooldown (issue #365).
//!
//! A large reward claim followed immediately by more claims creates
//! sustained sell pressure on the reward token. Once a claim through this
//! module exceeds an admin-configured threshold, the claiming staker is put
//! on a cooldown: their next claim through this module must wait
//! `cooldown_ledgers` before it will succeed, smoothing payout timing
//! instead of letting it all land at once.
//!
//! # Wiring
//!
//! Like `epoch_reward_cap.rs`, this exposes its own claim entrypoint
//! (`claim_with_anti_dump_cooldown`) rather than editing `vault.rs`'s
//! existing `claim()`, keeping the cooldown opt-in per caller.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance key: cooldown configuration.
const CONFIG_KEY: Symbol = symbol_short!("ad_cfg");
/// Persistent key prefix: ledger a user's cooldown ends at. Keyed by
/// `(COOLDOWN_KEY, user)`.
const COOLDOWN_KEY: Symbol = symbol_short!("ad_cool");

/// Admin-configured anti-dump cooldown terms (issue #365).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AntiDumpCooldownConfig {
    pub threshold_amount: i128,
    pub cooldown_ledgers: u32,
    pub active: bool,
}

fn get_config(env: &Env) -> Option<AntiDumpCooldownConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

fn set_config(env: &Env, config: &AntiDumpCooldownConfig) {
    env.storage().instance().set(&CONFIG_KEY, config);
}

fn get_cooldown_until(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&(COOLDOWN_KEY, user.clone()))
        .unwrap_or(0)
}

fn set_cooldown_until(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&(COOLDOWN_KEY, user.clone()), &ledger);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Configure the claim-size threshold and cooldown length. Admin only.
    pub fn set_anti_dump_cooldown(
        env: Env,
        threshold_amount: i128,
        cooldown_ledgers: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if threshold_amount <= 0 || cooldown_ledgers == 0 {
            return Err(VaultError::ZeroAmount);
        }

        crate::anti_dump_claim_cooldown::set_config(
            &env,
            &AntiDumpCooldownConfig {
                threshold_amount,
                cooldown_ledgers,
                active: true,
            },
        );

        env.events().publish(
            (symbol_short!("ad_set"),),
            (threshold_amount, cooldown_ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Disable the cooldown. Admin only. Any cooldown already in effect for
    /// a user is left in place until it naturally expires, but is no longer
    /// enforced since `claim_with_anti_dump_cooldown` requires an active
    /// config.
    pub fn disable_anti_dump_cooldown(env: Env) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let mut config = crate::anti_dump_claim_cooldown::get_config(&env)
            .ok_or(VaultError::NotInitialized)?;
        config.active = false;
        crate::anti_dump_claim_cooldown::set_config(&env, &config);
        Ok(())
    }

    /// Current cooldown configuration, if ever set.
    pub fn get_anti_dump_cooldown_config(env: Env) -> Option<AntiDumpCooldownConfig> {
        crate::anti_dump_claim_cooldown::get_config(&env)
    }

    /// The ledger `user`'s current cooldown ends at, `0` if none is active.
    pub fn get_claim_cooldown_until(env: Env, user: Address) -> u32 {
        crate::anti_dump_claim_cooldown::get_cooldown_until(&env, &user)
    }

    /// Ledgers remaining in `user`'s cooldown, `0` if none is active or it
    /// has already elapsed.
    pub fn get_claim_cooldown_remaining(env: Env, user: Address) -> u32 {
        let until = crate::anti_dump_claim_cooldown::get_cooldown_until(&env, &user);
        until.saturating_sub(env.ledger().sequence())
    }

    /// Claim accrued rewards, subject to the anti-dump cooldown. Reverts
    /// with `ClaimCooldownActive` if the caller is still inside a cooldown
    /// window from a prior large claim. If the payout exceeds the
    /// configured threshold, a new cooldown is started for the next claim.
    ///
    /// Returns the amount transferred (0 if nothing was accrued).
    pub fn claim_with_anti_dump_cooldown(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let config = crate::anti_dump_claim_cooldown::get_config(&env)
            .ok_or(VaultError::NotInitialized)?;
        if !config.active {
            return Err(VaultError::NotInitialized);
        }

        let now = env.ledger().sequence();
        let cooldown_until = crate::anti_dump_claim_cooldown::get_cooldown_until(&env, &user);
        if now < cooldown_until {
            return Err(VaultError::InvalidRate);
        }

        let accrued = balance::get_accrued_reward(&env, &user);
        if accrued <= 0 {
            return Ok(0);
        }

        let pool_balance = balance::get_reward_pool_balance(&env);
        if pool_balance < accrued {
            return Err(VaultError::InsufficientRewardPool);
        }
        balance::set_reward_pool_balance(&env, pool_balance - accrued);
        balance::set_accrued_reward(&env, &user, 0);

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&crate::storage::DataKey::Token)
            .ok_or(VaultError::NotInitialized)?;
        soroban_sdk::token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &user,
            &accrued,
        );
        events::claimed(&env, &user, accrued, now);

        if accrued > config.threshold_amount {
            let new_cooldown_until = now.saturating_add(config.cooldown_ledgers);
            crate::anti_dump_claim_cooldown::set_cooldown_until(&env, &user, new_cooldown_until);
            env.events().publish(
                (symbol_short!("ad_trig"), user),
                (accrued, new_cooldown_until),
            );
        }

        Ok(accrued)
    }
}
















