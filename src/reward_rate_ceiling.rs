//! Lower-only maximum reward rate ceiling (issue #532).
//!
//! `set_reward_rate_bps` is bounded by the hard-coded `balance::MAX_RATE_BPS`,
//! which is far above what most pools can sustain. The admin can tighten this
//! with `set_max_reward_rate()`, which only ever lowers the ceiling — once
//! lowered it can never be raised again, so a compromised or careless admin
//! cannot later set an unsustainable rate that drains the reward pool.
//!
//! The ceiling is enforced on both the direct `set_reward_rate_bps` setter
//! and the timelocked `SetRewardRate` admin action.
//!
//! # Storage
//!
//! Raw `Symbol`-keyed instance storage, matching `balance.rs`.
//!
//! - `symbol_short!("max_rate")` -> `u32` (unset = `balance::MAX_RATE_BPS`)

use soroban_sdk::{contractimpl, symbol_short, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultFeature4Error;
use crate::vault::{VaultContract, VaultContractClient};

const MAX_RATE_KEY: Symbol = symbol_short!("max_rate");

/// Current reward rate ceiling in basis points. Defaults to
/// `balance::MAX_RATE_BPS` when never lowered.
pub fn read_max_reward_rate(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&MAX_RATE_KEY)
        .unwrap_or(balance::MAX_RATE_BPS)
}

/// True when `rate_bps` is within the configured ceiling.
pub(crate) fn within_ceiling(env: &Env, rate_bps: u32) -> bool {
    rate_bps <= read_max_reward_rate(env)
}

#[contractimpl]
impl VaultContract {
    /// Admin: lower the maximum reward rate ceiling (basis points). The
    /// ceiling can never be raised. Reverts with `InvalidRateCeiling` when
    /// `max_bps` is zero, above the current ceiling, or below the currently
    /// active reward rate (lower the rate first).
    pub fn set_max_reward_rate(env: Env, max_bps: u32) -> Result<(), VaultFeature4Error> {
        admin::require_admin(&env)?;
        let current_ceiling = read_max_reward_rate(&env);
        if max_bps == 0
            || max_bps > current_ceiling
            || max_bps < balance::get_reward_rate_bps(&env)
        {
            return Err(VaultFeature4Error::InvalidRateCeiling);
        }
        env.storage().instance().set(&MAX_RATE_KEY, &max_bps);
        env.events()
            .publish((symbol_short!("max_rate"),), (current_ceiling, max_bps));
        Ok(())
    }

    /// Read-only: the current reward rate ceiling in basis points.
    pub fn get_max_reward_rate(env: Env) -> u32 {
        read_max_reward_rate(&env)
    }
}
