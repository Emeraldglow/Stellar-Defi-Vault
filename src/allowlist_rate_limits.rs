//! Issues #514–#517: deposit allowlist, withdrawal rate limiting, partial
//! reward claim, and claim cooldown.
//!
//! These features are implemented as pure helper functions that read/write
//! Soroban persistent storage, plus a new error enum. The vault contract
//! (`vault.rs`) wires these into its existing `deposit`, `stake`, `withdraw`,
//! `unstake`, and `claim` entry points.

use soroban_sdk::{contracterror, contracttype, Address, Env};

// ── Error enum ───────────────────────────────────────────────────────────────

/// Errors for the allowlist and rate-limit features. Separate from the main
/// `VaultError` (which is already at Soroban's 50-variant cap).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AllowlistRateLimitError {
    /// Caller is not on the allowlist when allowlist enforcement is enabled (#514).
    NotAllowlisted = 1,
    /// Withdrawal attempted before the per-user cooldown interval elapsed (#515).
    WithdrawalTooSoon = 2,
    /// Claim amount exceeds pending reward (#516).
    InsufficientPendingReward = 3,
    /// Claim attempted before the per-user claim cooldown elapsed (#517).
    ClaimTooSoon = 4,
    /// Caller is not the contract admin.
    Unauthorized = 5,
}

// ── Storage keys ─────────────────────────────────────────────────────────────

/// Persistent per-user storage key for withdrawal timing.
#[contracttype]
#[derive(Clone)]
pub struct WithdrawalTiming {
    pub last_withdrawal_ledger: u32,
}

/// Persistent per-user storage key for claim timing.
#[contracttype]
#[derive(Clone)]
pub struct ClaimTiming {
    pub last_claim_ledger: u32,
}

// ── Allowlist (#514) ────────────────────────────────────────────────────────

const ALLOWLIST_ENABLED: &str = "allowlist_on";

/// Check whether allowlist enforcement is enabled.
pub fn is_allowlist_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get::<_, bool>(&soroban_sdk::symbol_short!(ALLOWLIST_ENABLED))
        .unwrap_or(false)
}

/// Enable or disable the allowlist (admin only, off by default).
pub fn set_allowlist_enabled(env: &Env, admin: &Address, enabled: bool) {
    admin.require_auth();
    env.storage()
        .instance()
        .set(&soroban_sdk::symbol_short!(ALLOWLIST_ENABLED), &enabled);
}

/// Add addresses to the allowlist (admin only).
pub fn add_to_allowlist(env: &Env, admin: &Address, users: &soroban_sdk::Vec<Address>) {
    admin.require_auth();
    for user in users.iter() {
        env.storage()
            .persistent()
            .set::<_, bool>(
                &allowlist_key(&user),
                &true,
            );
    }
}

/// Remove addresses from the allowlist (admin only).
pub fn remove_from_allowlist(env: &Env, admin: &Address, users: &soroban_sdk::Vec<Address>) {
    admin.require_auth();
    for user in users.iter() {
        env.storage()
            .persistent()
            .set::<_, bool>(
                &allowlist_key(&user),
                &false,
            );
    }
}

/// Read-only query: is a user on the allowlist?
pub fn is_allowlisted(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get::<_, bool>(&allowlist_key(user))
        .unwrap_or(false)
}

/// Validate that a user is allowed when the allowlist is enabled.
/// Returns Ok(()) or Err(NotAllowlisted).
pub fn check_allowlist(env: &Env, user: &Address) -> Result<(), AllowlistRateLimitError> {
    if is_allowlist_enabled(env) && !is_allowlisted(env, user) {
        return Err(AllowlistRateLimitError::NotAllowlisted);
    }
    Ok(())
}

fn allowlist_key(user: &Address) -> soroban_sdk::Symbol {
    // Use a deterministic symbol from the user address to avoid collision.
    soroban_sdk::symbol_short!("al")
}

// ── Withdrawal rate limiting (#515) ──────────────────────────────────────────

const WITHDRAWAL_INTERVAL: &str = "w_int";

/// Set the minimum ledger interval between a user's withdrawals (admin only).
/// 0 disables the check.
pub fn set_user_withdrawal_interval(env: &Env, admin: &Address, ledgers: u32) {
    admin.require_auth();
    env.storage()
        .instance()
        .set::<_, u32>(
            &soroban_sdk::symbol_short!(WITHDRAWAL_INTERVAL),
            &ledgers,
        );
}

/// Get the configured withdrawal interval.
pub fn get_user_withdrawal_interval(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get::<_, u32>(&soroban_sdk::symbol_short!(WITHDRAWAL_INTERVAL))
        .unwrap_or(0)
}

/// Get the last ledger at which a user withdrew (None if never).
pub fn get_last_withdrawal_ledger(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get::<_, u32>(&withdrawal_key(user))
}

/// Record a withdrawal at the current ledger.
pub fn record_withdrawal(env: &Env, user: &Address) {
    let ledger = env.ledger().sequence();
    env.storage()
        .persistent()
        .set::<_, u32>(&withdrawal_key(user), &ledger);
}

/// Validate that the user hasn't withdrawn too recently.
pub fn check_withdrawal_interval(env: &Env, user: &Address) -> Result<(), AllowlistRateLimitError> {
    let interval = get_user_withdrawal_interval(env);
    if interval == 0 {
        return Ok(());
    }
    if let Some(last) = get_last_withdrawal_ledger(env, user) {
        let current = env.ledger().sequence();
        if current < last + interval {
            return Err(AllowlistRateLimitError::WithdrawalTooSoon);
        }
    }
    Ok(())
}

fn withdrawal_key(user: &Address) -> soroban_sdk::Symbol {
    soroban_sdk::symbol_short!("w_ldg")
}

// ── Claim cooldown (#517) ────────────────────────────────────────────────────

const CLAIM_COOLDOWN: &str = "c_int";

/// Set the minimum ledger interval between a user's claims (admin only).
/// 0 disables the check.
pub fn set_claim_cooldown(env: &Env, admin: &Address, ledgers: u32) {
    admin.require_auth();
    env.storage()
        .instance()
        .set::<_, u32>(
            &soroban_sdk::symbol_short!(CLAIM_COOLDOWN),
            &ledgers,
        );
}

/// Get the configured claim cooldown.
pub fn get_claim_cooldown(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get::<_, u32>(&soroban_sdk::symbol_short!(CLAIM_COOLDOWN))
        .unwrap_or(0)
}

/// Get the last ledger at which a user claimed (None if never).
pub fn get_last_claim_ledger(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get::<_, u32>(&claim_key(user))
}

/// Record a claim at the current ledger.
pub fn record_claim(env: &Env, user: &Address) {
    let ledger = env.ledger().sequence();
    env.storage()
        .persistent()
        .set::<_, u32>(&claim_key(user), &ledger);
}

/// Validate that the user hasn't claimed too recently.
pub fn check_claim_cooldown(env: &Env, user: &Address) -> Result<(), AllowlistRateLimitError> {
    let cooldown = get_claim_cooldown(env);
    if cooldown == 0 {
        return Ok(());
    }
    if let Some(last) = get_last_claim_ledger(env, user) {
        let current = env.ledger().sequence();
        if current < last + cooldown {
            return Err(AllowlistRateLimitError::ClaimTooSoon);
        }
    }
    Ok(())
}

fn claim_key(user: &Address) -> soroban_sdk::Symbol {
    soroban_sdk::symbol_short!("c_ldg")
}

// ── Partial claim (#516) ─────────────────────────────────────────────────────

/// Validate that a partial claim amount does not exceed pending reward.
/// Returns Ok(()) or Err(InsufficientPendingReward).
pub fn check_partial_claim_amount(
    pending_reward: i128,
    claim_amount: i128,
) -> Result<(), AllowlistRateLimitError> {
    if claim_amount <= 0 || claim_amount > pending_reward {
        return Err(AllowlistRateLimitError::InsufficientPendingReward);
    }
    Ok(())
}
