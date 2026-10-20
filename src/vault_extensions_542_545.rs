//! Vault extensions for issues #542, #543, #544, #545.
//!
//! - Issue #542: configurable minimum pool seed liquidity before public deposits.
//! - Issue #543: historical APY tracking via `get_apy_history()`.
//! - Issue #544: configurable reward-pool low-balance alert threshold.
//! - Issue #545: opt-in email/webhook notification registration.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap (see `storage.rs`), so everything
//! here uses raw `Symbol`-keyed storage, matching `balance.rs` and the other
//! feature modules.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Bytes, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

// ── Issue #543: APY history ─────────────────────────────────────────────────

/// Maximum APY snapshots retained (rolling buffer, oldest evicted first).
pub const MAX_APY_HISTORY: u32 = 90;

/// One point-in-time APY observation (issue #543).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ApySnapshot {
    pub apy_bps: u32,
    pub ledger: u32,
}

const APY_HIST_KEY: soroban_sdk::Symbol = symbol_short!("apy_hist");
const APY_LAST_KEY: soroban_sdk::Symbol = symbol_short!("apy_last");

fn get_apy_history_inner(env: &Env) -> Vec<ApySnapshot> {
    env.storage()
        .instance()
        .get(&APY_HIST_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_apy_history_inner(env: &Env, history: &Vec<ApySnapshot>) {
    env.storage().instance().set(&APY_HIST_KEY, history);
}

fn get_last_snapshot_ledger(env: &Env) -> Option<u32> {
    env.storage().instance().get(&APY_LAST_KEY)
}

fn set_last_snapshot_ledger(env: &Env, ledger: u32) {
    env.storage().instance().set(&APY_LAST_KEY, &ledger);
}

// ── Issue #544: low-balance alert ───────────────────────────────────────────

/// Instance key for the low-balance threshold in bps of outstanding
/// obligations (e.g. 11000 = warn when pool < 110% of owed rewards).
/// `0` (default) disables the alert.
const LOW_THRESHOLD_KEY: soroban_sdk::Symbol = symbol_short!("low_thr");
/// Instance flag recording whether the low-balance event has already been
/// emitted for the current low episode (prevents spamming every call).
const LOW_NOTIFIED_KEY: soroban_sdk::Symbol = symbol_short!("low_flag");

/// Sum of every staker's accrued-but-unclaimed reward.
///
/// Best-effort full enumeration over `AllStakers` (the same source
/// `minimum_reserve_ratio` uses, bounded there at 100). Here we iterate the
/// full list; for very large pools integrators should treat this as an
/// approximation — the flag is advisory, and topping up early is always safe.
pub fn total_outstanding_rewards(env: &Env) -> i128 {
    let all_stakers = balance::get_all_stakers(env);
    let mut total: i128 = 0;
    for staker in all_stakers.iter() {
        total = total.saturating_add(balance::get_accrued_reward(env, &staker));
    }
    total
}

fn get_low_threshold_bps(env: &Env) -> u32 {
    env.storage().instance().get(&LOW_THRESHOLD_KEY).unwrap_or(0)
}

/// Required pool balance at the current threshold: `obligations * bps / 10_000`.
fn required_pool_balance(env: &Env) -> i128 {
    let bps = get_low_threshold_bps(env);
    if bps == 0 {
        return 0;
    }
    let obligations = total_outstanding_rewards(env);
    obligations
        .saturating_mul(bps as i128)
        .checked_div(10_000)
        .unwrap_or(i128::MAX)
}

/// Whether the reward pool is currently below the configured threshold.
/// Always `false` when no threshold is configured (`0` = disabled).
pub fn is_low_inner(env: &Env) -> bool {
    let bps = get_low_threshold_bps(env);
    if bps == 0 {
        return false;
    }
    balance::get_reward_pool_balance(env) < required_pool_balance(env)
}

/// Check the low-balance condition and emit `reward_pool_low` exactly once
/// per low episode.
///
/// Called from state-changing paths (stake/unstake/claim/fund and the
/// threshold setter itself). When the pool recovers at or above the required
/// level the notified flag resets, so a *new* low episode emits again —
/// but repeated calls while continuously low never re-emit.
pub(crate) fn check_and_emit_low_balance(env: &Env) {
    if !is_low_inner(env) {
        // Recovered (or disabled): reset so the next episode can notify.
        if env.storage().instance().has(&LOW_NOTIFIED_KEY) {
            env.storage().instance().remove(&LOW_NOTIFIED_KEY);
        }
        return;
    }
    let already: bool = env
        .storage()
        .instance()
        .get(&LOW_NOTIFIED_KEY)
        .unwrap_or(false);
    if already {
        return;
    }
    env.storage().instance().set(&LOW_NOTIFIED_KEY, &true);
    events::reward_pool_low(
        env,
        balance::get_reward_pool_balance(env),
        total_outstanding_rewards(env),
        get_low_threshold_bps(env),
        env.ledger().sequence(),
    );
}

// ── Issue #542: seed liquidity ──────────────────────────────────────────────

/// Instance key for the minimum seeded reward-pool balance required before
/// public deposits are accepted. `0` (default) disables the gate, preserving
/// existing-pool behaviour.
const MIN_SEED_KEY: soroban_sdk::Symbol = symbol_short!("min_seed");

fn get_min_seed(env: &Env) -> i128 {
    env.storage().instance().get(&MIN_SEED_KEY).unwrap_or(0)
}

/// Whether the pool currently meets its seed requirement.
pub fn is_seeded_inner(env: &Env) -> bool {
    balance::get_reward_pool_balance(env) >= get_min_seed(env)
}

/// Enforce the seed gate on user deposit paths.
///
/// `VaultError` is at Soroban's 50-variant cap, so there is no dedicated
/// `PoolNotSeeded` variant; the seed-gate revert reuses
/// `VaultError::InsufficientRewardPool` (the pool cannot cover rewards until
/// seeded). Admin seeding via `fund_reward_pool` never calls this, so the
/// admin can always top the pool up past the threshold.
pub(crate) fn require_pool_seeded(env: &Env) -> Result<(), VaultError> {
    if is_seeded_inner(env) {
        Ok(())
    } else {
        Err(VaultError::InsufficientRewardPool)
    }
}

// ── Issue #545: notification preferences ────────────────────────────────────
//
// Purely a registration/storage function: the contract never sends email,
// HTTP requests, or any other off-chain message. An off-chain indexer/bot
// reads `get_notification_preference(user)` and delivers `claim-ready` /
// rate-change alerts itself. Only an opaque identifier is stored on-chain
// (e.g. the sha256 of a webhook URL), never the raw URL/email, for privacy.

/// Opaque per-user notification preference (issue #545).
///
/// - `endpoint_hash`: opaque identifier for the user's off-chain endpoint
///   (e.g. `sha256(webhook_url)`); never the raw URL itself.
/// - `allow_claim_ready` / `allow_rate_change`: which event classes the user
///   opted into; read by the off-chain indexer/bot.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationPreference {
    pub endpoint_hash: Bytes,
    pub allow_claim_ready: bool,
    pub allow_rate_change: bool,
}

fn notif_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "notif_pref"), user.clone())
}

fn get_notification_inner(env: &Env, user: &Address) -> Option<NotificationPreference> {
    env.storage().persistent().get(&notif_key(env, user))
}

fn set_notification_inner(env: &Env, user: &Address, pref: &NotificationPreference) {
    env.storage()
        .persistent()
        .set(&notif_key(env, user), pref);
}

fn clear_notification_inner(env: &Env, user: &Address) {
    env.storage().persistent().remove(&notif_key(env, user));
}

#[contractimpl]
impl VaultContract {
    // ── Issue #542: minimum seed liquidity ──────────────────────────────────

    /// Admin: set the minimum reward-pool balance required before public
    /// deposits are accepted. `0` disables the gate.
    pub fn set_min_seed_liquidity(
        env: Env,
        admin_addr: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;
        if amount < 0 {
            return Err(VaultError::ZeroAmount);
        }
        env.storage().instance().set(&MIN_SEED_KEY, &amount);
        env.events().publish(
            (symbol_short!("seed_set"), admin_addr),
            (amount, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: the configured minimum seed liquidity (`0` = no gate).
    pub fn get_min_seed_liquidity(env: Env) -> i128 {
        get_min_seed(&env)
    }

    /// Read-only: whether the reward pool currently meets the seed
    /// requirement. Always `true` when no minimum is configured.
    pub fn is_pool_seeded(env: Env) -> bool {
        is_seeded_inner(&env)
    }

    // ── Issue #543: APY history ─────────────────────────────────────────────

    /// Read-only point-in-time APY in basis points (currently the base
    /// reward rate). Stored snapshots capture this value.
    pub fn get_apy(env: Env) -> u32 {
        balance::get_reward_rate_bps(&env)
    }

    /// Record the current APY into the rolling 90-entry history.
    ///
    /// Keeper-callable: any authenticated caller may invoke it (typically an
    /// off-chain keeper bot); when the keeper registry is in use the call is
    /// attributed to the keeper's stats if registered. Rate-limited to at
    /// most one snapshot per `LEDGERS_PER_DAY`; a second call within the same
    /// day reverts with `EpochNotFinalized`.
    pub fn record_apy_snapshot(env: Env, keeper: Address) -> Result<(), VaultError> {
        keeper.require_auth();
        let now = env.ledger().sequence();
        if let Some(last) = get_last_snapshot_ledger(&env) {
            if now.saturating_sub(last) < LEDGERS_PER_DAY {
                return Err(VaultError::EpochNotFinalized);
            }
        }
        let apy_bps = balance::get_reward_rate_bps(&env);
        let mut history = get_apy_history_inner(&env);
        history.push_back(ApySnapshot {
            apy_bps,
            ledger: now,
        });
        // Rollover: drop oldest first once over capacity.
        while history.len() > MAX_APY_HISTORY {
            history.remove(0);
        }
        set_apy_history_inner(&env, &history);
        set_last_snapshot_ledger(&env, now);
        crate::keeper_registry::record_keeper_action(&env, &keeper, 0);
        events::apy_snapshot_recorded(&env, apy_bps, now);
        Ok(())
    }

    /// Read-only: the stored APY history, oldest first (max 90 entries).
    pub fn get_apy_history(env: Env) -> Vec<ApySnapshot> {
        get_apy_history_inner(&env)
    }

    // ── Issue #544: low-balance alert ───────────────────────────────────────

    /// Admin: set the low-balance threshold in bps of outstanding claim
    /// obligations (e.g. `11000` = warn while pool < 110% of owed rewards).
    /// `0` disables the alert.
    pub fn set_low_balance_threshold(
        env: Env,
        admin_addr: Address,
        bps_of_obligations: u32,
    ) -> Result<(), VaultError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;
        // 100_000 bps = 1000% — generous ceiling; anything higher is almost
        // certainly a misplaced decimal.
        if bps_of_obligations > 100_000 {
            return Err(VaultError::InvalidRate);
        }
        env.storage()
            .instance()
            .set(&LOW_THRESHOLD_KEY, &bps_of_obligations);
        env.events().publish(
            (symbol_short!("low_thr"), admin_addr),
            (bps_of_obligations, env.ledger().sequence()),
        );
        // A threshold change itself is a state-changing call: evaluate once
        // so operators get immediate feedback.
        check_and_emit_low_balance(&env);
        Ok(())
    }

    /// Read-only: the configured low-balance threshold in bps (`0` = disabled).
    pub fn get_low_balance_threshold(env: Env) -> u32 {
        get_low_threshold_bps(&env)
    }

    /// Read-only: whether the reward pool is currently below the configured
    /// threshold relative to total outstanding pending rewards. Always
    /// `false` when the alert is disabled.
    pub fn is_reward_pool_low(env: Env) -> bool {
        is_low_inner(&env)
    }

    // ── Issue #545: notification registration ───────────────────────────────
    //
    // Purely on-chain registration for off-chain delivery: this contract
    // never sends email/webhook traffic. An off-chain indexer/bot reads the
    // stored opaque preference and notifies the user of `claim-ready` or
    // large rate-change events.

    /// Opt in (or update) a user's off-chain notification endpoint.
    ///
    /// Stores only the opaque `endpoint_hash` (e.g. sha256 of a webhook URL),
    /// never a raw URL/email address, for privacy. No off-chain delivery is
    /// performed by this contract.
    pub fn set_notification_preference(
        env: Env,
        user: Address,
        preference: NotificationPreference,
    ) -> Result<(), VaultError> {
        user.require_auth();
        set_notification_inner(&env, &user, &preference);
        env.events().publish(
            (symbol_short!("notif_set"), user),
            (env.ledger().sequence(),),
        );
        Ok(())
    }

    /// Alias for `set_notification_preference` (issue title names this
    /// `set_notification_endpoint`). Identical semantics.
    pub fn set_notification_endpoint(
        env: Env,
        user: Address,
        preference: NotificationPreference,
    ) -> Result<(), VaultError> {
        Self::set_notification_preference(env, user, preference)
    }

    /// Clear a user's notification preference (opt out).
    pub fn clear_notification_preference(
        env: Env,
        user: Address,
    ) -> Result<(), VaultError> {
        user.require_auth();
        clear_notification_inner(&env, &user);
        env.events().publish(
            (symbol_short!("notif_clr"), user),
            (env.ledger().sequence(),),
        );
        Ok(())
    }

    /// Read-only: a user's notification preference, or `None` when unset.
    pub fn get_notification_preference(
        env: Env,
        user: Address,
    ) -> Option<NotificationPreference> {
        get_notification_inner(&env, &user)
    }
}
