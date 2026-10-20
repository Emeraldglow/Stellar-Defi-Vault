#![cfg(test)]

//! Issue #622: `contractmeta!` name/version/description entries.
//!
//! `contractmeta!`'s `key`/`val` arguments must be string literals (parsed by
//! the macro at expansion time), so the values embedded in `vault.rs` can't
//! be built from `CONTRACT_VERSION` or `env!("CARGO_PKG_VERSION")` directly.
//! This test is the drift guard called out in `vault.rs`'s comment next to
//! the `contractmeta!` calls: it fails in CI (`cargo test --features
//! testutils`) the moment `CONTRACT_VERSION`, the `contractmeta!` literal, or
//! `Cargo.toml`'s `version` field drift apart from each other.

use crate::vault::CONTRACT_VERSION;

/// Mirrors the literal passed to `contractmeta!(key = "version", val = ...)`
/// in `vault.rs`. Update this alongside that literal and `CONTRACT_VERSION`
/// whenever the contract version changes.
const CONTRACTMETA_VERSION_LITERAL: &str = "0.1.0";

#[test]
fn contract_version_matches_cargo_toml_version() {
    assert_eq!(
        CONTRACT_VERSION,
        env!("CARGO_PKG_VERSION"),
        "CONTRACT_VERSION in vault.rs must match the `version` field in Cargo.toml"
    );
}

#[test]
fn contractmeta_version_literal_matches_contract_version() {
    assert_eq!(
        CONTRACTMETA_VERSION_LITERAL, CONTRACT_VERSION,
        "the `val` literal on contractmeta!(key = \"version\", ...) in vault.rs \
         must be updated to match CONTRACT_VERSION"
    );
}
