//! Issue #513: role-based access control alongside the single admin.
//!
//! The admin can delegate narrow responsibilities to other keys without
//! handing over full admin power:
//!
//! - `Role::RateSetter`      — change the reward rate (`role_set_reward_rate_bps`)
//! - `Role::Pauser`          — pause / unpause the pool (`role_pause`, `role_unpause`)
//! - `Role::TreasuryManager` — fund the reward pool and set the unstake fee
//!                             (`role_fund_reward_pool`, `role_set_unstake_fee_bps`)
//!
//! The admin implicitly holds every role, so all existing admin-only
//! entrypoints keep working unchanged. Only the admin can grant or revoke
//! roles; a holder may renounce their own role.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so grants are kept under a
//! tuple key: `(symbol_short!("role"), Role, Address)` -> `bool` (persistent).

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultAccessError;
use crate::runway_guard;
use crate::storage::PauseReason;
use crate::vault::{VaultContract, VaultContractClient};

const ROLE_PREFIX: Symbol = symbol_short!("role");

/// Delegable responsibilities split out of the single admin.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    RateSetter,
    Pauser,
    TreasuryManager,
}

fn role_key(role: Role, account: &Address) -> (Symbol, Role, Address) {
    (ROLE_PREFIX, role, account.clone())
}

/// Whether `account` has been explicitly granted `role`. Does not consider
/// the admin's implicit ownership of every role — see `holds_role`.
pub fn is_role_granted(env: &Env, role: Role, account: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&role_key(role, account))
        .unwrap_or(false)
}

/// Whether `account` may act as `role`: the admin, or an explicit grantee.
pub fn holds_role(env: &Env, role: Role, account: &Address) -> bool {
    match admin::get_admin(env) {
        Ok(admin_addr) if &admin_addr == account => true,
        _ => is_role_granted(env, role, account),
    }
}

/// Authorizes `caller` and checks they hold `role`.
pub(crate) fn require_role(
    env: &Env,
    role: Role,
    caller: &Address,
) -> Result<(), VaultAccessError> {
    caller.require_auth();
    if !holds_role(env, role, caller) {
        return Err(VaultAccessError::MissingRole);
    }
    Ok(())
}

fn require_admin_caller(env: &Env, caller: &Address) -> Result<(), VaultAccessError> {
    caller.require_auth();
    let admin_addr = admin::get_admin(env)?;
    if &admin_addr != caller {
        return Err(VaultAccessError::Unauthorized);
    }
    Ok(())
}

#[contractimpl]
impl VaultContract {
    // ── Role management ─────────────────────────────────────────────────────

    /// Admin: grant `role` to `account`. Idempotent.
    pub fn grant_role(
        env: Env,
        admin_addr: Address,
        role: Role,
        account: Address,
    ) -> Result<(), VaultAccessError> {
        require_admin_caller(&env, &admin_addr)?;
        env.storage().persistent().set(&role_key(role, &account), &true);
        env.events()
            .publish((symbol_short!("role_grt"), role, account), admin_addr);
        Ok(())
    }

    /// Admin: revoke `role` from `account`. A no-op when not granted.
    pub fn revoke_role(
        env: Env,
        admin_addr: Address,
        role: Role,
        account: Address,
    ) -> Result<(), VaultAccessError> {
        require_admin_caller(&env, &admin_addr)?;
        let key = role_key(role, &account);
        if env.storage().persistent().has(&key) {
            env.storage().persistent().remove(&key);
            env.events()
                .publish((symbol_short!("role_rvk"), role, account), admin_addr);
        }
        Ok(())
    }

    /// Role holder: give up one's own `role`. A no-op when not granted.
    pub fn renounce_role(env: Env, account: Address, role: Role) {
        account.require_auth();
        let key = role_key(role, &account);
        if env.storage().persistent().has(&key) {
            env.storage().persistent().remove(&key);
            env.events()
                .publish((symbol_short!("role_rvk"), role, account.clone()), account);
        }
    }

    /// Read-only: whether `account` may act as `role` (admin always may).
    pub fn has_role(env: Env, role: Role, account: Address) -> bool {
        holds_role(&env, role, &account)
    }

    // ── RateSetter ──────────────────────────────────────────────────────────

    /// RateSetter: set the annual reward rate (basis points). Same bounds and
    /// runway guard as the admin's `set_reward_rate_bps`.
    pub fn role_set_reward_rate_bps(
        env: Env,
        caller: Address,
        rate_bps: u32,
    ) -> Result<(), VaultAccessError> {
        require_role(&env, Role::RateSetter, &caller)?;
        runway_guard::apply_reward_rate(&env, rate_bps)?;
        Ok(())
    }

    // ── Pauser ──────────────────────────────────────────────────────────────

    /// Pauser: pause all deposits and withdrawals.
    pub fn role_pause(
        env: Env,
        caller: Address,
        reason: PauseReason,
        message: soroban_sdk::String,
    ) -> Result<(), VaultAccessError> {
        require_role(&env, Role::Pauser, &caller)?;
        VaultContract::pause_by(&env, &caller, reason, message)?;
        Ok(())
    }

    /// Pauser: resume deposits and withdrawals.
    pub fn role_unpause(env: Env, caller: Address) -> Result<(), VaultAccessError> {
        require_role(&env, Role::Pauser, &caller)?;
        VaultContract::unpause_by(&env, &caller)?;
        Ok(())
    }

    // ── TreasuryManager ─────────────────────────────────────────────────────

    /// TreasuryManager: transfer `amount` of the reward token from `caller`
    /// into the reward pool.
    pub fn role_fund_reward_pool(
        env: Env,
        caller: Address,
        amount: i128,
    ) -> Result<(), VaultAccessError> {
        require_role(&env, Role::TreasuryManager, &caller)?;
        runway_guard::credit_reward_pool_from(&env, &caller, amount)?;
        Ok(())
    }

    /// TreasuryManager: set the unstake fee (max 500 bps).
    pub fn role_set_unstake_fee_bps(
        env: Env,
        caller: Address,
        bps: u32,
    ) -> Result<(), VaultAccessError> {
        require_role(&env, Role::TreasuryManager, &caller)?;
        // Routes through the ceiling check (MAX_FEE_BPS) and notice-period
        // gate: increases are queued; decreases apply immediately.
        crate::fee_change_notice::propose_fee_change(&env, bps)?;
        Ok(())
    }
}
