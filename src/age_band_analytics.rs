//! Issue #425: stake age band analytics.
//!
//! Groups the *currently active* stakers by how long they have held their
//! position, so operators can see how the pool splits between new and
//! long-term stakers. This is distinct from join-week cohort analytics: a
//! staker's band is derived from `current_ledger - staked_at_ledger`, so it
//! moves as time passes.
//!
//! Bands, in whole days (`LEDGERS_PER_DAY` = 17,280 ledgers):
//!
//! | Band          | Days      |
//! |---------------|-----------|
//! | `Fresh`       | 0 - 30    |
//! | `Growing`     | 31 - 90   |
//! | `Established` | 91 - 180  |
//! | `Mature`      | 181 - 365 |
//! | `Veteran`     | 366+      |
//!
//! Day 365 is the last day of `Mature`; `Veteran` starts at day 366.
//!
//! Both queries are read-only and need no auth.

use soroban_sdk::{contracterror, contractimpl, contracttype, Address, Env, Vec};

use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};
use crate::balance;

/// Most registered stakers `get_age_band_report` will scan in one call.
pub const MAX_AGE_BAND_STAKERS: u32 = 200;

/// How long a staker has held their current position.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AgeBand {
    Fresh,
    Growing,
    Established,
    Mature,
    Veteran,
}

/// Aggregate metrics for one age band. Empty bands are reported with zeros.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgeBandStats {
    pub band: AgeBand,
    pub staker_count: u32,
    pub total_staked: i128,
    pub avg_position: i128,
    pub avg_pending_reward: i128,
}

/// Errors for the age band queries.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AgeBandError {
    /// More than `MAX_AGE_BAND_STAKERS` stakers are registered.
    TooManyStakers = 1,
}

const BANDS: [AgeBand; 5] = [
    AgeBand::Fresh,
    AgeBand::Growing,
    AgeBand::Established,
    AgeBand::Mature,
    AgeBand::Veteran,
];

/// Maps a holding duration in whole days to its band.
pub fn band_for_days(days: u32) -> AgeBand {
    match days {
        0..=30 => AgeBand::Fresh,
        31..=90 => AgeBand::Growing,
        91..=180 => AgeBand::Established,
        181..=365 => AgeBand::Mature,
        _ => AgeBand::Veteran,
    }
}

fn band_index(band: AgeBand) -> usize {
    match band {
        AgeBand::Fresh => 0,
        AgeBand::Growing => 1,
        AgeBand::Established => 2,
        AgeBand::Mature => 3,
        AgeBand::Veteran => 4,
    }
}

/// The band `user` currently falls in, `None` when they have no position.
fn band_of(env: &Env, user: &Address) -> Option<AgeBand> {
    if balance::get_shares(env, user) == 0 {
        return None;
    }
    let now = env.ledger().sequence();
    let staked_at: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::StakedAtLedger(user.clone()))
        .unwrap_or(now);
    let days = now.saturating_sub(staked_at) / LEDGERS_PER_DAY;
    Some(band_for_days(days))
}

#[contractimpl]
impl VaultContract {
    /// Read-only: the age band `user`'s position is in, or `None` when they
    /// have no active position.
    pub fn get_user_age_band(env: Env, user: Address) -> Option<AgeBand> {
        band_of(&env, &user)
    }

    /// Read-only: aggregate stats for every age band across all active
    /// stakers. Always returns all five bands, in age order, with zeros for
    /// bands nobody is in. Averages are integer-truncated.
    ///
    /// Reverts with `TooManyStakers` when more than `MAX_AGE_BAND_STAKERS`
    /// stakers are registered, to bound the per-call storage reads.
    pub fn get_age_band_report(env: Env) -> Result<Vec<AgeBandStats>, AgeBandError> {
        let stakers = balance::get_all_stakers(&env);
        if stakers.len() > MAX_AGE_BAND_STAKERS {
            return Err(AgeBandError::TooManyStakers);
        }

        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);

        let mut counts = [0u32; 5];
        let mut staked = [0i128; 5];
        let mut pending = [0i128; 5];

        for user in stakers.iter() {
            let band = match band_of(&env, &user) {
                Some(band) => band,
                None => continue,
            };
            let i = band_index(band);
            let shares = balance::get_shares(&env, &user);
            let position = balance::shares_to_amount(total_shares, total_deposited, shares)
                .unwrap_or(0);
            let reward = VaultContract::calc_pending_reward(env.clone(), user.clone()).unwrap_or(0);
            counts[i] += 1;
            staked[i] += position;
            pending[i] += reward;
        }

        let mut report = Vec::new(&env);
        for band in BANDS {
            let i = band_index(band);
            let count = counts[i] as i128;
            report.push_back(AgeBandStats {
                band,
                staker_count: counts[i],
                total_staked: staked[i],
                avg_position: if count == 0 { 0 } else { staked[i] / count },
                avg_pending_reward: if count == 0 { 0 } else { pending[i] / count },
            });
        }
        Ok(report)
    }
}
