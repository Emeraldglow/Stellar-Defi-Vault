//! Staker-favor rounding mode (issue #457).
//!
//! Distinct from the configurable `RoundingPolicy` (Floor/Ceiling/Nearest).
//! When enabled, reward amounts always round up (ceiling division) and fee
//! amounts always round down, so the staker gets the benefit of any
//! rounding ambiguity in every mathematical operation.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw
//! `Symbol`-keyed instance storage, matching `balance.rs` and other feature
//! modules.
//!
//! Storage key: `symbol_short!("sfr_on")` -> `bool`

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultQuizError;
use crate::vault::{VaultContract, VaultContractClient};

const STAKER_FAVOR_KEY: Symbol = symbol_short!("sfr_on");

pub fn is_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&STAKER_FAVOR_KEY)
        .unwrap_or(false)
}

pub fn set_enabled(env: &Env, enabled: bool) {
    env.storage().instance().set(&STAKER_FAVOR_KEY, &enabled);
}

/// Ceiling division for non-negative operands.
fn ceiling_div(numerator: i128, denominator: i128) -> i128 {
    if denominator <= 0 {
        return 0;
    }
    (numerator + denominator - 1) / denominator
}

/// Applies the staker-favor rounding rule to a reward calculation
/// (`numerator / denominator`): rounds up when staker-favor mode is
/// enabled, otherwise uses ordinary floor division.
pub fn apply_reward_rounding(env: &Env, numerator: i128, denominator: i128) -> i128 {
    if denominator == 0 {
        return 0;
    }
    if is_enabled(env) {
        ceiling_div(numerator, denominator)
    } else {
        numerator / denominator
    }
}

/// Applies the staker-favor rounding rule to a fee calculation
/// (`numerator / denominator`): always rounds down (floor division), so
/// fees never take more from the staker than warranted.
pub fn apply_fee_rounding(_env: &Env, numerator: i128, denominator: i128) -> i128 {
    if denominator == 0 {
        return 0;
    }
    numerator / denominator
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #457: Admin toggles staker-favor rounding mode on or off.
    pub fn set_staker_favor_rounding(
        env: Env,
        admin_addr: Address,
        enabled: bool,
    ) -> Result<(), VaultQuizError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        crate::staker_favor_rounding::set_enabled(&env, enabled);

        env.events()
            .publish((symbol_short!("sfr_set"), admin_addr), enabled);

        Ok(())
    }

    /// Issue #457: Read-only query for whether staker-favor rounding is
    /// currently enabled.
    pub fn is_staker_favor_rounding_enabled(env: Env) -> bool {
        crate::staker_favor_rounding::is_enabled(&env)
    }
}
