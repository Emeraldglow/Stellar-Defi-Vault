//! Position and user share accounting storage accessors.
//!
//! Handles staker positions, share balances, deposits, lockups, unbonding, and delegations.
//! Storage access uses typed keys and enforces non-negative invariants.

use crate::errors::VaultInvariantError;
use crate::storage::{DataKey, UnbondingPosition};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, Vec};

pub(crate) fn assert_non_negative(env: &Env, value: i128) {
    if value < 0 {
        panic_with_error!(env, VaultInvariantError::NegativeBalance);
    }
}

pub fn get_shares(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::ShareBalance(user.clone()))
        .unwrap_or(0)
}

pub fn set_shares(env: &Env, user: &Address, amount: i128) {
    assert_non_negative(env, amount);
    // Issue #699: extra reward tokens accrue per share, so settle them at the
    // old share count before it changes.
    crate::vault::multi_reward_tokens::settle_user(env, user);
    env.storage()
        .persistent()
        .set(&DataKey::ShareBalance(user.clone()), &amount);
}

pub fn get_total_shares(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalShares)
        .unwrap_or(0)
}

pub fn set_total_shares(env: &Env, total: i128) {
    assert_non_negative(env, total);
    env.storage().instance().set(&DataKey::TotalShares, &total);
}

pub fn get_total_deposited(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalDeposited)
        .unwrap_or(0)
}

pub fn set_total_deposited(env: &Env, total: i128) {
    assert_non_negative(env, total);
    env.storage()
        .instance()
        .set(&DataKey::TotalDeposited, &total);
}

pub fn get_min_stake(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::MinStake)
        .unwrap_or(0)
}

pub fn set_min_stake(env: &Env, amount: i128) {
    env.storage().instance().set(&DataKey::MinStake, &amount);
}

pub fn get_stake_history(env: &Env, user: &Address) -> Option<Vec<(u32, i128)>> {
    env.storage()
        .persistent()
        .get(&DataKey::StakeHistory(user.clone()))
}

pub fn set_stake_history(env: &Env, user: &Address, history: &Vec<(u32, i128)>) {
    env.storage()
        .persistent()
        .set(&DataKey::StakeHistory(user.clone()), history);
}

pub fn append_stake_history(env: &Env, user: &Address, ledger: u32, amount: i128) {
    let mut history = get_stake_history(env, user).unwrap_or(Vec::new(env));
    history.push_back((ledger, amount));
    set_stake_history(env, user, &history);
}

pub fn get_staked_at_ledger(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::StakedAtLedger(user.clone()))
}

pub fn set_staked_at_ledger(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::StakedAtLedger(user.clone()), &ledger);
}

pub fn get_last_unstake_ledger(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::LastUnstakeLedger(user.clone()))
}

pub fn set_last_unstake_ledger(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::LastUnstakeLedger(user.clone()), &ledger);
}

pub fn get_unbonding_position(env: &Env, user: &Address) -> Option<UnbondingPosition> {
    env.storage()
        .persistent()
        .get(&DataKey::UnbondingPosition(user.clone()))
}

pub fn set_unbonding_position(env: &Env, user: &Address, pos: &UnbondingPosition) {
    env.storage()
        .persistent()
        .set(&DataKey::UnbondingPosition(user.clone()), pos);
}

pub fn remove_unbonding_position(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::UnbondingPosition(user.clone()));
}

pub fn is_position_frozen(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::FrozenAt(user.clone()))
}

pub fn freeze_position(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::FrozenAt(user.clone()), &ledger);
}

pub fn unfreeze_position(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::FrozenAt(user.clone()));
}

pub fn get_streak(env: &Env, user: &Address) -> u32 {
    let key = (symbol_short!("streak"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_streak(env: &Env, user: &Address, streak: u32) {
    let key = (symbol_short!("streak"), user.clone());
    env.storage().persistent().set(&key, &streak);
}

pub fn remove_streak(env: &Env, user: &Address) {
    let key = (symbol_short!("streak"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_first_staked_at(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::FirstStakedAt(user.clone()))
}

pub fn set_first_staked_at(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::FirstStakedAt(user.clone()), &ledger);
}

pub fn get_restaked(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::Restaked(user.clone()))
        .unwrap_or(false)
}

pub fn set_restaked(env: &Env, user: &Address, val: bool) {
    env.storage()
        .persistent()
        .set(&DataKey::Restaked(user.clone()), &val);
}

pub fn get_delegate(env: &Env, user: &Address) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&DataKey::Delegate(user.clone()))
}

pub fn set_delegate(env: &Env, user: &Address, delegate: &Address) {
    env.storage()
        .persistent()
        .set(&DataKey::Delegate(user.clone()), delegate);
}

pub fn remove_delegate(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::Delegate(user.clone()));
}

pub fn get_lock_period(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::LockPeriod)
        .unwrap_or(0)
}

pub fn set_lock_period(env: &Env, ledgers: u32) {
    env.storage().instance().set(&DataKey::LockPeriod, &ledgers);
}

pub fn is_lockup_waived(env: &Env, user: &Address) -> bool {
    let key = (symbol_short!("waived"), user.clone());
    env.storage().persistent().has(&key)
}

pub fn set_lockup_waived(env: &Env, user: &Address) {
    let key = (symbol_short!("waived"), user.clone());
    env.storage().persistent().set(&key, &true);
}

pub fn get_streak_claim_multiplier_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("stk_mult"))
        .unwrap_or(0)
}

pub fn set_streak_claim_multiplier_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("stk_mult"), &bps);
}

pub fn is_position_insured(env: &Env, user: &Address) -> bool {
    let key = (symbol_short!("ins_pos"), user.clone());
    env.storage().persistent().has(&key)
}

pub fn set_position_insured(env: &Env, user: &Address, insured: bool) {
    let key = (symbol_short!("ins_pos"), user.clone());
    if insured {
        env.storage().persistent().set(&key, &true);
    } else {
        env.storage().persistent().remove(&key);
    }
}

pub fn clear_position_insured(env: &Env, user: &Address) {
    let key = (symbol_short!("ins_pos"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_user_matching_stats(env: &Env, user: &Address) -> crate::storage::UserMatchingStats {
    let key = (symbol_short!("mtch_usr"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(crate::storage::UserMatchingStats {
            matched_amount: 0,
            last_match_ledger: 0,
        })
}

pub fn set_user_matching_stats(
    env: &Env,
    user: &Address,
    stats: &crate::storage::UserMatchingStats,
) {
    let key = (symbol_short!("mtch_usr"), user.clone());
    env.storage().persistent().set(&key, stats);
}

pub fn get_cohort_of(env: &Env, user: &Address) -> Option<u32> {
    let key = (symbol_short!("usr_chrt"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_cohort_of(env: &Env, user: &Address, cohort_id: u32) {
    let key = (symbol_short!("usr_chrt"), user.clone());
    env.storage().persistent().set(&key, &cohort_id);
}

pub fn get_split_positions(env: &Env, user: &Address) -> Vec<crate::storage::StakePosition> {
    let key = (symbol_short!("splt_pos"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_split_positions(env: &Env, user: &Address, positions: &Vec<crate::storage::StakePosition>) {
    let key = (symbol_short!("splt_pos"), user.clone());
    env.storage().persistent().set(&key, positions);
}
