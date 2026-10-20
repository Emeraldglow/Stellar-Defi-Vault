//! Vault extensions for issues #546, #547, #548, #549.
//!
//! - Issue #547: explicit decimals/precision documentation via `get_precision_info()`.
//! - Issue #548: configurable cool-off period after large deposits (`set_large_deposit_threshold`,
//!   per-deposit locks, FIFO unlocked accounting, `get_locked_shares`).
//! - Issue #549: configurable minimum reward-pool funding increment (`set_min_reward_funding`,
//!   `fund_reward_pool` validation, `get_min_reward_funding`).
//! - Issue #546: configurable per-user maximum position count (`set_max_positions_per_user`,
//!   `get_position_count`, guard for position-creating functions).
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap (see `storage.rs`), so everything
//! here uses raw `Symbol`-keyed storage, matching `balance.rs` and the other
//! feature modules.

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::{VaultError, VaultOpsError};
use crate::vault::VaultContract;

// ── Issue #547: precision info ──────────────────────────────────────────────

/// Instance key for the internal share-precision assumption. Written once at
/// `initialize` (equal to the resolved token decimals, since the first
/// deposit mints 1:1) so `get_precision_info` never guesses.
const SHARE_DECIMALS_KEY: Symbol = symbol_short!("shr_dec");

pub fn get_share_decimals(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&SHARE_DECIMALS_KEY)
        .unwrap_or_else(|| balance::get_stake_decimals(env))
}

pub fn set_share_decimals(env: &Env, decimals: u32) {
    env.storage().instance().set(&SHARE_DECIMALS_KEY, &decimals);
}

/// Resolve the deposit-token decimals at init time.
///
/// Preference order:
/// 1. Explicit `stake_decimals` argument when `Some`.
/// 2. The live deposit token's own `decimals()` when the token contract
///    answers (the "actual" configuration the issue asks for, not a hardcode).
/// 3. `DEFAULT_TOKEN_DECIMALS` fallback when the token is unreachable.
pub fn resolve_token_decimals(env: &Env, token_addr: &Address, explicit: Option<u32>) -> u32 {
    if let Some(d) = explicit {
        return d;
    }
    let client = token::Client::new(env, token_addr);
    match client.try_decimals() {
        Ok(Ok(d)) => d,
        _ => balance::DEFAULT_TOKEN_DECIMALS,
    }
}

// ── Issue #548: large-deposit cool-off ───────────────────────────────────────

/// Instance key for the deposit-size threshold (in token units) at or above
/// which a deposit becomes locked. `0` (default) disables the feature.
const LARGE_THRESHOLD_KEY: Symbol = symbol_short!("lg_thr");
/// Instance key for the lock duration in ledgers applied to qualifying deposits.
const LARGE_HOLDBACK_KEY: Symbol = symbol_short!("lg_hold");
/// Persistent key prefix for per-user locked share tranches: `(LG_TRANCHES_KEY, user)`.
const LG_TRANCHES_KEY: Symbol = symbol_short!("lg_tr");

/// One locked share tranche created by a single qualifying deposit.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LockedTranche {
    pub shares: i128,
    pub unlock_at: u32,
}

fn get_large_threshold(env: &Env) -> i128 {
    env.storage().instance().get(&LARGE_THRESHOLD_KEY).unwrap_or(0)
}

fn get_large_holdback(env: &Env) -> u32 {
    env.storage().instance().get(&LARGE_HOLDBACK_KEY).unwrap_or(0)
}

fn get_tranches(env: &Env, user: &Address) -> soroban_sdk::Vec<LockedTranche> {
    env.storage()
        .persistent()
        .get(&(LG_TRANCHES_KEY, user.clone()))
        .unwrap_or(soroban_sdk::Vec::new(env))
}

fn set_tranches(env: &Env, user: &Address, tranches: &soroban_sdk::Vec<LockedTranche>) {
    if tranches.is_empty() {
        env.storage()
            .persistent()
            .remove(&(LG_TRANCHES_KEY, user.clone()));
    } else {
        env.storage()
            .persistent()
            .set(&(LG_TRANCHES_KEY, user.clone()), tranches);
    }
}

/// Drop every tranche whose lock has expired. Keeps per-user storage bounded
/// and makes `get_locked_shares` cheap after the holdback passes.
fn prune_expired(env: &Env, user: &Address) {
    let now = env.ledger().sequence();
    let tranches = get_tranches(env, user);
    let mut live = soroban_sdk::Vec::new(env);
    for t in tranches.iter() {
        if t.unlock_at > now && t.shares > 0 {
            live.push_back(t);
        }
    }
    if live.len() != tranches.len() {
        set_tranches(env, user, &live);
    }
}

/// Sum of still-locked shares for `user` (prunes expired tranches first).
pub fn locked_shares_inner(env: &Env, user: &Address) -> i128 {
    prune_expired(env, user);
    let now = env.ledger().sequence();
    let tranches = get_tranches(env, user);
    let mut locked: i128 = 0;
    for t in tranches.iter() {
        if t.unlock_at > now {
            locked = locked.saturating_add(t.shares);
        }
    }
    // Locked bookkeeping can never exceed the live balance (e.g. after fee
    // paths that burn shares); clamp so unlocked math never goes negative.
    locked.min(balance::get_shares(env, user).max(0))
}

/// Record a deposit of `deposit_amount` (token units) that minted
/// `shares_minted`. Qualifying deposits (at or above the threshold, while the
/// feature is enabled) create a per-deposit lock lasting `holdback_ledgers`.
///
/// Small deposits are unaffected: nothing is stored for them.
pub(crate) fn maybe_lock_deposit(
    env: &Env,
    user: &Address,
    deposit_amount: i128,
    shares_minted: i128,
) {
    let threshold = get_large_threshold(env);
    let holdback = get_large_holdback(env);
    if threshold <= 0 || holdback == 0 {
        return;
    }
    if deposit_amount < threshold || shares_minted <= 0 {
        return;
    }
    prune_expired(env, user);
    let mut tranches = get_tranches(env, user);
    tranches.push_back(LockedTranche {
        shares: shares_minted,
        unlock_at: env.ledger().sequence().saturating_add(holdback),
    });
    set_tranches(env, user, &tranches);
}

/// Revert unless `requested_shares` can be covered from unlocked tranches
/// (FIFO: locked shares are the most recent qualifying deposits and are
/// excluded first). No-op while the feature is disabled.
///
/// `VaultError` is at Soroban's 50-variant cap, so the lock revert reuses the
/// existing `UseCooldownFlow` cooldown variant rather than adding a dedicated
/// `SharesLocked` case.
pub(crate) fn enforce_unlocked(env: &Env, user: &Address, requested_shares: i128) -> Result<(), VaultError> {
    if get_large_threshold(env) <= 0 || get_large_holdback(env) == 0 {
        return Ok(());
    }
    if requested_shares <= 0 {
        return Ok(());
    }
    let locked = locked_shares_inner(env, user);
    let total = balance::get_shares(env, user);
    let unlocked = total.saturating_sub(locked);
    if requested_shares > unlocked {
        return Err(VaultError::UseCooldownFlow);
    }
    Ok(())
}

// ── Issue #549: minimum reward funding ───────────────────────────────────────

/// Instance key for the minimum accepted `fund_reward_pool` amount.
/// `0` (default) disables the check, preserving current behaviour.
const MIN_REWARD_FUNDING_KEY: Symbol = symbol_short!("mn_rwdf");

pub fn get_min_reward_funding_inner(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&MIN_REWARD_FUNDING_KEY)
        .unwrap_or(0)
}

pub fn set_min_reward_funding_inner(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&MIN_REWARD_FUNDING_KEY, &amount);
}

/// Revert with `FundingBelowMinimum` when the feature is enabled (`min > 0`)
/// and `amount` is below it. Called before any token movement so a rejected
/// dust funding leaves no state behind.
pub(crate) fn enforce_min_reward_funding(env: &Env, amount: i128) -> Result<(), VaultOpsError> {
    let min = get_min_reward_funding_inner(env);
    if min > 0 && amount < min {
        return Err(VaultOpsError::FundingBelowMinimum);
    }
    Ok(())
}

// ── Issue #546: per-user maximum position count ──────────────────────────────

/// Instance key for the admin-configured cap on distinct open positions per
/// user. `0` (default) means unlimited. Capped at 10 to bound per-user
/// enumeration gas costs.
const MAX_POSITIONS_KEY: Symbol = symbol_short!("mx_pos");

/// Hard ceiling for `set_max_positions_per_user` (mirrors the existing
/// `VaultError::MaxPositionsTooHigh` contract).
pub const MAX_POSITIONS_CEILING: u32 = 10;

pub fn get_max_positions_inner(env: &Env) -> u32 {
    env.storage().instance().get(&MAX_POSITIONS_KEY).unwrap_or(0)
}

/// Number of distinct open positions for `user`.
///
/// Today the contract tracks exactly one primary position plus any number of
/// additive split positions (`position_split`): the count is
/// `(primary ? 1 : 0) + split_positions.len()`. Closing a position (fully
/// unstaking the primary, or merging/removing splits) frees a slot
/// automatically since nothing is stored separately.
pub fn position_count_inner(env: &Env, user: &Address) -> u32 {
    let primary = if balance::get_shares(env, user) > 0 { 1 } else { 0 };
    let splits = balance::get_split_positions(env, user).len();
    primary.saturating_add(splits)
}

/// Revert with `MaxPositionsReached` when the cap is enabled and `user`
/// already holds that many positions. Position-creating functions
/// (`position_split`, and future `lock_position` / `tokenize_position`)
/// call this before creating a new slot.
pub(crate) fn ensure_position_slot(env: &Env, user: &Address) -> Result<(), VaultError> {
    let cap = get_max_positions_inner(env);
    if cap == 0 {
        return Ok(());
    }
    if position_count_inner(env, user) >= cap {
        return Err(VaultError::MaxPositionsReached);
    }
    Ok(())
}

#[contractimpl]
impl VaultContract {
    // ── Issue #547 ──────────────────────────────────────────────────────────

    /// Read-only precision documentation for integrators (issue #547).
    ///
    /// Returns `(token_decimals, share_decimals)`:
    /// - `token_decimals`: the deposit token's decimals, resolved at `initialize`
    ///   from the live token contract when no explicit value was passed.
    /// - `share_decimals`: the internal share-precision assumption (1:1 with the
    ///   token at first deposit, so equal to `token_decimals` unless a future
    ///   migration changes it).
    ///
    /// External consumers of `preview_redeem` and similar functions should use
    /// this instead of guessing at precision. No auth, no state changes.
    pub fn get_precision_info(env: Env) -> (u32, u32) {
        let token_decimals = balance::get_stake_decimals(&env);
        let share_decimals = get_share_decimals(&env);
        (token_decimals, share_decimals)
    }

    // ── Issue #548 ──────────────────────────────────────────────────────────

    /// Admin: configure the large-deposit cool-off (issue #548).
    ///
    /// Deposits at or above `amount` (token units) mint shares locked for
    /// `holdback_ledgers` ledgers. `amount == 0` or `holdback_ledgers == 0`
    /// disables the feature. Small deposits are unaffected.
    pub fn set_large_deposit_threshold(
        env: Env,
        admin_addr: Address,
        amount: i128,
        holdback_ledgers: u32,
    ) -> Result<(), VaultError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;
        if admin_addr != admin::get_admin(&env)? {
            return Err(VaultError::Unauthorized);
        }
        if amount < 0 {
            return Err(VaultError::ZeroAmount);
        }
        env.storage().instance().set(&LARGE_THRESHOLD_KEY, &amount);
        env.storage()
            .instance()
            .set(&LARGE_HOLDBACK_KEY, &holdback_ledgers);
        env.events().publish(
            (symbol_short!("lg_thr"),),
            (amount, holdback_ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: the configured large-deposit threshold and holdback
    /// (`(amount, holdback_ledgers)`). `(0, _)` / `(_, 0)` means disabled.
    pub fn get_large_deposit_threshold(env: Env) -> (i128, u32) {
        (get_large_threshold(&env), get_large_holdback(&env))
    }

    /// Read-only: shares of `user` still locked by the large-deposit
    /// cool-off (issue #548). `0` when disabled, never locked, or fully
    /// unlocked. Expired tranches are pruned lazily on read.
    pub fn get_locked_shares(env: Env, user: Address) -> i128 {
        locked_shares_inner(&env, &user)
    }

    // ── Issue #549 ──────────────────────────────────────────────────────────

    /// Admin: set the minimum accepted `fund_reward_pool` amount (issue #549).
    /// `0` disables the check (current behaviour). Rejects negative amounts.
    pub fn set_min_reward_funding(
        env: Env,
        admin_addr: Address,
        amount: i128,
    ) -> Result<(), VaultOpsError> {
        admin_addr.require_auth();
        admin::require_admin(&env).map_err(|_| VaultOpsError::Unauthorized)?;
        if amount < 0 {
            return Err(VaultOpsError::ZeroAmount);
        }
        set_min_reward_funding_inner(&env, amount);
        env.events().publish(
            (symbol_short!("rw_min"),),
            (amount, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: the configured minimum reward-pool funding amount
    /// (`0` = check disabled).
    pub fn get_min_reward_funding(env: Env) -> i128 {
        get_min_reward_funding_inner(&env)
    }

    // ── Issue #546 ──────────────────────────────────────────────────────────

    /// Admin: cap how many distinct open positions one address may hold
    /// (issue #546). `0` disables the cap. Above 10 reverts with
    /// `MaxPositionsTooHigh` to bound per-user enumeration gas costs.
    ///
    /// Infrastructure for multi-position designs (locked / tokenized
    /// positions): enforced today by `position_split`, and future
    /// position-creating functions (`lock_position`, `tokenize_position`,
    /// etc.) should call `ensure_position_slot` the same way.
    pub fn set_max_positions_per_user(
        env: Env,
        admin_addr: Address,
        count: u32,
    ) -> Result<(), VaultError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env)? {
            return Err(VaultError::Unauthorized);
        }
        if count > MAX_POSITIONS_CEILING {
            return Err(VaultError::MaxPositionsTooHigh);
        }
        env.storage().instance().set(&MAX_POSITIONS_KEY, &count);
        env.events().publish(
            (symbol_short!("mx_pos"),),
            (count, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: the configured per-user position cap (`0` = unlimited).
    pub fn get_max_positions_per_user(env: Env) -> u32 {
        get_max_positions_inner(&env)
    }

    /// Read-only: how many distinct open positions `user` holds
    /// (primary + split positions). Closing a position frees a slot.
    pub fn get_position_count(env: Env, user: Address) -> u32 {
        position_count_inner(&env, &user)
    }
}
