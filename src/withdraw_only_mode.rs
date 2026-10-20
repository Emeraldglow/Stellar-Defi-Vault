//! Withdraw-only emergency mode (issue #705).
//!
//! A pool state distinct from `pause`: every inflow is blocked, but
//! withdrawals and claims stay fully open and their fees are waived, so the
//! pool can be wound down in an orderly way without freezing user funds.
//!
//! It is also distinct from `initiate_sunset`: entering this mode is
//! immediate and has no deadline, and the admin can reverse it with
//! `exit_withdraw_only_mode` (sunset is one-way). The mode never lifts by
//! itself.
//!
//! # Blocked inflows
//!
//! `deposit`, `stake`, `stake_for`, `stake_and_claim` and `deposit_for_many`
//! (through `VaultContract::require_no_sunset`), `compound`, `lock_position`
//! and the auto-restake of rewards during an unstake. Auto-restaked rewards
//! simply stay accrued and claimable.
//!
//! # Waived fees
//!
//! The unstake fee (in both withdrawal paths) and the claim fee.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultSafetyError;
use crate::vault::{VaultContract, VaultContractClient};

/// Instance-storage key for the withdraw-only flag.
const MODE_KEY: Symbol = symbol_short!("wd_only");

/// The pool's operating mode as reported by `get_pool_mode()`.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum PoolMode {
    /// Deposits and withdrawals are both open.
    Normal,
    /// The pool is paused.
    Paused,
    /// Inflows are blocked; withdrawals and claims are open and fee-free.
    WithdrawOnly,
}

/// Whether the pool is currently in withdraw-only mode.
pub fn is_active(env: &Env) -> bool {
    env.storage().instance().get(&MODE_KEY).unwrap_or(false)
}

#[contractimpl]
impl VaultContract {
    /// Admin: put the pool into withdraw-only mode (issue #705). Takes effect
    /// immediately and stays in effect until `exit_withdraw_only_mode`.
    ///
    /// # Authorization
    /// `admin` must sign and be the stored admin.
    ///
    /// # Errors
    /// - `Unauthorized`: `admin` is not the stored admin.
    /// - `NotInitialized`: the vault has no admin.
    ///
    /// # Events
    /// - `wd_only` with `true` and the ledger.
    pub fn enter_withdraw_only_mode(env: Env, admin: Address) -> Result<(), VaultSafetyError> {
        admin::require_admin_as(&env, &admin)?;
        env.storage().instance().set(&MODE_KEY, &true);
        env.events()
            .publish((symbol_short!("wd_only"), admin), (true, env.ledger().sequence()));
        Ok(())
    }

    /// Admin: leave withdraw-only mode and reopen inflows (issue #705).
    ///
    /// # Authorization
    /// `admin` must sign and be the stored admin.
    ///
    /// # Errors
    /// - `Unauthorized`: `admin` is not the stored admin.
    /// - `NotInitialized`: the vault has no admin.
    ///
    /// # Events
    /// - `wd_only` with `false` and the ledger.
    pub fn exit_withdraw_only_mode(env: Env, admin: Address) -> Result<(), VaultSafetyError> {
        admin::require_admin_as(&env, &admin)?;
        env.storage().instance().remove(&MODE_KEY);
        env.events()
            .publish((symbol_short!("wd_only"), admin), (false, env.ledger().sequence()));
        Ok(())
    }

    /// Read-only: the pool's current mode (issue #705). `Paused` takes
    /// precedence over `WithdrawOnly`, since a paused pool freezes
    /// withdrawals too.
    pub fn get_pool_mode(env: Env) -> PoolMode {
        if Self::is_paused(env.clone()) {
            PoolMode::Paused
        } else if is_active(&env) {
            PoolMode::WithdrawOnly
        } else {
            PoolMode::Normal
        }
    }
}
