//! Price peg stabilization for reward-token emissions.
//!
//! `DataKey` is at the Soroban contracttype variant cap, so all state here is
//! stored under short symbol keys, following the newer feature modules.

use soroban_sdk::{contractclient, contracttype, symbol_short, Address, Env, String, Symbol};

use crate::{
    admin, balance,
    errors::VaultError,
    vault::{DexRouterClient, VaultContract},
};

const PEG_CONFIG_KEY: Symbol = symbol_short!("peg_cfg");
const PEG_MAX_BUY_KEY: Symbol = symbol_short!("peg_max");
const PEG_HALTED_KEY: Symbol = symbol_short!("peg_hlt");

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PegConfig {
    pub target_price: i128,
    pub lower_band_bps: u32,
    pub upper_band_bps: u32,
    pub oracle: Address,
    pub asset_id: String,
}

#[contractclient(name = "PegOracleClient")]
pub trait PegOracle {
    fn get_price(env: Env, asset_id: String) -> i128;
}

pub fn emissions_halted(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&PEG_HALTED_KEY)
        .unwrap_or(false)
}

fn set_emissions_halted(env: &Env, halted: bool) {
    env.storage().instance().set(&PEG_HALTED_KEY, &halted);
}

fn get_peg_config(env: &Env) -> Option<PegConfig> {
    env.storage().instance().get(&PEG_CONFIG_KEY)
}

fn set_peg_config_internal(env: &Env, config: &PegConfig) {
    env.storage().instance().set(&PEG_CONFIG_KEY, config);
}

fn get_max_buyback_per_check(env: &Env) -> i128 {
    env.storage().instance().get(&PEG_MAX_BUY_KEY).unwrap_or(0)
}

fn set_max_buyback_per_check_internal(env: &Env, amount: i128) {
    env.storage().instance().set(&PEG_MAX_BUY_KEY, &amount);
}

fn validate_config(config: &PegConfig) -> Result<(), VaultError> {
    if config.target_price <= 0 {
        return Err(VaultError::ZeroAmount);
    }
    if config.lower_band_bps == 0
        || config.upper_band_bps == 0
        || config.lower_band_bps > 10_000
        || config.upper_band_bps > 10_000
    {
        return Err(VaultError::InvalidRate);
    }
    Ok(())
}

fn lower_bound(config: &PegConfig) -> Result<i128, VaultError> {
    config
        .target_price
        .checked_mul(10_000_i128.saturating_sub(config.lower_band_bps as i128))
        .and_then(|v| v.checked_div(10_000))
        .ok_or(VaultError::ArithmeticError)
}

fn upper_bound(config: &PegConfig) -> Result<i128, VaultError> {
    config
        .target_price
        .checked_mul(10_000_i128.saturating_add(config.upper_band_bps as i128))
        .and_then(|v| v.checked_div(10_000))
        .ok_or(VaultError::ArithmeticError)
}

fn emit_buyback(env: &Env, price: i128, target: i128, amount_spent: i128) {
    env.events().publish(
        (Symbol::new(env, "peg_buyback_triggered"),),
        (price, target, amount_spent, env.ledger().sequence() as i128),
    );
}

fn emit_halted(env: &Env, price: i128, target: i128) {
    env.events().publish(
        (Symbol::new(env, "emissions_halted_by_peg"),),
        (price, target, env.ledger().sequence() as i128),
    );
}

fn emit_restored(env: &Env, price: i128, target: i128) {
    env.events().publish(
        (Symbol::new(env, "emissions_restored_by_peg"),),
        (price, target, env.ledger().sequence() as i128),
    );
}

fn execute_buyback(env: &Env, spend_amount: i128) -> Result<i128, VaultError> {
    if spend_amount <= 0 {
        return Ok(0);
    }

    let stake_token = VaultContract::get_stake_token(env.clone())?;
    let reward_token = balance::get_reward_token(env).unwrap_or_else(|| stake_token.clone());

    if stake_token == reward_token {
        return Ok(spend_amount);
    }

    let router_address = balance::get_dex_router(env).ok_or(VaultError::NotYieldSource)?;
    let router = DexRouterClient::new(env, &router_address);
    let bought = router.swap(
        &stake_token,
        &reward_token,
        &spend_amount,
        &1,
        &env.current_contract_address(),
    );

    Ok(bought)
}

pub fn set_peg_config(env: Env, admin_addr: Address, config: PegConfig) -> Result<(), VaultError> {
    admin_addr.require_auth();
    if admin_addr != admin::get_admin(&env)? {
        return Err(VaultError::Unauthorized);
    }

    validate_config(&config)?;
    set_peg_config_internal(&env, &config);
    Ok(())
}

pub fn read_peg_config(env: Env) -> Option<PegConfig> {
    get_peg_config(&env)
}

pub fn set_max_buyback_per_check(
    env: Env,
    admin_addr: Address,
    amount: i128,
) -> Result<(), VaultError> {
    admin_addr.require_auth();
    if admin_addr != admin::get_admin(&env)? {
        return Err(VaultError::Unauthorized);
    }
    if amount <= 0 {
        return Err(VaultError::ZeroAmount);
    }

    set_max_buyback_per_check_internal(&env, amount);
    Ok(())
}

pub fn read_max_buyback_per_check(env: Env) -> i128 {
    get_max_buyback_per_check(&env)
}

pub fn emissions_halted_by_peg(env: Env) -> bool {
    emissions_halted(&env)
}
pub fn check_peg(env: Env) -> Result<(), VaultError> {
    let config = get_peg_config(&env).ok_or(VaultError::NotInitialized)?;
    let oracle = PegOracleClient::new(&env, &config.oracle);
    let price = oracle.get_price(&config.asset_id);

    if price < lower_bound(&config)? {
        let available = balance::get_reward_pool_balance(&env);
        let max_buyback = get_max_buyback_per_check(&env);
        let amount_to_spend = if max_buyback > 0 && max_buyback < available {
            max_buyback
        } else {
            available
        };

        if amount_to_spend > 0 {
            let _bought = execute_buyback(&env, amount_to_spend)?;
            balance::set_reward_pool_balance(&env, available.saturating_sub(amount_to_spend));
        }
        emit_buyback(&env, price, config.target_price, amount_to_spend);
        return Ok(());
    }

    if price > upper_bound(&config)? {
        if !emissions_halted(&env) {
            set_emissions_halted(&env, true);
            emit_halted(&env, price, config.target_price);
        }
        return Ok(());
    }

    if emissions_halted(&env) {
        set_emissions_halted(&env, false);
        emit_restored(&env, price, config.target_price);
    }

    Ok(())
}
