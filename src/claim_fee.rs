//! Optional percentage fee on reward claims.
//!
//! `DataKey` is already at Soroban's contracttype variant cap, so claim-fee
//! configuration and accounting use short symbol instance-storage keys.

use soroban_sdk::{symbol_short, token, Address, Env, Symbol};

use crate::{admin, errors::VaultError, storage::DataKey};

const CLAIM_FEE_BPS_KEY: Symbol = symbol_short!("clm_fee");
const CLAIM_FEE_RESERVE_KEY: Symbol = symbol_short!("clm_res");
const BPS_DENOMINATOR: i128 = 10_000;
pub const MAX_CLAIM_FEE_BPS: u32 = 500;

pub fn get_claim_fee_bps_internal(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&CLAIM_FEE_BPS_KEY)
        .unwrap_or(0)
}

pub fn get_claim_fee_reserve_internal(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&CLAIM_FEE_RESERVE_KEY)
        .unwrap_or(0)
}

fn set_claim_fee_reserve(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&CLAIM_FEE_RESERVE_KEY, &amount);
}

pub fn set_claim_fee_bps(env: Env, admin_addr: Address, fee_bps: u32) -> Result<(), VaultError> {
    admin_addr.require_auth();
    if admin_addr != admin::get_admin(&env)? {
        return Err(VaultError::Unauthorized);
    }
    if fee_bps > MAX_CLAIM_FEE_BPS {
        return Err(VaultError::UnstakeFeeTooHigh);
    }

    env.storage().instance().set(&CLAIM_FEE_BPS_KEY, &fee_bps);
    Ok(())
}

pub fn get_claim_fee_bps(env: Env) -> u32 {
    get_claim_fee_bps_internal(&env)
}

pub fn get_claim_fee_reserve(env: Env) -> i128 {
    get_claim_fee_reserve_internal(&env)
}

pub fn apply_claim_fee(
    env: &Env,
    user: &Address,
    reward_before_fee: i128,
) -> Result<i128, VaultError> {
    let fee_bps = get_claim_fee_bps_internal(env);
    // Issue #705: claims are fee-free while the pool is withdraw-only.
    if fee_bps == 0
        || reward_before_fee <= 0
        || crate::vault::withdraw_only_mode::is_active(env)
    {
        return Ok(reward_before_fee);
    }

    let fee_amount = reward_before_fee
        .checked_mul(fee_bps as i128)
        .ok_or(VaultError::ArithmeticError)?
        .checked_div(BPS_DENOMINATOR)
        .ok_or(VaultError::ArithmeticError)?;
    if fee_amount == 0 {
        return Ok(reward_before_fee);
    }

    let user_received = reward_before_fee
        .checked_sub(fee_amount)
        .ok_or(VaultError::ArithmeticError)?;
    // Issue #754: recycle a configured share of the fee into the reward pool
    // before booking the remainder in the claim-fee reserve.
    let recyclable_fee = fee_amount
        .checked_sub(crate::vault::fee_recycle::recycle_fee(env, fee_amount)?)
        .ok_or(VaultError::ArithmeticError)?;
    let reserve = get_claim_fee_reserve_internal(env)
        .checked_add(recyclable_fee)
        .ok_or(VaultError::ArithmeticError)?;
    set_claim_fee_reserve(env, reserve);

    env.events().publish(
        (Symbol::new(env, "claim_fee_collected"),),
        (
            user.clone(),
            reward_before_fee,
            fee_amount,
            user_received,
            env.ledger().sequence(),
        ),
    );

    Ok(user_received)
}

pub fn withdraw_claim_fees(env: Env, admin_addr: Address, amount: i128) -> Result<(), VaultError> {
    admin_addr.require_auth();
    if admin_addr != admin::get_admin(&env)? {
        return Err(VaultError::Unauthorized);
    }
    if amount <= 0 {
        return Err(VaultError::ZeroAmount);
    }

    let reserve = get_claim_fee_reserve_internal(&env);
    if amount > reserve {
        return Err(VaultError::InsufficientRewardPool);
    }

    let token_addr: Address = env
        .storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(VaultError::NotInitialized)?;
    let token_client = token::Client::new(&env, &token_addr);
    token_client.transfer(&env.current_contract_address(), &admin_addr, &amount);

    set_claim_fee_reserve(&env, reserve - amount);
    env.events().publish(
        (Symbol::new(&env, "claim_fees_withdrawn"),),
        (admin_addr, amount, env.ledger().sequence()),
    );

    Ok(())
}
