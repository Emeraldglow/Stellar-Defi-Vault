//! Countdown to the next reward halving (issue #376).
//!
//! Builds on the halving schedule from issue #231 — `HalvingConfig` and its
//! helpers (`balance::next_halving_at()`, `balance::halving_count_at()`,
//! `balance::halving_adjusted_rate()`) already compute when halvings occur
//! and what rate applies at a given ledger. This module just packages that
//! into a single read-only view: exact ledgers remaining, and an estimated
//! real-world day count derived from `vault::LEDGERS_PER_DAY` (the same
//! average-ledger-close-time constant the refill-alert and activity-heatmap
//! features already use).
//!
//! Read-only advisory, same convention as `reward_smoothing()`'s
//! `SmoothingStatus`: no `HalvingConfig` means every field comes back zero
//! rather than an error.

use soroban_sdk::{contractimpl, Env};

use crate::balance;
use crate::storage::HalvingCountdown;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Exact ledger and estimated real-world days remaining until the next
    /// reward halving (issue #376). Returns an all-zero `HalvingCountdown` if
    /// no halving schedule has ever been configured via
    /// `set_halving_config()`.
    pub fn halving_countdown(env: Env) -> HalvingCountdown {
        if balance::get_halving_config(&env).is_none() {
            return HalvingCountdown {
                next_halving_ledger: 0,
                ledgers_remaining: 0,
                estimated_days_remaining: 0,
                halvings_so_far: 0,
                current_rate_bps: 0,
                post_halving_rate_bps: 0,
            };
        }

        let current_ledger = env.ledger().sequence();
        let next_halving_ledger = balance::next_halving_at(&env).unwrap_or(current_ledger);
        let ledgers_remaining = next_halving_ledger.saturating_sub(current_ledger);
        // Ceiling division so a countdown of e.g. 1 ledger doesn't round down to "0 days".
        let estimated_days_remaining =
            (ledgers_remaining + LEDGERS_PER_DAY - 1) / LEDGERS_PER_DAY;

        let base_rate_bps = balance::get_reward_rate_bps(&env);
        let current_rate_bps = balance::halving_adjusted_rate(&env, base_rate_bps, current_ledger);
        let post_halving_rate_bps =
            balance::halving_adjusted_rate(&env, base_rate_bps, next_halving_ledger);

        HalvingCountdown {
            next_halving_ledger,
            ledgers_remaining,
            estimated_days_remaining,
            halvings_so_far: balance::halving_count_at(&env, current_ledger),
            current_rate_bps,
            post_halving_rate_bps,
        }
    }
}
