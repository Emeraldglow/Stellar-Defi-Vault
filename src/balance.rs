//! Typed storage accessor layer and state management.
//!
//! Split into focused submodules by responsibility (Issue #647, #648):
//! - `positions`: user positions, share balances, deposits, lockups, and unbonding
//! - `shares`: share/amount conversion math
//! - `rewards`: reward rates, rate history, pool balances, checkpoints, and precision
//! - `governance_store`: proposals, voting weights, delegates, vetoes, and pending actions
//! - `burns`: fees burned, fee reserves, penalties, receipts, and slashing treasury
//! - `config`: protocol configuration, staker registries, changelog, and modular features

pub mod burns;
pub mod config;
pub mod governance_store;
pub mod positions;
pub mod rewards;
pub mod shares;

pub use burns::*;
pub use config::*;
pub use governance_store::*;
pub use positions::*;
pub use rewards::*;
pub use shares::*;
use crate::errors::VaultInvariantError;
use crate::storage::{
    AccessTier, AdminProposal, AutoConvertConfig, BrandingConfig, ChangelogEntry, ClaimWindow,
    ContractDelegate, DataKey, DayBucket, DynamicFeeConfig, FeeRecipient, FlashStakeReceipt,
    GovernanceProposal, InsurancePolicy, InsuranceProduct, Loan, LoanConfig, LotteryConfig,
    Milestone, MultisigConfig, OnboardingChecklist, PendingAction, PriceCondition,
    PriorityBidRecord, Quiz, RateHistoryEntry, ReferralStats, RevenueShareMerkleRoot,
    RevenueSharingConfig, RewardTier, Season, StakePosition, SunsetState, VestingEntry,
};

use soroban_sdk::{panic_with_error, symbol_short, Address, Env, String, Symbol, Vec};

/// Issue #621: last-line-of-defense invariant for every accounting field
/// this module persists. `i128` is signed, so nothing at the type level stops
/// a caller from writing a negative share/reward/pool balance — this makes
/// that impossible in practice by rejecting the write instead. See the
/// `VaultInvariantError` doc comment in `errors.rs` for why this panics
/// rather than returning a `Result` (these setters have no `Result` in their
/// signature, and giving them one would mean updating every one of their
/// call sites across the crate for a condition that should never occur if
/// upstream business logic is correct).
fn assert_non_negative(env: &Env, value: i128) {
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
    crate::balance_checkpoints::record_user(env, user, amount);
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
    crate::balance_checkpoints::record_total(env, total);
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

/// Get the maximum number of quizzes allowed.
pub fn get_max_quizzes(_: &Env) -> u32 {
    MAX_QUIZZES
}

/// Get quiz data by ID.
pub fn get_quiz(env: &Env, quiz_id: u32) -> Option<Quiz> {
    let key = (Symbol::new(env, "quiz"), quiz_id);
    env.storage().persistent().get(&key)
}

/// Set quiz data by ID.
pub fn set_quiz(env: &Env, quiz: &Quiz) {
    let key = (Symbol::new(env, "quiz"), quiz.id);
    env.storage().persistent().set(&key, quiz);
}

/// Get the number of remaining attempts for a user on a specific quiz.
pub fn get_quiz_attempts_remaining(env: &Env, user: &Address, quiz_id: u32) -> u32 {
    let key = (Symbol::new(env, "quiz_attempts"), user.clone(), quiz_id);
    env.storage().persistent().get(&key).unwrap_or(0)
}

/// Returns `None` if the user has never had attempts initialized for this quiz
/// (i.e. they haven't submitted a wrong answer yet), `Some(n)` otherwise.
/// This lets callers distinguish between "not yet started" and "0 remaining".
pub fn get_quiz_attempts_remaining_opt(env: &Env, user: &Address, quiz_id: u32) -> Option<u32> {
    let key = (Symbol::new(env, "quiz_attempts"), user.clone(), quiz_id);
    env.storage().persistent().get(&key)
}

/// Set the number of remaining attempts for a user on a specific quiz.
pub fn set_quiz_attempts_remaining(env: &Env, user: &Address, quiz_id: u32, attempts: u32) {
    let key = (Symbol::new(env, "quiz_attempts"), user.clone(), quiz_id);
    env.storage().persistent().set(&key, &attempts);
}

/// Get the list of completed quiz IDs for a user.
pub fn get_completed_quizzes(env: &Env, user: &Address) -> Vec<u32> {
    let key = (Symbol::new(env, "completed_quizzes"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

/// Set the list of completed quiz IDs for a user.
pub fn set_completed_quizzes(env: &Env, user: &Address, completed_quizzes: &Vec<u32>) {
    let key = (Symbol::new(env, "completed_quizzes"), user.clone());
    env.storage().persistent().set(&key, completed_quizzes);
}

/// Get the highest reward tier unlocked by a user via quiz completion.
pub fn get_user_quiz_tier(env: &Env, user: &Address) -> u32 {
    let key = (Symbol::new(env, "user_quiz_tier"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

/// Set the highest reward tier unlocked by a user via quiz completion.
pub fn set_user_quiz_tier(env: &Env, user: &Address, tier: u32) {
    let key = (Symbol::new(env, "user_quiz_tier"), user.clone());
    env.storage().persistent().set(&key, &tier);
}

/// Get the total number of quizzes that have been created.
pub fn get_quiz_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&Symbol::new(env, "quiz_count"))
        .unwrap_or(0)
}

/// Set the total quiz count.
pub fn set_quiz_count(env: &Env, count: u32) {
    env.storage()
        .instance()
        .set(&Symbol::new(env, "quiz_count"), &count);
}

/// The rolling on-chain log of reward-rate changes written by `set_reward_rate_bps`
/// and exposed by `get_rate_history` (issue #522), oldest entry first.
pub fn get_rate_history(env: &Env) -> Vec<RateChange> {
    env.storage()
        .instance()
        .get(&DataKey::RateHistory)
        .unwrap_or(Vec::new(env))
}

pub fn set_rate_history(env: &Env, history: &Vec<RateChange>) {
    env.storage().instance().set(&DataKey::RateHistory, history);
}

/// Appends one reward-rate change to the rolling log, evicting the oldest
/// entries first once `MAX_RATE_HISTORY_ENTRIES` is reached. Changes are logged
/// even when the rate is unchanged, matching the `rate_changed` event.
pub fn record_rate_change(env: &Env, old_rate_bps: u32, new_rate_bps: u32) {
    let mut history = get_rate_history(env);
    while history.len() >= MAX_RATE_HISTORY_ENTRIES {
        history.remove(0);
    }
    history.push_back(RateChange {
        old_rate_bps,
        new_rate_bps,
        changed_at: env.ledger().sequence(),
    });
    set_rate_history(env, &history);
}

pub const MAX_RATE_HISTORY_ENTRIES: u32 = 50;
pub const MAX_QUIZZES: u32 = 20;

/// Maximum allowed reward rate in basis points (500% APR). Issue #72.
pub const MAX_RATE_BPS: u32 = 50_000;

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

pub fn get_withdrawal_limit(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::WithdrawalLimit)
}

pub fn set_withdrawal_limit(env: &Env, limit: i128) {
    env.storage()
        .instance()
        .set(&DataKey::WithdrawalLimit, &limit);
}

pub fn get_pool_cap(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("pool_cp"))
        .unwrap_or(0)
}

pub fn set_pool_cap(env: &Env, cap: i128) {
    env.storage()
        .instance()
        .set(&symbol_short!("pool_cp"), &cap);
}

pub fn get_unstake_fee_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::UnstakeFeeBps)
        .unwrap_or(0)
}

pub fn set_unstake_fee_bps(env: &Env, bps: u32) {
    env.storage().instance().set(&DataKey::UnstakeFeeBps, &bps);
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

pub fn set_last_claim_ledger(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::LastClaimLedger(user.clone()), &ledger);
}

pub fn get_accrued_reward(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::AccruedReward(user.clone()))
        .unwrap_or(0)
}

pub fn set_accrued_reward(env: &Env, user: &Address, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .persistent()
        .set(&DataKey::AccruedReward(user.clone()), &amount);
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

pub fn get_boost_schedule(env: &Env) -> Option<Vec<(u32, u32)>> {
    env.storage().instance().get(&DataKey::BoostSchedule)
}

pub fn set_boost_schedule(env: &Env, tiers: &Vec<(u32, u32)>) {
    env.storage().instance().set(&DataKey::BoostSchedule, tiers);
}

pub fn get_total_stakers(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::TotalStakers)
        .unwrap_or(0)
}

pub fn set_total_stakers(env: &Env, count: u32) {
    env.storage().instance().set(&DataKey::TotalStakers, &count);
}

pub fn get_total_rewards_paid(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalRewardsPaid)
        .unwrap_or(0)
}

pub fn set_total_rewards_paid(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&DataKey::TotalRewardsPaid, &amount);
}

pub fn get_last_claim_ledger(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::LastClaimLedger(user.clone()))
        .unwrap_or(0)
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

// ── Issue #38: slash treasury ────────────────────────────────────────────────

pub fn get_slash_treasury(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("sl_tr"))
}

pub fn set_slash_treasury(env: &Env, treasury: &Address) {
    env.storage()
        .instance()
        .set(&symbol_short!("sl_tr"), treasury);
}

// ── Issue #39: reward token ───────────────────────────────────────────────────

pub fn get_reward_token(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("rwd_tok"))
}

pub fn set_reward_token(env: &Env, token: &Address) {
    env.storage()
        .instance()
        .set(&symbol_short!("rwd_tok"), token);
}

// ── Issue #40: NFT contract ───────────────────────────────────────────────────

pub fn get_nft_contract(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("nft_con"))
}

pub fn set_nft_contract(env: &Env, nft: &Address) {
    env.storage().instance().set(&symbol_short!("nft_con"), nft);
}

// ── Issue #41: restake grace window ──────────────────────────────────────────

pub fn get_restake_window(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("rst_wnd"))
        .unwrap_or(0)
}

pub fn set_restake_window(env: &Env, ledgers: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("rst_wnd"), &ledgers);
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

pub fn is_restaked(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::Restaked(user.clone()))
        .unwrap_or(false)
}

pub fn set_restaked(env: &Env, user: &Address, value: bool) {
    env.storage()
        .persistent()
        .set(&DataKey::Restaked(user.clone()), &value);
}

pub fn remove_restaked(env: &Env, user: &Address) {
    let key = DataKey::Restaked(user.clone());
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
    }
}

// ── Issue #42: admin action count ────────────────────────────────────────────

pub fn get_admin_action_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("adm_cnt"))
        .unwrap_or(0)
}

pub fn increment_admin_action_count(env: &Env) {
    let count = get_admin_action_count(env);
    env.storage()
        .instance()
        .set(&symbol_short!("adm_cnt"), &(count + 1));
}

// ── Claim cap (issue #78) ─────────────────────────────────────────────────────

pub fn get_claim_cap(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("clm_cap"))
        .unwrap_or(0)
}

pub fn set_claim_cap(env: &Env, cap: i128) {
    env.storage()
        .instance()
        .set(&symbol_short!("clm_cap"), &cap);
}

pub fn get_claim_cap_window(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("clm_win"))
        .unwrap_or(0)
}

pub fn set_claim_cap_window(env: &Env, window_ledgers: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("clm_win"), &window_ledgers);
}

pub fn get_user_claim_window(env: &Env, user: &Address) -> Option<ClaimWindow> {
    env.storage()
        .persistent()
        .get(&DataKey::UserClaimWindow(user.clone()))
}

pub fn set_user_claim_window(env: &Env, user: &Address, window: &ClaimWindow) {
    env.storage()
        .persistent()
        .set(&DataKey::UserClaimWindow(user.clone()), window);
}

// ── Token decimals (reward normalization) ─────────────────────────────────────

/// Default decimal precision for Stellar tokens. Most tokens use 7 places,
/// but this is only a fallback for pools initialized without explicit values.
pub const DEFAULT_TOKEN_DECIMALS: u32 = 7;

pub fn get_stake_decimals(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("stk_dec"))
        .unwrap_or(DEFAULT_TOKEN_DECIMALS)
}

pub fn set_stake_decimals(env: &Env, decimals: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("stk_dec"), &decimals);
}

pub fn get_reward_decimals(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("rwd_dec"))
        .unwrap_or(DEFAULT_TOKEN_DECIMALS)
}

pub fn set_reward_decimals(env: &Env, decimals: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("rwd_dec"), &decimals);
}

// ── All-stakers list (issue #95) ──────────────────────────────────────────────

pub fn get_all_stakers(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::AllStakers)
        .unwrap_or(Vec::new(env))
}

pub fn set_all_stakers(env: &Env, stakers: &Vec<Address>) {
    env.storage().instance().set(&DataKey::AllStakers, stakers);
}

/// Records `user` in the staker registry if they are not already present and
/// refreshes the `total_stakers` count. Idempotent, so callers can invoke it on
/// every stake without worrying about duplicates. Keeps the registry (and thus
/// `get_pool_summary().depositor_count`) in sync with real depositors.
pub fn register_staker(env: &Env, user: &Address) {
    let mut stakers = get_all_stakers(env);
    let already_present = stakers.iter().any(|a| &a == user);
    if !already_present {
        stakers.push_back(user.clone());
        env.storage()
            .instance()
            .set(&DataKey::AllStakers, &stakers);
    }
    env.storage()
        .instance()
        .set(&DataKey::TotalStakers, &stakers.len());
}

// ── Share math ────────────────────────────────────────────────────────────────

/// Convert a deposit amount to shares using current vault ratio.
/// First deposit: scale the token amount into the vault's fixed share
/// precision. Subsequent deposits are proportional to the existing pool.
pub fn amount_to_shares(total_shares: i128, total_deposited: i128, amount: i128) -> Option<i128> {
    amount_to_shares_with_decimals(total_shares, total_deposited, amount, DEFAULT_TOKEN_DECIMALS)
}

/// Convert token base units to shares. Shares intentionally use seven decimal
/// places, while the underlying token may use a different precision. The
/// decimal argument is the precision queried from the configured token.
pub fn amount_to_shares_with_decimals(
    total_shares: i128,
    total_deposited: i128,
    amount: i128,
    token_decimals: u32,
) -> Option<i128> {
    if total_shares == 0 || total_deposited == 0 {
        scale_amount(amount, token_decimals, DEFAULT_TOKEN_DECIMALS)
    } else {
        amount
            .checked_mul(total_shares)?
            .checked_div(total_deposited)
    }
}

/// Convert shares to the underlying token amount.
pub fn shares_to_amount(total_shares: i128, total_deposited: i128, shares: i128) -> Option<i128> {
    shares_to_amount_with_decimals(total_shares, total_deposited, shares, DEFAULT_TOKEN_DECIMALS)
}

/// Convert shares in the fixed seven-decimal share precision to underlying
/// token base units. The ratio already includes the initial decimal scaling.
pub fn shares_to_amount_with_decimals(
    total_shares: i128,
    total_deposited: i128,
    shares: i128,
    _token_decimals: u32,
) -> Option<i128> {
    if total_shares == 0 {
        Some(0)
    } else {
        shares
            .checked_mul(total_deposited)?
            .checked_div(total_shares)
    }
}

fn scale_amount(amount: i128, from_decimals: u32, to_decimals: u32) -> Option<i128> {
    if from_decimals == to_decimals {
        return Some(amount);
    }

    if from_decimals < to_decimals {
        amount.checked_mul(10_i128.checked_pow(to_decimals - from_decimals)?)
    } else {
        amount.checked_div(10_i128.checked_pow(from_decimals - to_decimals)?)
    }
}

pub fn get_reward_remainder(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::RewardRemainder(user.clone()))
        .unwrap_or(0)
}

pub fn set_reward_remainder(env: &Env, user: &Address, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .persistent()
        .set(&DataKey::RewardRemainder(user.clone()), &amount);
}

// ── Issue #69: last updated ledger ───────────────────────────────────────────
// Uses symbol_short! to avoid pushing DataKey over the contracttype variant limit.

pub fn get_last_updated_ledger(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("lst_upd"))
        .unwrap_or(0)
}

pub fn set_last_updated_ledger(env: &Env, ledger: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("lst_upd"), &ledger);
}

// ── Issue #97: pool description ──────────────────────────────────────────────

pub fn get_pool_description(env: &Env) -> Option<soroban_sdk::String> {
    env.storage().instance().get(&symbol_short!("pool_desc"))
}

pub fn set_pool_description(env: &Env, description: &soroban_sdk::String) {
    env.storage()
        .instance()
        .set(&symbol_short!("pool_desc"), description);
}

// ── Issue #99: staking streak ────────────────────────────────────────────────

pub fn get_last_recorded_wave(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("last_wave"))
}

pub fn set_last_recorded_wave(env: &Env, wave_id: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("last_wave"), &wave_id);
}

pub fn get_user_streak(env: &Env, user: &Address) -> Option<crate::storage::StakeStreak> {
    let key = (Symbol::new(env, "strk"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_user_streak(env: &Env, user: &Address, streak: &crate::storage::StakeStreak) {
    let key = (Symbol::new(env, "strk"), user.clone());
    env.storage().persistent().set(&key, streak);
}

// ── Issue #135: per-user cumulative claimed counter ───────────────────────────
// Uses a tuple key to avoid exhausting the DataKey enum's contracttype limit.

pub fn get_user_total_claimed(env: &Env, user: &Address) -> i128 {
    let key = (Symbol::new(env, "t_claimed"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

// Issue #238: wired into `do_claim`, feeding the TotalRewardsClaimed milestone condition.
pub fn add_user_total_claimed(env: &Env, user: &Address, amount: i128) {
    let current = get_user_total_claimed(env, user);
    let key = (Symbol::new(env, "t_claimed"), user.clone());
    env.storage().persistent().set(&key, &(current + amount));
}

// ── Issue #114: on-chain admin changelog ─────────────────────────────────────
// Key "chlg" (4 chars, short symbol) stored in instance storage.

pub fn get_changelog(env: &Env) -> Vec<ChangelogEntry> {
    env.storage()
        .instance()
        .get(&symbol_short!("chlg"))
        .unwrap_or(Vec::new(env))
}

pub fn set_changelog(env: &Env, log: &Vec<ChangelogEntry>) {
    env.storage().instance().set(&symbol_short!("chlg"), log);
}

// ── Issue #115: last reward rate change ledger ────────────────────────────────
// Key "lrcl" (4 chars, short symbol) stored in instance storage.

pub fn get_last_rate_change_ledger(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("lrcl"))
        .unwrap_or(0)
}

pub fn set_last_rate_change_ledger(env: &Env, ledger: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("lrcl"), &ledger);
}

// ── Issue #116: per-user vesting entries ─────────────────────────────────────
// Key ("vest", user) stored in persistent storage (same pattern as streak).

pub fn get_vesting_entries(env: &Env, user: &Address) -> Vec<VestingEntry> {
    let key = (Symbol::new(env, "vest"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_vesting_entries(env: &Env, user: &Address, entries: &Vec<VestingEntry>) {
    let key = (Symbol::new(env, "vest"), user.clone());
    env.storage().persistent().set(&key, entries);
}

// ── Issue #117: pool initialization ledger ───────────────────────────────────
// Key "inal" (4 chars, short symbol) stored in instance storage.

pub fn get_initialized_at_ledger(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("inal"))
}

pub fn set_initialized_at_ledger(env: &Env, ledger: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("inal"), &ledger);
}

// ── Custom error messages (frontend metadata) ────────────────────────────────
// Not currently wired into any public entrypoint; kept for a future feature.

/// Maximum length for custom error messages (150 characters).
#[allow(dead_code)]
pub const MAX_ERROR_MESSAGE_LENGTH: u32 = 150;

/// Maximum number of custom error messages stored (20 messages).
#[allow(dead_code)]
pub const MAX_ERROR_MESSAGES: u32 = 20;

/// Get list of all error codes that have custom messages set.
#[allow(dead_code)]
pub fn get_error_message_codes(env: &Env) -> Vec<u32> {
    let key = symbol_short!("err_codes");
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

/// Set the list of error codes that have custom messages.
#[allow(dead_code)]
fn set_error_message_codes(env: &Env, codes: &Vec<u32>) {
    let key = symbol_short!("err_codes");
    env.storage().persistent().set(&key, codes);
}

/// Get custom error message for a specific error code.
/// Uses tuple key to avoid DataKey enum limit.
#[allow(dead_code)]
pub fn get_error_message(env: &Env, error_code: u32) -> Option<soroban_sdk::String> {
    let key = (Symbol::new(env, "err_msg"), error_code);
    env.storage().persistent().get(&key)
}

/// Set custom error message for a specific error code.
/// Enforces MAX_ERROR_MESSAGES limit by removing oldest when full.
/// Uses tuple key to avoid DataKey enum limit.
#[allow(dead_code)]
pub fn set_error_message(env: &Env, error_code: u32, message: &soroban_sdk::String) {
    // Store the message using tuple key
    let key = (Symbol::new(env, "err_msg"), error_code);
    env.storage().persistent().set(&key, message);

    // Update the codes list
    let mut codes = get_error_message_codes(env);

    // Check if this error code is already in the list
    let mut found = false;
    for i in 0..codes.len() {
        if codes.get(i).unwrap() == error_code {
            found = true;
            break;
        }
    }

    // If not found, add it to the list
    if !found {
        // If at capacity, remove the oldest (first) entry
        if codes.len() >= MAX_ERROR_MESSAGES {
            let oldest_code = codes.get(0).unwrap();
            let oldest_key = (Symbol::new(env, "err_msg"), oldest_code);
            env.storage().persistent().remove(&oldest_key);
            codes.remove(0);
        }
        codes.push_back(error_code);
        set_error_message_codes(env, &codes);
    }
}

// ── Issue #113: auto-restake toggle ───────────────────────────────────────────
// Key ("auto_rst", user) stored in persistent storage (same pattern as streak).

pub fn get_auto_restake(env: &Env, user: &Address) -> bool {
    let key = (Symbol::new(env, "auto_rst"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn set_auto_restake(env: &Env, user: &Address, enabled: bool) {
    let key = (Symbol::new(env, "auto_rst"), user.clone());
    env.storage().persistent().set(&key, &enabled);
}

// ── Issue #124: rich reward-rate history ─────────────────────────────────────
// Key "rwd_rth" stored in instance storage.

pub const MAX_RICH_RATE_HISTORY: u32 = 20;

pub fn get_reward_rate_history(env: &Env) -> Vec<RateHistoryEntry> {
    env.storage()
        .instance()
        .get(&symbol_short!("rwd_rth"))
        .unwrap_or(Vec::new(env))
}

pub fn set_reward_rate_history(env: &Env, history: &Vec<RateHistoryEntry>) {
    env.storage()
        .instance()
        .set(&symbol_short!("rwd_rth"), history);
}

// ── Referral system ───────────────────────────────────────────────────────────
// Tuple-keyed persistent storage to avoid growing the DataKey enum.

pub fn get_referrer_of(env: &Env, user: &Address) -> Option<Address> {
    let key = (Symbol::new(env, "ref_of"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_referrer_of(env: &Env, user: &Address, referrer: &Address) {
    let key = (Symbol::new(env, "ref_of"), user.clone());
    env.storage().persistent().set(&key, referrer);
}

pub fn get_referral_stats(env: &Env, referrer: &Address) -> ReferralStats {
    let key = (Symbol::new(env, "ref_st"), referrer.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(ReferralStats {
            total_referred_stake: 0,
            referral_count: 0,
        })
}

pub fn set_referral_stats(env: &Env, referrer: &Address, stats: &ReferralStats) {
    let key = (Symbol::new(env, "ref_st"), referrer.clone());
    env.storage().persistent().set(&key, stats);
}

pub fn get_referral_registry(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&symbol_short!("ref_reg"))
        .unwrap_or(Vec::new(env))
}

pub fn set_referral_registry(env: &Env, registry: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&symbol_short!("ref_reg"), registry);
}
// ── Issue #118: relayer approval ─────────────────────────────────────────────
pub fn get_approved_relayer(env: &Env, user: &Address) -> Option<Address> {
    let key = (Symbol::new(env, "app_rlyr"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_approved_relayer(env: &Env, user: &Address, relayer: &Address) {
    let key = (Symbol::new(env, "app_rlyr"), user.clone());
    env.storage().persistent().set(&key, relayer);
}

pub fn remove_approved_relayer(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "app_rlyr"), user.clone());
    env.storage().persistent().remove(&key);
}

// ── Issue #126: yield source whitelist ───────────────────────────────────────
pub fn is_yield_source(env: &Env, source: &Address) -> bool {
    let key = (Symbol::new(env, "y_source"), source.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn set_yield_source(env: &Env, source: &Address, approved: bool) {
    let key = (Symbol::new(env, "y_source"), source.clone());
    if approved {
        env.storage().persistent().set(&key, &true);
    } else {
        env.storage().persistent().remove(&key);
    }
}

pub fn get_total_rewards_added(env: &Env) -> i128 {
    let key = (Symbol::new(env, "tot_rwds"),);
    env.storage().instance().get(&key).unwrap_or(0)
}

pub fn set_total_rewards_added(env: &Env, total: i128) {
    assert_non_negative(env, total);
    let key = (Symbol::new(env, "tot_rwds"),);
    env.storage().instance().set(&key, &total);
}

// ── Issue #180: lifetime protocol fee revenue counter ────────────────────────
// Uses a direct symbol key (rather than a `DataKey` variant) because the
// `DataKey` union is already at the 50-case XDR spec limit.

pub fn get_protocol_fee_collected(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("prot_fee"))
        .unwrap_or(0)
}

pub fn add_protocol_fee_collected(env: &Env, amount: i128) {
    let total = get_protocol_fee_collected(env) + amount;
    env.storage()
        .instance()
        .set(&symbol_short!("prot_fee"), &total);
}

// ── Issue #182: off-chain webhook URL config ─────────────────────────────────
// Uses a direct symbol key for the same reason as above.

pub fn get_webhook_url(env: &Env) -> Option<String> {
    env.storage().instance().get(&symbol_short!("whk_url"))
}

pub fn set_webhook_url(env: &Env, url: &String) {
    env.storage().instance().set(&symbol_short!("whk_url"), url);
}

pub fn clear_webhook_url(env: &Env) {
    env.storage().instance().remove(&symbol_short!("whk_url"));
}

// ── Activity heatmap (7-day rolling buckets) ────────────────────────────────

pub fn get_activity_log(env: &Env) -> Vec<DayBucket> {
    env.storage()
        .instance()
        .get(&symbol_short!("act_log"))
        .unwrap_or(Vec::new(env))
}

pub fn set_activity_log(env: &Env, log: &Vec<DayBucket>) {
    env.storage().instance().set(&symbol_short!("act_log"), log);
}

// ── Issue #217: per-user claim history for tax reporting ─────────────────────

pub const MAX_CLAIM_HISTORY: u32 = 100;

pub fn get_claim_history(env: &Env, user: &Address) -> Vec<(u32, i128)> {
    let key = (Symbol::new(env, "clm_hist"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_claim_history(env: &Env, user: &Address, history: &Vec<(u32, i128)>) {
    let key = (Symbol::new(env, "clm_hist"), user.clone());
    env.storage().persistent().set(&key, history);
}

// ── Issue #219: pause info ───────────────────────────────────────────────────

pub fn get_pause_info(env: &Env) -> Option<crate::storage::PauseInfo> {
    env.storage().instance().get(&symbol_short!("ps_info"))
}

pub fn set_pause_info(env: &Env, info: &crate::storage::PauseInfo) {
    env.storage()
        .instance()
        .set(&symbol_short!("ps_info"), info);
}

pub fn clear_pause_info(env: &Env) {
    env.storage().instance().remove(&symbol_short!("ps_info"));
}

// ── Issue #556: scheduled auto-unpause ───────────────────────────────────────

/// Ledger sequence at which a `pause_until`-scheduled pause should be lifted,
/// if any. Lazily evaluated on the next call that checks pause state —
/// Soroban has no native scheduled execution, so nothing runs in the
/// background; this is just the target the next call compares against.
pub fn get_scheduled_unpause(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("sch_unp"))
}

pub fn set_scheduled_unpause(env: &Env, target_ledger: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("sch_unp"), &target_ledger);
}

pub fn clear_scheduled_unpause(env: &Env) {
    env.storage().instance().remove(&symbol_short!("sch_unp"));
}

/// Lazily lifts a `pause_until`-scheduled pause once `target_ledger` is
/// reached (issue #556). Soroban has no native scheduled execution, so this
/// only ever runs as a side effect of some other call arriving — if no one
/// calls the contract after the target ledger, the pause stays in storage
/// (still correctly reported as paused) until the next call clears it.
/// `pub(crate)` free function rather than a `VaultContract` method so every
/// mutating entrypoint that gates on pause state can call it, including
/// ones outside `vault.rs` (e.g. `xlm_wrapper_integration.rs`).
pub(crate) fn apply_scheduled_unpause_if_due(env: &Env) {
    if let Some(target_ledger) = get_scheduled_unpause(env) {
        if env.ledger().sequence() >= target_ledger {
            env.storage().instance().set(&DataKey::Paused, &false);
            clear_pause_info(env);
            clear_scheduled_unpause(env);
            let current_ledger = env.ledger().sequence();
            crate::events::auto_unpaused(env, current_ledger);
            set_last_updated_ledger(env, current_ledger);
        }
    }
}

// ── Issue #218: migration target ─────────────────────────────────────────────

pub fn get_migration_target(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("mig_tgt"))
}

pub fn set_migration_target(env: &Env, target: &Address) {
    env.storage()
        .instance()
        .set(&symbol_short!("mig_tgt"), target);
}

// ── Issue #220: rounding policy ──────────────────────────────────────────────

pub fn get_rounding_policy(env: &Env) -> crate::storage::RoundingPolicy {
    env.storage()
        .instance()
        .get(&symbol_short!("rnd_pol"))
        .unwrap_or(crate::storage::RoundingPolicy::Floor)
}

pub fn set_rounding_policy(env: &Env, policy: &crate::storage::RoundingPolicy) {
    env.storage()
        .instance()
        .set(&symbol_short!("rnd_pol"), policy);
}

// ── Issue #215: yield farming hook ────────────────────────────────────────────

pub fn get_yield_protocol(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("yld_prot"))
}

pub fn set_yield_protocol(env: &Env, protocol: &Address) {
    env.storage()
        .instance()
        .set(&symbol_short!("yld_prot"), protocol);
}

pub fn get_yield_deployed(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("yld_dep"))
        .unwrap_or(0)
}

pub fn set_yield_deployed(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&symbol_short!("yld_dep"), &amount);
}

// ── Issue #216: governance voting ─────────────────────────────────────────────

pub fn get_next_proposal_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("prop_nid"))
        .unwrap_or(0)
}

pub fn set_next_proposal_id(env: &Env, id: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("prop_nid"), &id);
}

pub fn get_open_proposal_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("prop_opn"))
        .unwrap_or(0)
}

pub fn set_open_proposal_count(env: &Env, count: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("prop_opn"), &count);
}

pub fn get_proposal(env: &Env, id: u32) -> Option<GovernanceProposal> {
    let key = (Symbol::new(env, "prop"), id);
    env.storage().persistent().get(&key)
}

pub fn set_proposal(env: &Env, id: u32, proposal: &GovernanceProposal) {
    let key = (Symbol::new(env, "prop"), id);
    env.storage().persistent().set(&key, proposal);
}

pub fn has_voted(env: &Env, proposal_id: u32, voter: &Address) -> bool {
    let key = (Symbol::new(env, "voted"), proposal_id, voter.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn set_voted(env: &Env, proposal_id: u32, voter: &Address) {
    let key = (Symbol::new(env, "voted"), proposal_id, voter.clone());
    env.storage().persistent().set(&key, &true);
}

// ── Issue #206: single-level reward-rate rollback ────────────────────────────
// DataKey is already at Soroban's 50-variant cap (see the note in
// storage.rs), so these use raw Symbol keys instead of a DataKey variant —
// the same workaround this file already uses for symbol_short!("pool_cp"),
// symbol_short!("sl_tr"), etc.

pub fn get_previous_rate(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("prevrate"))
}

pub fn set_previous_rate(env: &Env, rate_bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("prevrate"), &rate_bps);
}

pub fn clear_previous_rate(env: &Env) {
    env.storage().instance().remove(&symbol_short!("prevrate"));
}

// ── Issue #207: cross-chain bridge relayer hook ──────────────────────────────

pub fn get_bridge_packet_sequence(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&symbol_short!("brpktseq"))
        .unwrap_or(0)
}

pub fn set_bridge_packet_sequence(env: &Env, seq: u64) {
    env.storage()
        .instance()
        .set(&symbol_short!("brpktseq"), &seq);
}

pub fn is_bridge_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&symbol_short!("brenabld"))
        .unwrap_or(false)
}

pub fn set_bridge_enabled(env: &Env, enabled: bool) {
    env.storage()
        .instance()
        .set(&symbol_short!("brenabld"), &enabled);
}

// ── Issue #209: additive split positions ─────────────────────────────────────

pub fn get_split_positions(env: &Env, user: &Address) -> Vec<StakePosition> {
    let key = (Symbol::new(env, "splitpos"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_split_positions(env: &Env, user: &Address, positions: &Vec<StakePosition>) {
    let key = (Symbol::new(env, "splitpos"), user.clone());
    env.storage().persistent().set(&key, positions);
}

// ── Issue #205: DEX router used by swap_and_stake ────────────────────────────

pub fn get_dex_router(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("dexroutr"))
}

pub fn set_dex_router(env: &Env, router: &Address) {
    env.storage()
        .instance()
        .set(&symbol_short!("dexroutr"), router);
}

// ── Issue #163: lifetime total-ever-staked counter ───────────────────────────

pub fn get_total_ever_staked(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("everstk"))
        .unwrap_or(0)
}

pub fn set_total_ever_staked(env: &Env, total: i128) {
    assert_non_negative(env, total);
    env.storage()
        .instance()
        .set(&symbol_short!("everstk"), &total);
}

// ── Issue #197: fee splitting ─────────────────────────────────────────────────

pub fn get_fee_recipients(env: &Env) -> Vec<FeeRecipient> {
    env.storage()
        .instance()
        .get(&symbol_short!("feerecip"))
        .unwrap_or(Vec::new(env))
}

pub fn set_fee_recipients(env: &Env, recipients: &Vec<FeeRecipient>) {
    env.storage()
        .instance()
        .set(&symbol_short!("feerecip"), recipients);
}

// ── Issue #195: timelocked admin actions ─────────────────────────────────────

pub fn get_timelock_delay(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("tlockdly"))
        .unwrap_or(0)
}

pub fn set_timelock_delay(env: &Env, ledgers: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("tlockdly"), &ledgers);
}

pub fn get_pending_actions(env: &Env) -> Vec<PendingAction> {
    env.storage()
        .instance()
        .get(&symbol_short!("pendacts"))
        .unwrap_or(Vec::new(env))
}

pub fn set_pending_actions(env: &Env, actions: &Vec<PendingAction>) {
    env.storage()
        .instance()
        .set(&symbol_short!("pendacts"), actions);
}

pub fn next_action_id(env: &Env) -> u32 {
    let id: u32 = env
        .storage()
        .instance()
        .get(&symbol_short!("actidctr"))
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&symbol_short!("actidctr"), &(id + 1));
    id
}

// ── Issue #196: multi-sig admin ──────────────────────────────────────────────

pub fn get_multisig_config(env: &Env) -> Option<MultisigConfig> {
    env.storage().instance().get(&symbol_short!("msigcfg"))
}

pub fn set_multisig_config(env: &Env, config: &MultisigConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("msigcfg"), config);
}

pub fn get_admin_proposals(env: &Env) -> Vec<AdminProposal> {
    env.storage()
        .instance()
        .get(&symbol_short!("msigprop"))
        .unwrap_or(Vec::new(env))
}

pub fn set_admin_proposals(env: &Env, proposals: &Vec<AdminProposal>) {
    env.storage()
        .instance()
        .set(&symbol_short!("msigprop"), proposals);
}

pub fn next_proposal_id(env: &Env) -> u32 {
    let id: u32 = env
        .storage()
        .instance()
        .get(&symbol_short!("propidct"))
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&symbol_short!("propidct"), &(id + 1));
    id
}

// ── Missing helper functions for existing vault features ──────────────────────

pub fn get_stake_rate_limit(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("stk_rtl"))
        .unwrap_or(0)
}

pub fn set_stake_rate_limit(env: &Env, limit: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("stk_rtl"), &limit);
}

pub fn get_last_stake_ledger(env: &Env, user: &Address) -> Option<u32> {
    let key = (Symbol::new(env, "lst_stk"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_last_stake_ledger(env: &Env, user: &Address, ledger: u32) {
    let key = (Symbol::new(env, "lst_stk"), user.clone());
    env.storage().persistent().set(&key, &ledger);
}

pub fn get_claim_rate_limit(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("clm_rtl"))
        .unwrap_or(0)
}

pub fn set_claim_rate_limit(env: &Env, limit: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("clm_rtl"), &limit);
}

pub fn get_last_claim_action_ledger(env: &Env, user: &Address) -> Option<u32> {
    let key = (Symbol::new(env, "lst_clm"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_last_claim_action_ledger(env: &Env, user: &Address, ledger: u32) {
    let key = (Symbol::new(env, "lst_clm"), user.clone());
    env.storage().persistent().set(&key, &ledger);
}

// ── Issue #198: penalty redistribution ───────────────────────────────────────

pub fn get_penalty_redistribution_mode(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&symbol_short!("pen_rdst"))
        .unwrap_or(false)
}

/// Setter for the penalty-redistribution flag. No entrypoint toggles it yet —
/// the read side is wired up and this is here to complete the pair.
#[allow(dead_code)]
pub fn set_penalty_redistribution_mode(env: &Env, enabled: bool) {
    env.storage()
        .instance()
        .set(&symbol_short!("pen_rdst"), &enabled);
}

// ── Issue #199: insurance fund ────────────────────────────────────────────────

pub fn set_insurance_rate_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("ins_rate"), &bps);
}

pub fn get_insurance_rate_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("ins_rate"))
        .unwrap_or(0)
}

pub fn get_insurance_fund_balance(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("ins_fund"))
        .unwrap_or(0)
}

pub fn set_insurance_fund_balance(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&symbol_short!("ins_fund"), &amount);
}

// ── Issue #200: delegation chain ──────────────────────────────────────────────

pub fn get_delegation_chain(env: &Env, user: &Address) -> Option<crate::storage::DelegationChain> {
    let key = (Symbol::new(env, "dchain"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_delegation_chain(env: &Env, user: &Address, chain: &crate::storage::DelegationChain) {
    let key = (Symbol::new(env, "dchain"), user.clone());
    env.storage().persistent().set(&key, chain);
}

pub fn remove_delegation_chain(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "dchain"), user.clone());
    env.storage().persistent().remove(&key);
}

// ── Issue #202: bootstrap ─────────────────────────────────────────────────────

pub fn set_bootstrap_config(env: &Env, config: &crate::storage::BootstrapConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("bootcfg"), config);
}

pub fn get_bootstrap_config(env: &Env) -> Option<crate::storage::BootstrapConfig> {
    env.storage().instance().get(&symbol_short!("bootcfg"))
}

pub fn clear_bootstrap_config(env: &Env) {
    env.storage().instance().remove(&symbol_short!("bootcfg"));
}

// ── Issue #213: dynamic fee config ────────────────────────────────────────────

pub fn set_dynamic_fee_config(env: &Env, config: &DynamicFeeConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("dfeecfg"), config);
}

pub fn get_dynamic_fee_config(env: &Env) -> Option<DynamicFeeConfig> {
    env.storage().instance().get(&symbol_short!("dfeecfg"))
}

// ── Issue #214: user claim count (for reputation consistency score) ────────────

pub fn get_user_claim_count(env: &Env, user: &Address) -> u32 {
    let key = (Symbol::new(env, "clm_cnt"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn increment_user_claim_count(env: &Env, user: &Address) {
    let current = get_user_claim_count(env, user);
    let key = (Symbol::new(env, "clm_cnt"), user.clone());
    env.storage().persistent().set(&key, &(current + 1));
}

// ── Issue #210: Merkle Reward Distribution ────────────────────────────────────

pub fn set_merkle_root(env: &Env, root: &crate::storage::MerkleRoot) {
    env.storage().instance().set(&symbol_short!("merkle"), root);
}

pub fn get_merkle_root(env: &Env) -> Option<crate::storage::MerkleRoot> {
    env.storage().instance().get(&symbol_short!("merkle"))
}

pub fn is_merkle_claimed(env: &Env, user: &Address, epoch: u32) -> bool {
    let key = (Symbol::new(env, "mrk_clm"), user.clone(), epoch);
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn set_merkle_claimed(env: &Env, user: &Address, epoch: u32) {
    let key = (Symbol::new(env, "mrk_clm"), user.clone(), epoch);
    env.storage().persistent().set(&key, &true);
}

// ── Issue #211: Staking Tournament Competition ────────────────────────────────

pub fn set_tournament(env: &Env, tournament: &crate::storage::Tournament) {
    env.storage()
        .instance()
        .set(&symbol_short!("tourney"), tournament);
}

pub fn get_tournament(env: &Env) -> Option<crate::storage::Tournament> {
    env.storage().instance().get(&symbol_short!("tourney"))
}

/// Clears the stored tournament. `finalize_tournament` marks the tournament
/// finalized in place rather than deleting it, so nothing calls this yet.
#[allow(dead_code)]
pub fn remove_tournament(env: &Env) {
    env.storage().instance().remove(&symbol_short!("tourney"));
}

pub fn get_tournament_score(env: &Env, user: &Address) -> i128 {
    let key = (Symbol::new(env, "tour_scr"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_tournament_score(env: &Env, user: &Address, score: i128) {
    let key = (Symbol::new(env, "tour_scr"), user.clone());
    env.storage().persistent().set(&key, &score);
}

pub fn get_tournament_participants(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&symbol_short!("tour_par"))
        .unwrap_or(Vec::new(env))
}

pub fn set_tournament_participants(env: &Env, participants: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&symbol_short!("tour_par"), participants);
}

pub fn add_tournament_participant(env: &Env, user: &Address) {
    let mut participants = get_tournament_participants(env);
    let mut exists = false;
    for i in 0..participants.len() {
        if participants.get(i).unwrap() == *user {
            exists = true;
            break;
        }
    }
    if !exists {
        participants.push_back(user.clone());
        set_tournament_participants(env, &participants);
    }
}

// ── Issue #212: Buyback & Burn ────────────────────────────────────────────────

pub fn buyback_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&symbol_short!("bybk_enb"))
        .unwrap_or(false)
}

pub fn set_buyback_enabled(env: &Env, enabled: bool) {
    env.storage()
        .instance()
        .set(&symbol_short!("bybk_enb"), &enabled);
}

pub fn get_buyback_threshold(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("bybk_thr"))
        .unwrap_or(0)
}

pub fn set_buyback_threshold(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&symbol_short!("bybk_thr"), &amount);
}

pub fn get_total_tokens_burned(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("tot_burn"))
        .unwrap_or(0)
}

pub fn add_tokens_burned(env: &Env, amount: i128) {
    let total = get_total_tokens_burned(env) + amount;
    env.storage()
        .instance()
        .set(&symbol_short!("tot_burn"), &total);
    // Issue #452: check burn milestones without importing module to avoid cycle
    {
        let thresholds: soroban_sdk::Vec<i128> = env
            .storage()
            .instance()
            .get(&symbol_short!("burn_thr"))
            .unwrap_or(soroban_sdk::Vec::new(env));
        if !thresholds.is_empty() {
            let total_fees: i128 = env
                .storage()
                .instance()
                .get(&symbol_short!("fbb_brn"))
                .unwrap_or(0);
            let total_burned = total.saturating_add(total_fees);
            let mut reached: soroban_sdk::Vec<bool> = env
                .storage()
                .instance()
                .get(&symbol_short!("burn_hit"))
                .unwrap_or(soroban_sdk::Vec::new(env));
            if reached.len() != thresholds.len() {
                let mut new_reached = soroban_sdk::Vec::new(env);
                for _ in 0..thresholds.len() {
                    new_reached.push_back(false);
                }
                let min_len = if reached.len() < thresholds.len() {
                    reached.len()
                } else {
                    thresholds.len()
                };
                for i in 0..min_len {
                    new_reached.set(i, reached.get(i).unwrap());
                }
                reached = new_reached;
            }
            let ledger = env.ledger().sequence();
            let mut changed = false;
            for i in 0..thresholds.len() {
                let thr = thresholds.get(i).unwrap();
                let is_reached = reached.get(i).unwrap();
                if !is_reached && total_burned >= thr {
                    reached.set(i, true);
                    changed = true;
                    env.events().publish(
                        (symbol_short!("burn_ms"),),
                        (thr, total_burned, amount, ledger),
                    );
                }
            }
            if changed {
                env.storage()
                    .instance()
                    .set(&symbol_short!("burn_hit"), &reached);
            }
        }
    }
}

// ── Issue #231: Halving Schedule ──────────────────────────────────────────────

pub fn set_halving_config(env: &Env, config: &crate::storage::HalvingConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("halvcfg"), config);
}

pub fn get_halving_config(env: &Env) -> Option<crate::storage::HalvingConfig> {
    env.storage().instance().get(&symbol_short!("halvcfg"))
}

/// Compute the number of halvings that have occurred up to `ledger`.
pub fn halving_count_at(env: &Env, ledger: u32) -> u32 {
    match get_halving_config(env) {
        Some(config) if config.interval_ledgers > 0 && ledger > config.started_at => {
            (ledger - config.started_at) / config.interval_ledgers
        }
        _ => 0,
    }
}

/// Return the ledger at which the next halving will occur, if any.
pub fn next_halving_at(env: &Env) -> Option<u32> {
    get_halving_config(env).and_then(|config| {
        if config.interval_ledgers == 0 {
            return None;
        }
        let current = env.ledger().sequence();
        let count = if current > config.started_at {
            (current - config.started_at) / config.interval_ledgers
        } else {
            0
        };
        let next_boundary = config.started_at + (count + 1) * config.interval_ledgers;
        Some(next_boundary)
    })
}

/// Default per-interval reduction (50%) used when a halving schedule has no
/// explicit `reduction_bps` stored, i.e. a schedule written before issue #701.
pub const DEFAULT_HALVING_REDUCTION_BPS: u32 = 5_000;

/// Issue #701: per-interval reduction of the halving schedule, in bps.
pub fn get_halving_reduction_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("halv_red"))
        .unwrap_or(DEFAULT_HALVING_REDUCTION_BPS)
}

pub fn set_halving_reduction_bps(env: &Env, reduction_bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("halv_red"), &reduction_bps);
}

/// Issue #701: remove the halving schedule so the static rate applies again.
pub fn clear_halving_config(env: &Env) {
    env.storage().instance().remove(&symbol_short!("halvcfg"));
    env.storage().instance().remove(&symbol_short!("halv_red"));
}

/// Compute the effective rate with the reduction schedule applied: the base
/// rate is cut by `reduction_bps` once per elapsed interval (a 5_000 bps cut
/// is the classic halving, `base_rate / 2^halving_count`), never going below
/// `floor_rate_bps`. If no halving config exists, returns base_rate.
pub fn halving_adjusted_rate(env: &Env, base_rate_bps: u32, ledger: u32) -> i128 {
    match get_halving_config(env) {
        Some(config) => {
            let count = halving_count_at(env, ledger);
            let keep_bps = 10_000i128 - get_halving_reduction_bps(env) as i128;
            let mut effective = base_rate_bps as i128;
            let mut applied = 0u32;
            // Stops as soon as the floor is reached, so the loop is bounded by
            // the rate itself rather than by the number of elapsed intervals.
            while applied < count && effective > config.floor_rate_bps {
                effective = effective * keep_bps / 10_000;
                applied += 1;
            }
            effective.max(config.floor_rate_bps)
        }
        None => base_rate_bps as i128,
    }
}

// ── Issue #222: Staking Certificate ───────────────────────────────────────────

pub fn set_min_cert_amount(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&symbol_short!("min_cert"), &amount);
}

pub fn get_min_cert_amount(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("min_cert"))
        .unwrap_or(0)
}

pub fn get_certificate_counter(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("cert_cnt"))
        .unwrap_or(0)
}

pub fn set_certificate_counter(env: &Env, count: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("cert_cnt"), &count);
}

pub fn get_certificate(env: &Env, user: &Address) -> Option<crate::storage::StakingCertificate> {
    let key = (Symbol::new(env, "cert"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_certificate(env: &Env, user: &Address, cert: &crate::storage::StakingCertificate) {
    let key = (Symbol::new(env, "cert"), user.clone());
    env.storage().persistent().set(&key, cert);
}

pub fn remove_certificate(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "cert"), user.clone());
    env.storage().persistent().remove(&key);
}

// ── Issue #233: Minimum Pool Size to Activate ─────────────────────────────────

pub fn set_activation_threshold(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&symbol_short!("act_thr"), &amount);
}

pub fn get_activation_threshold(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("act_thr"))
        .unwrap_or(0)
}

pub fn get_pool_was_active(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&symbol_short!("pool_actv"))
        .unwrap_or(false)
}

pub fn set_pool_was_active(env: &Env, active: bool) {
    env.storage()
        .instance()
        .set(&symbol_short!("pool_actv"), &active);
}

// ── Issue #232: Stake Expiry ──────────────────────────────────────────────────

pub fn set_max_stake_duration(env: &Env, ledgers: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("max_st_d"), &ledgers);
}

pub fn get_max_stake_duration(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("max_st_d"))
        .unwrap_or(0)
}

pub fn set_position_expired_emitted(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "exp_emit"), user.clone());
    env.storage().persistent().set(&key, &true);
}

pub fn get_position_expired_emitted(env: &Env, user: &Address) -> bool {
    let key = (Symbol::new(env, "exp_emit"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

// ── Issue #234: Minimum Pool Size to Activate Rewards ─────────────────────────
// `min_tvl` holds the TVL threshold (0 = feature off); `rwd_actv` latches the
// ledger at which the pool first reached it.

pub fn set_min_pool_size(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&symbol_short!("min_tvl"), &amount);
}

pub fn get_min_pool_size(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("min_tvl"))
        .unwrap_or(0)
}

pub fn set_rewards_activated_at(env: &Env, ledger: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("rwd_actv"), &ledger);
}

pub fn get_rewards_activated_at(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("rwd_actv"))
}

// ── Issue #235: Reward Smoothing ──────────────────────────────────────────────

pub fn set_smoothing_period(env: &Env, ledgers: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("smth_per"), &ledgers);
}

pub fn get_smoothing_period(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("smth_per"))
        .unwrap_or(0)
}

pub fn set_smoothing_min_amount(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&symbol_short!("smth_min"), &amount);
}

pub fn get_smoothing_min_amount(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("smth_min"))
        .unwrap_or(0)
}

pub fn set_smoothing_schedule(env: &Env, schedule: &crate::storage::SmoothingSchedule) {
    env.storage()
        .instance()
        .set(&symbol_short!("smth_sch"), schedule);
}

pub fn get_smoothing_schedule(env: &Env) -> Option<crate::storage::SmoothingSchedule> {
    env.storage().instance().get(&symbol_short!("smth_sch"))
}

// ── Issue #236: Referral Tree ─────────────────────────────────────────────────
// Reverse index of the existing `ref_of` mapping: referrer -> direct referrals.

/// Maximum direct referrals recorded per referrer for tree traversal. Bounds the
/// worst-case read cost of `referral_tree_data()`; referral *stats* are still
/// credited for referrals beyond this cap.
pub const MAX_REFEREES_PER_NODE: u32 = 20;

pub fn get_referees(env: &Env, referrer: &Address) -> Vec<Address> {
    let key = (Symbol::new(env, "ref_kids"), referrer.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn add_referee(env: &Env, referrer: &Address, referee: &Address) {
    let mut referees = get_referees(env, referrer);
    if referees.len() >= MAX_REFEREES_PER_NODE {
        return;
    }
    referees.push_back(referee.clone());
    let key = (Symbol::new(env, "ref_kids"), referrer.clone());
    env.storage().persistent().set(&key, &referees);
}

// ── Issue #237: Capacity Auction ──────────────────────────────────────────────

pub fn set_capacity_auction(env: &Env, auction: &crate::storage::CapacityAuction) {
    env.storage()
        .instance()
        .set(&symbol_short!("cap_auct"), auction);
}

pub fn get_capacity_auction(env: &Env) -> Option<crate::storage::CapacityAuction> {
    env.storage().instance().get(&symbol_short!("cap_auct"))
}

pub fn get_auction_bids(env: &Env) -> Vec<crate::storage::AuctionBid> {
    env.storage()
        .instance()
        .get(&symbol_short!("auct_bid"))
        .unwrap_or(Vec::new(env))
}

pub fn set_auction_bids(env: &Env, bids: &Vec<crate::storage::AuctionBid>) {
    env.storage()
        .instance()
        .set(&symbol_short!("auct_bid"), bids);
}

pub fn has_pool_spot(env: &Env, user: &Address) -> bool {
    let key = (Symbol::new(env, "pool_spot"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn grant_pool_spot(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "pool_spot"), user.clone());
    env.storage().persistent().set(&key, &true);
}

pub fn set_auction_mode(env: &Env, enabled: bool) {
    env.storage()
        .instance()
        .set(&symbol_short!("auct_mod"), &enabled);
}

pub fn get_auction_mode(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&symbol_short!("auct_mod"))
        .unwrap_or(false)
}

// ── Issue #239: stake-weighted lottery ────────────────────────────────────────

pub fn get_lottery_config(env: &Env) -> Option<LotteryConfig> {
    env.storage().instance().get(&symbol_short!("lottery"))
}

pub fn set_lottery_config(env: &Env, config: &LotteryConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("lottery"), config);
}

// ── Issue #238: loyalty milestone badges ──────────────────────────────────────

pub fn get_milestones(env: &Env) -> Vec<Milestone> {
    env.storage()
        .instance()
        .get(&symbol_short!("milestns"))
        .unwrap_or(Vec::new(env))
}

pub fn set_milestones(env: &Env, milestones: &Vec<Milestone>) {
    env.storage()
        .instance()
        .set(&symbol_short!("milestns"), milestones);
}

pub fn get_user_milestones(env: &Env, user: &Address) -> Vec<u32> {
    let key = (Symbol::new(env, "usr_mstn"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_user_milestones(env: &Env, user: &Address, ids: &Vec<u32>) {
    let key = (Symbol::new(env, "usr_mstn"), user.clone());
    env.storage().persistent().set(&key, ids);
}

pub fn get_latest_achievement_ledger(env: &Env, user: &Address) -> u32 {
    let key = (Symbol::new(env, "lat_mstn"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_latest_achievement_ledger(env: &Env, user: &Address, ledger: u32) {
    let key = (Symbol::new(env, "lat_mstn"), user.clone());
    env.storage().persistent().set(&key, &ledger);
}

// ── Issue #240: oracle-triggered lock-up release ──────────────────────────────

pub fn get_oracle_contract(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("oracle"))
}

pub fn set_oracle_contract(env: &Env, oracle: &Address) {
    env.storage()
        .instance()
        .set(&symbol_short!("oracle"), oracle);
}

pub fn get_price_condition(env: &Env, user: &Address) -> Option<PriceCondition> {
    let key = (Symbol::new(env, "pcond"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_price_condition(env: &Env, user: &Address, condition: &PriceCondition) {
    let key = (Symbol::new(env, "pcond"), user.clone());
    env.storage().persistent().set(&key, condition);
}

pub fn is_lockup_waived(env: &Env, user: &Address) -> bool {
    let key = (Symbol::new(env, "lck_wvd"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn set_lockup_waived(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "lck_wvd"), user.clone());
    env.storage().persistent().set(&key, &true);
}

// ── Issue #241: governance proposal veto ──────────────────────────────────────

pub fn get_veto_threshold_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("veto_bps"))
        .unwrap_or(0)
}

pub fn set_veto_threshold_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("veto_bps"), &bps);
}

pub fn get_proposal_vetoer(env: &Env, proposal_id: u32) -> Option<Address> {
    let key = (Symbol::new(env, "vetoer"), proposal_id);
    env.storage().persistent().get(&key)
}

pub fn set_proposal_vetoer(env: &Env, proposal_id: u32, vetoer: &Address) {
    let key = (Symbol::new(env, "vetoer"), proposal_id);
    env.storage().persistent().set(&key, vetoer);
}

// ── Issue #256: governance vote weight delegation ─────────────────────────────

pub fn get_vote_delegate(env: &Env, user: &Address) -> Option<Address> {
    let key = (Symbol::new(env, "votedeleg"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_vote_delegate(env: &Env, user: &Address, delegate: &Address) {
    let key = (Symbol::new(env, "votedeleg"), user.clone());
    env.storage().persistent().set(&key, delegate);
}

pub fn remove_vote_delegate(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "votedeleg"), user.clone());
    env.storage().persistent().remove(&key);
}

/// Snapshot of the weight `user` contributed to their delegate at the moment
/// they delegated — used to subtract exactly this amount from the delegate's
/// `delegated_vote_weight` accumulator on revoke, rather than a possibly
/// since-changed live recomputation.
pub fn get_delegated_weight_snapshot(env: &Env, user: &Address) -> i128 {
    let key = (Symbol::new(env, "delegsnap"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_delegated_weight_snapshot(env: &Env, user: &Address, weight: i128) {
    let key = (Symbol::new(env, "delegsnap"), user.clone());
    env.storage().persistent().set(&key, &weight);
}

pub fn remove_delegated_weight_snapshot(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "delegsnap"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_delegated_vote_weight(env: &Env, delegate: &Address) -> i128 {
    let key = (Symbol::new(env, "delegwt"), delegate.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_delegated_vote_weight(env: &Env, delegate: &Address, weight: i128) {
    let key = (Symbol::new(env, "delegwt"), delegate.clone());
    env.storage().persistent().set(&key, &weight);
}

// ── Issue #257: auto-convert reward on claim ──────────────────────────────────

pub fn get_auto_convert_config(env: &Env, user: &Address) -> Option<AutoConvertConfig> {
    let key = (Symbol::new(env, "autoconv"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_auto_convert_config(env: &Env, user: &Address, config: &AutoConvertConfig) {
    let key = (Symbol::new(env, "autoconv"), user.clone());
    env.storage().persistent().set(&key, config);
}

pub fn remove_auto_convert_config(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "autoconv"), user.clone());
    env.storage().persistent().remove(&key);
}

// ── Issue #251: exit-queue priority bidding ───────────────────────────────────

pub fn get_exit_queue(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&symbol_short!("exitq"))
        .unwrap_or(Vec::new(env))
}

pub fn set_exit_queue(env: &Env, queue: &Vec<Address>) {
    env.storage().instance().set(&symbol_short!("exitq"), queue);
}

pub fn get_min_priority_bid(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("minpbid"))
        .unwrap_or(0)
}

pub fn set_min_priority_bid(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&symbol_short!("minpbid"), &amount);
}

pub fn get_priority_bids(env: &Env) -> Vec<PriorityBidRecord> {
    env.storage()
        .instance()
        .get(&symbol_short!("pbidrec"))
        .unwrap_or(Vec::new(env))
}

pub fn set_priority_bids(env: &Env, records: &Vec<PriorityBidRecord>) {
    env.storage()
        .instance()
        .set(&symbol_short!("pbidrec"), records);
}

// ── Issue #258: pool whitelabel branding ──────────────────────────────────────

pub fn get_branding(env: &Env) -> Option<BrandingConfig> {
    env.storage().instance().get(&symbol_short!("branding"))
}

pub fn set_branding(env: &Env, config: &BrandingConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("branding"), config);
}

// ── Issue #259: staking insurance ─────────────────────────────────────────────

pub fn get_insurance_product(env: &Env) -> Option<InsuranceProduct> {
    env.storage().instance().get(&symbol_short!("ins_prod"))
}

pub fn set_insurance_product(env: &Env, product: &InsuranceProduct) {
    env.storage()
        .instance()
        .set(&symbol_short!("ins_prod"), product);
}

pub fn get_insurance_policy(env: &Env, user: &Address) -> Option<InsurancePolicy> {
    let key = (Symbol::new(env, "ins_pol"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_insurance_policy(env: &Env, user: &Address, policy: &InsurancePolicy) {
    let key = (Symbol::new(env, "ins_pol"), user.clone());
    env.storage().persistent().set(&key, policy);
}

pub fn remove_insurance_policy(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "ins_pol"), user.clone());
    env.storage().persistent().remove(&key);
}

// ── Issue #260: flash stake ───────────────────────────────────────────────────

pub fn get_flash_stake_fee_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("fs_fee"))
        .unwrap_or(0)
}

pub fn set_flash_stake_fee_bps(env: &Env, bps: u32) {
    env.storage().instance().set(&symbol_short!("fs_fee"), &bps);
}

/// Monotonic counter backing `FlashStakeReceipt::receipt_id`. Returns the id
/// to use for the next receipt and advances the counter.
pub fn next_flash_receipt_id(env: &Env) -> u64 {
    let key = symbol_short!("fs_seq");
    let next: u64 = env.storage().instance().get(&key).unwrap_or(0) + 1;
    env.storage().instance().set(&key, &next);
    next
}

/// `FlashStakeReceiptLog(receipt_id)` — permanent proof, never removed.
pub fn get_flash_receipt(env: &Env, receipt_id: u64) -> Option<FlashStakeReceipt> {
    let key = (Symbol::new(env, "fs_rcpt"), receipt_id);
    env.storage().persistent().get(&key)
}

pub fn set_flash_receipt(env: &Env, receipt_id: u64, receipt: &FlashStakeReceipt) {
    let key = (Symbol::new(env, "fs_rcpt"), receipt_id);
    env.storage().persistent().set(&key, receipt);
}

// ── Issue #261: stake-backed loans ────────────────────────────────────────────

pub fn get_loan_config(env: &Env) -> Option<LoanConfig> {
    env.storage().instance().get(&symbol_short!("loan_cfg"))
}

pub fn set_loan_config(env: &Env, config: &LoanConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("loan_cfg"), config);
}

pub fn get_loan(env: &Env, user: &Address) -> Option<Loan> {
    let key = (Symbol::new(env, "loan"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_loan(env: &Env, user: &Address, loan: &Loan) {
    let key = (Symbol::new(env, "loan"), user.clone());
    env.storage().persistent().set(&key, loan);
}

pub fn remove_loan(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "loan"), user.clone());
    env.storage().persistent().remove(&key);
}

// ── Issue #276: seasonal reward multiplier ────────────────────────────────────

pub fn get_seasons(env: &Env) -> Vec<Season> {
    env.storage()
        .instance()
        .get(&symbol_short!("seasons"))
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_seasons(env: &Env, seasons: &Vec<Season>) {
    env.storage()
        .instance()
        .set(&symbol_short!("seasons"), seasons);
}

/// `starts_at` of the season `maybe_emit_season_transition()` last observed
/// as active, so it can detect start/end boundary crossings lazily. Absent
/// when no season has been observed active yet (or the last observed one
/// has since ended).
pub fn get_last_active_season_marker(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("seas_lst"))
}

pub fn set_last_active_season_marker(env: &Env, marker: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("seas_lst"), &marker);
}

pub fn clear_last_active_season_marker(env: &Env) {
    env.storage().instance().remove(&symbol_short!("seas_lst"));
}

// ── Issue #274: staker bio ────────────────────────────────────────────────────

pub fn get_staker_bio(env: &Env, user: &Address) -> Option<String> {
    let key = (Symbol::new(env, "bio"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_staker_bio(env: &Env, user: &Address, bio: &String) {
    let key = (Symbol::new(env, "bio"), user.clone());
    env.storage().persistent().set(&key, bio);
}

pub fn remove_staker_bio(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "bio"), user.clone());
    env.storage().persistent().remove(&key);
}

// ── Issue #298: pool sunsetting workflow ──────────────────────────────────────

pub fn get_sunset_state(env: &Env) -> SunsetState {
    env.storage()
        .instance()
        .get(&symbol_short!("snst_st"))
        .unwrap_or(SunsetState::Active)
}

pub fn set_sunset_state(env: &Env, state: SunsetState) {
    env.storage()
        .instance()
        .set(&symbol_short!("snst_st"), &state);
}

pub fn get_grace_period_end(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("snst_gpe"))
}

pub fn set_grace_period_end(env: &Env, ledger: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("snst_gpe"), &ledger);
}

/// The ledger by which existing users are asked to have exited a sunsetting
/// pool, set once by `initiate_sunset` (issue #525). `None` until then, and
/// never cleared afterwards: the sunset is a one-way action.
///
/// Symbol-keyed because `DataKey` is at Soroban's 50-variant cap, the same
/// reason the #298 sunset accessors above avoid a new variant.
pub fn get_sunset_exit_deadline(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("snst_dl"))
}

pub fn set_sunset_exit_deadline(env: &Env, deadline: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("snst_dl"), &deadline);
}

// ── Issue #281: Fee Revenue Sharing ──────────────────────────────────────────

pub fn get_revenue_sharing_config(env: &Env) -> Option<RevenueSharingConfig> {
    env.storage().instance().get(&symbol_short!("rev_cfg"))
}

pub fn set_revenue_sharing_config(env: &Env, config: &RevenueSharingConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("rev_cfg"), config);
}

pub fn get_revenue_share_pool(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("rev_pool"))
        .unwrap_or(0)
}

pub fn set_revenue_share_pool(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&symbol_short!("rev_pool"), &amount);
}

pub fn get_revenue_share_epoch(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("rev_ep"))
        .unwrap_or(0)
}

pub fn set_revenue_share_epoch(env: &Env, epoch: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("rev_ep"), &epoch);
}

pub fn get_revenue_share_merkle_root(env: &Env) -> Option<RevenueShareMerkleRoot> {
    env.storage().instance().get(&symbol_short!("rev_mrk"))
}

pub fn set_revenue_share_merkle_root(env: &Env, root: &RevenueShareMerkleRoot) {
    env.storage()
        .instance()
        .set(&symbol_short!("rev_mrk"), root);
}

pub fn is_revenue_share_claimed(env: &Env, user: &Address, epoch: u32) -> bool {
    let key = (Symbol::new(env, "rev_clm"), user.clone(), epoch);
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn set_revenue_share_claimed(env: &Env, user: &Address, epoch: u32) {
    let key = (Symbol::new(env, "rev_clm"), user.clone(), epoch);
    env.storage().persistent().set(&key, &true);
}

// ── Issue #280: New Staker Reward Escrow ────────────────────────────────────

pub fn get_escrow_period(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("esc_prd"))
        .unwrap_or(0)
}

pub fn set_escrow_period(env: &Env, ledgers: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("esc_prd"), &ledgers);
}

pub fn get_escrow_balance(env: &Env, user: &Address) -> i128 {
    let key = (Symbol::new(env, "esc_bal"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_escrow_balance(env: &Env, user: &Address, amount: i128) {
    assert_non_negative(env, amount);
    let key = (Symbol::new(env, "esc_bal"), user.clone());
    env.storage().persistent().set(&key, &amount);
}

pub fn remove_escrow_balance(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "esc_bal"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_escrow_release_ledger(env: &Env, user: &Address) -> Option<u32> {
    let key = (Symbol::new(env, "esc_rel"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_escrow_release_ledger(env: &Env, user: &Address, ledger: u32) {
    let key = (Symbol::new(env, "esc_rel"), user.clone());
    env.storage().persistent().set(&key, &ledger);
}

pub fn remove_escrow_release_ledger(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "esc_rel"), user.clone());
    env.storage().persistent().remove(&key);
}

// ── Issue #282: Stake-Gated Access ───────────────────────────────────────────

pub fn get_access_tiers(env: &Env) -> Vec<AccessTier> {
    env.storage()
        .instance()
        .get(&symbol_short!("acc_tier"))
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_access_tiers(env: &Env, tiers: &Vec<AccessTier>) {
    env.storage()
        .instance()
        .set(&symbol_short!("acc_tier"), tiers);
}

pub fn get_user_access_tier(env: &Env, user: &Address) -> Option<u32> {
    let key = (Symbol::new(env, "acc_u_tr"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_user_access_tier(env: &Env, user: &Address, tier_index: u32) {
    let key = (Symbol::new(env, "acc_u_tr"), user.clone());
    env.storage().persistent().set(&key, &tier_index);
}

pub fn remove_user_access_tier(env: &Env, user: &Address) {
    let key = (Symbol::new(env, "acc_u_tr"), user.clone());
    env.storage().persistent().remove(&key);
}

// ── Issue #313: anniversary bonus ────────────────────────────────────────────

pub fn get_anniversary_bonus_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&Symbol::new(env, "anniv_bps"))
        .unwrap_or(0)
}

pub fn set_anniversary_bonus_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&Symbol::new(env, "anniv_bps"), &bps);
}

pub fn get_anniversaries_paid(env: &Env, user: &Address) -> Vec<u32> {
    let key = (Symbol::new(env, "anniv_pd"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_anniversaries_paid(env: &Env, user: &Address, paid: &Vec<u32>) {
    let key = (Symbol::new(env, "anniv_pd"), user.clone());
    env.storage().persistent().set(&key, paid);
}

// ── Issue #314: withdrawal receipt ───────────────────────────────────────────

pub fn get_withdrawal_receipt_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&Symbol::new(env, "wd_rcpt_n"))
        .unwrap_or(0)
}

pub fn set_withdrawal_receipt_counter(env: &Env, counter: u64) {
    env.storage()
        .instance()
        .set(&Symbol::new(env, "wd_rcpt_n"), &counter);
}

pub fn get_withdrawal_receipt(
    env: &Env,
    receipt_id: u64,
) -> Option<crate::storage::WithdrawalReceipt> {
    let key = (Symbol::new(env, "wd_rcpt"), receipt_id);
    env.storage().persistent().get(&key)
}

pub fn set_withdrawal_receipt(env: &Env, receipt: &crate::storage::WithdrawalReceipt) {
    let key = (Symbol::new(env, "wd_rcpt"), receipt.receipt_id);
    env.storage().persistent().set(&key, receipt);
}

pub fn get_user_receipt_ids(env: &Env, user: &Address) -> Vec<u64> {
    let key = (Symbol::new(env, "wd_u_rcp"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_user_receipt_ids(env: &Env, user: &Address, ids: &Vec<u64>) {
    let key = (Symbol::new(env, "wd_u_rcp"), user.clone());
    env.storage().persistent().set(&key, ids);
}

// ── Issue #315: lot size ────────────────────────────────────────────────────

pub fn get_lot_size(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&Symbol::new(env, "lot_size"))
        .unwrap_or(0)
}

pub fn set_lot_size(env: &Env, lot_size: i128) {
    env.storage()
        .instance()
        .set(&Symbol::new(env, "lot_size"), &lot_size);
}

// ── Issue #308: unstake-fee-funded buyback & burn ────────────────────────────

pub fn fee_buyback_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&symbol_short!("fbb_enb"))
        .unwrap_or(false)
}

pub fn set_fee_buyback_enabled(env: &Env, enabled: bool) {
    env.storage()
        .instance()
        .set(&symbol_short!("fbb_enb"), &enabled);
}

pub fn get_unstake_fee_reserve(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("fbb_rsv"))
        .unwrap_or(0)
}

pub fn add_unstake_fee_reserve(env: &Env, amount: i128) {
    let total = get_unstake_fee_reserve(env) + amount;
    env.storage()
        .instance()
        .set(&symbol_short!("fbb_rsv"), &total);
}

pub fn set_unstake_fee_reserve(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&symbol_short!("fbb_rsv"), &amount);
}

pub fn get_fees_burned(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("fbb_brn"))
        .unwrap_or(0)
}

pub fn add_fees_burned(env: &Env, amount: i128) {
    let total = get_fees_burned(env) + amount;
    env.storage()
        .instance()
        .set(&symbol_short!("fbb_brn"), &total);
    // Issue #452: same milestone check as add_tokens_burned but with fees path
    {
        let thresholds: soroban_sdk::Vec<i128> = env
            .storage()
            .instance()
            .get(&symbol_short!("burn_thr"))
            .unwrap_or(soroban_sdk::Vec::new(env));
        if !thresholds.is_empty() {
            let total_tokens: i128 = env
                .storage()
                .instance()
                .get(&symbol_short!("tot_burn"))
                .unwrap_or(0);
            let total_burned = total.saturating_add(total_tokens);
            let mut reached: soroban_sdk::Vec<bool> = env
                .storage()
                .instance()
                .get(&symbol_short!("burn_hit"))
                .unwrap_or(soroban_sdk::Vec::new(env));
            if reached.len() != thresholds.len() {
                let mut new_reached = soroban_sdk::Vec::new(env);
                for _ in 0..thresholds.len() {
                    new_reached.push_back(false);
                }
                let min_len = if reached.len() < thresholds.len() {
                    reached.len()
                } else {
                    thresholds.len()
                };
                for i in 0..min_len {
                    new_reached.set(i, reached.get(i).unwrap());
                }
                reached = new_reached;
            }
            let ledger = env.ledger().sequence();
            let mut changed = false;
            for i in 0..thresholds.len() {
                let thr = thresholds.get(i).unwrap();
                let is_reached = reached.get(i).unwrap();
                if !is_reached && total_burned >= thr {
                    reached.set(i, true);
                    changed = true;
                    env.events().publish(
                        (symbol_short!("burn_ms"),),
                        (thr, total_burned, amount, ledger),
                    );
                }
            }
            if changed {
                env.storage()
                    .instance()
                    .set(&symbol_short!("burn_hit"), &reached);
            }
        }
    }
}

// ── Issue #309: staker onboarding checklist ──────────────────────────────────

pub fn get_onboarding_checklist(env: &Env, user: &Address) -> OnboardingChecklist {
    env.storage()
        .persistent()
        .get(&(Symbol::new(env, "onbchk"), user.clone()))
        .unwrap_or(OnboardingChecklist {
            has_staked: false,
            has_claimed: false,
            has_set_bio: false,
            has_enabled_streaming: false,
            has_set_auto_restake: false,
            completed_at: None,
        })
}

pub fn set_onboarding_checklist(env: &Env, user: &Address, checklist: &OnboardingChecklist) {
    env.storage()
        .persistent()
        .set(&(Symbol::new(env, "onbchk"), user.clone()), checklist);
}

pub fn get_streaming_enabled(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&(Symbol::new(env, "strmenb"), user.clone()))
        .unwrap_or(false)
}

pub fn set_streaming_enabled(env: &Env, user: &Address, enabled: bool) {
    env.storage()
        .persistent()
        .set(&(Symbol::new(env, "strmenb"), user.clone()), &enabled);
}

// ── Issue #310: contract allowance delegation ────────────────────────────────

pub fn get_contract_delegate(
    env: &Env,
    user: &Address,
    contract: &Address,
) -> Option<ContractDelegate> {
    env.storage()
        .persistent()
        .get(&(Symbol::new(env, "ctrdeleg"), user.clone(), contract.clone()))
}

pub fn set_contract_delegate(
    env: &Env,
    user: &Address,
    contract: &Address,
    delegate: &ContractDelegate,
) {
    env.storage().persistent().set(
        &(Symbol::new(env, "ctrdeleg"), user.clone(), contract.clone()),
        delegate,
    );
}

pub fn remove_contract_delegate(env: &Env, user: &Address, contract: &Address) {
    env.storage().persistent().remove(&(
        Symbol::new(env, "ctrdeleg"),
        user.clone(),
        contract.clone(),
    ));
}

// ── Issue #311: TVL-based reward-rate smoothing ──────────────────────────────

pub fn get_target_emission_per_ledger(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("tgt_emit"))
        .unwrap_or(0)
}

pub fn set_target_emission_per_ledger(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&symbol_short!("tgt_emit"), &amount);
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

// ── Insurance-backed penalty waiver (issue #243) ───────────────────────────

pub fn is_position_insured(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&(symbol_short!("insured"), user.clone()))
        .unwrap_or(false)
}

pub fn set_position_insured(env: &Env, user: &Address, insured: bool) {
    env.storage()
        .persistent()
        .set(&(symbol_short!("insured"), user.clone()), &insured);
}

pub fn clear_position_insured(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&(symbol_short!("insured"), user.clone()));
}

// ── Matching program (issue #242) ──────────────────────────────────────────

pub fn get_matching_program(env: &Env) -> Option<crate::storage::MatchingProgram> {
    env.storage().instance().get(&symbol_short!("match_pg"))
}

pub fn set_matching_program(env: &Env, program: &crate::storage::MatchingProgram) {
    env.storage()
        .instance()
        .set(&symbol_short!("match_pg"), program);
}

pub fn get_user_matching_stats(env: &Env, user: &Address) -> crate::storage::UserMatchingStats {
    env.storage()
        .persistent()
        .get(&(symbol_short!("match_st"), user.clone()))
        .unwrap_or(crate::storage::UserMatchingStats { total_matched: 0 })
}

pub fn set_user_matching_stats(
    env: &Env,
    user: &Address,
    stats: &crate::storage::UserMatchingStats,
) {
    env.storage()
        .persistent()
        .set(&(symbol_short!("match_st"), user.clone()), stats);
}

// ── Unstake insurance bps ──────────────────────────────────────────────────

pub fn get_unstake_insurance_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("unst_ins"))
        .unwrap_or(0)
}

pub fn set_unstake_insurance_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("unst_ins"), &bps);
}

// ── Output tokens whitelist (issue #244) ───────────────────────────────────

pub fn get_output_tokens(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&symbol_short!("out_toks"))
        .unwrap_or(Vec::new(env))
}

pub fn set_output_tokens(env: &Env, tokens: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&symbol_short!("out_toks"), tokens);
}

// ── Cohort tracking ────────────────────────────────────────────────────────

pub fn get_cohort_of(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&(symbol_short!("cohort"), user.clone()))
}

pub fn set_cohort_of(env: &Env, user: &Address, cohort_id: u32) {
    env.storage()
        .persistent()
        .set(&(symbol_short!("cohort"), user.clone()), &cohort_id);
}

pub fn get_cohort_ids(env: &Env) -> Vec<u32> {
    env.storage()
        .instance()
        .get(&symbol_short!("crt_ids"))
        .unwrap_or(Vec::new(env))
}

pub fn set_cohort_ids(env: &Env, ids: &Vec<u32>) {
    env.storage().instance().set(&symbol_short!("crt_ids"), ids);
}

pub fn get_cohort_stats(env: &Env, cohort_id: u32) -> Option<crate::storage::CohortStats> {
    env.storage()
        .instance()
        .get(&(symbol_short!("crt_st"), cohort_id))
}

pub fn set_cohort_stats(env: &Env, cohort_id: u32, stats: &crate::storage::CohortStats) {
    env.storage()
        .instance()
        .set(&(symbol_short!("crt_st"), cohort_id), stats);
}

// ── Staked at ledger (direct access) ───────────────────────────────────────

pub fn set_staked_at_ledger(env: &Env, user: &Address, ledger: u32) {
    env.storage().instance().set(
        &crate::storage::DataKey::StakedAtLedger(user.clone()),
        &ledger,
    );
}
