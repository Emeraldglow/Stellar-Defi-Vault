//! Historical balance checkpoints and time-weighted average balance
//! (issues #709 and #710).
//!
//! Every change to a user's share balance (or to the pool's total shares) is
//! recorded as a [`Checkpoint`] from `balance::set_shares` /
//! `balance::set_total_shares`, the single choke point all stake, unstake and
//! share-moving paths write through. Each checkpoint stores the balance that
//! applies from its ledger onward plus the cumulative `balance * ledgers`
//! product up to that ledger, so one structure answers both:
//!
//! - `balance_at` / `total_supply_at`: binary search for the latest checkpoint
//!   at or before the requested ledger.
//! - `get_twab` / `get_total_twab`: the difference of the cumulative product at
//!   the window edges divided by the window length. A deposit made on the last
//!   ledger of a window contributes almost nothing to it.
//!
//! Balances are vault shares, the unit `balance::get_shares` reports.
//! `vote_weight_at` keeps reading its own bounded stake history; this module
//! does not replace it.
//!
//! # Compaction
//!
//! - A write in the same ledger as the latest checkpoint overwrites it.
//! - A write that leaves the balance unchanged records nothing.
//! - A list holds at most [`MAX_CHECKPOINTS`] entries; the oldest is dropped
//!   first. Both rules above are exact. The cap is not: `balance_at` reads zero
//!   for ledgers older than the oldest retained checkpoint. `cumulative` is
//!   absolute, so TWAB windows that lie inside the retained range stay exact,
//!   and a window that starts before it is rejected rather than overstated.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! persistent storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::errors::VaultError;
use crate::vault::{VaultContract, VaultContractClient};

/// Persistent key prefix for a user's checkpoints, keyed by `(USER_KEY, user)`.
const USER_KEY: Symbol = symbol_short!("ck_usr");

/// Persistent key for the pool-wide total-shares checkpoints.
const TOTAL_KEY: Symbol = symbol_short!("ck_tot");

/// Maximum checkpoints retained per list; the oldest is dropped past this.
pub const MAX_CHECKPOINTS: u32 = 100;

/// Balance that applies from `ledger` onward.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct Checkpoint {
    pub ledger: u32,
    pub balance: i128,
    /// Sum of `balance * ledgers` over all time up to `ledger`.
    pub cumulative: i128,
}

/// Apply a balance change at `ledger` to `history`. Returns whether it changed.
fn push(history: &mut Vec<Checkpoint>, ledger: u32, balance: i128) -> bool {
    let len = history.len();
    if len == 0 {
        if balance == 0 {
            return false;
        }
        history.push_back(Checkpoint {
            ledger,
            balance,
            cumulative: 0,
        });
        return true;
    }

    let last = history.get(len - 1).unwrap();
    if last.balance == balance {
        return false;
    }
    if last.ledger == ledger {
        history.set(
            len - 1,
            Checkpoint {
                ledger,
                balance,
                cumulative: last.cumulative,
            },
        );
    } else {
        let elapsed = ledger.saturating_sub(last.ledger) as i128;
        history.push_back(Checkpoint {
            ledger,
            balance,
            cumulative: last
                .cumulative
                .saturating_add(last.balance.saturating_mul(elapsed)),
        });
        if history.len() > MAX_CHECKPOINTS {
            let _ = history.pop_front();
        }
    }
    true
}

/// The latest checkpoint at or before `ledger`, by binary search.
fn find(history: &Vec<Checkpoint>, ledger: u32) -> Option<Checkpoint> {
    let mut low = 0u32;
    let mut high = history.len();
    while low < high {
        let mid = low + (high - low) / 2;
        if history.get(mid).unwrap().ledger <= ledger {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    if low == 0 {
        None
    } else {
        history.get(low - 1)
    }
}

fn balance_in(history: &Vec<Checkpoint>, ledger: u32) -> i128 {
    find(history, ledger).map(|c| c.balance).unwrap_or(0)
}

fn cumulative_in(history: &Vec<Checkpoint>, ledger: u32) -> i128 {
    match find(history, ledger) {
        Some(c) => c.cumulative.saturating_add(
            c.balance
                .saturating_mul(ledger.saturating_sub(c.ledger) as i128),
        ),
        None => 0,
    }
}

fn twab_in(
    env: &Env,
    history: &Vec<Checkpoint>,
    from_ledger: u32,
    to_ledger: u32,
) -> Result<i128, VaultError> {
    if to_ledger <= from_ledger {
        return Err(VaultError::ZeroAmount);
    }
    if to_ledger > env.ledger().sequence() {
        return Err(VaultError::EpochNotFinalized);
    }
    // The oldest checkpoint of an unpruned list starts at cumulative 0, so a
    // non-zero start means older history was dropped and this window's start
    // cannot be answered exactly.
    if find(history, from_ledger).is_none()
        && history.first().map_or(false, |c| c.cumulative != 0)
    {
        return Err(VaultError::EpochNotFinalized);
    }
    let accumulated =
        cumulative_in(history, to_ledger).saturating_sub(cumulative_in(history, from_ledger));
    Ok(accumulated / (to_ledger - from_ledger) as i128)
}

fn require_past(env: &Env, ledger: u32) -> Result<(), VaultError> {
    if ledger > env.ledger().sequence() {
        return Err(VaultError::EpochNotFinalized);
    }
    Ok(())
}

fn user_history(env: &Env, user: &Address) -> Vec<Checkpoint> {
    env.storage()
        .persistent()
        .get(&(USER_KEY, user.clone()))
        .unwrap_or(Vec::new(env))
}

fn total_history(env: &Env) -> Vec<Checkpoint> {
    env.storage()
        .persistent()
        .get(&TOTAL_KEY)
        .unwrap_or(Vec::new(env))
}

/// Record `user`'s new share balance. Called from `balance::set_shares`.
pub fn record_user(env: &Env, user: &Address, balance: i128) {
    let mut history = user_history(env, user);
    if push(&mut history, env.ledger().sequence(), balance) {
        env.storage()
            .persistent()
            .set(&(USER_KEY, user.clone()), &history);
    }
}

/// Record the pool's new total shares. Called from `balance::set_total_shares`.
pub fn record_total(env: &Env, total: i128) {
    let mut history = total_history(env);
    if push(&mut history, env.ledger().sequence(), total) {
        env.storage().persistent().set(&TOTAL_KEY, &history);
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Read-only: `user`'s share balance as of `ledger`.
    ///
    /// # Errors
    /// - `EpochNotFinalized`: `ledger` is in the future.
    pub fn balance_at(env: Env, user: Address, ledger: u32) -> Result<i128, VaultError> {
        require_past(&env, ledger)?;
        Ok(balance_in(&user_history(&env, &user), ledger))
    }

    /// Read-only: the pool's total shares as of `ledger`.
    ///
    /// # Errors
    /// - `EpochNotFinalized`: `ledger` is in the future.
    pub fn total_supply_at(env: Env, ledger: u32) -> Result<i128, VaultError> {
        require_past(&env, ledger)?;
        Ok(balance_in(&total_history(&env), ledger))
    }

    /// Read-only: `user`'s time-weighted average share balance over
    /// `[from_ledger, to_ledger)`.
    ///
    /// # Errors
    /// - `ZeroAmount`: `to_ledger <= from_ledger`.
    /// - `EpochNotFinalized`: `to_ledger` is in the future.
    pub fn get_twab(
        env: Env,
        user: Address,
        from_ledger: u32,
        to_ledger: u32,
    ) -> Result<i128, VaultError> {
        twab_in(&env, &user_history(&env, &user), from_ledger, to_ledger)
    }

    /// Read-only: the pool's time-weighted average total shares over
    /// `[from_ledger, to_ledger)`.
    ///
    /// # Errors
    /// Same as [`Self::get_twab`].
    pub fn get_total_twab(env: Env, from_ledger: u32, to_ledger: u32) -> Result<i128, VaultError> {
        twab_in(&env, &total_history(&env), from_ledger, to_ledger)
    }
}
