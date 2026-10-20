//! Core accounting invariant checker (issue #531).
//!
//! `check_invariants()` verifies that the vault's bookkeeping is internally
//! consistent and backed by real tokens:
//!
//! 1. `total_shares`, `total_deposited` and the reward pool are non-negative.
//! 2. The sum of every registered staker's share balance equals
//!    `total_shares`.
//! 3. The vault's actual stake-token balance covers `total_deposited` (plus
//!    the reward pool when rewards are paid in the stake token).
//!
//! It is exposed read-only as `assert_invariants()` so tests, keepers and
//! frontends can use it as a public health check. Invariant 2 iterates the
//! staker registry, so its cost grows with the number of stakers.

use soroban_sdk::{contractimpl, token, Address, Env};

use crate::balance;
use crate::errors::VaultFeature4Error;
use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient};

/// Runs every invariant, returning the first violation found.
pub(crate) fn check_invariants(env: &Env) -> Result<(), VaultFeature4Error> {
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    let reward_pool = balance::get_reward_pool_balance(env);
    if total_shares < 0 || total_deposited < 0 || reward_pool < 0 {
        return Err(VaultFeature4Error::NegativeAccounting);
    }

    let mut share_sum: i128 = 0;
    for staker in balance::get_all_stakers(env).iter() {
        let shares = balance::get_shares(env, &staker);
        if shares < 0 {
            return Err(VaultFeature4Error::NegativeAccounting);
        }
        share_sum = share_sum
            .checked_add(shares)
            .ok_or(VaultFeature4Error::SharesMismatch)?;
    }
    if share_sum != total_shares {
        return Err(VaultFeature4Error::SharesMismatch);
    }

    let stake_token: Address = env
        .storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(VaultFeature4Error::NotInitialized)?;
    let mut required = total_deposited;
    if balance::get_reward_token(env).as_ref() == Some(&stake_token) {
        required = required
            .checked_add(reward_pool)
            .ok_or(VaultFeature4Error::Undercollateralized)?;
    }
    let held = token::Client::new(env, &stake_token).balance(&env.current_contract_address());
    if held < required {
        return Err(VaultFeature4Error::Undercollateralized);
    }

    Ok(())
}

#[contractimpl]
impl VaultContract {
    /// Read-only health check: reverts with `NegativeAccounting`,
    /// `SharesMismatch` or `Undercollateralized` when core vault accounting
    /// is inconsistent, and succeeds otherwise.
    pub fn assert_invariants(env: Env) -> Result<(), VaultFeature4Error> {
        check_invariants(&env)
    }
}
