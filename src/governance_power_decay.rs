//! Governance voting power decay for long-inactive participants (issue #404).
//!
//! Builds on issue #160 (governance voting) and the `GovernanceProposal` /
//! `vote_weight_at` groundwork already in `storage.rs` / `vault.rs`. A staker
//! whose position compounds for a long time but never actually votes
//! accumulates outsized voting power with no corresponding participation,
//! which is a centralization risk. This module decays the *effective* vote
//! weight reported for a user the longer they go without voting, without
//! touching their underlying staked position or rewards.
//!
//! # Wiring
//!
//! There is currently no callable `vote()` / `create_proposal()` entrypoint
//! on `main` to hook a "last voted" update into â€” `GovernanceProposal` and
//! `ProposableParam` exist as storage types (see `storage.rs`, `balance.rs`)
//! but nothing in this crate currently creates or votes on one (same gap
//! documented by `transfer_cooldown.rs` for `transfer_position`). So, matching
//! that module's approach, this exposes [`record_governance_vote`] as the one
//! call a restored `vote()` would need to add at the point a vote is cast,
//! and [`get_effective_vote_weight`] as the query any governance weight
//! lookup (a restored `vote()`, or the existing `vote_weight_at`/
//! `total_vote_weight`) would call instead of a user's raw position amount.
//! Both are directly callable and tested on their own in the meantime.
//!
//! # Epoch length
//!
//! The issue's `set_governance_decay_config` signature carries no epoch
//! length parameter, so "epoch" here uses the same day-length unit already
//! established for runway/reward calculations in `vault.rs`
//! (`LEDGERS_PER_DAY`) rather than introducing a second, decay-specific
//! epoch length to configure.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs` and `reputation_decay.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;
use crate::vault::{ BOOST_BPS_BASE, LEDGERS_PER_DAY};

/// Instance-storage key for the decay configuration.
const DECAY_CFG_KEY: Symbol = symbol_short!("gpd_cfg");

/// Persistent-storage key prefix for a user's last-voted ledger.
/// Keyed by `(LAST_VOTE_KEY, user)`.
const LAST_VOTE_KEY: Symbol = symbol_short!("gpd_lv");

/// Floor on effective vote weight as a fraction of the raw position amount,
/// in basis points: 20%, per the issue's decay formula.
const DECAY_FLOOR_BPS: u32 = 2_000;

/// Admin-configured governance decay parameters.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GovernanceDecayConfig {
    /// Number of inactivity epochs (days) a user may go without voting
    /// before decay starts being applied.
    pub inactivity_epochs: u32,
    /// Basis points of vote weight removed per epoch inactive beyond
    /// `inactivity_epochs`.
    pub decay_rate_bps: u32,
}

// â”€â”€ storage helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn get_config(env: &Env) -> Option<GovernanceDecayConfig> {
    env.storage().instance().get(&DECAY_CFG_KEY)
}

fn set_config(env: &Env, config: &GovernanceDecayConfig) {
    env.storage().instance().set(&DECAY_CFG_KEY, config);
}

fn get_last_vote_ledger(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&(LAST_VOTE_KEY, user.clone()))
}

fn set_last_vote_ledger(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&(LAST_VOTE_KEY, user.clone()), &ledger);
}

/// A user's raw governance weight: their currently staked token amount.
fn raw_weight(env: &Env, user: &Address) -> i128 {
    let shares = balance::get_shares(env, user);
    if shares == 0 {
        return 0;
    }
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    balance::shares_to_amount(total_shares, total_deposited, shares).unwrap_or(0)
}

/// The ledger to measure inactivity from: the user's last recorded vote, or
/// (if they have never voted) the ledger they first staked at, so a staker
/// who simply hasn't had a chance to vote yet isn't decayed unfairly.
fn inactivity_baseline(env: &Env, user: &Address) -> u32 {
    if let Some(last_vote) = get_last_vote_ledger(env, user) {
        return last_vote;
    }
    env.storage()
        .persistent()
        .get(&crate::storage::DataKey::StakedAtLedger(user.clone()))
        .unwrap_or(0)
}

/// Compute `(raw_weight, effective_weight, epochs_inactive)` for `user`.
///
/// `effective_weight` equals `raw_weight` unchanged when no decay config is
/// set, the position is empty, or the user is still within the configured
/// grace window. Otherwise: `raw * (10000 - decay_rate_bps * epochs_inactive)
/// / 10000`, floored at `DECAY_FLOOR_BPS` (20%) of `raw_weight`.
fn compute_effective_weight(env: &Env, user: &Address) -> (i128, i128, u32) {
    let raw = raw_weight(env, user);
    if raw == 0 {
        return (0, 0, 0);
    }

    let config = match get_config(env) {
        Some(c) if c.decay_rate_bps > 0 => c,
        _ => return (raw, raw, 0),
    };

    let baseline = inactivity_baseline(env, user);
    if baseline == 0 {
        return (raw, raw, 0);
    }

    let current = env.ledger().sequence();
    if current <= baseline {
        return (raw, raw, 0);
    }

    let elapsed_ledgers = current.saturating_sub(baseline);
    let epochs_elapsed = elapsed_ledgers / LEDGERS_PER_DAY;
    if epochs_elapsed <= config.inactivity_epochs {
        return (raw, raw, 0);
    }

    let epochs_inactive = epochs_elapsed - config.inactivity_epochs;
    let decay_bps = (config.decay_rate_bps as u64)
        .saturating_mul(epochs_inactive as u64)
        .min(BOOST_BPS_BASE as u64) as u32;

    let reduction = (raw as i128).saturating_mul(decay_bps as i128) / (BOOST_BPS_BASE as i128);
    let decayed = raw.saturating_sub(reduction);
    let floor = raw.saturating_mul(DECAY_FLOOR_BPS as i128) / (BOOST_BPS_BASE as i128);
    let effective = decayed.max(floor);

    (raw, effective, epochs_inactive)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Configure governance vote-weight decay. Admin only.
    ///
    /// `inactivity_epochs`: epochs (days) of grace before decay starts.
    /// `decay_rate_bps`: basis points removed per epoch inactive beyond that
    /// grace window (max 10 000).
    pub fn set_governance_decay_config(
        env: Env,
        inactivity_epochs: u32,
        decay_rate_bps: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if decay_rate_bps > BOOST_BPS_BASE {
            return Err(VaultError::InvalidRate);
        }

        let config = GovernanceDecayConfig {
            inactivity_epochs,
            decay_rate_bps,
        };
        set_config(&env, &config);

        env.events().publish(
            (symbol_short!("gpd_set"),),
            (inactivity_epochs, decay_rate_bps, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only query: the current governance decay configuration, or
    /// `None` if never configured.
    pub fn get_governance_decay_config(env: Env) -> Option<GovernanceDecayConfig> {
        get_config(&env)
    }

    /// A user's effective governance vote weight with inactivity decay
    /// applied. Public â€” no auth required. Emits `governance_decay_applied`
    /// when decay actually reduces the weight below the raw position amount.
    pub fn get_effective_vote_weight(env: Env, user: Address) -> i128 {
        let (raw, effective, epochs_inactive) = compute_effective_weight(&env, &user);

        if effective < raw {
            env.events().publish(
                (symbol_short!("gpd_dcy"), user),
                (raw, effective, epochs_inactive, env.ledger().sequence()),
            );
        }

        effective
    }

    /// Record that `user` just participated in governance, resetting their
    /// decay clock. Requires the user's own auth.
    ///
    /// Intended to be called from `vote()` once it is restored â€” see the
    /// module-level "Wiring" note. Callable and testable directly until then.
    pub fn record_governance_vote(env: Env, user: Address) -> Result<(), VaultError> {
        user.require_auth();

        if balance::get_shares(&env, &user) == 0 {
            return Err(VaultError::PositionNotFound);
        }

        set_last_vote_ledger(&env, &user, env.ledger().sequence());
        Ok(())
    }

    /// Read-only query: the ledger `user` last voted (or recorded a vote) at,
    /// if any.
    pub fn get_last_vote_ledger(env: Env, user: Address) -> Option<u32> {
        get_last_vote_ledger(&env, &user)
    }
}









