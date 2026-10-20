//! Cross-pool liquidity bridge.
//!
//! Distinct from the pool-to-pool migration flow used to sunset a deprecated
//! pool: this lets a user move a portion of their stake from this pool into
//! another *active*, admin-approved pool in a single call, instead of
//! unstaking here and restaking there by hand.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so the approved-target list is
//! kept under a raw `Symbol`-keyed instance entry, matching the pattern
//! already established in `balance.rs` / `price_oracle.rs`.

use soroban_sdk::{contractimpl, symbol_short, token, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

const APPROVED_TARGETS_KEY: Symbol = symbol_short!("brdg_tgt");

fn approved_targets(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&APPROVED_TARGETS_KEY)
        .unwrap_or(Vec::new(env))
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Whitelists `target_pool` as a valid destination for
    /// `cross_pool_liquidity_bridge()`. Admin only.
    pub fn approve_bridge_target(
        env: Env,
        admin: Address,
        target_pool: Address,
    ) -> Result<(), VaultError> {
        let stored_admin = crate::admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        let mut targets = approved_targets(&env);
        if !targets.contains(&target_pool) {
            targets.push_back(target_pool.clone());
            env.storage().instance().set(&APPROVED_TARGETS_KEY, &targets);
        }

        env.events()
            .publish((symbol_short!("brdg_apr"),), target_pool);
        Ok(())
    }

    /// Removes `target_pool` from the approved bridge destination list.
    /// Admin only.
    pub fn revoke_bridge_target(
        env: Env,
        admin: Address,
        target_pool: Address,
    ) -> Result<(), VaultError> {
        let stored_admin = crate::admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        let targets = approved_targets(&env);
        let mut updated = Vec::new(&env);
        for t in targets.iter() {
            if t != target_pool {
                updated.push_back(t);
            }
        }
        env.storage().instance().set(&APPROVED_TARGETS_KEY, &updated);

        env.events()
            .publish((symbol_short!("brdg_rvk"),), target_pool);
        Ok(())
    }

    /// The current list of approved bridge destination pools.
    pub fn get_approved_bridge_targets(env: Env) -> Vec<Address> {
        approved_targets(&env)
    }

    /// Atomically moves `amount` of the caller's stake from this pool into
    /// `target_pool`: burns the equivalent shares and pool accounting here,
    /// then stakes the underlying tokens into the target pool on the
    /// caller's behalf. `target_pool` must be on the approved bridge list.
    pub fn cross_pool_liquidity_bridge(
        env: Env,
        user: Address,
        target_pool: Address,
        amount: i128,
    ) -> Result<i128, VaultError> {
        user.require_auth();

        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        if !approved_targets(&env).contains(&target_pool) {
            return Err(VaultError::Unauthorized);
        }

        let current_shares = balance::get_shares(&env, &user);
        if amount > current_shares {
            return Err(VaultError::InsufficientShares);
        }

        // Burn the shares/accounting here, mirroring the proportional payout
        // `withdraw()` computes, but route the underlying tokens straight to
        // the target pool instead of back to the user.
        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let token_amount = if total_shares == 0 {
            0
        } else {
            amount
                .checked_mul(total_deposited)
                .and_then(|v| v.checked_div(total_shares))
                .ok_or(VaultError::ArithmeticError)?
        };

        balance::set_shares(&env, &user, current_shares - amount);
        balance::set_total_shares(&env, total_shares - amount);
        balance::set_total_deposited(&env, total_deposited - token_amount);

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&symbol_short!("token"))
            .ok_or(VaultError::NotInitialized)?;
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &target_pool,
            &token_amount,
        );

        let target_client = VaultContractClient::new(&env, &target_pool);
        target_client.stake(&user, &token_amount, &0);

        env.events().publish(
            (symbol_short!("brdg_mv"), user),
            (target_pool, token_amount),
        );
        Ok(token_amount)
    }
}















