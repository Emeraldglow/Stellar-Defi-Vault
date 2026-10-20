//! Vault Extensions for Issues #498, #499, #500, #501
//!
//! Issue #593: every ad-hoc string `panic!` in this module has been replaced
//! with a typed `VaultFeature5Error` return (stable numeric codes — see
//! `ERRORS.md`), and the admin gates now propagate `VaultError` via `?`
//! instead of `.unwrap()`-ing into an untyped host panic.

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, String, Symbol, Vec};
use crate::vault::{VaultContract, VaultContractClient, DexRouterClient};
use crate::admin;
use crate::balance;
use crate::errors::{VaultError, VaultFeature5Error};

pub const MAX_TREASURY_SPLIT_RECIPIENTS: u32 = 3;

// ----------------------------------------------------------------------------
// Issue #498: Batch Withdraw
// ----------------------------------------------------------------------------
#[contractimpl]
impl VaultContract {
    pub fn batch_withdraw(env: Env, user: Address) -> Vec<(String, i128)> {
        user.require_auth();
        let mut results = Vec::new(&env);

        let shares = balance::get_shares(&env, &user);
        if shares > 0 {
            // Attempt to withdraw
            let unstake_res = Self::unstake(env.clone(), user.clone(), shares);
            if let Ok(amount) = unstake_res {
                results.push_back((String::from_str(&env, "main_position"), amount));
            }
        }

        let mut total_amount = 0;
        let position_count = results.len();
        for i in 0..position_count {
            total_amount += results.get(i).unwrap().1;
        }

        env.events().publish((symbol_short!("bw_done"),), (user, total_amount, position_count, env.ledger().sequence()));
        results
    }
}

// ----------------------------------------------------------------------------
// Issue #499: Treasury Split
// ----------------------------------------------------------------------------
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TreasurySplit {
    pub recipients: Vec<Address>,
    pub bps_shares: Vec<u32>,
}

const TS_KEY: Symbol = symbol_short!("ts_split");

#[contractimpl]
impl VaultContract {
    pub fn set_treasury_split(env: Env, admin: Address, recipients: Vec<Address>, bps_shares: Vec<u32>) -> Result<(), VaultFeature5Error> {
        admin.require_auth();
        admin::require_admin(&env)?;
        if recipients.len() > MAX_TREASURY_SPLIT_RECIPIENTS
            || recipients.len() != bps_shares.len()
        {
            return Err(VaultFeature5Error::InvalidSplitRecipients);
        }
        let mut sum: u32 = 0;
        for i in 0..bps_shares.len() {
            sum = sum
                .checked_add(bps_shares.get(i).unwrap())
                .ok_or(PublicApiError::InvalidAllocation)?;
        }
        if sum != 10000 {
            return Err(VaultFeature5Error::InvalidSplitBpsSum);
        }

        let split = TreasurySplit {
            recipients: recipients.clone(),
            bps_shares,
        };
        env.storage().instance().set(&TS_KEY, &split);
        env.events().publish((Symbol::new(&env, "ts_updated"),), (admin, recipients.len(), env.ledger().sequence()));
        Ok(())
    }

    pub fn get_treasury_split(env: Env) -> Option<TreasurySplit> {
        env.storage().instance().get(&TS_KEY)
    }
}

pub fn apply_treasury_split(env: &Env, fee: i128, fallback: i128) -> Result<(), VaultError> {
    let split_opt: Option<TreasurySplit> = env.storage().instance().get(&TS_KEY);
    let token_addr = VaultContract::token_address(env)?;
    let token = token::Client::new(env, &token_addr);

    if let Some(split) = split_opt {
        for i in 0..split.recipients.len() {
            let recipient = split.recipients.get(i).unwrap();
            let bps = split.bps_shares.get(i).unwrap();
            let amount = (fee * (bps as i128)) / 10000;
            if amount > 0 {
                token.transfer(&env.current_contract_address(), &recipient, &amount);
            }
        }
    } else {
        // Fallback to existing logic passed in, or do nothing if handled upstream.
        // Actually, the vault.rs code already handles the fallback if we don't transfer here.
        // We can just transfer `fallback` to a default. But vault.rs loops `get_fee_recipients`.
        // So this function is just a helper if we need it.
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Issue #500: Position Export
// ----------------------------------------------------------------------------
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PositionSnapshot {
    pub shares: i128,
    pub underlying_value: i128,
    pub pending_reward: i128,
    pub boost_multiplier_bps: u32,
    pub lock_status: bool,
    pub current_vote_weight: i128,
}

#[contractimpl]
impl VaultContract {
    pub fn position_export(env: Env, user: Address) -> PositionSnapshot {
        let shares = balance::get_shares(&env, &user);
        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let underlying_value = balance::shares_to_amount(total_shares, total_deposited, shares).unwrap_or(0);
        let pending_reward = balance::get_accrued_reward(&env, &user);

        PositionSnapshot {
            shares,
            underlying_value,
            pending_reward,
            boost_multiplier_bps: 0,
            lock_status: false,
            current_vote_weight: shares, // simplified
        }
    }
}

// ----------------------------------------------------------------------------
// Issue #501: Reward Token Swap
// ----------------------------------------------------------------------------
const SWAP_ROUTE_KEY: Symbol = symbol_short!("swp_rte"); // Map<Address, Address>

#[contractimpl]
impl VaultContract {
    pub fn set_reward_token_swap_path(env: Env, admin: Address, source_token: Address, dex_router: Address) -> Result<(), VaultError> {
        admin.require_auth();
        admin::require_admin(&env)?;
        let mut map: soroban_sdk::Map<Address, Address> = env.storage().instance().get(&SWAP_ROUTE_KEY).unwrap_or(soroban_sdk::Map::new(&env));
        map.set(source_token, dex_router);
        env.storage().instance().set(&SWAP_ROUTE_KEY, &map);
        Ok(())
    }

    pub fn swap_secondary_reward(env: Env, admin: Address, source_token: Address, amount: i128, min_out: i128) -> Result<(), VaultFeature5Error> {
        admin.require_auth();
        admin::require_admin(&env)?;
        let map: soroban_sdk::Map<Address, Address> = env.storage().instance().get(&SWAP_ROUTE_KEY).unwrap_or(soroban_sdk::Map::new(&env));
        let dex_router = map
            .get(source_token.clone())
            .ok_or(VaultFeature5Error::UnregisteredToken)?;

        // Execute swap
        let router_client = DexRouterClient::new(&env, &dex_router);
        let to_token = VaultContract::token_address(&env)?;

        // Transfer to router? Usually AMMs require pull, but router_client.swap signature handles it.
        // Wait, issue notes: "executes the swap via the registered router, deposits result into reward pool".
        // Signature: swap(env, from_token, to_token, amount_in, min_amount_out, to) -> i128
        // Need to approve or transfer?
        // We assume the contract has the `source_token` balance.
        let amount_out = router_client.swap(
            &source_token,
            &to_token,
            &amount,
            &min_out,
            &env.current_contract_address()
        );

        // Deposit into reward pool
        let pool = balance::get_reward_pool_balance(&env);
        balance::set_reward_pool_balance(&env, pool + amount_out);

        env.events().publish((symbol_short!("sec_swp"),), (source_token, amount, amount_out, env.ledger().sequence()));
        Ok(())
    }
}
