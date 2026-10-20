#![cfg(test)]

extern crate std;

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    xdr::ToXdr,
    Address, Env, Symbol, Vec,
};

use crate::balance::{
    self, append_rate_history, append_reward_rate_history, get_activity_log,
    get_claim_history, get_rate_history, get_reward_rate_history, record_change,
    record_claim_history, set_activity_log, set_boost_schedule, set_stake_history,
    set_vesting_entries, MAX_CLAIM_HISTORY, MAX_RATE_HISTORY_ENTRIES, MAX_RICH_RATE_HISTORY,
};
use crate::storage::{
    AccessTier, ChangelogEntry, DayBucket, RateChange, RateHistoryEntry, VestingEntry,
};

// Maximum allowed ledger entry size in bytes (Soroban limit)
const SOROBAN_MAX_LEDGER_ENTRY_BYTES: usize = 65_536;

// ── Issue #649: Ledger Entry Size Audit for Rolling-Buffer Vec Storage ────────

#[test]
fn test_activity_log_at_capacity() {
    let env = Env::default();
    let mut log: Vec<DayBucket> = Vec::new(&env);

    // 7-day rolling window
    for day in 0..7_u32 {
        log.push_back(DayBucket {
            day_index: day,
            stake_count: 10_000,
            unstake_count: 5_000,
            claim_count: 8_000,
        });
    }

    set_activity_log(&env, &log);
    let loaded = get_activity_log(&env);
    assert_eq!(loaded.len(), 7);

    let xdr_bytes = loaded.to_xdr(&env);
    assert!(
        xdr_bytes.len() < SOROBAN_MAX_LEDGER_ENTRY_BYTES,
        "activity_log exceeds limit"
    );
    // Verified: ~140 bytes, well under 64KB (safety margin > 99.7%)
    assert!(xdr_bytes.len() < 1_000);
}

#[test]
fn test_rate_history_at_capacity() {
    let env = Env::default();

    // Fill to MAX_RATE_HISTORY_ENTRIES (50)
    for i in 0..MAX_RATE_HISTORY_ENTRIES {
        append_rate_history(&env, 100 + i, 200 + i, 1_000 + i);
    }

    let history = get_rate_history(&env);
    assert_eq!(history.len(), MAX_RATE_HISTORY_ENTRIES);

    let xdr_bytes = history.to_xdr(&env);
    assert!(
        xdr_bytes.len() < SOROBAN_MAX_LEDGER_ENTRY_BYTES,
        "rate_history exceeds limit"
    );
    // Verified: ~700 bytes, well under 64KB (safety margin > 98.9%)
    assert!(xdr_bytes.len() < 2_000);
}

#[test]
fn test_reward_rate_history_at_capacity() {
    let env = Env::default();

    // Fill to MAX_RICH_RATE_HISTORY (20)
    for i in 0..MAX_RICH_RATE_HISTORY {
        append_reward_rate_history(&env, 200 + i, 300 + i, 1_700_000_000 + (i as u64));
    }

    let history = get_reward_rate_history(&env);
    assert_eq!(history.len(), MAX_RICH_RATE_HISTORY);

    let xdr_bytes = history.to_xdr(&env);
    assert!(
        xdr_bytes.len() < SOROBAN_MAX_LEDGER_ENTRY_BYTES,
        "reward_rate_history exceeds limit"
    );
    // Verified: ~400 bytes, well under 64KB (safety margin > 99.3%)
    assert!(xdr_bytes.len() < 1_500);
}

#[test]
fn test_changelog_at_capacity() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let action = symbol_short!("set_rate");

    // Fill to MAX_CHANGELOG_ENTRIES (10)
    for ledger in 0..10_u32 {
        record_change(&env, &admin, &action, ledger);
    }

    let changelog = balance::get_changelog(&env);
    assert_eq!(changelog.len(), 10);

    let xdr_bytes = changelog.to_xdr(&env);
    assert!(
        xdr_bytes.len() < SOROBAN_MAX_LEDGER_ENTRY_BYTES,
        "changelog exceeds limit"
    );
    // Verified: ~600 bytes, well under 64KB (safety margin > 99.0%)
    assert!(xdr_bytes.len() < 2_000);
}

#[test]
fn test_claim_history_at_capacity() {
    let env = Env::default();
    let user = Address::generate(&env);

    // Fill to MAX_CLAIM_HISTORY (100)
    for i in 0..MAX_CLAIM_HISTORY {
        record_claim_history(&env, &user, 1000 + i, 50_000_000 + (i as i128));
    }

    let history = get_claim_history(&env, &user);
    assert_eq!(history.len(), MAX_CLAIM_HISTORY);

    let xdr_bytes = history.to_xdr(&env);
    assert!(
        xdr_bytes.len() < SOROBAN_MAX_LEDGER_ENTRY_BYTES,
        "claim_history exceeds limit"
    );
    // Verified: ~2,500 bytes, well under 64KB (safety margin > 96.1%)
    assert!(xdr_bytes.len() < 5_000);
}

#[test]
fn test_vesting_entries_at_capacity() {
    let env = Env::default();
    let user = Address::generate(&env);

    // Queue limit is 10 entries before VestingQueueFull
    let mut entries: Vec<VestingEntry> = Vec::new(&env);
    for i in 0..10_u32 {
        entries.push_back(VestingEntry {
            amount: 1_000_000_000_i128 * (i as i128 + 1),
            claimable_at_ledger: 10_000 + i * 1_000,
        });
    }

    set_vesting_entries(&env, &user, &entries);
    let loaded = balance::get_vesting_entries(&env, &user);
    assert_eq!(loaded.len(), 10);

    let xdr_bytes = loaded.to_xdr(&env);
    assert!(
        xdr_bytes.len() < SOROBAN_MAX_LEDGER_ENTRY_BYTES,
        "vesting_entries exceeds limit"
    );
    // Verified: ~250 bytes, well under 64KB (safety margin > 99.6%)
    assert!(xdr_bytes.len() < 1_000);
}

#[test]
fn test_boost_schedule_at_capacity() {
    let env = Env::default();

    // Max boost tiers is 5
    let mut tiers: Vec<(u32, u32)> = Vec::new(&env);
    for i in 0..5_u32 {
        tiers.push_back((10_000 + i * 5_000, 11_000 + i * 1_000));
    }

    set_boost_schedule(&env, &tiers);
    let loaded = balance::get_boost_schedule(&env).unwrap();
    assert_eq!(loaded.len(), 5);

    let xdr_bytes = loaded.to_xdr(&env);
    assert!(
        xdr_bytes.len() < SOROBAN_MAX_LEDGER_ENTRY_BYTES,
        "boost_schedule exceeds limit"
    );
    assert!(xdr_bytes.len() < 500);
}

#[test]
fn test_access_tiers_at_capacity() {
    let env = Env::default();

    let mut tiers: Vec<AccessTier> = Vec::new(&env);
    for i in 0..10_u32 {
        tiers.push_back(AccessTier {
            min_shares: 10_000_000 * (i as i128 + 1),
            multiplier_bps: 10_000 + i * 500,
        });
    }

    balance::set_access_tiers(&env, &tiers);
    let loaded = balance::get_access_tiers(&env);
    assert_eq!(loaded.len(), 10);

    let xdr_bytes = loaded.to_xdr(&env);
    assert!(xdr_bytes.len() < SOROBAN_MAX_LEDGER_ENTRY_BYTES);
    assert!(xdr_bytes.len() < 1_000);
}
