//! Reward accounting and emission storage accessors.
//!
//! Handles reward rates, rate history, reward pool balances, user checkpoints,
//! claims, accumulator states, and precision decimals.

use super::positions::assert_non_negative;
use crate::storage::{
    AutoConvertConfig, DataKey, DynamicFeeConfig, RateHistoryEntry, RewardTier,
};
use soroban_sdk::{symbol_short, Address, Env, Symbol, Vec};

pub const DEFAULT_TOKEN_DECIMALS: u32 = 7;
pub const MAX_RATE_HISTORY_ENTRIES: u32 = 50;
pub const MAX_RICH_RATE_HISTORY: u32 = 20;
pub const MAX_CLAIM_HISTORY: u32 = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateChange {
    pub old_rate_bps: u32,
    pub new_rate_bps: u32,
    pub ledger: u32,
}

pub fn get_reward_rate_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::RewardRateBps)
        .unwrap_or(0)
}

pub fn set_reward_rate_bps(env: &Env, rate_bps: u32) {
    env.storage()
        .instance()
        .set(&DataKey::RewardRateBps, &rate_bps);
}

pub fn get_reward_tiers(env: &Env) -> Vec<RewardTier> {
    env.storage()
        .instance()
        .get(&symbol_short!("rwtiers"))
        .unwrap_or(Vec::new(env))
}

pub fn set_reward_tiers(env: &Env, tiers: &Vec<RewardTier>) {
    env.storage()
        .instance()
        .set(&symbol_short!("rwtiers"), tiers);
}

pub fn get_rate_history(env: &Env) -> Vec<RateChange> {
    let raw: Vec<(u32, u32, u32)> = env
        .storage()
        .instance()
        .get(&DataKey::RateHistory)
        .unwrap_or(Vec::new(env));
    let mut out = Vec::new(env);
    for entry in raw.iter() {
        out.push_back(RateChange {
            old_rate_bps: entry.0,
            new_rate_bps: entry.1,
            ledger: entry.2,
        });
    }
    out
}

pub fn set_rate_history(env: &Env, history: &Vec<RateChange>) {
    let mut raw: Vec<(u32, u32, u32)> = Vec::new(env);
    for entry in history.iter() {
        raw.push_back((entry.old_rate_bps, entry.new_rate_bps, entry.ledger));
    }
    env.storage().instance().set(&DataKey::RateHistory, &raw);
}

pub fn append_rate_history(env: &Env, old_rate_bps: u32, new_rate_bps: u32, ledger: u32) {
    let mut history = get_rate_history(env);
    while history.len() >= MAX_RATE_HISTORY_ENTRIES {
        history.pop_front();
    }
    history.push_back(RateChange {
        old_rate_bps,
        new_rate_bps,
        ledger,
    });
    set_rate_history(env, &history);
}

pub fn get_reward_rate_history(env: &Env) -> Vec<RateHistoryEntry> {
    env.storage()
        .instance()
        .get(&symbol_short!("rr_hist"))
        .unwrap_or(Vec::new(env))
}

pub fn set_reward_rate_history(env: &Env, history: &Vec<RateHistoryEntry>) {
    env.storage()
        .instance()
        .set(&symbol_short!("rr_hist"), history);
}

pub fn append_reward_rate_history(
    env: &Env,
    old_rate_bps: u32,
    new_rate_bps: u32,
    changed_at: u64,
) {
    let mut history = get_reward_rate_history(env);
    while history.len() >= MAX_RICH_RATE_HISTORY {
        history.pop_front();
    }
    history.push_back(RateHistoryEntry {
        old_rate_bps,
        new_rate_bps,
        changed_at,
    });
    set_reward_rate_history(env, &history);
}

pub fn get_reward_pool_balance(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::RewardPoolBalance)
        .unwrap_or(0)
}

pub fn set_reward_pool_balance(env: &Env, balance: i128) {
    assert_non_negative(env, balance);
    env.storage()
        .instance()
        .set(&DataKey::RewardPoolBalance, &balance);
}

pub fn add_reward_pool_balance(env: &Env, amount: i128) {
    let current = get_reward_pool_balance(env);
    set_reward_pool_balance(env, current.saturating_add(amount));
}

pub fn sub_reward_pool_balance(env: &Env, amount: i128) {
    let current = get_reward_pool_balance(env);
    set_reward_pool_balance(env, current.saturating_sub(amount));
}

pub fn get_total_rewards_paid(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalRewardsPaid)
        .unwrap_or(0)
}

pub fn set_total_rewards_paid(env: &Env, total: i128) {
    assert_non_negative(env, total);
    env.storage()
        .instance()
        .set(&DataKey::TotalRewardsPaid, &total);
}

pub fn add_total_rewards_paid(env: &Env, amount: i128) {
    let current = get_total_rewards_paid(env);
    set_total_rewards_paid(env, current.saturating_add(amount));
}

pub fn get_reward_checkpoint_ledger(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::RewardCheckpointLedger(user.clone()))
}

pub fn set_reward_checkpoint_ledger(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::RewardCheckpointLedger(user.clone()), &ledger);
}

pub fn get_last_claim_ledger(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::LastClaimLedger(user.clone()))
}

pub fn set_last_claim_ledger(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::LastClaimLedger(user.clone()), &ledger);
}

/// Instance-storage key for the sum of every user's accrued-but-unclaimed
/// reward (issue #753).
///
/// `DataKey` is at Soroban's 50-variant cap, so this uses a raw symbol key,
/// matching the rest of `balance.rs`. The total is maintained incrementally by
/// [`set_accrued_reward`] instead of scanning the staker registry.
const TOTAL_ACCRUED_UNCLAIMED_KEY: Symbol = symbol_short!("tot_acr");

pub fn get_accrued_reward(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::AccruedReward(user.clone()))
        .unwrap_or(0)
}

/// Sum of every user's accrued-but-unclaimed reward (issue #753).
pub fn get_total_accrued_unclaimed(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&TOTAL_ACCRUED_UNCLAIMED_KEY)
        .unwrap_or(0)
}

pub fn set_total_accrued_unclaimed(env: &Env, total: i128) {
    assert_non_negative(env, total);
    env.storage()
        .instance()
        .set(&TOTAL_ACCRUED_UNCLAIMED_KEY, &total);
}

pub fn set_accrued_reward(env: &Env, user: &Address, amount: i128) {
    assert_non_negative(env, amount);
    // Issue #753: keep the pool-wide outstanding-reward total in sync without
    // requiring `get_solvency_report` to scan the staker registry. Clamping at
    // zero keeps a stale/missing accumulator from tripping the non-negative
    // invariant while it is being brought up to date.
    let previous = get_accrued_reward(env, user);
    if amount != previous {
        let adjusted = get_total_accrued_unclaimed(env)
            .saturating_add(amount)
            .saturating_sub(previous);
        set_total_accrued_unclaimed(env, adjusted.max(0));
    }
    env.storage()
        .persistent()
        .set(&DataKey::AccruedReward(user.clone()), &amount);
}

pub fn add_accrued_reward(env: &Env, user: &Address, amount: i128) {
    let current = get_accrued_reward(env, user);
    set_accrued_reward(env, user, current.saturating_add(amount));
}

pub fn get_reward_remainder(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::RewardRemainder(user.clone()))
        .unwrap_or(0)
}

pub fn set_reward_remainder(env: &Env, user: &Address, remainder: i128) {
    assert_non_negative(env, remainder);
    env.storage()
        .persistent()
        .set(&DataKey::RewardRemainder(user.clone()), &remainder);
}

pub fn get_user_claim_window(env: &Env, user: &Address) -> Option<crate::storage::ClaimWindow> {
    env.storage()
        .persistent()
        .get(&DataKey::UserClaimWindow(user.clone()))
}

pub fn set_user_claim_window(env: &Env, user: &Address, window: &crate::storage::ClaimWindow) {
    env.storage()
        .persistent()
        .set(&DataKey::UserClaimWindow(user.clone()), window);
}

pub fn get_boost_schedule(env: &Env) -> Option<Vec<(u32, u32)>> {
    env.storage().instance().get(&DataKey::BoostSchedule)
}

pub fn set_boost_schedule(env: &Env, tiers: &Vec<(u32, u32)>) {
    env.storage()
        .instance()
        .set(&DataKey::BoostSchedule, tiers);
}

pub fn get_claim_history(env: &Env, user: &Address) -> Vec<(u32, i128)> {
    let key = (symbol_short!("clm_hist"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_claim_history(env: &Env, user: &Address, history: &Vec<(u32, i128)>) {
    let key = (symbol_short!("clm_hist"), user.clone());
    env.storage().persistent().set(&key, history);
}

pub fn record_claim_history(env: &Env, user: &Address, ledger: u32, amount: i128) {
    let mut history = get_claim_history(env, user);
    while history.len() >= MAX_CLAIM_HISTORY {
        history.pop_front();
    }
    history.push_back((ledger, amount));
    set_claim_history(env, user, &history);
}

pub fn get_reward_token(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("r_token"))
}

pub fn set_reward_token(env: &Env, token: &Address) {
    env.storage()
        .instance()
        .set(&symbol_short!("r_token"), token);
}

pub fn get_stake_decimals(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("s_dec"))
        .unwrap_or(DEFAULT_TOKEN_DECIMALS)
}

pub fn set_stake_decimals(env: &Env, decimals: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("s_dec"), &decimals);
}

pub fn get_reward_decimals(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("r_dec"))
        .unwrap_or(DEFAULT_TOKEN_DECIMALS)
}

pub fn set_reward_decimals(env: &Env, decimals: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("r_dec"), &decimals);
}

pub fn get_target_emission_per_ledger(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("tgt_emiss"))
        .unwrap_or(0)
}

pub fn set_target_emission_per_ledger(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&symbol_short!("tgt_emiss"), &amount);
}

pub fn is_tvl_smoothing_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&symbol_short!("tvl_smth"))
        .unwrap_or(false)
}

pub fn set_tvl_smoothing_enabled(env: &Env, enabled: bool) {
    env.storage()
        .instance()
        .set(&symbol_short!("tvl_smth"), &enabled);
}

pub fn get_streaming_enabled(env: &Env, user: &Address) -> bool {
    let key = (symbol_short!("strm_en"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn set_streaming_enabled(env: &Env, user: &Address, enabled: bool) {
    let key = (symbol_short!("strm_en"), user.clone());
    env.storage().persistent().set(&key, &enabled);
}

pub fn get_anniversary_bonus_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("anniv_bps"))
        .unwrap_or(0)
}

pub fn set_anniversary_bonus_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("anniv_bps"), &bps);
}

pub fn get_anniversaries_paid(env: &Env, user: &Address) -> Vec<u32> {
    let key = (symbol_short!("anniv_pd"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_anniversaries_paid(env: &Env, user: &Address, paid: &Vec<u32>) {
    let key = (symbol_short!("anniv_pd"), user.clone());
    env.storage().persistent().set(&key, paid);
}

pub fn get_auto_convert_config(env: &Env, user: &Address) -> Option<AutoConvertConfig> {
    let key = (symbol_short!("auto_cnv"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_auto_convert_config(env: &Env, user: &Address, config: &AutoConvertConfig) {
    let key = (symbol_short!("auto_cnv"), user.clone());
    env.storage().persistent().set(&key, config);
}

pub fn remove_auto_convert_config(env: &Env, user: &Address) {
    let key = (symbol_short!("auto_cnv"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_dynamic_fee_config(env: &Env) -> Option<DynamicFeeConfig> {
    env.storage().instance().get(&symbol_short!("dyn_fee"))
}

pub fn set_dynamic_fee_config(env: &Env, config: &DynamicFeeConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("dyn_fee"), config);
}
