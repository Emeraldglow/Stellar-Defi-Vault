use soroban_sdk::{contracttype, symbol_short, token, Address, Env, String, Symbol};

use crate::{admin, balance, errors::VaultError, events, storage::DataKey};

const THRESHOLD_KEY: Symbol = symbol_short!("mev_thr");
const MIN_DELAY_LEDGERS: u32 = 1;
const MAX_DELAY_LEDGERS: u32 = 10;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingClaim {
    pub amount: i128,
    pub executable_at: u32,
}

fn pending_claim_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "pending_claim"), user.clone())
}

fn pending_delay_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "pending_claim_delay"), user.clone())
}

pub fn set_mev_protection_threshold(
    env: &Env,
    admin_addr: &Address,
    amount: i128,
) -> Result<(), VaultError> {
    admin_addr.require_auth();
    let stored_admin = admin::get_admin(env)?;
    if admin_addr != &stored_admin {
        return Err(VaultError::Unauthorized);
    }
    if amount < 0 {
        return Err(VaultError::ZeroAmount);
    }
    env.storage().instance().set(&THRESHOLD_KEY, &amount);
    Ok(())
}

pub fn get_mev_protection_threshold(env: &Env) -> i128 {
    env.storage().instance().get(&THRESHOLD_KEY).unwrap_or(0)
}

pub fn get_pending_claim(env: &Env, user: &Address) -> Option<PendingClaim> {
    env.storage()
        .persistent()
        .get(&pending_claim_key(env, user))
}

pub fn maybe_queue_large_claim(env: &Env, user: &Address) -> Result<bool, VaultError> {
    let threshold = get_mev_protection_threshold(env);
    if threshold <= 0 {
        return Ok(false);
    }

    let amount = balance::get_accrued_reward(env, user);
    if amount < threshold {
        return Ok(false);
    }
    if get_pending_claim(env, user).is_some() {
        return Err(VaultError::AlreadyInitialized);
    }

    let delay_u64: u64 = env.prng().gen_range(MIN_DELAY_LEDGERS as u64..=MAX_DELAY_LEDGERS as u64);
    let delay = delay_u64 as u32;
    let current_ledger = env.ledger().sequence();
    let executable_at = current_ledger
        .checked_add(delay)
        .ok_or(VaultError::ArithmeticError)?;
    let pending = PendingClaim {
        amount,
        executable_at,
    };

    env.storage()
        .persistent()
        .set(&pending_claim_key(env, user), &pending);
    env.storage()
        .persistent()
        .set(&pending_delay_key(env, user), &delay);
    balance::set_accrued_reward(env, user, 0);

    env.events().publish(
        (Symbol::new(env, "claim_queued"), user),
        (amount, executable_at, current_ledger),
    );
    Ok(true)
}

pub fn execute_pending_claim(env: &Env, user: &Address) -> Result<i128, VaultError> {
    user.require_auth();
    let pending = get_pending_claim(env, user).ok_or(VaultError::PositionNotFound)?;
    let current_ledger = env.ledger().sequence();
    if current_ledger < pending.executable_at {
        return Err(VaultError::EpochNotFinalized);
    }

    let token_addr: Address = env
        .storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(VaultError::NotInitialized)?;
    token::Client::new(env, &token_addr).transfer(
        &env.current_contract_address(),
        user,
        &pending.amount,
    );

    env.storage()
        .persistent()
        .remove(&pending_claim_key(env, user));
    let delay_used = env
        .storage()
        .persistent()
        .get(&pending_delay_key(env, user))
        .unwrap_or(0);
    env.storage()
        .persistent()
        .remove(&pending_delay_key(env, user));

    let total_paid = balance::get_total_rewards_paid(env);
    balance::set_total_rewards_paid(env, total_paid + pending.amount);
    crate::reward_token_audit_trail::log_reward_movement(
        env,
        crate::reward_token_audit_trail::MovementType::RewardPaid,
        env.current_contract_address(),
        user.clone(),
        pending.amount,
        String::from_str(env, "execute_pending_claim"),
    );
    events::claimed(env, user, pending.amount, current_ledger);
    env.events().publish(
        (Symbol::new(env, "claim_executed"), user),
        (pending.amount, delay_used, current_ledger),
    );

    Ok(pending.amount)
}

pub fn cancel_pending_claim(env: &Env, user: &Address) -> Result<(), VaultError> {
    user.require_auth();
    let pending = get_pending_claim(env, user).ok_or(VaultError::PositionNotFound)?;
    let restored = balance::get_accrued_reward(env, user)
        .checked_add(pending.amount)
        .ok_or(VaultError::ArithmeticError)?;
    balance::set_accrued_reward(env, user, restored);
    env.storage()
        .persistent()
        .remove(&pending_claim_key(env, user));
    env.storage()
        .persistent()
        .remove(&pending_delay_key(env, user));
    Ok(())
}
