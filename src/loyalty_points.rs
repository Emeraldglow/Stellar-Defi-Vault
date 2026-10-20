//! Non-transferable loyalty points ledger (issue #756).
//!
//! Tracks a record-keeping-only points balance for each staker, earned from how
//! much they have staked and for how long. The figure is for off-chain use
//! (campaigns, allowlists, dashboards) and is **not** a token, a share, a
//! reward, or a claim of any kind:
//!
//! - points cannot be transferred between accounts,
//! - they cannot be spent, redeemed, withdrawn, or exchanged on-chain, and
//! - they confer no ownership, yield, or governance right.
//!
//! Integrators must present points as informational only, never as a financial
//! claim or an expectation of payment.
//!
//! # Accrual
//!
//! Points accrue per ledger as:
//!
//! ```text
//! points = staked_amount * elapsed_ledgers * points_rate
//! ```
//!
//! `staked_amount` is the user's current position converted back to token
//! units, `elapsed_ledgers` runs from the user's `StakedAtLedger` to the
//! current ledger, and `points_rate` is the admin-configured rate. The rate is
//! stored as a short history of `(ledger, rate)` checkpoints, so a rate change
//! only affects ledgers after it is set (the same accumulator style the reward
//! accounting uses). A rate of `0` disables accrual.
//!
//! # Time-weighting
//!
//! The vault keeps no separate time-weighted-average-balance accumulator for
//! points, so the ledger integrates over `(ledger, rate)` checkpoints directly,
//! yielding the exact `balance * ledgers * rate` TWAB integral. Where an
//! off-chain indexer already has a TWAB window, it can be reconstructed from
//! these same checkpoints. `get_points` is read-only and never mutates state.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this module uses raw
//! `Symbol`-keyed instance storage, matching `balance.rs` and the other feature
//! modules.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::vault::{VaultContract, VaultContractArgs, VaultContractClient};

/// Instance key for the bounded history of `(ledger, points_rate)` checkpoints.
const RATE_HISTORY_KEY: Symbol = symbol_short!("pts_hist");

/// Maximum number of rate checkpoints retained. Older entries are dropped.
pub const MAX_RATE_CHECKPOINTS: u32 = 50;

fn get_rate_history(env: &Env) -> Vec<(u32, i128)> {
    env.storage()
        .instance()
        .get(&RATE_HISTORY_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_rate_history(env: &Env, history: &Vec<(u32, i128)>) {
    env.storage().instance().set(&RATE_HISTORY_KEY, history);
}

/// The rate currently in effect (the newest checkpoint), or `0` when the
/// ledger has never been configured.
fn current_rate(env: &Env) -> i128 {
    let history = get_rate_history(env);
    if history.is_empty() {
        0
    } else {
        history.get(history.len() - 1).unwrap().1
    }
}

/// The user's staked position converted back to token units.
///
/// Mirrors `balance::shares_to_amount` (shares are minted in a fixed
/// seven-decimal precision, so the ratio already accounts for scale).
fn position_amount(env: &Env, user: &Address) -> i128 {
    let shares = balance::get_shares(env, user);
    if shares <= 0 {
        return 0;
    }
    let total_shares = balance::get_total_shares(env);
    if total_shares <= 0 {
        return shares;
    }
    let total_deposited = balance::get_total_deposited(env);
    shares
        .checked_mul(total_deposited)
        .and_then(|value| value.checked_div(total_shares))
        .unwrap_or(shares)
}

/// Accrued points for `user`, integrating the rate checkpoints over the
/// position's lifetime. Read-only: no storage is written.
fn accrued_points(env: &Env, user: &Address) -> i128 {
    let amount = position_amount(env, user);
    if amount <= 0 {
        return 0;
    }

    // Prefer the live stake ledger; fall back to the first-ever stake ledger so
    // a position that predates `StakedAtLedger` still has a duration anchor.
    let baseline = match balance::get_staked_at_ledger(env, user)
        .or_else(|| balance::get_first_staked_at(env, user))
    {
        Some(ledger) => ledger,
        // Without a recorded stake ledger there is no duration to measure.
        None => return 0,
    };

    let current_ledger = env.ledger().sequence();
    if current_ledger <= baseline {
        return 0;
    }

    let history = get_rate_history(env);
    if history.is_empty() {
        return 0;
    }

    let mut total: i128 = 0;
    let count = history.len();
    let mut i = 0u32;
    while i < count {
        let (checkpoint_ledger, rate) = history.get(i).unwrap();
        let segment_start = if checkpoint_ledger > baseline {
            checkpoint_ledger
        } else {
            baseline
        };
        let segment_end = if i + 1 < count {
            let next_ledger = history.get(i + 1).unwrap().0;
            if next_ledger < current_ledger {
                next_ledger
            } else {
                current_ledger
            }
        } else {
            current_ledger
        };

        if segment_end > segment_start && rate > 0 {
            let ledgers = (segment_end - segment_start) as i128;
            let segment_points = amount.saturating_mul(ledgers).saturating_mul(rate);
            total = total.saturating_add(segment_points);
        }
        i += 1;
    }

    total
}

#[contractimpl]
impl VaultContract {
    /// Admin: set the rate at which loyalty points accrue, in points per token
    /// unit per ledger. `0` disables accrual from this ledger onward.
    ///
    /// Points are informational only and carry no on-chain value; see the
    /// module docs. They can never be transferred or spent.
    ///
    /// # Authorization
    /// `admin` must sign and be the configured admin.
    ///
    /// # Errors
    /// - `InvalidRate`: `rate` is negative.
    /// - `Unauthorized` / `NotInitialized`: admin check failed.
    pub fn set_points_rate(env: Env, admin: Address, rate: i128) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        if rate < 0 {
            return Err(VaultError::InvalidRate);
        }

        let ledger = env.ledger().sequence();
        let mut history = get_rate_history(&env);
        history.push_back((ledger, rate));
        while history.len() > MAX_RATE_CHECKPOINTS {
            history.remove(0);
        }
        set_rate_history(&env, &history);

        env.events()
            .publish((symbol_short!("pts_rate"),), (rate, ledger));
        Ok(())
    }

    /// Read-only: the points rate currently in effect (`0` = accrual disabled).
    pub fn get_points_rate(env: Env) -> i128 {
        current_rate(&env)
    }

    /// Read-only: `user`'s accrued loyalty points.
    ///
    /// Points are record-keeping only: they cannot be transferred, spent, or
    /// redeemed on-chain and imply no financial claim.
    pub fn get_points(env: Env, user: Address) -> i128 {
        accrued_points(&env, &user)
    }
}
