//! Pure reward-accrual math (issue #661).
//!
//! This is the single source of truth for the reward formula that
//! `VaultContract::reward_for_ledgers` uses during accrual. It is free of any
//! `Env`/storage access so it can be exercised directly by unit tests,
//! property tests (`proptest`) and `cargo-fuzz` targets without a contract
//! instance.
//!
//! Multipliers stack multiplicatively (issue #276): the tier multiplier, then
//! the campaign multiplier (#48), then the season multiplier:
//!
//! ```text
//! effective_rate = rate_bps * tier_mult / 10_000
//!                  * campaign_mult / 10_000
//!                  * season_mult  / 10_000
//! reward         = amount * effective_rate * elapsed_ledgers
//!                  / 10_000 / STELLAR_LEDGERS_PER_YEAR
//! ```
//!
//! Every intermediate uses checked arithmetic and returns
//! [`VaultError::ArithmeticError`] instead of panicking, so a fuzz target can
//! assert "no panic; error only via the documented variant".

use crate::errors::VaultError;
use crate::vault::{BOOST_BPS_BASE, STELLAR_LEDGERS_PER_YEAR};

/// Reward owed on `amount` over `elapsed_ledgers`, with all three multipliers
/// stacked and the result annualised to a per-ledger rate. Returns `Ok(0)` for
/// a zero amount or zero elapsed ledgers.
pub fn reward_for_ledgers(
    amount: i128,
    rate_bps: u32,
    multiplier_bps: u32,
    campaign_multiplier_bps: u32,
    season_multiplier_bps: u32,
    elapsed_ledgers: u32,
) -> Result<i128, VaultError> {
    if elapsed_ledgers == 0 || amount == 0 {
        return Ok(0);
    }

    // Apply tier multiplier: effective_rate = rate_bps * tier_mult / 10000
    let effective_rate_bps = (rate_bps as i128)
        .checked_mul(multiplier_bps as i128)
        .ok_or(VaultError::ArithmeticError)?
        .checked_div(BOOST_BPS_BASE as i128)
        .ok_or(VaultError::ArithmeticError)?;

    // Stack campaign multiplier: boosted_rate = effective_rate * campaign_mult / 10000
    let boosted_rate_bps = effective_rate_bps
        .checked_mul(campaign_multiplier_bps as i128)
        .ok_or(VaultError::ArithmeticError)?
        .checked_div(BOOST_BPS_BASE as i128)
        .ok_or(VaultError::ArithmeticError)?;

    // Issue #276: stack season multiplier on top of tier + campaign.
    let seasoned_rate_bps = boosted_rate_bps
        .checked_mul(season_multiplier_bps as i128)
        .ok_or(VaultError::ArithmeticError)?
        .checked_div(BOOST_BPS_BASE as i128)
        .ok_or(VaultError::ArithmeticError)?;

    amount
        .checked_mul(seasoned_rate_bps)
        .ok_or(VaultError::ArithmeticError)?
        .checked_mul(elapsed_ledgers as i128)
        .ok_or(VaultError::ArithmeticError)?
        .checked_div(BOOST_BPS_BASE as i128)
        .ok_or(VaultError::ArithmeticError)?
        .checked_div(STELLAR_LEDGERS_PER_YEAR as i128)
        .ok_or(VaultError::ArithmeticError)
}
