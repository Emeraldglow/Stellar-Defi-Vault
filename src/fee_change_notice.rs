//! Fee change notice period and hard ceiling (issue: fee-ceiling-and-notice).
//!
//! # Motivation
//!
//! Prevent surprise fee hikes by:
//! 1. Capping every unstake-fee setter at `MAX_FEE_BPS` (2 000 bps = 20%).
//! 2. Requiring that *increases* be queued and only become effective after
//!    `fee_notice_ledgers` have elapsed. Decreases apply immediately so users
//!    always benefit from a reduction without delay.
//!
//! # How it integrates
//!
//! All three existing fee-setter call sites (`set_unstake_fee_bps` in vault.rs,
//! `role_set_unstake_fee_bps` in access_roles.rs, and `set_fee` in
//! admin_roles.rs) are wired through `propose_fee_change()` instead of calling
//! `balance::set_unstake_fee_bps` directly.
//!
//! `propose_fee_change()`:
//! - Rejects anything above `MAX_FEE_BPS`.
//! - If the new value is ≤ the current effective fee, applies it instantly.
//! - Otherwise stores a `PendingFeeChange` and returns without writing the live
//!   fee; callers of `balance::get_unstake_fee_bps` are unaffected until
//!   `settle_pending_fee_change()` is called (which every state-changing
//!   entrypoint that reads the fee should call first).
//!
//! `settle_pending_fee_change()` is cheap (one instance-storage read that
//! returns immediately when there is nothing pending or the delay has not
//! elapsed).
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this module uses raw
//! `symbol_short!` instance-storage keys, following the pattern established
//! in `balance.rs` and `claim_fee.rs`.
//!
//! Keys:
//! - `"fn_delay"` → `u32`  — notice delay in ledgers (default 0)
//! - `"fn_pend"` → `PendingFeeChange` — pending increase, if any

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::vault::VaultContract;

// ── Constants ────────────────────────────────────────────────────────────────

/// Hard ceiling for the unstake fee: 2 000 bps = 20%.
///
/// No fee setter anywhere in the contract may exceed this value.
/// It is intentionally higher than the previous per-setter soft cap of 500 bps
/// so that existing operations keep working, but it provides an absolute,
/// immutable guard that can never be overridden by configuration.
pub const MAX_FEE_BPS: u32 = 2_000;

// ── Storage keys ─────────────────────────────────────────────────────────────

const NOTICE_DELAY_KEY: Symbol = symbol_short!("fn_delay");
const PENDING_FEE_KEY: Symbol = symbol_short!("fn_pend");

// ── Data types ───────────────────────────────────────────────────────────────

/// A queued fee increase that has not yet become effective.
///
/// Once `env.ledger().sequence() >= effective_at`, `settle_pending_fee_change`
/// will write `new_bps` to `DataKey::UnstakeFeeBps` and clear this entry.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingFeeChange {
    /// The new fee value, in basis points.
    pub new_bps: u32,
    /// The earliest ledger at which the change may be applied.
    pub effective_at: u32,
}

// ── Storage helpers ───────────────────────────────────────────────────────────

/// The configured notice delay in ledgers. Defaults to 0 (increases apply
/// immediately) until `set_fee_notice_ledgers` has been called.
pub fn get_fee_notice_ledgers(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&NOTICE_DELAY_KEY)
        .unwrap_or(0)
}

fn set_fee_notice_ledgers_storage(env: &Env, ledgers: u32) {
    env.storage().instance().set(&NOTICE_DELAY_KEY, &ledgers);
}

/// The pending queued fee increase, or `None` when no increase is queued.
pub fn get_pending_fee_change(env: &Env) -> Option<PendingFeeChange> {
    env.storage().instance().get(&PENDING_FEE_KEY)
}

fn set_pending_fee_change(env: &Env, change: &PendingFeeChange) {
    env.storage().instance().set(&PENDING_FEE_KEY, change);
}

fn clear_pending_fee_change(env: &Env) {
    env.storage().instance().remove(&PENDING_FEE_KEY);
}

// ── Core logic ────────────────────────────────────────────────────────────────

/// Check whether the pending fee change has matured and, if so, apply it.
///
/// Call this at the start of any entrypoint that reads or applies the
/// unstake fee so that users always see an up-to-date value without
/// requiring a separate keeper transaction.
pub fn settle_pending_fee_change(env: &Env) {
    if let Some(pending) = get_pending_fee_change(env) {
        if env.ledger().sequence() >= pending.effective_at {
            balance::set_unstake_fee_bps(env, pending.new_bps);
            clear_pending_fee_change(env);
            env.events().publish(
                (symbol_short!("fee_apld"),),
                (pending.new_bps, env.ledger().sequence()),
            );
        }
    }
}

/// Route a fee-change request through the ceiling check and notice-period gate.
///
/// - Returns `UnstakeFeeTooHigh` if `new_bps > MAX_FEE_BPS`.
/// - If `new_bps` is a *decrease* (or equal to the current fee), it is applied
///   immediately and any previously queued increase is cancelled.
/// - If `new_bps` is an *increase* and `fee_notice_ledgers > 0`, it is queued;
///   the live fee is unchanged until `settle_pending_fee_change` fires.
/// - If `fee_notice_ledgers == 0`, increases also apply immediately.
///
/// Returns `Ok(true)` when the change was applied immediately, `Ok(false)` when
/// it was queued.
pub fn propose_fee_change(env: &Env, new_bps: u32) -> Result<bool, VaultError> {
    if new_bps > MAX_FEE_BPS {
        return Err(VaultError::UnstakeFeeTooHigh);
    }

    // Settle any mature pending change first so the "current fee" comparison
    // is accurate.
    settle_pending_fee_change(env);

    let current_bps = balance::get_unstake_fee_bps(env);
    let notice_ledgers = get_fee_notice_ledgers(env);

    if new_bps <= current_bps || notice_ledgers == 0 {
        // Decrease (or equal) — apply instantly; also clear any queued increase
        // so a previously scheduled hike cannot override this reduction.
        // Same for the notice-free case (delay == 0).
        balance::set_unstake_fee_bps(env, new_bps);
        clear_pending_fee_change(env);
        env.events().publish(
            (symbol_short!("fee_set"),),
            (new_bps, env.ledger().sequence()),
        );
        Ok(true)
    } else {
        // Increase with a notice period — queue it.
        let effective_at = env
            .ledger()
            .sequence()
            .saturating_add(notice_ledgers);
        let change = PendingFeeChange { new_bps, effective_at };
        set_pending_fee_change(env, &change);
        env.events().publish(
            (symbol_short!("fee_queued"),),
            (new_bps, effective_at, env.ledger().sequence()),
        );
        Ok(false)
    }
}

// ── Contract entrypoints ──────────────────────────────────────────────────────

#[contractimpl]
impl VaultContract {
    /// Admin: configure the notice period (in ledgers) that must elapse before
    /// a fee *increase* takes effect.
    ///
    /// Pass `0` to disable the delay (all changes apply immediately).
    /// There is no upper bound enforced here; a reasonable value is
    /// `17_280` (~1 day at 5 s/ledger on Stellar) to `172_800` (~10 days).
    ///
    /// # Authorization
    ///
    /// Requires admin.
    ///
    /// # Errors
    ///
    /// - `Unauthorized` — caller is not the admin.
    /// - `NotInitialized` — admin not set.
    pub fn set_fee_notice_ledgers(
        env: Env,
        admin: Address,
        ledgers: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        set_fee_notice_ledgers_storage(&env, ledgers);
        env.events().publish(
            (symbol_short!("fn_cfg"),),
            (ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: the configured notice delay in ledgers (`0` = no delay).
    pub fn get_fee_notice_ledgers(env: Env) -> u32 {
        get_fee_notice_ledgers(&env)
    }

    /// Read-only: the pending queued fee increase, if any.
    ///
    /// Returns `Some((new_bps, effective_at))` when an increase has been
    /// queued and has not yet matured or been overridden.  Returns `None`
    /// when no change is pending (no queued increase, or it already fired).
    ///
    /// Note: if the returned `effective_at ≤ current_ledger`, the change is
    /// mature and will be applied on the next state-changing fee call.
    pub fn get_pending_fee_change(env: Env) -> Option<(u32, u32)> {
        get_pending_fee_change(&env).map(|p| (p.new_bps, p.effective_at))
    }
}
