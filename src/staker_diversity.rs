//! Staker diversity / stake-concentration score (issue #407).
//!
//! Distinct from `get_reward_gini_coefficient` (issue #275), which measures
//! inequality of *pending reward* distribution. This measures how evenly
//! the *staked principal itself* is spread across active stakers, using the
//! Herfindahl-Hirschman Index (HHI) â€” a standard market-concentration
//! metric: `HHI = sum(share_i^2)`, where `share_i` is each staker's
//! fraction of total stake. See e.g. the U.S. DOJ/FTC Horizontal Merger
//! Guidelines for the canonical definition. A single staker holding
//! everything gives `HHI = 1` (maximum concentration); many equal-sized
//! stakers push `HHI` toward `0`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Env, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Most active stakers `get_staker_diversity_report` will process in one
/// call before reverting with `TooManyStakers` (issue #407, mirrors
/// `MAX_GINI_STAKERS` for issue #275).
pub const MAX_DIVERSITY_STAKERS: u32 = 200;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DiversityReport {
    pub diversity_score_bps: u32,
    pub top_1_pct_share_bps: u32,
    pub top_10_pct_share_bps: u32,
    pub herfindahl_index: u32,
    pub staker_count: u32,
}

/// Sum of the largest `n` entries of `sorted_desc` (descending-sorted stake
/// amounts), as a share of `total_staked` in basis points.
fn top_n_share_bps(sorted_desc: &Vec<i128>, n: u32, total_staked: i128) -> u32 {
    let mut sum: i128 = 0;
    let mut i = 0u32;
    while i < n && i < sorted_desc.len() {
        sum += sorted_desc.get(i).unwrap();
        i += 1;
    }
    if total_staked == 0 {
        0
    } else {
        ((sum * 10_000) / total_staked).clamp(0, 10_000) as u32
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Read-only report on how evenly staked principal is distributed
    /// across all active stakers (issue #407). Admin only, since sorting up
    /// to `MAX_DIVERSITY_STAKERS` stakers is comparatively expensive â€”
    /// mirrors `get_reward_gini_coefficient`'s own reasoning (issue #275).
    ///
    /// Reverts with `TooManyStakers` above `MAX_DIVERSITY_STAKERS` active
    /// stakers.
    pub fn get_staker_diversity_report(env: Env) -> Result<DiversityReport, VaultError> {
        admin::require_admin(&env)?;

        let all_stakers = balance::get_all_stakers(&env);

        // Descending insertion sort of active stake amounts, bounded by
        // MAX_DIVERSITY_STAKERS, mirroring the ascending sort in
        // `get_reward_gini_coefficient`.
        let mut sorted_desc: Vec<i128> = Vec::new(&env);
        let mut total_staked: i128 = 0;
        let mut staker_count: u32 = 0;

        for i in 0..all_stakers.len() {
            let staker = all_stakers.get(i).unwrap();
            let amount = balance::get_shares(&env, &staker);
            if amount <= 0 {
                continue;
            }

            staker_count += 1;
            if staker_count > MAX_DIVERSITY_STAKERS {
                return Err(VaultError::TooManyStakers);
            }
            total_staked += amount;

            let mut ins = sorted_desc.len();
            let mut j = 0u32;
            while j < sorted_desc.len() {
                if amount > sorted_desc.get(j).unwrap() {
                    ins = j;
                    break;
                }
                j += 1;
            }
            sorted_desc.insert(ins, amount);
        }

        if staker_count == 0 || total_staked == 0 {
            return Ok(DiversityReport {
                diversity_score_bps: 0,
                top_1_pct_share_bps: 0,
                top_10_pct_share_bps: 0,
                herfindahl_index: 0,
                staker_count: 0,
            });
        }

        let mut herfindahl_index: i128 = 0;
        for i in 0..sorted_desc.len() {
            let amount = sorted_desc.get(i).unwrap();
            let share_bps = (amount * 10_000) / total_staked;
            herfindahl_index += (share_bps * share_bps) / 10_000;
        }
        let herfindahl_index = herfindahl_index.clamp(0, 10_000) as u32;
        let diversity_score_bps = 10_000u32.saturating_sub(herfindahl_index);

        // "Top 1% / 10% of stakers" â€” at least one staker each, since a
        // fractional staker count rounds up to a whole staker.
        let top_1_pct_count = ((staker_count as u64 + 99) / 100).max(1) as u32;
        let top_10_pct_count = ((staker_count as u64 * 10 + 99) / 100).max(1) as u32;

        let top_1_pct_share_bps = top_n_share_bps(&sorted_desc, top_1_pct_count, total_staked);
        let top_10_pct_share_bps = top_n_share_bps(&sorted_desc, top_10_pct_count, total_staked);

        env.events().publish(
            (symbol_short!("dv_rept"),),
            (diversity_score_bps, herfindahl_index, staker_count, env.ledger().sequence()),
        );

        Ok(DiversityReport {
            diversity_score_bps,
            top_1_pct_share_bps,
            top_10_pct_share_bps,
            herfindahl_index,
            staker_count,
        })
    }
}















