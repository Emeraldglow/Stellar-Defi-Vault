//! Limit order buyback mechanism for reward token buybacks (issue #421).
//!
//! Distinct from the market-order buyback in Issue #156. Places a limit order
//! that sits on-chain until the DEX price meets the specified maximum price,
//! preventing overpayment during volatile periods.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::keeper_registry;
use crate::vault::{DexRouterClient, VaultContract, VaultContractClient};

const MAX_CONCURRENT_ORDERS: u32 = 5;
const ORDER_COUNTER_KEY: Symbol = symbol_short!("lo_ctr");
const ORDER_KEY: Symbol = symbol_short!("lo_ord");
const ORDER_LIST_KEY: Symbol = symbol_short!("lo_list");

/// A limit order for reward token buyback.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LimitOrder {
    pub order_id: u32,
    pub max_price_bps: i128,
    pub amount_to_spend: i128,
    pub placed_at: u32,
    pub expires_at: u32,
    pub filled: bool,
}

fn get_order_counter(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ORDER_COUNTER_KEY)
        .unwrap_or(0)
}

fn increment_order_counter(env: &Env) -> u32 {
    let id = get_order_counter(env) + 1;
    env.storage().instance().set(&ORDER_COUNTER_KEY, &id);
    id
}

fn get_order(env: &Env, order_id: u32) -> Option<LimitOrder> {
    env.storage().persistent().get(&(ORDER_KEY, order_id))
}

fn set_order(env: &Env, order: &LimitOrder) {
    env.storage()
        .persistent()
        .set(&(ORDER_KEY, order.order_id), order);
}

fn remove_order(env: &Env, order_id: u32) {
    env.storage().persistent().remove(&(ORDER_KEY, order_id));
}

fn get_order_ids(env: &Env) -> Vec<u32> {
    env.storage()
        .instance()
        .get(&ORDER_LIST_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_order_ids(env: &Env, ids: &Vec<u32>) {
    env.storage().instance().set(&ORDER_LIST_KEY, ids);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Place a limit order for reward token buyback. Admin only.
    /// `max_price_bps` is the maximum price in basis points the admin is
    /// willing to pay per reward token. `amount_to_spend` is the amount of
    /// stake tokens to use for the buyback. `expiry_ledgers` is the number
    /// of ledgers from now until the order expires.
    /// Max 5 concurrent limit orders.
    pub fn place_buyback_limit_order(
        env: Env,
        admin: Address,
        max_price_bps: i128,
        amount_to_spend: i128,
        expiry_ledgers: u32,
    ) -> Result<u32, VaultError> {
        admin.require_auth();
        admin::require_admin(&env)?;

        if amount_to_spend <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        if max_price_bps <= 0 {
            return Err(VaultError::InvalidRate);
        }

        let ids = get_order_ids(&env);
        if ids.len() >= MAX_CONCURRENT_ORDERS {
            return Err(VaultError::MaxPositionsReached);
        }

        let order_id = increment_order_counter(&env);
        let current_ledger = env.ledger().sequence();
        let order = LimitOrder {
            order_id,
            max_price_bps,
            amount_to_spend,
            placed_at: current_ledger,
            expires_at: current_ledger + expiry_ledgers,
            filled: false,
        };

        set_order(&env, &order);
        let mut ids = get_order_ids(&env);
        ids.push_back(order_id);
        set_order_ids(&env, &ids);

        Ok(order_id)
    }

    /// Cancel an unfilled limit order. Admin only. Returns funds to the
    /// treasury (the contract's reward pool).
    pub fn cancel_limit_order(env: Env, admin: Address, order_id: u32) -> Result<(), VaultError> {
        admin.require_auth();
        admin::require_admin(&env)?;

        let order = get_order(&env, order_id).ok_or(VaultError::PositionNotFound)?;
        if order.filled {
            return Err(VaultError::InvalidRate);
        }

        remove_order(&env, order_id);
        let mut ids = get_order_ids(&env);
        let n = ids.len();
        let mut i = 0u32;
        while i < n {
            if ids.get(i).unwrap() == order_id {
                ids.remove(i);
                break;
            }
            i += 1;
        }
        set_order_ids(&env, &ids);

        Ok(())
    }

    /// Execute a limit order: the keeper checks the current DEX price and
    /// executes if the price is <= max_price_bps. The keeper earns 0.5% of
    /// the swap output as incentive.
    pub fn execute_limit_order(env: Env, keeper: Address, order_id: u32) -> Result<(), VaultError> {
        keeper.require_auth();

        let order = get_order(&env, order_id).ok_or(VaultError::PositionNotFound)?;
        if order.filled {
            return Err(VaultError::InvalidRate);
        }

        let current_ledger = env.ledger().sequence();
        if current_ledger > order.expires_at {
            remove_order(&env, order_id);
            env.events().publish(
                (symbol_short!("lo_exp"),),
                (order_id, order.placed_at, current_ledger),
            );
            return Err(VaultError::PositionNotFound);
        }

        // Get the DEX router and perform the swap
        let router_address = balance::get_dex_router(&env).ok_or(VaultError::NotYieldSource)?;
        let router = DexRouterClient::new(&env, &router_address);
        let reward_token = balance::get_reward_token(&env).ok_or(VaultError::NotInitialized)?;
        let stake_token: Address = env
            .storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("token"))
            .ok_or(VaultError::NotInitialized)?;

        // Swap amount_to_spend of stake tokens for reward tokens via the DEX
        // The contract must hold the stake tokens already
        let tokens_bought = router.swap(
            &stake_token,
            &reward_token,
            &order.amount_to_spend,
            &1, // min_amount_out = 1 (we trust the limit price check)
            &env.current_contract_address(),
        );

        // Keeper incentive: 0.5% of tokens bought
        let keeper_incentive = tokens_bought.saturating_mul(50) / 10_000; // 0.5% = 50 bps
        let tokens_to_burn = tokens_bought.saturating_sub(keeper_incentive);

        if keeper_incentive > 0 {
            let token_client = soroban_sdk::token::Client::new(&env, &reward_token);
            token_client.transfer(&env.current_contract_address(), &keeper, &keeper_incentive);
            keeper_registry::record_keeper_action(&env, &keeper, keeper_incentive);
        }

        // Burn the remaining tokens
        balance::add_tokens_burned(&env, tokens_to_burn);

        // Mark as filled
        let mut order = order;
        order.filled = true;
        set_order(&env, &order);

        env.events().publish(
            (symbol_short!("lo_fill"),),
            (
                order_id,
                order.max_price_bps,
                tokens_bought,
                tokens_to_burn,
                current_ledger,
            ),
        );

        Ok(())
    }

    /// Read-only query: all active (unfilled, unexpired) limit orders.
    pub fn get_active_limit_orders(env: Env) -> Vec<LimitOrder> {
        let ids = get_order_ids(&env);
        let current_ledger = env.ledger().sequence();
        let mut orders = Vec::new(&env);
        let n = ids.len();
        let mut i = 0u32;
        while i < n {
            if let Some(order) = get_order(&env, ids.get(i).unwrap()) {
                if !order.filled && current_ledger <= order.expires_at {
                    orders.push_back(order);
                }
            }
            i += 1;
        }
        orders
    }
}
