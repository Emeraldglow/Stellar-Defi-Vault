//! Per-user on-chain deposit/withdrawal history (issue #530).
//!
//! Every deposit (`stake` / `deposit` / `stake_and_claim`) and withdrawal
//! (`withdraw` / `unstake` / exits built on `do_unstake`) appends an entry to
//! the user's own activity log, so a wallet or frontend can show a user their
//! history with `get_activity_log()` without running an off-chain indexer.
//!
//! Each user's log is capped at `MAX_ACTIVITY_ENTRIES`; once full, the oldest
//! entry is dropped to make room, bounding storage and read cost.
//!
//! # Storage
//!
//! Raw `Symbol`-keyed persistent storage, matching `balance.rs`.
//!
//! - `(symbol_short!("act_log"), user)` -> `Vec<ActivityEntry>` (oldest first)

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::vault::{VaultContract, VaultContractClient};

const ACTIVITY_LOG_KEY: Symbol = symbol_short!("act_log");

/// Maximum entries retained per user; older entries are dropped first.
pub const MAX_ACTIVITY_ENTRIES: u32 = 100;

/// Maximum entries returned by a single `get_activity_log()` call.
pub const MAX_ACTIVITY_PAGE: u32 = 50;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ActivityKind {
    Deposit = 0,
    Withdrawal = 1,
}

/// One deposit or withdrawal. `amount` is the gross token amount (before any
/// unstake fee); `shares` is the number of shares minted or burned.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ActivityEntry {
    pub kind: ActivityKind,
    pub amount: i128,
    pub shares: i128,
    pub ledger: u32,
    pub timestamp: u64,
}

fn read_log(env: &Env, user: &Address) -> Vec<ActivityEntry> {
    env.storage()
        .persistent()
        .get(&(ACTIVITY_LOG_KEY, user.clone()))
        .unwrap_or(Vec::new(env))
}

/// Appends an entry to `user`'s activity log, evicting the oldest entry when
/// the log is full. Called from the vault's deposit and withdrawal paths.
pub(crate) fn record(env: &Env, user: &Address, kind: ActivityKind, amount: i128, shares: i128) {
    // Issue #707: every deposit and withdrawal is an owner interaction.
    crate::vault::beneficiary_inheritance::touch(env, user);
    let mut log = read_log(env, user);
    if log.len() >= MAX_ACTIVITY_ENTRIES {
        log.pop_front();
    }
    log.push_back(ActivityEntry {
        kind,
        amount,
        shares,
        ledger: env.ledger().sequence(),
        timestamp: env.ledger().timestamp(),
    });
    env.storage()
        .persistent()
        .set(&(ACTIVITY_LOG_KEY, user.clone()), &log);
}

#[contractimpl]
impl VaultContract {
    /// Read-only: a page of `user`'s deposit/withdrawal history, oldest
    /// first. `start` is the index into the retained log; `limit` is capped
    /// at `MAX_ACTIVITY_PAGE`. Returns an empty list past the end.
    pub fn get_activity_log(env: Env, user: Address, start: u32, limit: u32) -> Vec<ActivityEntry> {
        let log = read_log(&env, &user);
        let len = log.len();
        if start >= len || limit == 0 {
            return Vec::new(&env);
        }
        let end = start.saturating_add(limit.min(MAX_ACTIVITY_PAGE)).min(len);
        log.slice(start..end)
    }

    /// Read-only: number of entries currently retained in `user`'s log.
    pub fn get_activity_count(env: Env, user: Address) -> u32 {
        read_log(&env, &user).len()
    }
}
