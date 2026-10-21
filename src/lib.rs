//! Stellar DeFi Vault: a non-custodial yield vault built on Soroban.
//!
//! The canonical public API is the [`interface::VaultTrait`] trait, implemented
//! by [`vault::VaultContract`]. Entrypoint docs follow a common layout:
//! a summary, then `# Authorization`, `# Errors` and `# Events` sections
//! (issue #675).
#![no_std]
// Issue #658: the contract has never needed `unsafe`, and a deployed Soroban
// contract must not. Forbidding it crate-wide also keeps unsafe blocks out of
// the test-only code that shares this crate.
#![forbid(unsafe_code)]
// Issue #675: every public item should be documented. Left at `warn` while the
// existing ~1,000 undocumented items are worked through; flip to `deny` once
// `cargo doc` is clean.
#![warn(missing_docs)]

mod admin;
mod balance;
pub mod claim_receipts;
mod errors;
mod events;
pub mod example_consumer;
pub mod interface;
pub mod nft;
pub mod reward_accumulator;
mod reward_math;
mod storage;
pub mod ttl_management;
pub mod user_ttl;
pub mod vault;

pub mod commitment;
pub mod content_curation;
pub mod insurance;
pub mod nft_fractionalize;
pub mod position_mirroring;
pub mod price_oracle;
pub mod reputation_decay;
pub mod stake_quota;

// Features added as their own modules rather than inside `vault.rs`. Soroban
// supports several `#[contractimpl]` blocks for one contract type, and
// `nft.rs` already establishes the pattern, so each of these keeps its storage
// keys, types, and entrypoints together instead of appending to a 25k-line
// file. `DataKey` is at Soroban's 50-variant cap for `#[contracttype]` enums,
// so all of them use raw `Symbol`-keyed storage as `balance.rs` does.
pub mod balance_checkpoints; // issues #709, #710 — balance checkpoints and TWAB
pub mod cross_pool_identity; // issue #470 — cross-pool identity
pub mod daily_community_tip; // issue #458 — daily stake-weighted featured tip vote
pub mod governance_params; // issue #712 — governance quorum and pass threshold
pub mod lockdrop_campaign; // issue #460 — lockdrop campaign
pub mod minimum_unstake_amount; // issue #441 — minimum unstake amount
pub mod peg_stabilization;
pub mod position_health_auto_recovery; // issue #459 — position health auto-recovery
pub mod position_value_appreciation_log; // issue #469 — position value appreciation log
pub mod proof_of_humanity_hook; // issue #461 — proof-of-humanity hook
pub mod reward_token_audit_trail; // issue #467 — reward token audit trail
pub mod roadmap_voting; // issue #462 — roadmap voting
pub mod stake_funded_bug_bounty; // issue #468 — stake-funded bug bounty
pub mod stake_hooks; // issue #711 — stake and unstake hook contracts
pub mod staker_favor_rounding; // issue #457 — always round in the staker's favor
pub mod staker_network_graph; // issue #456 — staker delegation/referral/mirror network graph
pub mod time_locked_admin_proposal; // issue #455 -- time-locked admin config-change announcements
pub mod vault_extensions_463_466; // issues #463-#466 — clawback, NFT boost, milestone, param log
pub mod vault_extensions_761_764; // issues #761-#764 — proposal categories, expedited proposals, delegation expiry, strategy adapters
pub mod vault_extensions_792_795; // issues #792-#795 — reward decimals, funder attribution, treasury rate limit, surplus rescue
pub mod vesting_cliff; // issue #287 — reward vesting cliff // reward-token price peg stabilization

pub use nft::StakeReceiptNFT;
pub use vault::VaultContract;

#[cfg(test)]
mod test_issues_757_760;

#[cfg(test)]
mod test;

#[cfg(test)]
mod test_share_decimals;

#[cfg(test)]
mod test_issues_628_631;

#[cfg(test)]
mod test_content_curation;

#[cfg(test)]
mod test_integration;

#[cfg(test)]
mod test_nft_fractionalize;

#[cfg(test)]
mod test_reputation_decay;

#[cfg(test)]
mod test_validator_rewards;

#[cfg(test)]
mod test_features_287_290;

#[cfg(test)]
mod test_issues_463_466;

#[cfg(test)]
mod test_issues_701_704;

// Issue #658: this module was missing its `#[cfg(test)]` gate, so its test
// helpers (and their `testutils` dependencies) were compiled into the release
// WASM. Every test module must stay behind `#[cfg(test)]`.
#[cfg(test)]
mod test_issues_467_470;

#[cfg(test)]
mod test_issues_697_700;

#[cfg(test)]
mod test_issues_459_462;

#[cfg(test)]
mod test_peg_stabilization;

#[cfg(test)]
mod test_issues_709_712;

// ── Issue #659: storage-layout upgrade-safety snapshot ───────────────────────
#[cfg(test)]
mod test_storage_layout;

// ── Issue #660: resource-footprint limits for batch/loop entrypoints ─────────
#[cfg(test)]
mod test_resource_footprint;

#[cfg(test)]
mod test_issues_646_650_737_738;

#[cfg(test)]
mod test_token_interface_compliance;

#[cfg(test)]
mod test_vec_storage_capacity;

#[cfg(test)]
mod test_loyalty_points;

#[cfg(test)]
mod test_claim_streak;

#[cfg(test)]
mod test_issue_754;

#[cfg(test)]
mod test_solvency_report;

/// Pure math surfaces exposed for property tests and `cargo-fuzz` (issue #661).
///
/// Gated so it never lands in the deployed WASM: the fuzz crate enables the
/// `testutils` feature, and in-crate tests get it via `cfg(test)`.
#[cfg(any(test, feature = "testutils"))]
pub mod fuzz_support {
    pub use crate::balance::{amount_to_shares_with_decimals, shares_to_amount_with_decimals};
    pub use crate::reward_math::reward_for_ledgers;
}
#[cfg(test)]
mod test_resource_costs;

#[cfg(test)]
mod test_runbook_interface_snapshot;

// ── Issue #770: upgrade test using previously-released WASM ────────────────
#[cfg(test)]
mod test_issues_770_upgrade;

// ── Issue #769: differential tests with reference math models ──────────────
#[cfg(test)]
mod test_issues_769_differential;
