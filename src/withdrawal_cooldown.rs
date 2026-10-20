//! Withdrawal cooldown period: request-and-execute two-phase withdrawal.
//!
//! When the cooldown is set to a non-zero number of ledgers, a withdrawal
//! must be requested first (`request_withdrawal`) and can only be executed
//! after the configured delay has elapsed (`execute_withdrawal`).
//!
//! Setting the cooldown to `0` (the default) disables this feature;
//! normal instant `withdraw`/`unstake` behavior is unaffected.
//!
//! # Flow
//!
//! 1. `set_withdrawal_cooldown(admin, ledgers)` — admin configures the wait.
//! 2. `request_withdrawal(user, shares)` — locks the user's shares and
//!    records the request ledger. Shares are removed from the user's live
//!    balance immediately so they cannot be double-spent.
//! 3. `execute_withdrawal(user)` — once `cooldown_ledgers` have elapsed,
//!    converts the locked shares to tokens and pays the user. Internally
//!    restores the shares briefly so `do_unstake` can run its full path
//!    (fee, treasury routing, events).
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap; raw `Symbol`-keyed storage is
//! used throughout (matching `balance.rs`).

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env};

use crate::{
    admin,
    balance,
    errors::VaultError,
    VaultContract,
};

const COOLDOWN_CFG_KEY: soroban_sdk::Symbol = symbol_short!("wd_cd");
const COOLDOWN_REQ_KEY: soroban_sdk::Symbol = symbol_short!("wd_req");

/// A pending withdrawal request created by `request_withdrawal`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct WithdrawalRequest {
    /// Number of shares locked for this withdrawal.
    pub shares: i128,
    /// Ledger sequence at which the request was created.
    pub requested_at: u32,
}

/// Read the configured cooldown period in ledgers (0 = disabled).
pub fn get_cooldown_ledgers(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&COOLDOWN_CFG_KEY)
        .unwrap_or(0)
}

fn store_cooldown_ledgers(env: &Env, ledgers: u32) {
    env.storage().instance().set(&COOLDOWN_CFG_KEY, &ledgers);
}

/// Read a user's pending withdrawal request, if any.
pub fn get_request(env: &Env, user: &Address) -> Option<WithdrawalRequest> {
    env.storage()
        .persistent()
        .get(&(COOLDOWN_REQ_KEY, user.clone()))
}

fn store_request(env: &Env, user: &Address, req: &WithdrawalRequest) {
    env.storage()
        .persistent()
        .set(&(COOLDOWN_REQ_KEY, user.clone()), req);
}

fn remove_request(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&(COOLDOWN_REQ_KEY, user.clone()));
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Admin: set the number of ledgers a user must wait between
    /// `request_withdrawal` and `execute_withdrawal`. Pass `0` to disable
    /// the cooldown (current instant-withdraw behavior is preserved).
    pub fn set_withdrawal_cooldown(
        env: Env,
        admin: Address,
        ledgers: u32,
    ) -> Result<(), VaultError> {
        admin.require_auth();
        if admin != crate::admin::get_admin(&env)? {
            return Err(VaultError::Unauthorized);
        }
        store_cooldown_ledgers(&env, ledgers);
        env.events().publish(
            (symbol_short!("wd_cd_s"),),
            ledgers,
        );
        Ok(())
    }

    /// Read-only: the configured withdrawal cooldown in ledgers (0 = disabled).
    pub fn get_withdrawal_cooldown(env: Env) -> u32 {
        get_cooldown_ledgers(&env)
    }

    /// Lock `shares` into a pending withdrawal request and start the
    /// cooldown timer. Shares are deducted from the user's live balance
    /// immediately to prevent double-spending. Call `execute_withdrawal`
    /// once the cooldown has elapsed.
    ///
    /// Reverts with `ZeroAmount` for non-positive share counts, and with
    /// `InsufficientShares` when the user's balance is below `shares`.
    pub fn request_withdrawal(
        env: Env,
        user: Address,
        shares: i128,
    ) -> Result<(), VaultError> {
        user.require_auth();
        if shares <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        let current_shares = balance::get_shares(&env, &user);
        if current_shares < shares {
            return Err(VaultError::InsufficientShares);
        }

        // Lock the shares by removing them from the user's spendable balance.
        balance::set_shares(&env, &user, current_shares - shares);

        store_request(
            &env,
            &user,
            &WithdrawalRequest {
                shares,
                requested_at: env.ledger().sequence(),
            },
        );
        env.events().publish(
            (symbol_short!("wd_req_e"),),
            (user, shares),
        );
        Ok(())
    }

    /// Execute a pending withdrawal request after the cooldown period.
    ///
    /// Reverts with `PositionNotFound` when no pending request exists, and
    /// with `UseCooldownFlow` when the configured cooldown has not yet elapsed.
    /// On success, the locked shares are converted to tokens and paid to the
    /// user (applying the normal unstake fee), and the pending request is removed.
    pub fn execute_withdrawal(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let req = get_request(&env, &user).ok_or(VaultError::PositionNotFound)?;

        let cooldown = get_cooldown_ledgers(&env);
        let now = env.ledger().sequence();
        if cooldown > 0 && !crate::ledger_boundary::duration_elapsed(now, req.requested_at, cooldown) {
            return Err(VaultError::UseCooldownFlow);
        }

        // Remove the request before executing to prevent re-entrancy.
        remove_request(&env, &user);

        // Restore the locked shares temporarily so do_unstake can deduct
        // them from the user's balance in its standard accounting path.
        let current = balance::get_shares(&env, &user);
        balance::set_shares(&env, &user, current + req.shares);

        Self::do_unstake(&env, &user, req.shares)
    }

    /// Read-only: the user's pending withdrawal request, if any.
    pub fn get_pending_withdrawal(env: Env, user: Address) -> Option<WithdrawalRequest> {
        get_request(&env, &user)
    }
}
