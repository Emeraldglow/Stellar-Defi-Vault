#![cfg(test)]
//! Storage-layout snapshot for upgrade safety (issue #659).
//!
//! In-place upgrades are only safe if storage written by the old code stays
//! readable by the new code. Every value in the vault is encoded with Soroban's
//! XDR form derived from the `#[contracttype]` definition, so an incompatible
//! change (reordered fields, a changed integer width, a reordered enum, an added
//! required field) silently changes the bytes on disk and orphans live state.
//!
//! This test serialises a curated set of core keys and values to their XDR form
//! and compares the result against a committed golden snapshot
//! (`test-data/storage_layout.snapshot`). Any change to the encoding fails the
//! test with the offending entry named.
//!
//! ## Updating the snapshot deliberately
//!
//! A layout change is only allowed together with a migration. To refresh the
//! golden file as part of such a change:
//!
//! ```bash
//! UPDATE_STORAGE_SNAPSHOT=1 cargo test --features testutils test_storage_layout
//! cargo test --features testutils test_storage_layout   # confirm it now passes
//! ```
//!
//! then commit `test-data/storage_layout.snapshot` alongside the migration.
//! See `docs/STORAGE_LAYOUT.md` for the full process.

extern crate std;

use soroban_sdk::{xdr::ToXdr, Address, Bytes, Env};

use crate::storage::{ContractDelegate, DataKey};

const SNAPSHOT_PATH: &str = "test-data/storage_layout.snapshot";

fn hex(bytes: &Bytes) -> std::string::String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = std::string::String::with_capacity((bytes.len() as usize) * 2);
    for i in 0..bytes.len() {
        let byte = bytes.get(i).unwrap();
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// The curated core types whose encoding must not change without a migration.
fn snapshot_lines(env: &Env) -> std::vec::Vec<(std::string::String, std::string::String)> {
    let user = Address::generate(env);

    let mut out: std::vec::Vec<(std::string::String, std::string::String)> = std::vec::Vec::new();

    // Unit DataKey variants (global config / pool-wide state).
    let unit_variants: std::vec::Vec<(&str, DataKey)> = std::vec![
        ("DataKey::Admin", DataKey::Admin),
        ("DataKey::Token", DataKey::Token),
        ("DataKey::TotalShares", DataKey::TotalShares),
        ("DataKey::TotalDeposited", DataKey::TotalDeposited),
        ("DataKey::RewardPoolBalance", DataKey::RewardPoolBalance),
        ("DataKey::Paused", DataKey::Paused),
        ("DataKey::LeaderboardSize", DataKey::LeaderboardSize),
        ("DataKey::VestingPeriod", DataKey::VestingPeriod),
        ("DataKey::UnstakeFeeBps", DataKey::UnstakeFeeBps),
    ];
    for (name, key) in unit_variants {
        out.push((name.into(), hex(&key.to_xdr(env))));
    }

    // Address-keyed DataKey variants (per-user persistent state).
    let address_variants: std::vec::Vec<(&str, DataKey)> = std::vec![
        ("DataKey::ShareBalance", DataKey::ShareBalance(user.clone())),
        (
            "DataKey::AccruedReward",
            DataKey::AccruedReward(user.clone())
        ),
        (
            "DataKey::StakedAtLedger",
            DataKey::StakedAtLedger(user.clone())
        ),
        (
            "DataKey::UnbondingPosition",
            DataKey::UnbondingPosition(user.clone())
        ),
        (
            "DataKey::VestingEntries",
            DataKey::VestingEntries(user.clone())
        ),
    ];
    for (name, key) in address_variants {
        out.push((name.into(), hex(&key.to_xdr(env))));
    }

    // Core value types.
    let delegate = ContractDelegate {
        contract_address: user.clone(),
        max_stake_per_call: 100_000,
        total_authorized: 500_000,
        total_used: 12_345,
    };
    out.push(("ContractDelegate".into(), hex(&delegate.to_xdr(env))));

    out
}

fn render(env: &Env) -> std::string::String {
    let mut text = std::string::String::from(
        "# Golden storage-layout snapshot (issue #659). Do not edit by hand; see\n# docs/STORAGE_LAYOUT.md.\n",
    );
    for (name, encoding) in snapshot_lines(env) {
        text.push_str(&std::format!("{name} = {encoding}\n"));
    }
    text
}

#[test]
fn storage_layout_matches_snapshot() {
    let env = Env::default();
    let actual = render(&env);

    let update = std::env::var("UPDATE_STORAGE_SNAPSHOT").is_ok();

    match std::fs::read_to_string(SNAPSHOT_PATH) {
        Ok(golden) if !update => {
            assert_eq!(
                golden.trim_end(),
                actual.trim_end(),
                "Storage layout changed. If this is intentional it requires a \
                 migration for any already-deployed contract; regenerate the \
                 snapshot with UPDATE_STORAGE_SNAPSHOT=1 and commit it with the \
                 migration (see docs/STORAGE_LAYOUT.md)."
            );
        }
        _ => {
            if let Some(parent) = std::path::Path::new(SNAPSHOT_PATH).parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(SNAPSHOT_PATH, &actual).unwrap();
            if update {
                std::println!("Updated storage-layout snapshot at {SNAPSHOT_PATH}");
            } else {
                std::println!(
                    "No storage-layout snapshot found; wrote {SNAPSHOT_PATH}. \
                     Commit it so future changes are checked against it."
                );
            }
        }
    }
}
