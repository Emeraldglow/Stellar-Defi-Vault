//! Stake-weighted tip jar (issue #354).
//!
//! Peer-to-peer tipping where active stakers can send small reward-token tips
//! to other stakers. Minimum tip scales with sender's position to prevent
//! dust spam: `max(1, sender_amount * MIN_TIP_BPS / 10_000)`.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap (`storage.rs:72-80`), so this
//! module uses raw `Symbol`-keyed persistent storage, matching `balance.rs`,
//! `partial_freeze.rs`, and `content_curation.rs`.

use soroban_sdk::{contractimpl, symbol_short, token, Address, Env, Symbol};

use crate::balance;
use crate::errors::VaultError;
use crate::storage::DataKey;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Minimum tip in basis points of sender's position (0.01% = 1 bps).
pub const MIN_TIP_BPS: u32 = 1;
const BPS_DENOM: i128 = 10_000;

/// Persistent-storage keys for lifetime totals (per-user).
const TIPS_RECV_KEY: Symbol = symbol_short!("tip_recv");
const TIPS_SENT_KEY: Symbol = symbol_short!("tip_sent");

fn get_tips_received(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&(TIPS_RECV_KEY, user.clone()))
        .unwrap_or(0)
}

fn get_tips_sent(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&(TIPS_SENT_KEY, user.clone()))
        .unwrap_or(0)
}

fn add_tips_received(env: &Env, user: &Address, amount: i128) {
    let cur = get_tips_received(env, user);
    let new = cur.checked_add(amount).unwrap_or(i128::MAX);
    env.storage()
        .persistent()
        .set(&(TIPS_RECV_KEY, user.clone()), &new);
}

fn add_tips_sent(env: &Env, user: &Address, amount: i128) {
    let cur = get_tips_sent(env, user);
    let new = cur.checked_add(amount).unwrap_or(i128::MAX);
    env.storage()
        .persistent()
        .set(&(TIPS_SENT_KEY, user.clone()), &new);
}

/// Resolve a user's current staked token amount from shares.
/// Returns `None` if no active position.
fn get_position_amount(env: &Env, user: &Address) -> Option<i128> {
    let shares = balance::get_shares(env, user);
    if shares == 0 {
        return None;
    }
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    balance::shares_to_amount(total_shares, total_deposited, shares)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Send a reward-token tip from `sender` to `recipient`.
    ///
    /// Both parties must have active staking positions. `amount` is
    /// transferred directly from `sender`'s wallet (not from their locked
    /// stake) in the reward token denomination.
    ///
    /// Minimum tip = `max(1, sender_position * MIN_TIP_BPS / 10_000)` to
    /// prevent dust spam. Reverts with `BelowMinimumStake` if below minimum,
    /// `PositionNotFound` if either party has no position, `InvalidAddress`
    /// if `sender == recipient`, `VaultPaused` if paused, `ContractStopped`
    /// if stopped, `ZeroAmount` if `amount <= 0`.
    pub fn send_tip(
        env: Env,
        sender: Address,
        recipient: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        sender.require_auth();

        if env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Err(VaultError::VaultPaused);
        }
        if env
            .storage()
            .instance()
            .has(&DataKey::Stopped)
            && env.storage().instance().get(&DataKey::Stopped).unwrap_or(false)
        {
            return Err(VaultError::ContractStopped);
        }
        if env.storage().instance().has(&symbol_short!("stopped"))
            && env
                .storage()
                .instance()
                .get(&symbol_short!("stopped"))
                .unwrap_or(false)
        {
            return Err(VaultError::ContractStopped);
        }

        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        if sender == recipient {
            return Err(VaultError::InvalidAddress);
        }

        let sender_amount =
            get_position_amount(&env, &sender).ok_or(VaultError::PositionNotFound)?;
        let _recipient_amount =
            get_position_amount(&env, &recipient).ok_or(VaultError::PositionNotFound)?;

        let min_tip = sender_amount
            .checked_mul(MIN_TIP_BPS as i128)
            .ok_or(VaultError::ArithmeticError)?
            .checked_div(BPS_DENOM)
            .ok_or(VaultError::ArithmeticError)?
            .max(1);

        if amount < min_tip {
            return Err(VaultError::BelowMinimumStake);
        }

        let reward_token = balance::get_reward_token(&env)
            .or_else(|| env.storage().instance().get(&DataKey::Token))
            .ok_or(VaultError::NotInitialized)?;

        let token_client = token::Client::new(&env, &reward_token);
        token_client.transfer(&sender, &recipient, &amount);

        add_tips_sent(&env, &sender, amount);
        add_tips_received(&env, &recipient, amount);

        env.events().publish(
            (symbol_short!("tip_sent"), sender.clone()),
            (recipient.clone(), amount, env.ledger().sequence()),
        );

        Ok(())
    }

    /// Read-only query for lifetime tip stats.
    ///
    /// Returns `(sent, received)` totals for `user`. Zero if no tips.
    pub fn get_tip_stats(env: Env, user: Address) -> (i128, i128) {
        (get_tips_sent(&env, &user), get_tips_received(&env, &user))
    }
}

















