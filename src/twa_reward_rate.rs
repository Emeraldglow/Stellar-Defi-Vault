//! Time-weighted average reward rate for smoother pending-reward estimates
//! (issue #400).
//!
//! The pool's reward rate can change over a position's lifetime (decay,
//! campaigns, TVL rebalancing), so a spot-rate estimate can misstate a
//! long-held position. This module keeps a bounded history of rate changes
//! and derives a time-weighted average rate over a ledger range, plus a
//! TWA-based pending-reward estimate for comparison against the spot figure.
//!
//! # Wiring
//!
//! There is no hook into a lower-level rate setter to append a checkpoint
//! automatically, so â€” matching the pattern `governance_power_decay.rs`
//! uses for the same kind of gap â€” `record_rate_checkpoint` is the
//! entrypoint to call on every rate change; it both records the checkpoint
//! and updates the live spot rate, so it's a drop-in for changing the rate
//! going forward.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::{STELLAR_LEDGERS_PER_YEAR};
use crate::vault::VaultContractClient;

const CHECKPOINTS_KEY: Symbol = symbol_short!("twa_cps");
const MAX_CHECKPOINTS: u32 = 50;

/// One recorded reward-rate change.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RateCheckpoint {
    pub rate_bps: i128,
    pub valid_from: u32,
}

fn get_checkpoints(env: &Env) -> Vec<RateCheckpoint> {
    env.storage()
        .instance()
        .get(&CHECKPOINTS_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_checkpoints(env: &Env, checkpoints: &Vec<RateCheckpoint>) {
    env.storage().instance().set(&CHECKPOINTS_KEY, checkpoints);
}

fn position_amount(env: &Env, user: &Address) -> i128 {
    let shares = balance::get_shares(env, user);
    if shares == 0 {
        return 0;
    }
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    balance::shares_to_amount(total_shares, total_deposited, shares).unwrap_or(0)
}

fn staked_at_ledger(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&crate::storage::DataKey::StakedAtLedger(user.clone()))
        .unwrap_or(0)
}

/// Time-weighted average rate (bps) between `from_ledger` and `to_ledger`.
/// Falls back to the current spot rate when there's no checkpoint history
/// (e.g. the rate has never changed), so a constant-rate pool returns the
/// same figure as the spot rate.
fn calc_twa_rate(env: &Env, from_ledger: u32, to_ledger: u32) -> i128 {
    if to_ledger <= from_ledger {
        return balance::get_reward_rate_bps(env) as i128;
    }

    let checkpoints = get_checkpoints(env);
    if checkpoints.is_empty() {
        return balance::get_reward_rate_bps(env) as i128;
    }

    let total_span = (to_ledger - from_ledger) as i128;
    let mut weighted_sum: i128 = 0;
    let n = checkpoints.len();

    let mut i = 0u32;
    while i < n {
        let cp = checkpoints.get(i).unwrap();
        let segment_start = if cp.valid_from > from_ledger {
            cp.valid_from
        } else {
            from_ledger
        };
        let segment_end = if i + 1 < n {
            let next_from = checkpoints.get(i + 1).unwrap().valid_from;
            if next_from < to_ledger {
                next_from
            } else {
                to_ledger
            }
        } else {
            to_ledger
        };

        if segment_end > segment_start {
            let duration = (segment_end - segment_start) as i128;
            weighted_sum = weighted_sum.saturating_add(cp.rate_bps.saturating_mul(duration));
        }
        i += 1;
    }

    if weighted_sum == 0 {
        return checkpoints.get(0).unwrap().rate_bps;
    }

    weighted_sum / total_span
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Record a reward-rate checkpoint and update the live spot rate.
    /// Admin only. Oldest checkpoint is dropped once more than
    /// `MAX_CHECKPOINTS` (50) are recorded.
    pub fn record_rate_checkpoint(env: Env, rate_bps: i128) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        if rate_bps < 0 {
            return Err(VaultError::InvalidRate);
        }

        let mut checkpoints = get_checkpoints(&env);
        let current_ledger = env.ledger().sequence();
        checkpoints.push_back(RateCheckpoint {
            rate_bps,
            valid_from: current_ledger,
        });

        if checkpoints.len() > MAX_CHECKPOINTS {
            checkpoints.remove(0);
        }

        set_checkpoints(&env, &checkpoints);
        balance::set_reward_rate_bps(&env, rate_bps as u32);

        env.events()
            .publish((symbol_short!("twa_rate"),), (rate_bps, current_ledger));
        Ok(())
    }

    /// Read-only query: the recorded rate-checkpoint history, oldest first.
    pub fn get_rate_checkpoints(env: Env) -> Vec<RateCheckpoint> {
        get_checkpoints(&env)
    }

    /// `user`'s pending reward estimated using the time-weighted average
    /// rate over their position's lifetime, instead of the current spot
    /// rate. Read-only / advisory â€” actual claims are unaffected.
    pub fn get_pending_reward_twa(env: Env, user: Address) -> i128 {
        let amount = position_amount(&env, &user);
        if amount == 0 {
            return 0;
        }

        let from_ledger = staked_at_ledger(&env, &user);
        let current_ledger = env.ledger().sequence();
        if current_ledger <= from_ledger {
            return 0;
        }

        let twa_rate = calc_twa_rate(&env, from_ledger, current_ledger);
        let elapsed = (current_ledger - from_ledger) as i128;

        amount.saturating_mul(twa_rate).saturating_mul(elapsed)
            / (10_000i128.saturating_mul(STELLAR_LEDGERS_PER_YEAR as i128))
    }

    /// Difference between the spot pending-reward estimate and the
    /// TWA-based estimate: `spot - twa`.
    pub fn get_rate_accuracy_delta(env: Env, user: Address) -> i128 {
        let spot = balance::get_accrued_reward(&env, &user);
        let twa = Self::get_pending_reward_twa(env, user);
        spot - twa
    }
}















