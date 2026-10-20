//! Position value appreciation log (issue #469).
//!
//! Personal value appreciation log per staker that periodically snapshots their
//! total position value (principal + pending rewards) and tracks the change over
//! time. Gives stakers a clear view of how their wealth is growing through
//! staking, expressed as both absolute and percentage appreciation.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! persistent storage, matching `balance.rs` and other feature modules.
//!
//! Storage keys:
//! - Per-user snapshot log: `(symbol_short!("val_app"), user: Address)` -> `Vec<ValueSnapshot>` (max 52 entries)

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::balance;

const VAL_APP_KEY: Symbol = symbol_short!("val_app");

/// Maximum weekly snapshot history: 52 entries (1 year).
pub const MAX_SNAPSHOT_ENTRIES: u32 = 52;

/// A single valuation snapshot at a specific ledger.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ValueSnapshot {
    pub principal: i128,
    pub pending_reward: i128,
    pub total_value: i128,
    /// Appreciation in basis points since the previous snapshot.
    /// Note: this value can be negative if position or reward value dropped.
    pub appreciation_bps_since_last: i128,
    pub snapshot_at: u32,
}

pub fn get_appreciation_log(env: &Env, user: &Address) -> Vec<ValueSnapshot> {
    env.storage()
        .persistent()
        .get(&(VAL_APP_KEY, user.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_appreciation_log(env: &Env, user: &Address, log: &Vec<ValueSnapshot>) {
    env.storage()
        .persistent()
        .set(&(VAL_APP_KEY, user.clone()), log);
}

/// Takes and stores a new position value snapshot for `user`.
pub fn take_value_snapshot_inner(env: &Env, user: &Address) -> ValueSnapshot {
    let principal = balance::get_shares(env, user);
    let pending_reward = balance::get_accrued_reward(env, user);
    let total_value = principal.saturating_add(pending_reward);

    let mut log = get_appreciation_log(env, user);
    let appreciation_bps_since_last = if let Some(last) = log.last() {
        if last.total_value > 0 {
            total_value
                .saturating_sub(last.total_value)
                .saturating_mul(10_000)
                / last.total_value
        } else {
            0
        }
    } else {
        0
    };

    let snapshot = ValueSnapshot {
        principal,
        pending_reward,
        total_value,
        appreciation_bps_since_last,
        snapshot_at: env.ledger().sequence(),
    };

    if log.len() >= MAX_SNAPSHOT_ENTRIES {
        log.remove(0); // rolling FIFO: discard oldest snapshot
    }
    log.push_back(snapshot.clone());
    set_appreciation_log(env, user, &log);

    // Emit value_snapshot_taken event: (user, total_value, appreciation_bps_since_last, ledger)
    env.events().publish(
        (symbol_short!("snap_tkn"), user.clone()),
        (
            total_value,
            appreciation_bps_since_last,
            env.ledger().sequence(),
        ),
    );

    snapshot
}

/// Helper called on claim: automatically takes snapshot if last snapshot was
/// taken more than LEDGERS_PER_DAY * 7 ago (or if no snapshot exists).
pub fn maybe_auto_snapshot(env: &Env, user: &Address) {
    let log = get_appreciation_log(env, user);
    let should_snapshot = match log.last() {
        None => true,
        Some(last) => {
            let now = env.ledger().sequence();
            now.saturating_sub(last.snapshot_at) >= (crate::vault::LEDGERS_PER_DAY * 7)
        }
    };

    if should_snapshot {
        take_value_snapshot_inner(env, user);
    }
}

/// Calculate appreciation in basis points from the first recorded snapshot to the latest.
pub fn calculate_total_appreciation_bps(env: &Env, user: &Address) -> i128 {
    let log = get_appreciation_log(env, user);
    if log.len() < 2 {
        return 0;
    }
    let first = log.get(0).unwrap();
    let latest = log.get(log.len() - 1).unwrap();
    if first.total_value > 0 {
        latest
            .total_value
            .saturating_sub(first.total_value)
            .saturating_mul(10_000)
            / first.total_value
    } else {
        0
    }
}
