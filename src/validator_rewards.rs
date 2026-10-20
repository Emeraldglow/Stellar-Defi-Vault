//! Validator node reward integration.
//!
//! Links a Stellar validator node to the pool. Validator rewards earned by the
//! node are deposited into a separate balance and distributed proportionally
//! to stakers by position size. Distribution credits each staker's claimable
//! balance (not auto-transferred). Stakers claim via `claim_validator_rewards`.
//!
//! # Storage
//!
//! Uses raw `Symbol`-keyed instance and persistent storage, matching
//! `balance.rs` and `commitment.rs`, since `DataKey` is at Soroban's
//! 50-variant cap.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance-storage key for the linked validator node address.
const VALIDATOR_KEY: Symbol = symbol_short!("vr_node");

/// Instance-storage key for the total validator reward pool balance.
const VR_POOL_KEY: Symbol = symbol_short!("vr_pool");

/// Persistent-storage key for per-user claimable validator reward balance.
/// Keyed by `(VR_BAL_KEY, user)`.
const VR_BAL_KEY: Symbol = symbol_short!("vr_bal");

// â”€â”€ storage helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn get_validator_node(env: &Env) -> Option<Address> {
    env.storage().instance().get(&VALIDATOR_KEY)
}

fn set_validator_node(env: &Env, node: &Address) {
    env.storage().instance().set(&VALIDATOR_KEY, node);
}

fn get_vr_pool(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&VR_POOL_KEY)
        .unwrap_or(0)
}

fn set_vr_pool(env: &Env, balance: i128) {
    env.storage().instance().set(&VR_POOL_KEY, &balance);
}

fn get_vr_balance(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&(VR_BAL_KEY, user.clone()))
        .unwrap_or(0)
}

fn set_vr_balance(env: &Env, user: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&(VR_BAL_KEY, user.clone()), &amount);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Link a validator node address to the pool. Admin only.
    ///
    /// Only the registered node may call `deposit_validator_rewards`.
    pub fn set_validator_node(env: Env, node_address: Address) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        if node_address == env.current_contract_address() {
            return Err(VaultError::InvalidAddress);
        }
        set_validator_node(&env, &node_address);

        let admin = admin::get_admin(&env)?;
        env.events().publish(
            (symbol_short!("vr_set"), admin),
            (node_address, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only query: return the linked validator node address, or `None`.
    pub fn get_validator_node(env: Env) -> Option<Address> {
        get_validator_node(&env)
    }

    /// Deposit validator rewards into the pool. Only the linked validator node
    /// address may call this.
    ///
    /// The `amount` is added to the `ValidatorRewardPool` balance, which is
    /// later distributed to stakers via `distribute_validator_rewards`.
    pub fn deposit_validator_rewards(
        env: Env,
        validator_node: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        validator_node.require_auth();

        let registered = get_validator_node(&env).ok_or(VaultError::NotInitialized)?;
        if validator_node != registered {
            return Err(VaultError::Unauthorized);
        }
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let current = get_vr_pool(&env);
        let new_balance = current
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticError)?;
        set_vr_pool(&env, new_balance);

        env.events().publish(
            (symbol_short!("vr_dep"), validator_node),
            (amount, new_balance, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Distribute the accumulated validator reward pool proportionally to all
    /// stakers by position size. Admin only.
    ///
    /// Each staker's share is credited to their persistent validator reward
    /// balance (not auto-transferred). The pool is zeroed after distribution.
    pub fn distribute_validator_rewards(env: Env) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let pool = get_vr_pool(&env);
        if pool <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        if total_shares == 0 || total_deposited == 0 {
            return Err(VaultError::PositionNotFound);
        }

        let stakers = balance::get_all_stakers(&env);
        let mut distributed: i128 = 0;
        let mut staker_count: u32 = 0;

        for staker in stakers.iter() {
            let shares = balance::get_shares(&env, &staker);
            if shares == 0 {
                continue;
            }
            let position_amount = balance::shares_to_amount(total_shares, total_deposited, shares);
            let position_amount = match position_amount {
                Some(a) if a > 0 => a,
                _ => continue,
            };

            // share = pool * position_amount / total_deposited
            let share = (pool as i128)
                .checked_mul(position_amount)
                .and_then(|v| v.checked_div(total_deposited))
                .unwrap_or(0);
            if share <= 0 {
                continue;
            }

            let current = get_vr_balance(&env, &staker);
            let new_bal = current
                .checked_add(share)
                .ok_or(VaultError::ArithmeticError)?;
            set_vr_balance(&env, &staker, new_bal);
            distributed = distributed
                .checked_add(share)
                .ok_or(VaultError::ArithmeticError)?;
            staker_count += 1;
        }

        // Zero the pool. Any rounding dust stays in the contract.
        set_vr_pool(&env, 0);

        let admin = admin::get_admin(&env)?;
        env.events().publish(
            (symbol_short!("vr_dist"), admin),
            (distributed, staker_count, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Claim accumulated validator reward balance. Returns the amount claimed.
    ///
    /// The credited amount is transferred from the contract's token balance to
    /// the user. Reverts if the user has no validator rewards to claim or the
    /// contract holds insufficient tokens.
    pub fn claim_validator_rewards(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let amount = get_vr_balance(&env, &user);
        if amount <= 0 {
            return Err(VaultError::NothingToWithdraw);
        }

        // Clear the balance before transferring to prevent re-entrancy.
        set_vr_balance(&env, &user, 0);

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&crate::storage::DataKey::Token)
            .ok_or(VaultError::NotInitialized)?;
        let token_client = soroban_sdk::token::Client::new(&env, &token_addr);
        token_client.transfer(&env.current_contract_address(), &user, &amount);

        env.events().publish(
            (symbol_short!("vr_claim"), user),
            (amount, env.ledger().sequence()),
        );
        Ok(amount)
    }

    /// Read-only query: return a user's unclaimed validator reward balance.
    pub fn get_validator_reward_balance(env: Env, user: Address) -> i128 {
        get_vr_balance(&env, &user)
    }

    /// Read-only query: return the total undistributed validator reward pool.
    pub fn get_validator_reward_pool(env: Env) -> i128 {
        get_vr_pool(&env)
    }
}















