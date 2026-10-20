//! Automatic fee recycling into the reward pool (issue #754).
//!
//! The admin can route a percentage of every collected protocol fee back into
//! the reward pool, so fee revenue extends the reward runway without a manual
//! `fund_reward_pool` call. `0` disables recycling and preserves the existing
//! routing exactly.
//!
//! # Ordering
//!
//! Fee collection is applied in this order, and callers must keep it:
//!
//! 1. The gross fee is collected.
//! 2. The insurance share comes off the top
//!    (`set_insurance_fee_share_bps` / `take_insurance_share`).
//! 3. The recycle share is taken from the post-insurance remainder
//!    ([`recycle_fee`]) and credited to the reward pool.
//! 4. Revenue-share is taken from what is left.
//! 5. The remainder is routed by the caller: treasury-split recipients when a
//!    split is configured, otherwise the fee recipients, otherwise the reward
//!    pool.
//!
//! Recycling is deliberately taken *before* treasury routing, so
//! `set_treasury_split` and the fee-recipient fallback only ever see the
//! post-recycle amount. Insurance is taken first, so its configured share is
//! still calculated on the gross fee and is unaffected by the recycle setting.
//!
//! With `fee_recycle_bps == 0` (the default) this module is a no-op and every
//! downstream route receives exactly the amount it did before.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this module uses a raw
//! `Symbol`-keyed instance entry, matching `balance.rs`:
//!
//! - `fee_rcyc` -> `u32`

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::vault::{VaultContract, VaultContractArgs, VaultContractClient};

/// Instance key holding the recycle share in basis points.
const FEE_RECYCLE_BPS_KEY: Symbol = symbol_short!("fee_rcyc");

/// Basis-point denominator used for the recycle share.
const BPS_DENOMINATOR: i128 = 10_000;

/// Highest recycle share the admin may configure (100%).
pub const MAX_FEE_RECYCLE_BPS: u32 = 10_000;

/// Current recycle share in basis points (`0` = disabled).
pub fn get_fee_recycle_bps_internal(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&FEE_RECYCLE_BPS_KEY)
        .unwrap_or(0)
}

/// Move the configured share of `fee` into the reward pool and return the
/// amount recycled.
///
/// `fee` is the amount left after the insurance share has been taken. The
/// computed share is floored, so a zero result leaves the pool untouched. The
/// `fees_recycled` event carries the recycled amount and the new pool balance.
pub(crate) fn recycle_fee(env: &Env, fee: i128) -> Result<i128, VaultError> {
    let bps = get_fee_recycle_bps_internal(env);
    if bps == 0 || fee <= 0 {
        return Ok(0);
    }

    let recycled = fee
        .checked_mul(bps as i128)
        .and_then(|value| value.checked_div(BPS_DENOMINATOR))
        .ok_or(VaultError::ArithmeticError)?;
    if recycled <= 0 {
        return Ok(0);
    }

    let new_pool_balance = balance::get_reward_pool_balance(env)
        .checked_add(recycled)
        .ok_or(VaultError::ArithmeticError)?;
    balance::set_reward_pool_balance(env, new_pool_balance);

    env.events().publish(
        (Symbol::new(env, "fees_recycled"),),
        (recycled, new_pool_balance),
    );

    Ok(recycled)
}

#[contractimpl]
impl VaultContract {
    /// Admin: set the share of collected fees recycled into the reward pool.
    ///
    /// `bps` is in basis points (`5_000` = 50%) and must not exceed
    /// [`MAX_FEE_RECYCLE_BPS`]. `0` disables recycling and preserves the
    /// previous fee routing.
    ///
    /// # Authorization
    /// `admin` must sign and be the configured admin.
    ///
    /// # Errors
    /// - `InvalidRate`: `bps` is above [`MAX_FEE_RECYCLE_BPS`].
    /// - `Unauthorized` / `NotInitialized`: admin check failed.
    pub fn set_fee_recycle_bps(env: Env, admin: Address, bps: u32) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        if bps > MAX_FEE_RECYCLE_BPS {
            return Err(VaultError::InvalidRate);
        }

        env.storage().instance().set(&FEE_RECYCLE_BPS_KEY, &bps);
        env.events()
            .publish((symbol_short!("frc_set"),), (bps, env.ledger().sequence()));
        Ok(())
    }

    /// Read-only: the configured fee-recycle share in basis points.
    pub fn get_fee_recycle_bps(env: Env) -> u32 {
        get_fee_recycle_bps_internal(&env)
    }

    /// Read-only: split `fee` into `(recycled, remainder)` under the current
    /// configuration, without touching storage. Negative input is treated as
    /// zero. Useful for off-chain previews and for documenting the ordering
    /// described in the module docs.
    pub fn preview_fee_recycle(env: Env, fee: i128) -> (i128, i128) {
        if fee <= 0 {
            return (0, 0);
        }
        let bps = get_fee_recycle_bps_internal(&env);
        if bps == 0 {
            return (0, fee);
        }
        let recycled = fee
            .checked_mul(bps as i128)
            .and_then(|value| value.checked_div(BPS_DENOMINATOR))
            .unwrap_or(0);
        let recycled = recycled.clamp(0, fee);
        (recycled, fee - recycled)
    }
}
