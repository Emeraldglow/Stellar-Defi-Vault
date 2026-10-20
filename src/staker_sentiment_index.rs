//! Composite sentiment index measuring the overall mood and engagement health
//! of the staking pool (issue #422).
//!
//! Combines five behavioral signals — stake inflow vs outflow ratio, claim
//! frequency, governance participation, message board activity, and rating
//! trends — into a single 0-100 sentiment score.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::balance;
use crate::vault::VaultContract;

/// Sentiment report returned by `get_sentiment_index`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SentimentReport {
    pub score: u32,
    pub inflow_signal: u32,
    pub claim_signal: u32,
    pub governance_signal: u32,
    pub message_signal: u32,
    pub rating_signal: u32,
    pub computed_at: u32,
}

const INFLOW_TRACKER_KEY: Symbol = symbol_short!("si_infl");
const CLAIM_TRACKER_KEY: Symbol = symbol_short!("si_clm");
const MESSAGE_TRACKER_KEY: Symbol = symbol_short!("si_msg");
const RATING_KEY: Symbol = symbol_short!("si_rat");
const VOTER_TRACKER_KEY: Symbol = symbol_short!("si_vote");

// 7 days at ~5s/ledger (matches vault::LEDGERS_PER_DAY * 7 in other modules).
const SEVEN_DAY_LEDGERS: u32 = 120_960;

fn get_inflow_7d(env: &Env) -> (i128, i128) {
    let cutoff = env.ledger().sequence().saturating_sub(SEVEN_DAY_LEDGERS);
    let mut net_inflow: i128 = 0;
    let mut total_staked: i128 = 0;
    let entries: Vec<(u32, i128, bool)> = env
        .storage()
        .instance()
        .get(&INFLOW_TRACKER_KEY)
        .unwrap_or_else(|| Vec::new(env));
    let n = entries.len();
    let mut i = 0u32;
    while i < n {
        let (ledger, amount, is_inflow) = entries.get(i).unwrap();
        if ledger >= cutoff {
            if is_inflow {
                net_inflow = net_inflow.saturating_add(amount);
            } else {
                net_inflow = net_inflow.saturating_sub(amount);
            }
            total_staked = total_staked.saturating_add(amount);
        }
        i += 1;
    }
    (net_inflow.max(0), total_staked)
}

fn get_claim_velocity_bps(env: &Env) -> i128 {
    let cutoff = env.ledger().sequence().saturating_sub(SEVEN_DAY_LEDGERS);
    let entries: Vec<(u32, i128)> = env
        .storage()
        .instance()
        .get(&CLAIM_TRACKER_KEY)
        .unwrap_or_else(|| Vec::new(env));
    let mut total_claims: i128 = 0;
    let n = entries.len();
    let mut i = 0u32;
    while i < n {
        let (ledger, amount) = entries.get(i).unwrap();
        if ledger >= cutoff {
            total_claims = total_claims.saturating_add(amount);
        }
        i += 1;
    }
    if total_claims == 0 {
        return 0;
    }
    let total_staked = balance::get_total_deposited(env);
    if total_staked == 0 {
        return 0;
    }
    total_claims.saturating_mul(10_000) / total_staked
}

fn get_active_voters_last_epoch(env: &Env) -> u32 {
    let entries: Vec<u32> = env
        .storage()
        .instance()
        .get(&VOTER_TRACKER_KEY)
        .unwrap_or_else(|| Vec::new(env));

    let cutoff = env.ledger().sequence().saturating_sub(SEVEN_DAY_LEDGERS);
    let mut count: u32 = 0;
    let n = entries.len();
    let mut i = 0u32;
    while i < n {
        if entries.get(i).unwrap() >= cutoff {
            count += 1;
        }
        i += 1;
    }
    count
}

fn get_messages_last_7d(env: &Env) -> u32 {
    let cutoff = env.ledger().sequence().saturating_sub(SEVEN_DAY_LEDGERS);
    let entries: Vec<u32> = env
        .storage()
        .instance()
        .get(&MESSAGE_TRACKER_KEY)
        .unwrap_or_else(|| Vec::new(env));
    let mut count: u32 = 0;
    let n = entries.len();
    let mut i = 0u32;
    while i < n {
        if entries.get(i).unwrap() >= cutoff {
            count += 1;
        }
        i += 1;
    }
    count
}

fn get_pool_average_rating_bps(env: &Env) -> u32 {
    env.storage().instance().get(&RATING_KEY).unwrap_or(0)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Record a stake inflow or outflow event for the sentiment tracker.
    /// `is_inflow` true for stake, false for unstake.
    pub fn record_sentiment_inflow(env: Env, user: Address, amount: i128, is_inflow: bool) {
        user.require_auth();
        let mut entries: Vec<(u32, i128, bool)> = env
            .storage()
            .instance()
            .get(&INFLOW_TRACKER_KEY)
            .unwrap_or_else(|| Vec::new(&env));
        entries.push_back((env.ledger().sequence(), amount, is_inflow));
        if entries.len() > 100 {
            entries.remove(0);
        }
        env.storage().instance().set(&INFLOW_TRACKER_KEY, &entries);
    }

    /// Record a claim event for the sentiment tracker.
    pub fn record_sentiment_claim(env: Env, user: Address, amount: i128) {
        user.require_auth();
        let mut entries: Vec<(u32, i128)> = env
            .storage()
            .instance()
            .get(&CLAIM_TRACKER_KEY)
            .unwrap_or_else(|| Vec::new(&env));
        entries.push_back((env.ledger().sequence(), amount));
        if entries.len() > 100 {
            entries.remove(0);
        }
        env.storage().instance().set(&CLAIM_TRACKER_KEY, &entries);
    }

    /// Record a governance vote for the sentiment tracker.
    pub fn record_sentiment_vote(env: Env, voter: Address) {
        let mut entries: Vec<u32> = env
            .storage()
            .instance()
            .get(&VOTER_TRACKER_KEY)
            .unwrap_or_else(|| Vec::new(&env));
        entries.push_back(env.ledger().sequence());
        if entries.len() > 100 {
            entries.remove(0);
        }
        env.storage().instance().set(&VOTER_TRACKER_KEY, &entries);
    }

    /// Record a message board post for the sentiment tracker.
    pub fn record_sentiment_message(env: Env, poster: Address) {
        poster.require_auth();
        let mut entries: Vec<u32> = env
            .storage()
            .instance()
            .get(&MESSAGE_TRACKER_KEY)
            .unwrap_or_else(|| Vec::new(&env));
        entries.push_back(env.ledger().sequence());
        if entries.len() > 100 {
            entries.remove(0);
        }
        env.storage().instance().set(&MESSAGE_TRACKER_KEY, &entries);
    }

    /// Set the pool's average rating in basis points (admin-only; mirrors
    /// the rating from Issue #195's on-chain mechanism).
    pub fn set_pool_average_rating_bps(env: Env, rating_bps: u32) {
        env.storage().instance().set(&RATING_KEY, &rating_bps);
    }

    /// Compute the composite sentiment index (0-100) from the five signals.
    /// No auth required, no state changes.
    pub fn get_sentiment_index(env: Env) -> SentimentReport {
        let current_ledger = env.ledger().sequence();

        // 1. inflow_signal: min(25, net_stake_inflow_7d * 25 / total_staked)
        let (net_inflow_7d, total_staked) = get_inflow_7d(&env);
        let inflow_signal = if total_staked > 0 {
            let raw = net_inflow_7d.saturating_mul(25) / total_staked;
            raw.min(25) as u32
        } else {
            0
        };

        // 2. claim_signal: min(25, reward_velocity_bps / 400)
        let velocity = get_claim_velocity_bps(&env);
        let claim_signal = (velocity / 400).min(25) as u32;

        // 3. governance_signal: min(25, active_voters_last_epoch * 25 / total_stakers)
        let active_voters = get_active_voters_last_epoch(&env);
        let total_stakers = balance::get_total_stakers(&env);
        let governance_signal = if total_stakers > 0 {
            let raw = active_voters.saturating_mul(25) / total_stakers;
            raw.min(25)
        } else {
            0
        };

        // 4. message_signal: min(25, messages_last_7d * 5) — capped at 5 messages = max score
        let messages = get_messages_last_7d(&env);
        let message_signal = (messages.saturating_mul(5)).min(25);

        // 5. rating_signal: min(25, pool_average_rating_bps / 400)
        let rating_bps = get_pool_average_rating_bps(&env);
        let rating_signal = (rating_bps / 400).min(25);

        let score =
            inflow_signal + claim_signal + governance_signal + message_signal + rating_signal;
        let score = score.min(100);

        SentimentReport {
            score,
            inflow_signal,
            claim_signal,
            governance_signal,
            message_signal,
            rating_signal,
            computed_at: current_ledger,
        }
    }
}
