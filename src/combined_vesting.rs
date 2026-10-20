//! Combined cliff-then-linear reward vesting (issue #346).
//!
//! Issues #35 (linear vesting schedule) and #287 (`vesting_cliff.rs`) each
//! gate reward availability on their own. This module is the standard VC-style
//! curve that combines them: nothing is claimable during `cliff_ledgers`, the
//! cliff's worth of rewards unlock all at once the instant the cliff clears,
//! and everything accrued *after* the cliff then releases linearly over
//! `linear_period_ledgers`, counted from the cliff date (not the stake date).
//!
//! Per the issue notes, setting a combined config overrides the standalone
//! `vesting_cliff` config for any position this module's chokepoint function
//! is applied to â€” `vesting_cliff::apply_cliff` is not called from here.
//!
//! # Wiring
//!
//! Like `vesting_cliff.rs`, this only defines the chokepoint the reward path
//! is meant to call (`apply_combined_vesting`) plus its config/admin surface
//! and read-only queries â€” mirroring the level of integration already
//! established by `vesting_cliff.rs`/`price_oracle.rs`, since no `claim()`
//! entrypoint that would call either chokepoint exists yet in this crate (see
//! this PR's description for the fuller picture).
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `vesting_cliff.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::storage::DataKey;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance-storage key for the combined vesting config.
const CONFIG_KEY: Symbol = symbol_short!("cmb_vst");

/// Persistent-storage key prefix recording that a user's cliff unlock under
/// the combined schedule has already been announced.
const CLIFF_EVENT_KEY: Symbol = symbol_short!("cmb_evt");

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CombinedVestingConfig {
    pub cliff_ledgers: u32,
    pub linear_period_ledgers: u32,
    pub cliff_set_at: u32,
}

/// The ledger at which `user` last staked. Read the same way
/// `vesting_cliff.rs` does â€” `balance.rs` exposes no accessor for it.
fn staked_at_ledger(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get::<_, u32>(&DataKey::StakedAtLedger(user.clone()))
}

/// The current combined vesting config, if one has been set.
pub fn get_config(env: &Env) -> Option<CombinedVestingConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

/// The ledger at which `user`'s position clears the combined-schedule cliff.
fn cliff_unlock_ledger_for(
    config: &CombinedVestingConfig,
    staked_at: u32,
) -> u32 {
    staked_at.saturating_add(config.cliff_ledgers)
}

/// The ledger at which `user`'s position is 100% vested under the linear
/// leg â€” cliff unlock plus the linear period.
fn fully_vested_ledger_for(config: &CombinedVestingConfig, staked_at: u32) -> u32 {
    cliff_unlock_ledger_for(config, staked_at).saturating_add(config.linear_period_ledgers)
}

/// Split `raw_reward` into (vested, unvested) for `user` at the current
/// ledger, under `config`. Both figures always sum to `raw_reward`.
///
/// - Before the cliff: `(0, raw_reward)` â€” nothing claimable yet.
/// - At/after the cliff, before full vesting: linear interpolation of
///   `raw_reward` over the elapsed fraction of `linear_period_ledgers` since
///   the cliff cleared. `linear_period_ledgers == 0` means the cliff amount
///   unlocks in full immediately with no further drip.
/// - After the linear period ends: `(raw_reward, 0)` â€” fully vested.
fn split_vested(
    env: &Env,
    config: &CombinedVestingConfig,
    staked_at: u32,
    raw_reward: i128,
) -> (i128, i128) {
    let now = env.ledger().sequence();
    let cliff_unlock = cliff_unlock_ledger_for(config, staked_at);

    if now < cliff_unlock {
        return (0, raw_reward);
    }

    if config.linear_period_ledgers == 0 {
        return (raw_reward, 0);
    }

    let fully_vested = fully_vested_ledger_for(config, staked_at);
    if now >= fully_vested {
        return (raw_reward, 0);
    }

    let elapsed = (now - cliff_unlock) as i128;
    let period = config.linear_period_ledgers as i128;
    // raw_reward * elapsed / period â€” checked, since a hostile-sized reward
    // times a large elapsed count could in principle overflow i128 before
    // the division brings it back down.
    let vested = raw_reward
        .checked_mul(elapsed)
        .map(|v| v / period)
        .unwrap_or(0)
        .min(raw_reward)
        .max(0);
    let unvested = raw_reward.saturating_sub(vested);
    (vested, unvested)
}

/// The chokepoint the reward path is meant to call: given `user`'s raw
/// accrued reward, returns the claimable portion under the combined
/// cliff-then-linear schedule. Returns `raw_reward` unchanged (fully
/// claimable) when no combined config is set â€” same "off by default"
/// contract `vesting_cliff::apply_cliff` makes, so introducing this feature
/// never silently freezes an existing pool's rewards.
pub fn apply_combined_vesting(env: &Env, user: &Address, raw_reward: i128) -> i128 {
    let config = match get_config(env) {
        Some(c) => c,
        None => return raw_reward,
    };
    let staked_at = match staked_at_ledger(env, user) {
        Some(s) => s,
        // No recorded stake: nothing to gate, matching vesting_cliff's
        // "unstaked address is never reported as locked" rule.
        None => return raw_reward,
    };

    let (vested, _unvested) = split_vested(env, &config, staked_at, raw_reward);
    vested
}

/// Emit `cliff_reached` the first time `user` interacts after clearing the
/// combined schedule's cliff, then remember that it fired. Mirrors
/// `vesting_cliff::maybe_emit_cliff_unlocked`'s lazy-emission approach.
pub fn maybe_emit_cliff_reached(env: &Env, user: &Address) {
    let config = match get_config(env) {
        Some(c) => c,
        None => return,
    };
    let staked_at = match staked_at_ledger(env, user) {
        Some(s) => s,
        None => return,
    };

    let cliff_unlock = cliff_unlock_ledger_for(&config, staked_at);
    if env.ledger().sequence() < cliff_unlock {
        return;
    }

    let key = (CLIFF_EVENT_KEY, user.clone());
    if env.storage().persistent().has(&key) {
        return;
    }
    env.storage().persistent().set(&key, &true);

    let raw_reward = crate::balance::get_accrued_reward(env, user);
    let (cliff_amount_unlocked, _) = split_vested(
        env,
        &config,
        staked_at,
        raw_reward,
    );

    env.events().publish(
        (symbol_short!("cliff_rch"), user.clone()),
        (cliff_amount_unlocked, env.ledger().sequence()),
    );
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Set the combined cliff-then-linear vesting schedule. Admin only.
    ///
    /// Overrides the standalone `vesting_cliff` config for any code path
    /// that calls `apply_combined_vesting` instead of `vesting_cliff::apply_cliff`.
    pub fn set_combined_vesting(
        env: Env,
        cliff_ledgers: u32,
        linear_period_ledgers: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let config = CombinedVestingConfig {
            cliff_ledgers,
            linear_period_ledgers,
            cliff_set_at: env.ledger().sequence(),
        };
        env.storage().instance().set(&CONFIG_KEY, &config);

        let admin = admin::get_admin(&env)?;
        env.events().publish(
            (symbol_short!("cmb_vst_s"), admin),
            (cliff_ledgers, linear_period_ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// The current combined vesting config, if one has been set.
    pub fn get_combined_vesting_config(env: Env) -> Option<CombinedVestingConfig> {
        get_config(&env)
    }

    /// The claimable portion of `user`'s current raw accrued reward under
    /// the combined schedule, at the current ledger.
    pub fn get_vested_amount(env: Env, user: Address) -> i128 {
        let raw_reward = crate::balance::get_accrued_reward(&env, &user);
        apply_combined_vesting(&env, &user, raw_reward)
    }

    /// The still-locked portion of `user`'s current raw accrued reward under
    /// the combined schedule, at the current ledger.
    pub fn get_unvested_amount(env: Env, user: Address) -> i128 {
        let raw_reward = crate::balance::get_accrued_reward(&env, &user);
        let vested = apply_combined_vesting(&env, &user, raw_reward);
        raw_reward.saturating_sub(vested)
    }
}















