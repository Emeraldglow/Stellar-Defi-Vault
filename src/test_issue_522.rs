#![cfg(test)]
//! Tests for issue #522: on-chain reward-rate change history via
//! `get_rate_history()`.
//!
//! Covers the three behaviours the issue asks for: every accepted
//! `set_reward_rate_bps` update is recorded with the rate it replaced, the rate
//! it set and the ledger it happened at; the log is a rolling 50-entry buffer
//! that drops the oldest entry first; and it is empty before the first change.
//! Two extra cases pin the edges: an update that does not move the rate is
//! still recorded (matching the `rate_changed` event), and a rate change that
//! fails authorization leaves no entry behind.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

use crate::storage::RateChange;
use crate::vault::{VaultContract, VaultContractClient};

/// Moves the ledger to `sequence`, mirroring the helper the other suites use.
fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 1_000;
        });

        let admin = Address::generate(&env);
        // The rate log never moves tokens, so the pool only needs *a* token.
        let token_addr = env.register_stellar_asset_contract(admin.clone());

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        // Start at a 0% rate so the first recorded change shows old_rate 0.
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        Fixture { env, vault }
    }

    /// Admin: move the rate to `rate_bps` at ledger `sequence`.
    fn set_rate(&self, sequence: u32, rate_bps: u32) {
        set_ledger(&self.env, sequence);
        self.vault.set_reward_rate_bps(&rate_bps);
    }
}

// ── Empty before the first change ────────────────────────────────────────────

#[test]
fn rate_history_is_empty_before_the_first_change() {
    let f = Fixture::new();

    // `initialize` seeds the rate directly, so deployment is not a "change".
    assert!(
        f.vault.get_rate_history().is_empty(),
        "a freshly initialized pool must report no rate changes"
    );
}

// ── Every change is recorded, oldest first ───────────────────────────────────

#[test]
fn each_rate_change_records_old_rate_new_rate_and_ledger() {
    let f = Fixture::new();

    f.set_rate(100, 1_000);
    f.set_rate(200, 2_000);

    let history = f.vault.get_rate_history();
    assert_eq!(history.len(), 2);

    let first = history.get(0).unwrap();
    assert_eq!(
        first,
        RateChange {
            old_rate_bps: 0,
            new_rate_bps: 1_000,
            changed_at: 100,
        },
        "the first entry chains from the rate the pool was initialized with"
    );

    let second = history.get(1).unwrap();
    assert_eq!(
        second,
        RateChange {
            old_rate_bps: 1_000,
            new_rate_bps: 2_000,
            changed_at: 200,
        }
    );
}

#[test]
fn unchanged_rate_is_still_recorded() {
    let f = Fixture::new();

    f.set_rate(100, 1_000);
    f.set_rate(200, 1_000);

    let history = f.vault.get_rate_history();
    assert_eq!(
        history.len(),
        2,
        "a re-submitted rate is an update, mirroring the rate_changed event"
    );
    let last = history.get(1).unwrap();
    assert_eq!(last.old_rate_bps, 1_000);
    assert_eq!(last.new_rate_bps, 1_000);
    assert_eq!(last.changed_at, 200);
}

#[test]
fn an_unauthorized_rate_change_is_not_recorded() {
    let f = Fixture::new();
    f.set_rate(100, 1_000);

    // Real auth (not mocked) so the stored admin's signature cannot be forged.
    f.env.set_auths(&[]);
    let res = f.vault.try_set_reward_rate_bps(&2_000);
    assert!(res.is_err());

    let history = f.vault.get_rate_history();
    assert_eq!(history.len(), 1, "a reverted change must leave no entry");
    assert_eq!(history.get(0).unwrap().new_rate_bps, 1_000);
}

// ── Rolling buffer: 50 entries, oldest dropped first ─────────────────────────

#[test]
fn rate_history_rolls_over_at_fifty_entries_dropping_the_oldest_first() {
    let f = Fixture::new();

    // 55 changes at ledgers 10..550, rates 1001..1055.
    for i in 1..=55u32 {
        f.set_rate(i * 10, 1_000 + i);
    }

    let history = f.vault.get_rate_history();
    assert_eq!(history.len(), 50, "the buffer never grows past 50 entries");

    // The first five changes were evicted, so entry 0 is change #6.
    let first = history.get(0).unwrap();
    assert_eq!(
        first,
        RateChange {
            old_rate_bps: 1_005,
            new_rate_bps: 1_006,
            changed_at: 60,
        }
    );

    // The newest change is still last, with the full chain intact.
    let last = history.get(49).unwrap();
    assert_eq!(
        last,
        RateChange {
            old_rate_bps: 1_054,
            new_rate_bps: 1_055,
            changed_at: 550,
        }
    );

    // Every retained entry chains to the one before it: nothing is corrupted by
    // the eviction, only the oldest entries are dropped.
    for i in 1..history.len() {
        let prev = history.get(i - 1).unwrap();
        let cur = history.get(i).unwrap();
        assert_eq!(prev.new_rate_bps, cur.old_rate_bps);
        assert!(
            cur.changed_at > prev.changed_at,
            "history stays chronological"
        );
    }
}
