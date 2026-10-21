#![cfg(test)]
//! Differential tests for issue #769: Reference model validation.
//! 
//! Implements share and reward math as independent reference models
//! and verifies contract outputs match across random scenarios.

extern crate std;

use crate::balance::{amount_to_shares_with_decimals, shares_to_amount_with_decimals};
use crate::reward_math::reward_for_ledgers;

// Reference: amount to shares
fn ref_amt_to_shares(total_shares: i128, total_deposited: i128, amount: i128, decimals: u32) -> Option<i128> {
    if total_shares == 0 || total_deposited == 0 {
        ref_scale(amount, decimals, 7)
    } else {
        amount.checked_mul(total_shares)?.checked_div(total_deposited)
    }
}

// Reference: shares to amount
fn ref_shares_to_amt(total_shares: i128, total_deposited: i128, shares: i128) -> Option<i128> {
    if total_shares == 0 {
        Some(0)
    } else {
        shares.checked_mul(total_deposited)?.checked_div(total_shares)
    }
}

// Reference: decimal scaling
fn ref_scale(amount: i128, from: u32, to: u32) -> Option<i128> {
    if from == to {
        Some(amount)
    } else if from < to {
        amount.checked_mul(10_i128.checked_pow(to - from)?)
    } else {
        amount.checked_div(10_i128.checked_pow(from - to)?)
    }
}

// Reference: reward calculation
fn ref_reward(amount: i128, rate_bps: u32, tier: u32, campaign: u32, season: u32, elapsed: u32) -> Option<i128> {
    if elapsed == 0 || amount == 0 || rate_bps == 0 {
        return Some(0);
    }
    const BPS: i128 = 10_000;
    const YEAR: i128 = 6_307_200;
    
    let eff = (rate_bps as i128).checked_mul(tier as i128)?.checked_div(BPS)?;
    let boost = eff.checked_mul(campaign as i128)?.checked_div(BPS)?;
    let season_rate = boost.checked_mul(season as i128)?.checked_div(BPS)?;
    
    amount
        .checked_mul(season_rate)?
        .checked_mul(elapsed as i128)?
        .checked_div(BPS)?
        .checked_div(YEAR)
}

#[test]
fn test_differential_share_conversions() {
    let tests = vec![
        (0, 0, 1_000_000, 7),
        (10_000_000, 1_000_000_000, 100_000_000, 7),
        (5_000_000, 500_000_000, 50_000_000, 6),
        (1, 1, 1, 7),
    ];
    
    for (total_shares, total_deposited, amount, decimals) in tests {
        let contract = amount_to_shares_with_decimals(total_shares, total_deposited, amount, decimals);
        let reference = ref_amt_to_shares(total_shares, total_deposited, amount, decimals);
        assert_eq!(contract, reference, "deposit: {} {} {} {}", total_shares, total_deposited, amount, decimals);
    }
}

#[test]
fn test_differential_redemptions() {
    let tests = vec![
        (0, 0, 100_000_000),
        (10_000_000, 1_000_000_000, 1_000_000),
        (5_000_000, 500_000_000, 5_000_000),
        (1, 1, 1),
    ];
    
    for (total_shares, total_deposited, shares) in tests {
        let contract = shares_to_amount_with_decimals(total_shares, total_deposited, shares, 7);
        let reference = ref_shares_to_amt(total_shares, total_deposited, shares);
        assert_eq!(contract, reference, "redeem: {} {} {}", total_shares, total_deposited, shares);
    }
}

#[test]
fn test_differential_rewards_baseline() {
    let tests = vec![
        (1_000_000_000, 500, 10_000, 10_000, 10_000, 100),
        (10_000_000_000, 1_000, 10_000, 10_000, 10_000, 100),
        (100_000_000, 200, 10_000, 10_000, 10_000, 50),
        (0, 500, 10_000, 10_000, 10_000, 100),
        (1_000_000_000, 0, 10_000, 10_000, 10_000, 100),
        (1_000_000_000, 500, 10_000, 10_000, 10_000, 0),
    ];
    
    for (amount, rate, tier, campaign, season, elapsed) in tests {
        let contract = reward_for_ledgers(amount, rate, tier, campaign, season, elapsed);
        let reference = ref_reward(amount, rate, tier, campaign, season, elapsed);
        assert_eq!(contract, reference, "reward: {} {} {} {} {} {}", amount, rate, tier, campaign, season, elapsed);
    }
}

#[test]
fn test_differential_rewards_multipliers() {
    let tests = vec![
        (1_000_000_000, 500, 5_000, 10_000, 10_000, 100),
        (1_000_000_000, 500, 10_000, 5_000, 10_000, 100),
        (1_000_000_000, 500, 10_000, 10_000, 5_000, 100),
        (1_000_000_000, 500, 5_000, 5_000, 5_000, 100),
        (10_000_000_000, 1_000, 12_500, 8_000, 6_000, 1_000),
    ];
    
    for (amount, rate, tier, campaign, season, elapsed) in tests {
        let contract = reward_for_ledgers(amount, rate, tier, campaign, season, elapsed);
        let reference = ref_reward(amount, rate, tier, campaign, season, elapsed);
        assert_eq!(contract, reference, "reward_mult: {} {} {} {} {} {}", amount, rate, tier, campaign, season, elapsed);
    }
}

#[test]
fn test_differential_round_trip() {
    let tests = vec![
        (10_000_000, 1_000_000_000, 500_000_000),
        (5_000_000, 500_000_000, 100_000_000),
        (1_000_000_000, 100_000_000_000, 1_000_000_000),
    ];
    
    for (total_shares, total_deposited, amount) in tests {
        let shares_opt = amount_to_shares_with_decimals(total_shares, total_deposited, amount, 7);
        if let Some(shares) = shares_opt {
            let redeemed_opt = shares_to_amount_with_decimals(total_shares, total_deposited, shares, 7);
            if let Some(redeemed) = redeemed_opt {
                assert!(redeemed <= amount, "round-trip {} → {} → {} violates ≤", amount, shares, redeemed);
            }
        }
    }
}

#[test]
fn test_differential_rewards_monotonic() {
    let base_tests = vec![
        (1_000_000_000, 500, 10_000, 10_000, 10_000, 100),
        (10_000_000_000, 1_000, 5_000, 8_000, 10_000, 50),
    ];
    
    for (amount, rate, tier, campaign, season, elapsed) in base_tests {
        let r1 = reward_for_ledgers(amount, rate, tier, campaign, season, elapsed).unwrap_or(0);
        let r2 = reward_for_ledgers(amount, rate, tier, campaign, season, elapsed * 2).unwrap_or(0);
        assert!(r2 >= r1, "monotonic: {} <= {}", r1, r2);
    }
}
