//! Position heartbeat (issue #414).
//!
//! Boost multipliers are meant to reward actively-engaged stakers, not
//! passive holders who staked once and never came back. This module adds an
//! opt-in engagement requirement: a staker must send a periodic heartbeat
//! (or take an action that counts as one) or their boost multiplier is
//! suspended until they check back in. The base reward rate is never
//! suspended â€” only the boost portion.
//!
//! # Wiring
//!
//! Like `epoch_reward_cap.rs` and `anti_dump_claim_cooldown.rs`, this
//! exposes its own opt-in entrypoints (`stake_with_heartbeat`,
//! `claim_with_heartbeat`) rather than editing `vault.rs`'s existing
//! `stake()` / `claim()`, keeping the requirement additive. Boost strength
//! is read from the existing `balance::get_boost_schedule()` tiers (the
//! highest configured tier is the multiplier a heartbeat-current staker is
//! entitled to); with no schedule configured, or while suspended, the
//! multiplier is `BOOST_BPS_BASE` (1x, i.e. no boost).
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw
//! `Symbol`-keyed storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::VaultContract;
use crate::vault::{ BOOST_BPS_BASE};
use crate::vault::VaultContractClient;

/// Instance key: max ledgers a staker may go silent before their boost is
/// suspended. `0` disables the heartbeat requirement entirely.
const INTERVAL_KEY: Symbol = symbol_short!("hb_max");
/// Persistent key prefix: `(HEARTBEAT_KEY, user) -> HeartbeatLog`.
const HEARTBEAT_KEY: Symbol = symbol_short!("hb_log");
/// Persistent key prefix: `(SUSPENDED_KEY, user) -> bool`, tracks whether a
/// `boost_suspended` event has already been emitted for the user's current
/// silence period so it isn't re-emitted on every read.
const SUSPENDED_KEY: Symbol = symbol_short!("hb_susp");

/// Last recorded heartbeat for a staker (issue #414).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HeartbeatLog {
    pub last_heartbeat_at: u32,
}

fn get_interval(env: &Env) -> u32 {
    env.storage().instance().get(&INTERVAL_KEY).unwrap_or(0)
}

fn set_interval(env: &Env, max_silence_ledgers: u32) {
    env.storage().instance().set(&INTERVAL_KEY, &max_silence_ledgers);
}

fn get_log(env: &Env, user: &Address) -> Option<HeartbeatLog> {
    env.storage().persistent().get(&(HEARTBEAT_KEY, user.clone()))
}

fn set_log(env: &Env, user: &Address, log: &HeartbeatLog) {
    env.storage().persistent().set(&(HEARTBEAT_KEY, user.clone()), log);
}

fn is_suspended_flag(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&(SUSPENDED_KEY, user.clone()))
        .unwrap_or(false)
}

fn set_suspended_flag(env: &Env, user: &Address, suspended: bool) {
    env.storage()
        .persistent()
        .set(&(SUSPENDED_KEY, user.clone()), &suspended);
}

/// Whether `user`'s boost is currently considered active: the heartbeat
/// requirement is disabled, or the user checked in within `max_silence_ledgers`.
/// A user who has never sent a heartbeat is not current once the requirement
/// is enabled.
fn is_current(env: &Env, user: &Address) -> bool {
    let interval = get_interval(env);
    if interval == 0 {
        return true;
    }
    match get_log(env, user) {
        Some(log) => env.ledger().sequence().saturating_sub(log.last_heartbeat_at) <= interval,
        None => false,
    }
}

/// Records a heartbeat for `user` at the current ledger. If the user was
/// previously flagged as suspended, clears the flag and emits `boost_restored`.
fn record_heartbeat(env: &Env, user: &Address) {
    let now = env.ledger().sequence();
    set_log(env, user, &HeartbeatLog { last_heartbeat_at: now });

    if is_suspended_flag(env, user) {
        set_suspended_flag(env, user, false);
        env.events()
            .publish((symbol_short!("hb_rest"), user.clone()), (now,));
    }
}

/// Lazily flags `user` as suspended (emitting `boost_suspended` the first
/// time it's noticed) and returns whether the boost is currently suspended.
fn check_and_flag_suspended(env: &Env, user: &Address) -> bool {
    if is_current(env, user) {
        return false;
    }

    if !is_suspended_flag(env, user) {
        set_suspended_flag(env, user, true);
        let last = get_log(env, user).map(|l| l.last_heartbeat_at).unwrap_or(0);
        let silent_for = env.ledger().sequence().saturating_sub(last);
        env.events().publish(
            (symbol_short!("hb_susp"), user.clone()),
            (silent_for, env.ledger().sequence()),
        );
    }
    true
}

/// The boost multiplier (bps) a heartbeat-current staker is entitled to:
/// the highest tier in `balance::get_boost_schedule()`, or `BOOST_BPS_BASE`
/// (no boost) if no schedule is configured.
fn top_boost_bps(env: &Env) -> u32 {
    match balance::get_boost_schedule(env) {
        Some(tiers) if !tiers.is_empty() => {
            let mut best = BOOST_BPS_BASE;
            for (_, bps) in tiers.iter() {
                if bps > best {
                    best = bps;
                }
            }
            best
        }
        _ => BOOST_BPS_BASE,
    }
}

/// The reward `base` amount scaled by the boost multiplier if `user`'s
/// heartbeat is current, or left at the unboosted base rate otherwise.
fn apply_heartbeat_boost(env: &Env, user: &Address, base: i128) -> i128 {
    if check_and_flag_suspended(env, user) {
        return base;
    }
    let bps = top_boost_bps(env) as i128;
    base.saturating_mul(bps) / (BOOST_BPS_BASE as i128)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Sets the maximum number of ledgers a staker may go silent before
    /// their boost multiplier is suspended. Admin only. `0` disables the
    /// heartbeat requirement (boosts are never suspended).
    pub fn set_heartbeat_interval(env: Env, max_silence_ledgers: u32) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        crate::position_heartbeat::set_interval(&env, max_silence_ledgers);
        env.events().publish(
            (symbol_short!("hb_set"),),
            (max_silence_ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// The configured heartbeat interval, in ledgers. `0` means disabled.
    pub fn get_heartbeat_interval(env: Env) -> u32 {
        crate::position_heartbeat::get_interval(&env)
    }

    /// Sends a heartbeat for the caller, resetting their silence timer and
    /// restoring a suspended boost.
    pub fn send_heartbeat(env: Env, user: Address) -> Result<(), VaultError> {
        user.require_auth();
        crate::position_heartbeat::record_heartbeat(&env, &user);
        Ok(())
    }

    /// Whether `user`'s boost is currently active (heartbeat requirement
    /// disabled, or the user checked in recently enough).
    pub fn is_heartbeat_current(env: Env, user: Address) -> bool {
        crate::position_heartbeat::is_current(&env, &user)
    }

    /// The ledger of `user`'s last recorded heartbeat, `0` if none was ever recorded.
    pub fn get_last_heartbeat(env: Env, user: Address) -> u32 {
        crate::position_heartbeat::get_log(&env, &user)
            .map(|l| l.last_heartbeat_at)
            .unwrap_or(0)
    }

    /// Stakes `amount` for `user` (same behavior as `stake()`) and counts
    /// the call as a heartbeat.
    pub fn stake_with_heartbeat(env: Env, user: Address, amount: i128) -> Result<i128, VaultError> {
        let minted = Self::stake(env.clone(), user.clone(), amount)?;
        crate::position_heartbeat::record_heartbeat(&env, &user);
        Ok(minted)
    }

    /// Claims accrued rewards, counting the call as a heartbeat. If the
    /// caller's heartbeat is current the payout is boosted per
    /// `balance::get_boost_schedule()`; otherwise only the unboosted base
    /// amount is paid. Returns the amount transferred (0 if nothing accrued).
    pub fn claim_with_heartbeat(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let base = balance::get_accrued_reward(&env, &user);
        if base <= 0 {
            crate::position_heartbeat::record_heartbeat(&env, &user);
            return Ok(0);
        }

        // Evaluate the boost using the status as of *before* this call's
        // heartbeat is recorded below, so a claim that arrives after a long
        // silence is paid at the base rate for the period that just ended,
        // while still restoring the boost going forward.
        let payout = crate::position_heartbeat::apply_heartbeat_boost(&env, &user, base);
        crate::position_heartbeat::record_heartbeat(&env, &user);

        let pool_balance = balance::get_reward_pool_balance(&env);
        if pool_balance < payout {
            return Err(VaultError::InsufficientRewardPool);
        }
        balance::set_reward_pool_balance(&env, pool_balance - payout);
        balance::set_accrued_reward(&env, &user, 0);

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&crate::storage::DataKey::Token)
            .ok_or(VaultError::NotInitialized)?;
        soroban_sdk::token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &user,
            &payout,
        );
        events::claimed(&env, &user, payout, env.ledger().sequence());

        Ok(payout)
    }

    /// Read-only preview of what `claim_with_heartbeat` would pay out right
    /// now, without recording a heartbeat or mutating any state.
    pub fn get_pending_reward_with_heartbeat(env: Env, user: Address) -> i128 {
        let base = balance::get_accrued_reward(&env, &user);
        if base <= 0 {
            return 0;
        }
        if crate::position_heartbeat::is_current(&env, &user) {
            let bps = crate::position_heartbeat::top_boost_bps(&env) as i128;
            base.saturating_mul(bps) / (BOOST_BPS_BASE as i128)
        } else {
            base
        }
    }
}
















