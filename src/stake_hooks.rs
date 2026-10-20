//! Registered hook contracts notified on stake and unstake (issue #711).
//!
//! The admin registers up to [`MAX_HOOKS`] contracts. After a stake or an
//! unstake has finalized its state, each hook is called as
//! `on_action(user: Address, action: Symbol, amount: i128)` where `action` is
//! `"stake"` or `"unstake"`, so partner protocols can mirror positions without
//! polling.
//!
//! Hooks are best effort. Each call goes through `try_invoke_contract`, so a
//! hook that traps or returns an error never reverts the user's action; the
//! failure is reported with a `hook_fail` event instead and the callee's own
//! state changes are rolled back by the host. A hook cannot call back into the
//! vault, since Soroban rejects re-entering a contract that is already
//! executing.
//!
//! Known limit: a hook that exhausts the transaction's CPU or memory budget
//! cannot be caught from inside the contract. The hook count is capped and
//! only the admin can register hooks, so register only trusted contracts;
//! `remove_hook` takes a misbehaving one out immediately.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, IntoVal, Symbol, Val, Vec};

use crate::admin;
use crate::errors::VaultError;
use crate::vault::{VaultContract, VaultContractClient};

/// Instance-storage key for the registered hook addresses.
const HOOKS_KEY: Symbol = symbol_short!("stk_hooks");

/// Maximum number of registered hooks.
pub const MAX_HOOKS: u32 = 3;

/// Read the registered hooks, in registration order.
pub fn hooks(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&HOOKS_KEY)
        .unwrap_or(Vec::new(env))
}

/// Notify every registered hook that `user` performed `action` for `amount`.
/// Never fails: a failing hook only emits `hook_fail`.
pub fn notify(env: &Env, user: &Address, action: Symbol, amount: i128) {
    for hook in hooks(env).iter() {
        let args: Vec<Val> = (user.clone(), action.clone(), amount).into_val(env);
        let result = env.try_invoke_contract::<Val, soroban_sdk::Error>(
            &hook,
            &symbol_short!("on_action"),
            args,
        );
        if !matches!(result, Ok(Ok(_))) {
            env.events().publish(
                (symbol_short!("hook_fail"), user.clone()),
                (hook, action.clone(), amount, env.ledger().sequence()),
            );
        }
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Admin: register `hook` to be notified after every stake and unstake.
    ///
    /// # Authorization
    /// The stored admin must sign.
    ///
    /// # Errors
    /// - `Unauthorized`: `admin` is not the stored admin.
    /// - `InvalidAddress`: `hook` is this contract.
    /// - `AlreadyInitialized`: `hook` is already registered.
    /// - `MaxPositionsReached`: [`MAX_HOOKS`] hooks are already registered.
    pub fn register_hook(env: Env, admin: Address, hook: Address) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        if hook == env.current_contract_address() {
            return Err(VaultError::InvalidAddress);
        }
        let mut registered = hooks(&env);
        if registered.contains(&hook) {
            return Err(VaultError::AlreadyInitialized);
        }
        if registered.len() >= MAX_HOOKS {
            return Err(VaultError::MaxPositionsReached);
        }
        registered.push_back(hook);
        env.storage().instance().set(&HOOKS_KEY, &registered);
        Ok(())
    }

    /// Admin: stop notifying `hook`.
    ///
    /// # Authorization
    /// The stored admin must sign.
    ///
    /// # Errors
    /// - `Unauthorized`: `admin` is not the stored admin.
    /// - `PositionNotFound`: `hook` is not registered.
    pub fn remove_hook(env: Env, admin: Address, hook: Address) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        let mut registered = hooks(&env);
        let index = registered
            .first_index_of(&hook)
            .ok_or(VaultError::PositionNotFound)?;
        registered.remove(index);
        env.storage().instance().set(&HOOKS_KEY, &registered);
        Ok(())
    }

    /// Read-only: the registered hook contracts.
    pub fn get_hooks(env: Env) -> Vec<Address> {
        hooks(&env)
    }
}
