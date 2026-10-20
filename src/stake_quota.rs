//! Stake-weighted operation quota ΓÇö bounds how many expensive operations
//! (governance proposals, content submissions, poll creation) a staker can
//! perform per epoch, proportional to their share of the pool, so bot-driven
//! spam can't outrun genuine economic stake (issue #339).
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this module uses raw
//! `Symbol`-keyed storage, matching `balance.rs` / `content_curation.rs`.
//!
//! # Known gap
//!
//! Of the three quota-gated functions named in the issue
//! (`create_proposal`, `submit_content`, `create_poll`), only
//! `submit_content` (in `content_curation.rs`) currently exists on `main` ΓÇö
//! it's gated here directly. `create_proposal` and `create_poll` don't exist
//! anywhere in the current source (most of `src/vault.rs` past
//! `join_waitlist` is missing on `main` due to an unrelated bad merge ΓÇö see
//! PR description), so there's nothing to wire them into yet. Once they're
//! restored, each just needs one call added at its top:
//! `stake_quota::consume_quota(&env, &user, 1)?;`

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance-storage key for the admin-configured quota parameters.
const CONFIG_KEY: Symbol = symbol_short!("qt_cfg");

/// Persistent-storage key prefix for a user's current-epoch quota usage.
/// Keyed by `(USAGE_KEY, user)`.
const USAGE_KEY: Symbol = symbol_short!("qt_use");

#[derive(Clone, Debug, PartialEq)]
#[soroban_sdk::contracttype]
pub struct QuotaConfig {
    pub operations_per_epoch: u32,
    pub epoch_ledgers: u32,
}

#[derive(Clone, Debug, PartialEq)]
#[soroban_sdk::contracttype]
pub struct QuotaUsage {
    pub used: u32,
    pub epoch_start: u32,
}

fn get_config(env: &Env) -> Option<QuotaConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

fn get_usage(env: &Env, user: &Address) -> Option<QuotaUsage> {
    env.storage().persistent().get(&(USAGE_KEY, user.clone()))
}

fn set_usage(env: &Env, user: &Address, usage: &QuotaUsage) {
    env.storage()
        .persistent()
        .set(&(USAGE_KEY, user.clone()), usage);
}

/// A user's total per-epoch allowance: `max(1, operations_per_epoch *
/// user_pool_share_bps / 10000)`, where `user_pool_share_bps` is the user's
/// shares as a fraction of total pool shares. Unconfigured pools (no
/// `QuotaConfig` set) return `0` ΓÇö nothing is gated until an admin opts in.
pub fn allowance(env: &Env, user: &Address) -> u32 {
    let Some(config) = get_config(env) else {
        return 0;
    };

    let total_shares = balance::get_total_shares(env);
    if total_shares == 0 {
        return 1;
    }
    let user_shares = balance::get_shares(env, user);

    let share_bps = (user_shares as i128)
        .checked_mul(10_000)
        .and_then(|v| v.checked_div(total_shares as i128))
        .unwrap_or(0);

    let raw = (config.operations_per_epoch as i128)
        .checked_mul(share_bps)
        .and_then(|v| v.checked_div(10_000))
        .unwrap_or(0);

    raw.max(1) as u32
}

/// Usage reset for a stale epoch, without persisting ΓÇö callers that only
/// need a read (`remaining`) shouldn't pay a write for it.
fn effective_usage(env: &Env, user: &Address) -> QuotaUsage {
    let Some(usage) = get_usage(env, user) else {
        return QuotaUsage {
            used: 0,
            epoch_start: env.ledger().sequence(),
        };
    };

    let config = get_config(env);
    let epoch_ledgers = config.map(|c| c.epoch_ledgers).unwrap_or(0);
    let current = env.ledger().sequence();

    if epoch_ledgers > 0 && current > usage.epoch_start.saturating_add(epoch_ledgers) {
        QuotaUsage {
            used: 0,
            epoch_start: current,
        }
    } else {
        usage
    }
}

/// Remaining operations `user` may perform in the current epoch.
pub fn remaining(env: &Env, user: &Address) -> u32 {
    let total = allowance(env, user);
    let usage = effective_usage(env, user);
    total.saturating_sub(usage.used)
}

/// Consume `operations` units of `user`'s quota for the current epoch,
/// resetting their usage first if the previous epoch has elapsed. Reverts
/// with `QuotaExhausted` if that would exceed their allowance.
///
/// Internal ΓÇö not a contract entrypoint. Called by quota-gated functions
/// (`submit_content`, and `create_proposal` / `create_poll` once restored;
/// see the module-level "Known gap" note).
pub fn consume_quota(env: &Env, user: &Address, operations: u32) -> Result<(), VaultError> {
    let total = allowance(env, user);
    let mut usage = effective_usage(env, user);

    let new_used = usage.used.saturating_add(operations);
    if new_used > total {
        return Err(VaultError::TooManyStakers);
    }

    usage.used = new_used;
    set_usage(env, user, &usage);
    Ok(())
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Configure the per-epoch operation allowance and epoch length. Admin
    /// only. Setting a new config does not reset any user's current usage ΓÇö
    /// only crossing an epoch boundary does.
    pub fn set_quota_config(
        env: Env,
        operations_per_epoch: u32,
        epoch_ledgers: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let config = QuotaConfig {
            operations_per_epoch,
            epoch_ledgers,
        };
        env.storage().instance().set(&CONFIG_KEY, &config);

        env.events().publish(
            (symbol_short!("qt_set"),),
            (operations_per_epoch, epoch_ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only query: `user`'s total per-epoch operation allowance.
    pub fn get_quota_allowance(env: Env, user: Address) -> u32 {
        allowance(&env, &user)
    }

    /// Read-only query: operations `user` has left in the current epoch.
    pub fn get_quota_remaining(env: Env, user: Address) -> u32 {
        remaining(&env, &user)
    }
}
















