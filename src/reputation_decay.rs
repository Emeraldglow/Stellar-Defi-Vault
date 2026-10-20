//! Reputation score time-decay mechanism.
//!
//! Reputation scores gradually decrease when a staker is inactive (no claims,
//! no stakes, no unstakes) for an extended period. The decay rate and epoch
//! length are admin-configurable. Decay applies to `total_score` only â€” not
//! individual sub-components.
//!
//! # Storage
//!
//! Uses raw `Symbol`-keyed instance and persistent storage, matching
//! `balance.rs` and `commitment.rs`, since `DataKey` is at Soroban's
//! 50-variant cap.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::storage::ReputationScore;
use crate::VaultContract;
use crate::vault::VaultContractClient;
use crate::vault::{ BOOST_BPS_BASE};

/// Instance-storage key for the decay configuration.
const DECAY_CFG_KEY: Symbol = symbol_short!("rd_cfg");

/// Persistent-storage key for per-user last-activity ledger.
/// Keyed by `(RD_ACTIVITY_KEY, user)`.
const RD_ACTIVITY_KEY: Symbol = symbol_short!("rd_act");

/// Admin-configured decay parameters.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DecayConfig {
    /// Basis points subtracted from `total_score` per elapsed epoch.
    pub decay_bps_per_epoch: u32,
    /// Number of ledgers in one epoch.
    pub epoch_ledgers: u32,
}

// â”€â”€ storage helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn get_decay_config(env: &Env) -> Option<DecayConfig> {
    env.storage().instance().get(&DECAY_CFG_KEY)
}

fn set_decay_config(env: &Env, config: &DecayConfig) {
    env.storage().instance().set(&DECAY_CFG_KEY, config);
}

fn get_last_activity(env: &Env, user: &Address) -> u32 {
    let stored: u32 = env
        .storage()
        .persistent()
        .get(&(RD_ACTIVITY_KEY, user.clone()))
        .unwrap_or(0);
    if stored > 0 {
        return stored;
    }
    // Fall back to the most recent of existing activity markers.
    let staked_at: u32 = env
        .storage()
        .persistent()
        .get(&crate::storage::DataKey::StakedAtLedger(user.clone()))
        .unwrap_or(0);
    let last_claim = balance::get_last_claim_ledger(env, user);
    let last_unstake = balance::get_last_unstake_ledger(env, user).unwrap_or(0);
    staked_at.max(last_claim).max(last_unstake)
}

pub fn set_last_activity(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&(RD_ACTIVITY_KEY, user.clone()), &ledger);
}

/// Apply time-based decay to a user's `total_score`.
///
/// Returns `(decayed_score, old_score, epochs_elapsed)`. If no decay config is
/// set, decay rate is zero, or no epochs have elapsed, the score is returned
/// unchanged.
pub fn compute_decayed_score(
    env: &Env,
    user: &Address,
    score: &ReputationScore,
) -> (u32, u32, u32) {
    let config = match get_decay_config(env) {
        Some(c) if c.decay_bps_per_epoch > 0 && c.epoch_ledgers > 0 => c,
        _ => return (score.total_score, score.total_score, 0),
    };

    let last_active = get_last_activity(env, user);
    if last_active == 0 {
        return (score.total_score, score.total_score, 0);
    }

    let current = env.ledger().sequence();
    if current <= last_active {
        return (score.total_score, score.total_score, 0);
    }

    let elapsed_ledgers = current.saturating_sub(last_active);
    let epochs_elapsed = elapsed_ledgers / config.epoch_ledgers;
    if epochs_elapsed == 0 {
        return (score.total_score, score.total_score, 0);
    }

    let decay = (config.decay_bps_per_epoch as u64)
        .saturating_mul(epochs_elapsed as u64)
        .min(BOOST_BPS_BASE as u64) as u32;
    let decayed = score.total_score.saturating_sub(decay);
    (decayed, score.total_score, epochs_elapsed)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Minimal base reputation score computed from a user's current position,
    /// before decay is applied. Reduced to `total_score` only â€” no per-sub-score
    /// business logic (that lives on the full reputation feature, not this
    /// decay-only module) â€” so every sub-score is folded into `total_score`
    /// directly, capped at `BOOST_BPS_BASE` (10 000).
    fn get_reputation_score_raw(env: &Env, user: &Address) -> ReputationScore {
        let shares = balance::get_shares(env, user);
        if shares == 0 {
            return ReputationScore {
                duration_score: 0,
                consistency_score: 0,
                size_score: 0,
                streak_score: 0,
                total_score: 0,
            };
        }

        let staked_at: u32 = env
            .storage()
            .persistent()
            .get(&crate::storage::DataKey::StakedAtLedger(user.clone()))
            .unwrap_or(0);
        let current = env.ledger().sequence();
        let tenure_ledgers = current.saturating_sub(staked_at);

        // Score linearly with tenure, capped at the max â€” a full year staked
        // reaches the cap.
        let total_score = ((tenure_ledgers as u64 * BOOST_BPS_BASE as u64)
            / crate::vault::STELLAR_LEDGERS_PER_YEAR as u64)
            .min(BOOST_BPS_BASE as u64) as u32;

        ReputationScore {
            duration_score: total_score,
            consistency_score: 0,
            size_score: 0,
            streak_score: 0,
            total_score,
        }
    }

    /// Configure the reputation decay rate. Admin only.
    ///
    /// `decay_bps_per_epoch`: basis points subtracted from `total_score` per
    ///   elapsed epoch of inactivity (max 10 000).
    /// `epoch_ledgers`: number of ledgers in one epoch (must be > 0).
    pub fn set_reputation_decay_rate(
        env: Env,
        decay_bps_per_epoch: u32,
        epoch_ledgers: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if decay_bps_per_epoch > BOOST_BPS_BASE {
            return Err(VaultError::InvalidRate);
        }
        if epoch_ledgers == 0 {
            return Err(VaultError::InvalidRate);
        }

        let config = DecayConfig {
            decay_bps_per_epoch,
            epoch_ledgers,
        };
        set_decay_config(&env, &config);

        let admin = admin::get_admin(&env)?;
        env.events().publish(
            (symbol_short!("rd_set"), admin),
            (decay_bps_per_epoch, epoch_ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only query: return the current decay configuration, or `None` if
    /// never configured.
    pub fn get_reputation_decay_config(env: Env) -> Option<DecayConfig> {
        get_decay_config(&env)
    }

    /// Recalculate and return the user's reputation score with time decay
    /// applied. The affected user must authorize the checkpoint update.
    /// applied. Public â€” no auth required.
    ///
    /// If the user has no active position, returns all zeros. Otherwise computes
    /// the base score, applies decay based on elapsed epochs since last activity,
    /// and emits a `reputation_decayed` event if the score decreased.
    pub fn apply_reputation_decay(env: Env, user: Address) -> ReputationScore {
        user.require_auth();
        let base = Self::get_reputation_score_raw(&env, &user);
        let (decayed, old_score, epochs_elapsed) =
            compute_decayed_score(&env, &user, &base);

        if decayed < old_score {
            set_last_activity(&env, &user, env.ledger().sequence());
            env.events().publish(
                (symbol_short!("rd_decay"), user.clone()),
                (old_score, decayed, epochs_elapsed, env.ledger().sequence()),
            );
        }

        ReputationScore {
            total_score: decayed,
            ..base
        }
    }
}









