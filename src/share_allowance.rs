//! Share withdrawal allowances (issue #697).
//!
//! An owner can approve another address (a keeper, a contract, a custodian) to
//! withdraw up to a set number of their shares, without handing over the
//! position. The spender calls `withdraw_from()`, which burns the owner's
//! shares and pays the proceeds to a recipient the spender names.
//!
//! The allowance covers withdrawal only. It never grants the right to claim
//! rewards or to change where the owner's rewards are paid.
//!
//! * Each `(owner, spender)` pair has one allowance. `approve_withdrawal()`
//!   overwrites it, and approving `0` shares revokes it.
//! * An allowance stops working at `expires_at` (the ledger at which it is no
//!   longer valid) and shrinks by the shares used on each `withdraw_from()`.
//! * `withdraw_from()` applies the same minimum-unstake and rolling daily
//!   withdrawal-limit checks as `unstake()`, charged to the owner, and runs
//!   the same unstake path (lock, penalty, fee), so an allowance cannot be
//!   used to get around any of them.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! persistent storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::balance;
use crate::errors::VaultFeature6Error;
use crate::vault::VaultContractClient;
use crate::VaultContract;

const ALLOWANCE_KEY: Symbol = symbol_short!("wd_allow");

/// A spender's remaining withdrawal allowance on an owner's shares.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct WithdrawalAllowance {
    /// Shares the spender may still withdraw.
    pub shares: i128,
    /// Ledger at which the allowance is no longer valid.
    pub expires_at: u32,
}

fn key(owner: &Address, spender: &Address) -> (Symbol, Address, Address) {
    (ALLOWANCE_KEY, owner.clone(), spender.clone())
}

fn get_allowance(env: &Env, owner: &Address, spender: &Address) -> Option<WithdrawalAllowance> {
    env.storage().persistent().get(&key(owner, spender))
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Lets `spender` withdraw up to `shares` of `owner`'s shares until the
    /// ledger `expires_at`. Replaces any earlier allowance for the pair;
    /// `shares == 0` revokes it.
    ///
    /// # Authorization
    /// `owner` must sign.
    ///
    /// # Errors
    /// - `InvalidAmount`: `shares` is negative.
    /// - `InvalidExpiry`: `shares > 0` and `expires_at` is not after the
    ///   current ledger.
    ///
    /// # Events
    /// - `wd_apprv` on approval, `wd_revoke` on revocation.
    pub fn approve_withdrawal(
        env: Env,
        owner: Address,
        spender: Address,
        shares: i128,
        expires_at: u32,
    ) -> Result<(), VaultFeature6Error> {
        owner.require_auth();
        if shares < 0 {
            return Err(VaultFeature6Error::InvalidAmount);
        }
        if shares == 0 {
            env.storage().persistent().remove(&key(&owner, &spender));
            env.events()
                .publish((symbol_short!("wd_revoke"), owner, spender), ());
            return Ok(());
        }
        if expires_at <= env.ledger().sequence() {
            return Err(VaultFeature6Error::InvalidExpiry);
        }
        env.storage().persistent().set(
            &key(&owner, &spender),
            &WithdrawalAllowance { shares, expires_at },
        );
        env.events()
            .publish((symbol_short!("wd_apprv"), owner, spender), (shares, expires_at));
        Ok(())
    }

    /// Shares `spender` may still withdraw for `owner`. `0` when there is no
    /// allowance or it has expired.
    pub fn get_withdrawal_allowance(env: Env, owner: Address, spender: Address) -> i128 {
        match get_allowance(&env, &owner, &spender) {
            Some(allowance) if env.ledger().sequence() < allowance.expires_at => allowance.shares,
            _ => 0,
        }
    }

    /// Burns `shares` of `owner`'s position using `spender`'s allowance and
    /// pays the proceeds to `recipient`. Returns the amount paid out.
    ///
    /// The allowance is reduced by `shares`; the call reverts as a whole, so
    /// the allowance is untouched, if any later step fails.
    ///
    /// # Authorization
    /// `spender` must sign. `owner` and `recipient` do not.
    ///
    /// # Errors
    /// - `ZeroAmount`: `shares <= 0`.
    /// - `InsufficientAllowance`: no allowance, or it is below `shares`.
    /// - `AllowanceExpired`: the allowance is past its `expires_at`.
    /// - `InsufficientShares`: `owner` holds fewer than `shares`.
    /// - `BelowMinimumStake` / `WithdrawalLimitExceeded`: the owner's minimum
    ///   unstake or rolling daily limit would be broken.
    /// - Errors from the unstake path (`VaultPaused`, `UseCooldownFlow`,
    ///   `ArithmeticError`, ...).
    ///
    /// # Events
    /// The same `withdraw` (and `position_closed`) events as `unstake()`.
    pub fn withdraw_from(
        env: Env,
        spender: Address,
        owner: Address,
        shares: i128,
        recipient: Address,
    ) -> Result<i128, VaultFeature6Error> {
        spender.require_auth();
        if shares <= 0 {
            return Err(VaultFeature6Error::ZeroAmount);
        }

        let allowance = get_allowance(&env, &owner, &spender)
            .ok_or(VaultFeature6Error::InsufficientAllowance)?;
        if env.ledger().sequence() >= allowance.expires_at {
            return Err(VaultFeature6Error::AllowanceExpired);
        }
        if allowance.shares < shares {
            return Err(VaultFeature6Error::InsufficientAllowance);
        }
        let remaining = allowance.shares - shares;
        if remaining == 0 {
            env.storage().persistent().remove(&key(&owner, &spender));
        } else {
            env.storage().persistent().set(
                &key(&owner, &spender),
                &WithdrawalAllowance {
                    shares: remaining,
                    expires_at: allowance.expires_at,
                },
            );
        }

        let owner_shares = balance::get_shares(&env, &owner);
        if owner_shares < shares {
            return Err(VaultFeature6Error::InsufficientShares);
        }
        crate::minimum_unstake_amount::enforce_min_unstake(&env, shares, owner_shares)?;
        let projected = balance::shares_to_amount(
            balance::get_total_shares(&env),
            balance::get_total_deposited(&env),
            shares,
        )
        .ok_or(VaultFeature6Error::ArithmeticError)?;
        crate::vault::daily_withdrawal_limit::enforce_and_record(&env, &owner, projected)?;

        Ok(VaultContract::do_unstake_to(&env, &owner, shares, &recipient)?)
    }
}
