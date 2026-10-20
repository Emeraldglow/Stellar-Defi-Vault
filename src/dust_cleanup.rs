//! Dust position cleanup (issue #708).
//!
//! Abandoned micro-positions keep persistent storage entries alive for as long
//! as they exist. This lets a keeper close a position whose whole value is
//! below an admin-configured threshold and sweep it out of storage.
//!
//! # Consent
//!
//! No consent is required from the owner. To keep that safe, the position's
//! value is paid out in full (less the optional keeper tip) to the owner,
//! never to the keeper, unless the admin has configured a dust treasury, in
//! which case the payout goes there instead. Locked positions are skipped, and
//! accrued rewards are settled into the owner's claimable balance rather than
//! forfeited.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! instance storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, token, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultSafetyError;
use crate::events;
use crate::nft::StakeReceiptNFTClient;
use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient};

/// Instance key: positions worth less than this (token units) are dust.
/// `0` (default) disables dust cleanup.
const THRESHOLD_KEY: Symbol = symbol_short!("dust_thr");
/// Instance key: keeper tip in bps of the closed amount.
const TIP_BPS_KEY: Symbol = symbol_short!("dust_tip");
/// Instance key: optional treasury that receives dust instead of the owner.
const TREASURY_KEY: Symbol = symbol_short!("dust_trs");

/// Highest keeper tip the admin may configure: 10%.
pub const MAX_KEEPER_TIP_BPS: u32 = 1_000;

fn get_threshold(env: &Env) -> i128 {
    env.storage().instance().get(&THRESHOLD_KEY).unwrap_or(0)
}

#[contractimpl]
impl VaultContract {
    /// Admin: set the dust threshold in token units (issue #708). `0`
    /// disables dust cleanup.
    ///
    /// # Authorization
    /// `admin` must sign and be the stored admin.
    ///
    /// # Errors
    /// - `ZeroAmount`: `amount` is negative.
    /// - `Unauthorized` / `NotInitialized`: admin check failed.
    pub fn set_dust_threshold(
        env: Env,
        admin: Address,
        amount: i128,
    ) -> Result<(), VaultSafetyError> {
        admin::require_admin_as(&env, &admin)?;
        if amount < 0 {
            return Err(VaultSafetyError::ZeroAmount);
        }
        env.storage().instance().set(&THRESHOLD_KEY, &amount);
        Ok(())
    }

    /// Admin: set the keeper tip as basis points of the closed amount, capped
    /// at `MAX_KEEPER_TIP_BPS` (issue #708). `0` (default) pays no tip.
    ///
    /// # Authorization
    /// `admin` must sign and be the stored admin.
    ///
    /// # Errors
    /// - `InvalidTipBps`: `bps` exceeds the cap.
    /// - `Unauthorized` / `NotInitialized`: admin check failed.
    pub fn set_dust_keeper_tip_bps(
        env: Env,
        admin: Address,
        bps: u32,
    ) -> Result<(), VaultSafetyError> {
        admin::require_admin_as(&env, &admin)?;
        if bps > MAX_KEEPER_TIP_BPS {
            return Err(VaultSafetyError::InvalidTipBps);
        }
        env.storage().instance().set(&TIP_BPS_KEY, &bps);
        Ok(())
    }

    /// Admin: route closed dust to `treasury` instead of back to the owner,
    /// or pass `None` to return it to the owner again (issue #708).
    ///
    /// # Authorization
    /// `admin` must sign and be the stored admin.
    ///
    /// # Errors
    /// - `Unauthorized` / `NotInitialized`: admin check failed.
    pub fn set_dust_treasury(
        env: Env,
        admin: Address,
        treasury: Option<Address>,
    ) -> Result<(), VaultSafetyError> {
        admin::require_admin_as(&env, &admin)?;
        match treasury {
            Some(address) => env.storage().instance().set(&TREASURY_KEY, &address),
            None => env.storage().instance().remove(&TREASURY_KEY),
        }
        Ok(())
    }

    /// Read-only: the configured dust threshold (0 = disabled).
    pub fn get_dust_threshold(env: Env) -> i128 {
        get_threshold(&env)
    }

    /// Close `user`'s position if its whole value (primary plus split
    /// positions) is below the dust threshold, and return the amount paid to
    /// the owner (or the dust treasury) after the keeper tip (issue #708).
    /// All of the position's storage entries are removed.
    ///
    /// # Authorization
    /// `keeper` must sign. The owner's consent is not required.
    ///
    /// # Errors
    /// - `VaultPaused`: the pool is paused.
    /// - `DustThresholdNotSet`: no threshold is configured.
    /// - `PositionNotFound`: `user` holds no position.
    /// - `PositionLocked`: the position is under a voluntary lock.
    /// - `EscrowPending`: the position has escrowed rewards not yet claimed.
    /// - `NotDustPosition`: the position is at or above the threshold.
    /// - `ArithmeticError`: share conversion or fee overflow.
    ///
    /// # Events
    /// - `pos_clos` (`events::position_closed`) and `dust_clos` with the
    ///   amount paid out, the keeper tip and the ledger.
    pub fn close_dust_position(
        env: Env,
        keeper: Address,
        user: Address,
    ) -> Result<i128, VaultSafetyError> {
        keeper.require_auth();
        if Self::is_paused(env.clone()) {
            return Err(VaultSafetyError::VaultPaused);
        }
        let threshold = get_threshold(&env);
        if threshold <= 0 {
            return Err(VaultSafetyError::DustThresholdNotSet);
        }

        let primary = balance::get_shares(&env, &user);
        let splits = balance::get_split_positions(&env, &user);
        let mut user_shares = primary;
        for position in splits.iter() {
            user_shares = user_shares
                .checked_add(position.amount)
                .ok_or(VaultSafetyError::ArithmeticError)?;
        }
        if user_shares <= 0 {
            return Err(VaultSafetyError::PositionNotFound);
        }
        if Self::get_lock_status(env.clone(), user.clone()).is_some() {
            return Err(VaultSafetyError::PositionLocked);
        }
        // Escrowed rewards are released by a later claim; closing the position
        // would drop them, so wait until they have been claimed.
        if balance::get_escrow_balance(&env, &user) > 0 {
            return Err(VaultSafetyError::EscrowPending);
        }

        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let amount = balance::shares_to_amount(total_shares, total_deposited, user_shares)
            .ok_or(VaultSafetyError::ArithmeticError)?;
        if amount >= threshold {
            return Err(VaultSafetyError::NotDustPosition);
        }

        // Settle pending rewards into the owner's accrued (claimable) balance
        // so closing the position forfeits nothing.
        Self::accrue_rewards(&env, &user, primary)?;

        let tip_bps: u32 = env.storage().instance().get(&TIP_BPS_KEY).unwrap_or(0);
        let tip = amount
            .checked_mul(tip_bps as i128)
            .and_then(|v| v.checked_div(10_000))
            .ok_or(VaultSafetyError::ArithmeticError)?;
        let payout = amount - tip;
        let recipient: Address = env
            .storage()
            .instance()
            .get(&TREASURY_KEY)
            .unwrap_or(user.clone());

        // Remove the position from storage and the pool totals.
        balance::set_total_shares(&env, total_shares - user_shares);
        balance::set_total_deposited(&env, total_deposited - amount);
        env.storage()
            .persistent()
            .remove(&DataKey::ShareBalance(user.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::StakedAtLedger(user.clone()));
        if !splits.is_empty() {
            balance::set_split_positions(&env, &user, &Vec::new(&env));
        }
        balance::remove_escrow_release_ledger(&env, &user);
        balance::remove_escrow_balance(&env, &user);
        balance::set_last_unstake_ledger(&env, &user, env.ledger().sequence());
        balance::remove_restaked(&env, &user);
        balance::clear_position_insured(&env, &user);
        let total_stakers = balance::get_total_stakers(&env);
        if total_stakers > 0 {
            balance::set_total_stakers(&env, total_stakers - 1);
        }
        Self::remove_from_staker_list(&env, &user);
        Self::record_stake_snapshot(&env, &user, 0);
        Self::update_leaderboard(&env, &user, 0);
        if let Some(nft_addr) = balance::get_nft_contract(&env) {
            StakeReceiptNFTClient::new(&env, &nft_addr).burn(&user);
        }

        let token_addr = Self::token_address(&env)?;
        let token_client = token::Client::new(&env, &token_addr);
        if payout > 0 {
            token_client.transfer(&env.current_contract_address(), &recipient, &payout);
        }
        if tip > 0 {
            token_client.transfer(&env.current_contract_address(), &keeper, &tip);
        }

        events::position_closed(&env, &user);
        env.events().publish(
            (symbol_short!("dust_clos"), user),
            (payout, tip, env.ledger().sequence()),
        );
        Ok(payout)
    }
}
