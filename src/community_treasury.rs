use soroban_sdk::{contracttype, symbol_short, token, Address, Env, String, Symbol};

use crate::{admin, balance, errors::VaultError, storage::DataKey};

const TREASURY_CONTRIBUTION_BPS_KEY: Symbol = symbol_short!("ct_bps");
const COMMUNITY_TREASURY_BALANCE_KEY: Symbol = symbol_short!("ct_bal");
const NEXT_SPENDING_PROPOSAL_ID_KEY: Symbol = symbol_short!("ct_next");
const BPS_DENOMINATOR: i128 = 10_000;
const MAX_TREASURY_CONTRIBUTION_BPS: u32 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpendingProposal {
    pub id: u32,
    pub proposer: Address,
    pub recipient: Address,
    pub amount: i128,
    pub purpose: String,
    pub votes_for: i128,
    pub votes_against: i128,
    pub deadline: u32,
    pub executed: bool,
}

fn proposal_key(env: &Env, proposal_id: u32) -> (Symbol, u32) {
    (Symbol::new(env, "spending_proposal"), proposal_id)
}

fn voted_key(env: &Env, proposal_id: u32, user: &Address) -> (Symbol, u32, Address) {
    (
        Symbol::new(env, "spending_voted"),
        proposal_id,
        user.clone(),
    )
}

pub fn set_treasury_contribution_bps(
    env: &Env,
    admin_addr: &Address,
    bps: u32,
) -> Result<(), VaultError> {
    admin_addr.require_auth();
    let stored_admin = admin::get_admin(env)?;
    if admin_addr != &stored_admin {
        return Err(VaultError::Unauthorized);
    }
    if bps > MAX_TREASURY_CONTRIBUTION_BPS {
        return Err(VaultError::InvalidRate);
    }
    env.storage()
        .instance()
        .set(&TREASURY_CONTRIBUTION_BPS_KEY, &bps);
    Ok(())
}

pub fn get_treasury_contribution_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&TREASURY_CONTRIBUTION_BPS_KEY)
        .unwrap_or(0)
}

pub fn get_community_treasury_balance(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&COMMUNITY_TREASURY_BALANCE_KEY)
        .unwrap_or(0)
}

pub(crate) fn set_community_treasury_balance(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&COMMUNITY_TREASURY_BALANCE_KEY, &amount);
}

pub fn route_fee_revenue(env: &Env, fee_amount: i128) -> Result<i128, VaultError> {
    if fee_amount <= 0 {
        return Ok(0);
    }

    let bps = get_treasury_contribution_bps(env);
    if bps == 0 {
        return Ok(0);
    }

    let contribution = fee_amount
        .checked_mul(bps as i128)
        .and_then(|v| v.checked_div(BPS_DENOMINATOR))
        .ok_or(VaultError::ArithmeticError)?;
    if contribution == 0 {
        return Ok(0);
    }

    let balance = get_community_treasury_balance(env)
        .checked_add(contribution)
        .ok_or(VaultError::ArithmeticError)?;
    set_community_treasury_balance(env, balance);
    Ok(contribution)
}

fn next_proposal_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&NEXT_SPENDING_PROPOSAL_ID_KEY)
        .unwrap_or(1)
}

fn set_next_proposal_id(env: &Env, id: u32) {
    env.storage()
        .instance()
        .set(&NEXT_SPENDING_PROPOSAL_ID_KEY, &id);
}

pub fn get_spending_proposal(env: &Env, proposal_id: u32) -> Option<SpendingProposal> {
    env.storage()
        .persistent()
        .get(&proposal_key(env, proposal_id))
}

pub fn propose_spending(
    env: &Env,
    user: &Address,
    recipient: Address,
    amount: i128,
    purpose: String,
    duration_ledgers: u32,
) -> Result<u32, VaultError> {
    user.require_auth();
    if amount <= 0 || duration_ledgers == 0 {
        return Err(VaultError::ZeroAmount);
    }
    if balance::get_shares(env, user) <= 0 {
        return Err(VaultError::PositionNotFound);
    }

    let id = next_proposal_id(env);
    let deadline = env
        .ledger()
        .sequence()
        .checked_add(duration_ledgers)
        .ok_or(VaultError::ArithmeticError)?;
    let proposal = SpendingProposal {
        id,
        proposer: user.clone(),
        recipient,
        amount,
        purpose,
        votes_for: 0,
        votes_against: 0,
        deadline,
        executed: false,
    };

    env.storage()
        .persistent()
        .set(&proposal_key(env, id), &proposal);
    set_next_proposal_id(env, id.checked_add(1).ok_or(VaultError::ArithmeticError)?);
    Ok(id)
}

fn stake_weight(env: &Env, user: &Address) -> Result<i128, VaultError> {
    let shares = balance::get_shares(env, user);
    if shares <= 0 {
        return Err(VaultError::PositionNotFound);
    }
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    Ok(balance::shares_to_amount(total_shares, total_deposited, shares).unwrap_or(shares))
}

pub fn vote_spending(
    env: &Env,
    user: &Address,
    proposal_id: u32,
    support: bool,
) -> Result<(), VaultError> {
    user.require_auth();
    let mut proposal =
        get_spending_proposal(env, proposal_id).ok_or(VaultError::PositionNotFound)?;
    if proposal.executed || env.ledger().sequence() > proposal.deadline {
        return Err(VaultError::EpochNotFinalized);
    }

    let key = voted_key(env, proposal_id, user);
    if env.storage().persistent().has(&key) {
        return Err(VaultError::TooManyStakers);
    }

    let weight = stake_weight(env, user)?;
    if support {
        proposal.votes_for = proposal
            .votes_for
            .checked_add(weight)
            .ok_or(VaultError::ArithmeticError)?;
    } else {
        proposal.votes_against = proposal
            .votes_against
            .checked_add(weight)
            .ok_or(VaultError::ArithmeticError)?;
    }

    env.storage()
        .persistent()
        .set(&proposal_key(env, proposal_id), &proposal);
    env.storage().persistent().set(&key, &true);
    Ok(())
}

pub fn execute_spending(env: &Env, proposal_id: u32) -> Result<(), VaultError> {
    let mut proposal =
        get_spending_proposal(env, proposal_id).ok_or(VaultError::PositionNotFound)?;
    if proposal.executed {
        return Err(VaultError::AlreadyInitialized);
    }
    if env.ledger().sequence() <= proposal.deadline {
        return Err(VaultError::EpochNotFinalized);
    }
    if proposal.votes_for <= proposal.votes_against {
        return Err(VaultError::Unauthorized);
    }

    let current_balance = get_community_treasury_balance(env);
    if proposal.amount > current_balance {
        return Err(VaultError::InsufficientRewardPool);
    }

    let token_addr: Address = env
        .storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(VaultError::NotInitialized)?;
    token::Client::new(env, &token_addr).transfer(
        &env.current_contract_address(),
        &proposal.recipient,
        &proposal.amount,
    );

    set_community_treasury_balance(env, current_balance - proposal.amount);
    proposal.executed = true;
    env.storage()
        .persistent()
        .set(&proposal_key(env, proposal_id), &proposal);
    env.events().publish(
        (Symbol::new(env, "spending_executed"),),
        (
            proposal_id,
            proposal.recipient,
            proposal.amount,
            env.ledger().sequence(),
        ),
    );
    Ok(())
}
