//! Issue #488: admin-configurable maximum deposit per user.
//!
//! The cap applies to a user's *total* position (their current position value
//! plus the incoming deposit), not just the incoming amount. `0` disables it.
//! It only limits new deposits: lowering the cap never reduces an existing
//! position, so a position already above a newly-lowered cap stays as it is and
//! simply cannot grow until it drops back under the cap.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so the cap lives under a raw
//! `Symbol` key in instance storage.

use soroban_sdk::{
    contracterror, contractimpl, panic_with_error, symbol_short, Address, Env, Symbol,
};

use crate::vault::{VaultContract, VaultContractClient};
use crate::{admin, balance};

const CAP_KEY: Symbol = symbol_short!("usr_cap");

/// Errors for the per-user deposit cap.
///
/// `DepositCapExceeded` is raised from inside `stake`/`deposit`, which return
/// `VaultError`. `VaultError` already uses every code from 1 to 50, so this
/// code sits above that range: a client decoding the failure as `VaultError`
/// then sees an unknown contract error (`51`) instead of misreading it as an
/// unrelated `VaultError` variant.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DepositCapError {
    /// The caller is not the stored admin.
    Unauthorized = 1,
    /// The cap passed to `set_max_deposit_cap` was negative.
    InvalidCap = 2,
    /// The pool has not been initialized.
    NotInitialized = 3,
    /// The user's resulting position would exceed the configured cap.
    DepositCapExceeded = 51,
}

/// Current cap in stake-token units, `0` when disabled.
pub fn get_cap(env: &Env) -> i128 {
    env.storage().instance().get(&CAP_KEY).unwrap_or(0)
}

/// Reverts with `DepositCapExceeded` when depositing `amount` more would put
/// `user`'s total position above the cap. No-op while the cap is disabled.
pub fn enforce(env: &Env, user: &Address, amount: i128) {
    let cap = get_cap(env);
    if cap == 0 {
        return;
    }
    let shares = balance::get_shares(env, user);
    let existing = if shares == 0 {
        0
    } else {
        balance::shares_to_amount(
            balance::get_total_shares(env),
            balance::get_total_deposited(env),
            shares,
        )
        .unwrap_or(0)
    };
    let resulting = existing.checked_add(amount).unwrap_or(i128::MAX);
    if resulting > cap {
        panic_with_error!(env, DepositCapError::DepositCapExceeded);
    }
}

#[contractimpl]
impl VaultContract {
    /// Admin: set the maximum total position a single user may hold, in
    /// stake-token units. `0` disables the cap. Emits
    /// `max_deposit_cap_updated` as `(admin, old_cap, new_cap, ledger)`.
    pub fn set_max_deposit_cap(
        env: Env,
        admin: Address,
        amount: i128,
    ) -> Result<(), DepositCapError> {
        admin.require_auth();
        let stored = admin::get_admin(&env).map_err(|_| DepositCapError::NotInitialized)?;
        if admin != stored {
            return Err(DepositCapError::Unauthorized);
        }
        if amount < 0 {
            return Err(DepositCapError::InvalidCap);
        }
        let old_cap = get_cap(&env);
        env.storage().instance().set(&CAP_KEY, &amount);
        env.events().publish(
            (Symbol::new(&env, "max_deposit_cap_updated"),),
            (admin, old_cap, amount, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: the per-user deposit cap in stake-token units, `0` when
    /// disabled.
    pub fn get_max_deposit_cap(env: Env) -> i128 {
        get_cap(&env)
    }
}
