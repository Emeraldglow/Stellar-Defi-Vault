//! Historical reward emission rate sampling for charting (issue #440).
//!
//! Unlike the admin-change rate history (issue #88), this records the
//! *effective* emission rate — including all active modifiers (halving,
//! boost tiers, campaigns) — sampled at regular intervals by the admin, so
//! frontends can chart what stakers actually earn over time.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::balance;
use crate::errors::VaultError;
use crate::vault::{VaultContract, BOOST_BPS_BASE, LEDGERS_PER_DAY, STELLAR_LEDGERS_PER_YEAR};

const EMISSION_HISTORY_KEY: Symbol = symbol_short!("em_hist");
const MAX_SAMPLES: u32 = 100;
const BPS_DENOM: i128 = 10_000;

/// One sampled emission-rate snapshot.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EmissionDataPoint {
    pub ledger: u32,
    pub base_rate_bps: i128,
    pub effective_rate_bps: i128,
    pub total_staked_at_sample: i128,
    pub daily_emission: i128,
}

/// Pool age in ledgers (0 when the pool was never initialized).
fn pool_age_ledgers(env: &Env) -> u32 {
    let current = env.ledger().sequence();
    let initialized = balance::get_initialized_at_ledger(env).unwrap_or(current);
    current.saturating_sub(initialized)
}

/// The applicable boost-tier multiplier for the pool's current age, or 10000
/// (1x) when no schedule is configured or no tier has been reached.
fn current_boost_multiplier(env: &Env) -> i128 {
    let age = pool_age_ledgers(env);
    let mut multiplier: i128 = BOOST_BPS_BASE as i128;
    if let Some(schedule) = balance::get_boost_schedule(env) {
        let n = schedule.len();
        let mut i = 0u32;
        while i < n {
            let (tier_ledger, tier_multiplier) = schedule.get(i).unwrap();
            if age >= tier_ledger {
                multiplier = tier_multiplier as i128;
            }
            i += 1;
        }
    }
    multiplier
}

/// The campaign multiplier in effect at the current ledger, or 10000 (1x)
/// when no campaign is active.
fn current_campaign_multiplier(env: &Env) -> i128 {
    let campaign: Option<crate::storage::CampaignInfo> = env
        .storage()
        .instance()
        .get(&crate::storage::DataKey::BoostCampaign);
    match campaign {
        Some(c) => {
            let ledger = env.ledger().sequence();
            if ledger >= c.starts_at_ledger && ledger <= c.ends_at_ledger {
                c.multiplier_bps as i128
            } else {
                BOOST_BPS_BASE as i128
            }
        }
        None => BOOST_BPS_BASE as i128,
    }
}

/// Compute the effective emission rate (bps) at the current ledger by
/// stacking halving, boost-tier, and campaign multipliers on the base rate.
fn compute_effective_rate_bps(env: &Env) -> i128 {
    let base = balance::get_reward_rate_bps(env) as i128;
    let ledger = env.ledger().sequence();
    let halved = balance::halving_adjusted_rate(env, base as u32, ledger);
    let boosted = halved
        .saturating_mul(current_boost_multiplier(env))
        .saturating_div(BPS_DENOM);
    boosted
        .saturating_mul(current_campaign_multiplier(env))
        .saturating_div(BPS_DENOM)
}

/// `daily_emission = total_staked * effective_rate_bps * LEDGERS_PER_DAY /
/// (BPS_DENOM * LEDGERS_PER_YEAR)`.
fn compute_daily_emission(total_staked: i128, effective_rate_bps: i128) -> i128 {
    let denominator = BPS_DENOM.saturating_mul(STELLAR_LEDGERS_PER_YEAR as i128);
    total_staked
        .saturating_mul(effective_rate_bps)
        .saturating_mul(LEDGERS_PER_DAY as i128)
        .saturating_div(denominator)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Record the current effective emission rate as a history sample.
    /// Admin only. History is capped at 100 samples (oldest dropped).
    pub fn take_emission_sample(env: Env, admin: Address) -> Result<(), VaultError> {
        admin.require_auth();
        crate::admin::require_admin(&env)?;

        let ledger = env.ledger().sequence();
        let base_rate_bps = balance::get_reward_rate_bps(&env) as i128;
        let effective_rate_bps = compute_effective_rate_bps(&env);
        let total_staked = balance::get_total_deposited(&env);
        let daily_emission = compute_daily_emission(total_staked, effective_rate_bps);

        let mut history: Vec<EmissionDataPoint> = env
            .storage()
            .instance()
            .get(&EMISSION_HISTORY_KEY)
            .unwrap_or_else(|| Vec::new(&env));

        history.push_back(EmissionDataPoint {
            ledger,
            base_rate_bps,
            effective_rate_bps,
            total_staked_at_sample: total_staked,
            daily_emission,
        });

        while history.len() > MAX_SAMPLES {
            history.remove(0);
        }

        env.storage()
            .instance()
            .set(&EMISSION_HISTORY_KEY, &history);

        env.events().publish(
            (symbol_short!("em_samp"),),
            (effective_rate_bps, total_staked, daily_emission, ledger),
        );
        Ok(())
    }

    /// Read-only query: the full emission history, oldest first.
    pub fn get_emission_history(env: Env) -> Vec<EmissionDataPoint> {
        env.storage()
            .instance()
            .get(&EMISSION_HISTORY_KEY)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Read-only query: emission samples recorded at or after `ledger`.
    pub fn get_emission_history_since(env: Env, ledger: u32) -> Vec<EmissionDataPoint> {
        let history: Vec<EmissionDataPoint> = env
            .storage()
            .instance()
            .get(&EMISSION_HISTORY_KEY)
            .unwrap_or_else(|| Vec::new(&env));
        let mut filtered = Vec::new(&env);
        let n = history.len();
        let mut i = 0u32;
        while i < n {
            let sample = history.get(i).unwrap();
            if sample.ledger >= ledger {
                filtered.push_back(sample);
            }
            i += 1;
        }
        filtered
    }
}
