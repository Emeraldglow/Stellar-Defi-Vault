//! Position price oracle (issue #290).
//!
//! Publishes the fair value of a staking position â€” principal plus accrued
//! rewards â€” as an on-chain reference price, so a secondary market trading
//! stake receipts or debt NFTs has something to quote against.
//!
//! The price is **informational only**. Nothing in this module feeds back into
//! staking, reward, or withdrawal logic; publishing a price can never change
//! what a position is actually worth on redemption.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so history is kept under raw
//! `Symbol`-keyed persistent storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;
use crate::vesting_cliff;

/// Most recent price snapshots retained per user. Older entries roll off.
pub const MAX_PRICE_HISTORY: u32 = 10;

/// Maximum positions priceable in a single `bulk_publish_prices` call.
///
/// Caps the work one transaction can schedule, so a large `users` vector
/// cannot push the call past the ledger's resource limits and fail wholesale.
pub const MAX_BULK_USERS: u32 = 20;

/// Persistent-storage key prefix for a user's rolling price history.
const HISTORY_KEY: Symbol = symbol_short!("pos_prc");

/// A published fair-value snapshot for one position.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PositionPrice {
    pub user: Address,
    pub principal: i128,
    pub pending_reward: i128,
    pub fair_value: i128,
    pub published_at: u32,
}

/// Read a user's price history, oldest first.
pub fn history_for(env: &Env, user: &Address) -> Vec<PositionPrice> {
    env.storage()
        .persistent()
        .get(&(HISTORY_KEY, user.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

/// Build a snapshot for `user` at the current ledger.
///
/// `fair_value` is the plain sum of principal and pending reward, as the issue
/// specifies â€” no discounting for lock-up, cliff, or early-exit penalty. A
/// consumer pricing a position for sale should apply its own haircut.
///
/// The pending-reward figure passes through the vesting cliff, so a position
/// still inside its cliff prices at principal alone rather than advertising
/// rewards that would not yet be payable.
fn snapshot(env: &Env, user: &Address) -> Result<PositionPrice, VaultError> {
    let principal = balance::get_shares(env, user);
    let raw_reward = balance::get_accrued_reward(env, user);
    let pending_reward = vesting_cliff::apply_cliff(env, user, raw_reward);

    let fair_value = principal
        .checked_add(pending_reward)
        .ok_or(VaultError::ArithmeticError)?;

    Ok(PositionPrice {
        user: user.clone(),
        principal,
        pending_reward,
        fair_value,
        published_at: env.ledger().sequence(),
    })
}

/// Append a snapshot to `user`'s history, rolling the oldest entry off once
/// the cap is reached, and emit the published event.
fn record(env: &Env, price: &PositionPrice) {
    let mut history = history_for(env, &price.user);

    // Roll before pushing, so the stored vector never momentarily exceeds the
    // cap â€” a Vec that grows unbounded is how per-user storage turns into an
    // unpayable archival bill.
    while history.len() >= MAX_PRICE_HISTORY {
        history.remove(0);
    }
    history.push_back(price.clone());

    env.storage()
        .persistent()
        .set(&(HISTORY_KEY, price.user.clone()), &history);

    env.events().publish(
        (symbol_short!("pos_price"), price.user.clone()),
        (price.fair_value, price.published_at),
    );
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Publish a fair-value snapshot for one position. Admin only.
    pub fn publish_position_price(env: Env, user: Address) -> Result<PositionPrice, VaultError> {
        admin::require_admin(&env)?;

        let price = snapshot(&env, &user)?;
        record(&env, &price);
        Ok(price)
    }

    /// Publish snapshots for up to [`MAX_BULK_USERS`] positions in one call.
    ///
    /// Rejects an oversized batch outright rather than silently pricing a
    /// prefix, so a caller cannot believe it published more than it did.
    pub fn bulk_publish_prices(env: Env, users: Vec<Address>) -> Result<u32, VaultError> {
        admin::require_admin(&env)?;

        if users.len() > MAX_BULK_USERS {
            return Err(VaultError::TooManyActiveUsers);
        }

        let mut published = 0u32;
        for user in users.iter() {
            let price = snapshot(&env, &user)?;
            record(&env, &price);
            published += 1;
        }
        Ok(published)
    }

    /// The most recent published price for `user`, if any.
    pub fn get_latest_position_price(env: Env, user: Address) -> Option<PositionPrice> {
        let history = history_for(&env, &user);
        history.get(history.len().saturating_sub(1))
    }

    /// A user's full retained price history, oldest first.
    pub fn get_position_price_history(env: Env, user: Address) -> Vec<PositionPrice> {
        history_for(&env, &user)
    }
}















