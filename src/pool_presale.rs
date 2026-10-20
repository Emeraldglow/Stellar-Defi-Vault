//! Pool pre-sale reserved staking spots (issue #369).
//!
//! Lets the admin open a pre-sale window before the pool is generally
//! available. During the window, buyers pay a non-refundable reservation fee
//! (in the stake token, paid straight to the admin) to reserve a spot up to
//! some amount. Once the pre-sale's `opens_at` ledger is reached, every
//! reservation holder can redeem their spot by staking the reserved amount â€”
//! guaranteed, regardless of any pool cap or waitlist position, since
//! `redeem_presale_reservation` mints shares directly rather than routing
//! through a capped entrypoint.
//!
//! # Wiring
//!
//! Like `epoch_reward_cap.rs` and `compound_optimizer.rs`, this exposes its
//! own entrypoints rather than editing `vault.rs`'s existing `stake()`.
//! `redeem_presale_reservation` mirrors `stake()`'s share-minting math itself
//! (transfer-in, then mint shares at the current share price) since that is
//! the one piece of `stake()`'s behavior a guaranteed reservation needs to
//! reuse, and it deliberately skips any cap/whitelist checks â€” that's the
//! entire point of a reserved spot.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance key: the active (or most recently configured) presale.
const CONFIG_KEY: Symbol = symbol_short!("ps_cfg");
/// Persistent key prefix for a buyer's reservation. Keyed by `(RESERVATION_KEY, buyer)`.
const RESERVATION_KEY: Symbol = symbol_short!("ps_res");

/// Admin-configured pre-sale terms (issue #369).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PresaleConfig {
    pub reservation_fee_bps: u32,
    pub max_reservation_per_user: i128,
    pub total_reservation_cap: i128,
    pub total_reserved: i128,
    pub opens_at: u32,
    pub active: bool,
}

/// A buyer's reserved spot (issue #369).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PresaleReservation {
    pub reserved_amount: i128,
    pub fee_paid: i128,
    pub redeemed: bool,
}

fn get_config(env: &Env) -> Option<PresaleConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

fn set_config(env: &Env, config: &PresaleConfig) {
    env.storage().instance().set(&CONFIG_KEY, config);
}

fn get_reservation(env: &Env, buyer: &Address) -> Option<PresaleReservation> {
    env.storage()
        .persistent()
        .get(&(RESERVATION_KEY, buyer.clone()))
}

fn set_reservation(env: &Env, buyer: &Address, reservation: &PresaleReservation) {
    env.storage()
        .persistent()
        .set(&(RESERVATION_KEY, buyer.clone()), reservation);
}

fn token_address(env: &Env) -> Result<Address, VaultError> {
    env.storage()
        .instance()
        .get(&crate::storage::DataKey::Token)
        .ok_or(VaultError::NotInitialized)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Open a pre-sale window. Admin only.
    ///
    /// `opens_at` is the ledger at which the pool is considered officially
    /// open for this pre-sale's purposes: reservations are only accepted
    /// before it, and redemptions are only allowed at or after it.
    pub fn start_presale(
        env: Env,
        reservation_fee_bps: u32,
        max_reservation_per_user: i128,
        total_reservation_cap: i128,
        opens_at: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if let Some(existing) = crate::pool_presale::get_config(&env) {
            if existing.active {
                return Err(VaultError::InvalidRate);
            }
        }
        if reservation_fee_bps > 10_000 {
            return Err(VaultError::InvalidRate);
        }
        if max_reservation_per_user <= 0 || total_reservation_cap <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        if opens_at <= env.ledger().sequence() {
            return Err(VaultError::ZeroAmount);
        }

        crate::pool_presale::set_config(
            &env,
            &PresaleConfig {
                reservation_fee_bps,
                max_reservation_per_user,
                total_reservation_cap,
                total_reserved: 0,
                opens_at,
                active: true,
            },
        );

        env.events().publish(
            (symbol_short!("ps_start"),),
            (
                reservation_fee_bps,
                total_reservation_cap,
                opens_at,
                env.ledger().sequence(),
            ),
        );
        Ok(())
    }

    /// Cancel the active pre-sale. Admin only. Existing reservations are left
    /// in place (queryable) but can no longer be redeemed or extended, since
    /// `active` gates both `reserve_presale_spot` and
    /// `redeem_presale_reservation`.
    pub fn cancel_presale(env: Env) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let mut config =
            crate::pool_presale::get_config(&env).ok_or(VaultError::InvalidRate)?;
        config.active = false;
        crate::pool_presale::set_config(&env, &config);

        env.events()
            .publish((symbol_short!("ps_cncl"),), env.ledger().sequence());
        Ok(())
    }

    /// The current (or most recently configured) pre-sale terms.
    pub fn get_presale_config(env: Env) -> Option<PresaleConfig> {
        crate::pool_presale::get_config(&env)
    }

    /// A buyer's reservation state, if they have ever reserved a spot.
    pub fn get_presale_reservation(env: Env, buyer: Address) -> Option<PresaleReservation> {
        crate::pool_presale::get_reservation(&env, &buyer)
    }

    /// Reserve a staking spot up to `amount`, paying the configured
    /// reservation fee. Fee is paid immediately, in the stake token, straight
    /// to the admin â€” it is non-refundable even if the buyer never redeems.
    ///
    /// Calling this again before redeeming adds to the existing reservation
    /// (and pays a fee on the additional amount only), up to
    /// `max_reservation_per_user` in total.
    pub fn reserve_presale_spot(env: Env, buyer: Address, amount: i128) -> Result<i128, VaultError> {
        buyer.require_auth();

        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let mut config =
            crate::pool_presale::get_config(&env).ok_or(VaultError::InvalidRate)?;
        if !config.active {
            return Err(VaultError::InvalidRate);
        }
        if env.ledger().sequence() >= config.opens_at {
            return Err(VaultError::InvalidRate);
        }

        let mut reservation =
            crate::pool_presale::get_reservation(&env, &buyer).unwrap_or(PresaleReservation {
                reserved_amount: 0,
                fee_paid: 0,
                redeemed: false,
            });

        let new_reserved = reservation
            .reserved_amount
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticError)?;
        if new_reserved > config.max_reservation_per_user {
            return Err(VaultError::InvalidRate);
        }
        let new_total = config
            .total_reserved
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticError)?;
        if new_total > config.total_reservation_cap {
            return Err(VaultError::PoolCapReached);
        }

        let fee = amount
            .checked_mul(config.reservation_fee_bps as i128)
            .and_then(|v| v.checked_div(10_000))
            .ok_or(VaultError::ArithmeticError)?;

        if fee > 0 {
            let admin_addr = admin::get_admin(&env)?;
            let token = crate::pool_presale::token_address(&env)?;
            soroban_sdk::token::Client::new(&env, &token).transfer(&buyer, &admin_addr, &fee);
        }

        reservation.reserved_amount = new_reserved;
        reservation.fee_paid = reservation
            .fee_paid
            .checked_add(fee)
            .ok_or(VaultError::ArithmeticError)?;
        crate::pool_presale::set_reservation(&env, &buyer, &reservation);

        config.total_reserved = new_total;
        crate::pool_presale::set_config(&env, &config);

        env.events().publish(
            (symbol_short!("ps_resv"), buyer),
            (amount, fee, new_reserved, env.ledger().sequence()),
        );
        Ok(new_reserved)
    }

    /// Redeem a reserved spot once the pre-sale's `opens_at` ledger has been
    /// reached, staking the full reserved amount and minting shares at the
    /// current share price. Bypasses any pool cap or waitlist â€” the whole
    /// point of a reservation is that it is honored unconditionally.
    ///
    /// Returns the number of shares minted.
    pub fn redeem_presale_reservation(env: Env, buyer: Address) -> Result<i128, VaultError> {
        buyer.require_auth();

        let config = crate::pool_presale::get_config(&env).ok_or(VaultError::InvalidRate)?;
        if !config.active {
            return Err(VaultError::InvalidRate);
        }
        if env.ledger().sequence() < config.opens_at {
            return Err(VaultError::InvalidRate);
        }

        let mut reservation = crate::pool_presale::get_reservation(&env, &buyer)
            .ok_or(VaultError::InvalidRate)?;
        if reservation.redeemed {
            return Err(VaultError::InvalidRate);
        }
        if reservation.reserved_amount <= 0 {
            return Err(VaultError::InvalidRate);
        }

        let amount = reservation.reserved_amount;
        let token = crate::pool_presale::token_address(&env)?;
        soroban_sdk::token::Client::new(&env, &token).transfer(
            &buyer,
            &env.current_contract_address(),
            &amount,
        );

        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let shares_minted = if total_shares == 0 || total_deposited == 0 {
            amount
        } else {
            amount
                .checked_mul(total_shares)
                .and_then(|v| v.checked_div(total_deposited))
                .ok_or(VaultError::ArithmeticError)?
        };

        let current_shares = balance::get_shares(&env, &buyer);
        let is_new_staker = current_shares == 0;
        balance::set_shares(&env, &buyer, current_shares + shares_minted);
        balance::set_total_shares(&env, total_shares + shares_minted);
        balance::set_total_deposited(&env, total_deposited + amount);
        if is_new_staker {
            balance::set_total_stakers(&env, balance::get_total_stakers(&env) + 1);
        }

        reservation.redeemed = true;
        crate::pool_presale::set_reservation(&env, &buyer, &reservation);

        env.events().publish(
            (symbol_short!("ps_redm"), buyer),
            (amount, shares_minted, env.ledger().sequence()),
        );
        Ok(shares_minted)
    }
}





















