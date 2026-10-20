//! Maximum pause duration with a permissionless forced unpause (issue #533).
//!
//! Without a bound, an admin (or a compromised admin key) can pause the vault
//! and leave user funds frozen indefinitely. The admin can configure a
//! maximum pause duration in ledgers; once a pause has lasted that long, the
//! pause can no longer be held by the admin alone — anyone may call
//! `force_unpause()` to lift it.
//!
//! After a forced unpause the admin cannot re-pause for another
//! `max_pause_ledgers`, so the admin cannot simply re-pause in a loop to keep
//! funds frozen. Keeping the vault paused beyond that point requires broader
//! governance action (e.g. an admin change through governance).
//!
//! The duration cannot be changed while the vault is paused, so an in-flight
//! pause can never have its deadline extended.
//!
//! # Storage
//!
//! Raw `Symbol`-keyed instance storage, matching `balance.rs`.
//!
//! - `symbol_short!("max_pse")` -> `u32` (0 = no limit, the default)
//! - `symbol_short!("fpse_cd")` -> `u32` ledger until which `pause()` is blocked

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultFeature4Error;
use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

const MAX_PAUSE_KEY: Symbol = symbol_short!("max_pse");
const REPAUSE_COOLDOWN_KEY: Symbol = symbol_short!("fpse_cd");

/// Smallest non-zero maximum pause duration accepted (one day), so the limit
/// can't be set so tight that legitimate incident response is impossible.
pub const MIN_MAX_PAUSE_LEDGERS: u32 = LEDGERS_PER_DAY;

/// Configured maximum pause duration in ledgers (`0` = unlimited).
pub fn read_max_pause_ledgers(env: &Env) -> u32 {
    env.storage().instance().get(&MAX_PAUSE_KEY).unwrap_or(0)
}

fn is_paused_raw(env: &Env) -> bool {
    balance::apply_scheduled_unpause_if_due(env);
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

/// Ledger at which the current pause becomes force-unpausable, if the vault
/// is paused and a maximum duration is configured.
pub fn pause_deadline(env: &Env) -> Option<u32> {
    let max = read_max_pause_ledgers(env);
    if max == 0 || !is_paused_raw(env) {
        return None;
    }
    let paused_at = balance::get_pause_info(env)
        .map(|info| info.paused_at)
        .unwrap_or(0);
    Some(paused_at.saturating_add(max))
}

/// True while the admin is blocked from re-pausing after a forced unpause.
/// Called from `pause()` / `pause_until()`.
pub(crate) fn repause_blocked(env: &Env) -> bool {
    let until: u32 = env
        .storage()
        .instance()
        .get(&REPAUSE_COOLDOWN_KEY)
        .unwrap_or(0);
    env.ledger().sequence() < until
}

#[contractimpl]
impl VaultContract {
    /// Admin: configure the maximum number of ledgers the vault may remain
    /// paused before anyone can call `force_unpause()`. `0` disables the
    /// limit. Reverts with `InvalidPauseDuration` while the vault is paused
    /// or when a non-zero value is below `MIN_MAX_PAUSE_LEDGERS`.
    pub fn set_max_pause_duration(env: Env, ledgers: u32) -> Result<(), VaultFeature4Error> {
        admin::require_admin(&env)?;
        if is_paused_raw(&env) {
            return Err(VaultFeature4Error::InvalidPauseDuration);
        }
        if ledgers != 0 && ledgers < MIN_MAX_PAUSE_LEDGERS {
            return Err(VaultFeature4Error::InvalidPauseDuration);
        }
        let old = read_max_pause_ledgers(&env);
        env.storage().instance().set(&MAX_PAUSE_KEY, &ledgers);
        env.events()
            .publish((symbol_short!("max_pse"),), (old, ledgers));
        Ok(())
    }

    /// Read-only: the configured maximum pause duration (`0` = unlimited).
    pub fn get_max_pause_duration(env: Env) -> u32 {
        read_max_pause_ledgers(&env)
    }

    /// Read-only: the ledger at which the current pause can be forcibly
    /// lifted, or `None` when not paused or no limit is configured.
    pub fn get_pause_deadline(env: Env) -> Option<u32> {
        pause_deadline(&env)
    }

    /// Permissionless: lift a pause that has exceeded the configured maximum
    /// duration. Blocks the admin from re-pausing for another
    /// `max_pause_ledgers`.
    pub fn force_unpause(env: Env, caller: Address) -> Result<(), VaultFeature4Error> {
        caller.require_auth();
        if !is_paused_raw(&env) {
            return Err(VaultFeature4Error::NotPaused);
        }
        let deadline = pause_deadline(&env).ok_or(VaultFeature4Error::GracePeriodNotElapsed)?;
        let now = env.ledger().sequence();
        if now < deadline {
            return Err(VaultFeature4Error::GracePeriodNotElapsed);
        }

        env.storage().instance().set(&DataKey::Paused, &false);
        balance::clear_pause_info(&env);
        balance::clear_scheduled_unpause(&env);

        let cooldown_until = now.saturating_add(read_max_pause_ledgers(&env));
        env.storage()
            .instance()
            .set(&REPAUSE_COOLDOWN_KEY, &cooldown_until);
        balance::set_last_updated_ledger(&env, now);

        env.events()
            .publish((symbol_short!("frc_unps"),), (caller, now, cooldown_until));
        Ok(())
    }
}
