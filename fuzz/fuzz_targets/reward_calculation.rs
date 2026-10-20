#![no_main]
//! Fuzz target: reward calculation (issue #661).
//!
//! Drives the pure reward formula extracted into `src/reward_math.rs` (the same
//! code `VaultContract::reward_for_ledgers` calls during accrual) with arbitrary
//! inputs and asserts:
//!
//! - **No panic.** Every step uses checked arithmetic; overflow must surface as
//!   the documented `VaultError::ArithmeticError`, never a panic.
//! - **Zero guards.** A zero amount, zero elapsed ledgers, or a zero rate must
//!   yield exactly `0`.
//! - **No negative rewards.** For positive amounts and non-negative rate /
//!   multiplier / ledger inputs the reward must be non-negative.
//! - **Monotonic in elapsed ledgers** is exercised implicitly by the value
//!   property below: a longer accrual window can never yield less than a
//!   shorter one at the same rate.

use libfuzzer_sys::fuzz_target;
use stellar_defi_vault::fuzz_support::reward_for_ledgers;

fuzz_target!(|data: (i128, u32, u32, u32, u32, u32)| {
    let (amount, rate_bps, tier_bps, campaign_bps, season_bps, elapsed_ledgers) = data;

    let result = reward_for_ledgers(
        amount,
        rate_bps,
        tier_bps,
        campaign_bps,
        season_bps,
        elapsed_ledgers,
    );

    match result {
        Ok(reward) => {
            if amount <= 0 || elapsed_ledgers == 0 || rate_bps == 0 {
                assert_eq!(
                    reward, 0,
                    "a zero amount / zero ledgers / zero rate must pay zero reward"
                );
            } else {
                assert!(
                    reward >= 0,
                    "positive amount at a non-negative rate with non-zero ledgers must not pay a negative reward"
                );
            }

            // Monotonicity in elapsed ledgers: doubling the window cannot pay
            // less (integer division is monotone).
            if amount > 0 && rate_bps > 0 && elapsed_ledgers > 0 {
                if let Ok(double) = reward_for_ledgers(
                    amount,
                    rate_bps,
                    tier_bps,
                    campaign_bps,
                    season_bps,
                    elapsed_ledgers.saturating_mul(2),
                ) {
                    assert!(
                        double >= reward,
                        "reward must be monotonic non-decreasing in elapsed ledgers"
                    );
                }
            }
        }
        Err(_) => {
            // Documented arithmetic overflow; acceptable, not a crash.
        }
    }
});
