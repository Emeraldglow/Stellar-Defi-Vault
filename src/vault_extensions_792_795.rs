//! Issues #792–#795: reward decimals, funder attribution, treasury rate
//! limit, and reward-surplus rescue.
//!
//! `DataKey` is at Soroban's 50-variant cap (see `storage.rs`), so everything
//! here uses raw `Symbol`-keyed storage, matching `balance.rs` and the other
//! extension modules.

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::{VaultError, VaultOpsError};
use crate::vault::VaultContract;

// ── Issue #792: reward-token decimal normalisation ───────────────────────────

/// Scale `amount` from `from_decimals` precision to `to_decimals` precision.
///
/// Upscaling multiplies by `10^(to - from)`. Downscaling divides by
/// `10^(from - to)`, truncating toward zero — the staker-favouring direction
/// for reward payouts (rounding down never over-pays). Documented in
/// `get_precision_info` and the module docs.
pub fn scale_decimals(amount: i128, from_decimals: u32, to_decimals: u32) -> Option<i128> {
    if from_decimals == to_decimals {
        return Some(amount);
    }
    let diff = from_decimals.abs_diff(to_decimals);
    if diff > 38 {
        return None;
    }
    let factor = 10i128.checked_pow(diff)?;
    if to_decimals > from_decimals {
        amount.checked_mul(factor)
    } else {
        // Downscaling: truncate toward zero (staker-favouring for rewards).
        Some(amount / factor)
    }
}

// ── Issue #793: funder attribution ───────────────────────────────────────────

const FUNDER_TOTAL_PREFIX: Symbol = symbol_short!("fnd_tot");

fn funder_key(funder: &Address) -> (Symbol, Address) {
    (FUNDER_TOTAL_PREFIX, funder.clone())
}

/// Read-only: total amount `funder` has contributed to the reward pool.
pub fn get_funder_total(env: &Env, funder: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&funder_key(funder))
        .unwrap_or(0)
}

/// Record `amount` contributed by `funder` and emit `reward_pool_funded`.
pub fn record_funder_contribution(
    env: &Env,
    funder: &Address,
    amount: i128,
    new_pool_balance: i128,
) {
    let key = funder_key(funder);
    let previous: i128 = env.storage().persistent().get(&key).unwrap_or(0);
    let updated = previous.saturating_add(amount);
    env.storage().persistent().set(&key, &updated);
    env.storage().persistent().extend_ttl(&key, 365 * 24 * 60 * 5, 365 * 24 * 60 * 10);

    // reward_pool_funded event
    env.events().publish(
        (symbol_short!("rwd_fund"), funder),
        (amount, new_pool_balance),
    );
}

// ── Issue #794: treasury withdrawal rate limit ───────────────────────────────

const TREASURY_LIMIT_AMOUNT_KEY: Symbol = symbol_short!("t_lim_amt");
const TREASURY_LIMIT_PERIOD_KEY: Symbol = symbol_short!("t_lim_prd");
const TREASURY_WITHDRAWN_KEY: Symbol = symbol_short!("t_wd_amt");
const TREASURY_WINDOW_START_KEY: Symbol = symbol_short!("t_wd_win");
const TREASURY_NOTICE_DELAY_KEY: Symbol = symbol_short!("t_lim_ndl");

/// Default notice delay for raising the limit: 7 days of ledgers.
const DEFAULT_NOTICE_DELAY: u32 = 60 * 60 * 24 * 7 / 5; // ~120,960 ledgers

/// Returns `(amount_per_period, period_ledgers)`. `(0, _)` means no limit.
pub fn get_treasury_limit(env: &Env) -> (i128, u32) {
    let amount: i128 = env
        .storage()
        .instance()
        .get(&TREASURY_LIMIT_AMOUNT_KEY)
        .unwrap_or(0);
    let period: u32 = env
        .storage()
        .instance()
        .get(&TREASURY_LIMIT_PERIOD_KEY)
        .unwrap_or(0);
    (amount, period)
}

fn get_treasury_withdrawn_in_window(env: &Env) -> (i128, u32) {
    let withdrawn: i128 = env
        .storage()
        .instance()
        .get(&TREASURY_WITHDRAWN_KEY)
        .unwrap_or(0);
    let window_start: u32 = env
        .storage()
        .instance()
        .get(&TREASURY_WINDOW_START_KEY)
        .unwrap_or(0);
    (withdrawn, window_start)
}

/// Reset the rolling window if the current period has elapsed.
/// Returns `(withdrawn_in_window, window_start_ledger)`.
pub fn maybe_rollover_window(env: &Env, period_ledgers: u32) -> (i128, u32) {
    let (withdrawn, window_start) = get_treasury_withdrawn_in_window(env);
    let now = env.ledger().sequence();
    if period_ledgers == 0 || now.saturating_sub(window_start) >= period_ledgers {
        env.storage().instance().set(&TREASURY_WITHDRAWN_KEY, &0i128);
        env.storage().instance().set(&TREASURY_WINDOW_START_KEY, &now);
        (0, now)
    } else {
        (withdrawn, window_start)
    }
}

// ── Issue #795: reward-surplus rescue ────────────────────────────────────────

const RUNWAY_RESERVE_KEY: Symbol = symbol_short!("rnwy_rsv");

/// Read-only: number of ledgers of emission to keep as runway reserve.
pub fn get_runway_reserve_ledgers(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&RUNWAY_RESERVE_KEY)
        .unwrap_or(0)
}

// ── Contract entrypoints ─────────────────────────────────────────────────────

#[contractimpl]
impl VaultContract {
    // ── Issue #792 ──────────────────────────────────────────────────────────

    /// Read-only: scale `amount` from stake-token to reward-token precision.
    /// Exposed for integrators to mirror the contract's internal normalisation.
    pub fn preview_reward_decimals_scale(env: Env, amount: i128) -> i128 {
        let stake_dec = balance::get_stake_decimals(&env);
        let reward_dec = balance::get_reward_decimals(&env);
        scale_decimals(amount, stake_dec, reward_dec).unwrap_or(amount)
    }

    // ── Issue #793 ──────────────────────────────────────────────────────────

    /// Read-only: total amount `funder` has contributed to the reward pool.
    pub fn get_funder_total(env: Env, funder: Address) -> i128 {
        get_funder_total(&env, &funder)
    }

    // ── Issue #794 ──────────────────────────────────────────────────────────

    /// Admin: set the treasury withdrawal rate limit.
    ///
    /// `amount_per_period` is the maximum that can be withdrawn per
    /// `period_ledgers` window. `amount_per_period == 0` disables the limit.
    /// Lowering takes effect immediately; raising is subject to a notice delay.
    pub fn set_treasury_withdrawal_limit(
        env: Env,
        admin_addr: Address,
        amount_per_period: i128,
        period_ledgers: u32,
    ) -> Result<(), VaultError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;
        if amount_per_period < 0 {
            return Err(VaultError::ZeroAmount);
        }
        if amount_per_period > 0 && period_ledgers == 0 {
            return Err(VaultError::ArithmeticError);
        }

        let (current_limit, _) = get_treasury_limit(&env);
        // Raising the limit is subject to a notice delay; lowering is immediate.
        if amount_per_period > current_limit && current_limit > 0 {
            let delay: u32 = env
                .storage()
                .instance()
                .get(&TREASURY_NOTICE_DELAY_KEY)
                .unwrap_or(DEFAULT_NOTICE_DELAY);
            let set_at: u32 = env
                .storage()
                .instance()
                .get(&symbol_short!("t_lim_sat"))
                .unwrap_or(0);
            if env.ledger().sequence() < set_at.saturating_add(delay) {
                return Err(VaultError::Unauthorized);
            }
        }

        env.storage()
            .instance()
            .set(&TREASURY_LIMIT_AMOUNT_KEY, &amount_per_period);
        env.storage()
            .instance()
            .set(&TREASURY_LIMIT_PERIOD_KEY, &period_ledgers);
        env.storage()
            .instance()
            .set(&symbol_short!("t_lim_sat"), &env.ledger().sequence());

        env.events().publish(
            (symbol_short!("t_lim_set"),),
            (amount_per_period, period_ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: how much can be withdrawn from the treasury right now
    /// without hitting the rate limit.
    pub fn get_treasury_available_now(env: Env) -> i128 {
        let (limit, period) = get_treasury_limit(&env);
        if limit == 0 {
            return i128::MAX;
        }
        let (withdrawn, _) = maybe_rollover_window(&env, period);
        (limit - withdrawn).max(0)
    }

    // ── Issue #795 ──────────────────────────────────────────────────────────

    /// Admin: set the runway reserve (ledgers of emission to keep).
    pub fn set_runway_reserve(env: Env, admin_addr: Address, ledgers: u32) -> Result<(), VaultError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;
        env.storage().instance().set(&RUNWAY_RESERVE_KEY, &ledgers);
        env.events().publish(
            (symbol_short!("rnwy_set"),),
            (ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: the currently recoverable reward surplus
    /// (`reward_pool - accrued_unclaimed - runway_reserve_value`).
    /// Returns 0 when there is no surplus.
    pub fn get_recoverable_reward_surplus(env: Env) -> i128 {
        let pool = balance::get_reward_pool_balance(&env);
        let accrued = balance::get_total_accrued_unclaimed(&env);
        let reserve_ledgers = get_runway_reserve_ledgers(&env);

        // Runway reserve: reward emission per ledger * reserve_ledgers.
        let reserve_value = if reserve_ledgers > 0 {
            let rate_bps = balance::get_reward_rate_bps(&env);
            let total_deposited = balance::get_total_deposited(&env);
            // Per-ledger emission ≈ total_deposited * rate_bps / 10_000 / ledgers_per_year
            let per_ledger = total_deposited
                .saturating_mul(rate_bps as i128)
                .saturating_div(10_000)
                .saturating_div(crate::vault::STELLAR_LEDGERS_PER_YEAR as i128);
            per_ledger.saturating_mul(reserve_ledgers as i128)
        } else {
            0
        };

        pool.saturating_sub(accrued).saturating_sub(reserve_value)
    }

    /// Admin: recover only the provable surplus from the reward pool
    /// (`balance - accrued_unclaimed - runway_reserve`).
    ///
    /// Reverts with `NoSurplus` when the computed surplus is zero or negative.
    pub fn recover_reward_surplus(
        env: Env,
        admin_addr: Address,
        amount: i128,
        to: Address,
    ) -> Result<(), VaultOpsError> {
        admin_addr.require_auth();
        admin::require_admin(&env).map_err(|_| VaultOpsError::Unauthorized)?;

        if amount <= 0 {
            return Err(VaultOpsError::ZeroAmount);
        }

        let surplus = Self::get_recoverable_reward_surplus(env.clone());
        if surplus <= 0 || amount > surplus {
            return Err(VaultOpsError::NoSurplus);
        }

        // Transfer reward tokens to `to`.
        let token_addr: Address = match balance::get_reward_token(&env) {
            Some(t) => t,
            None => env
                .storage()
                .instance()
                .get(&crate::storage::DataKey::Token)
                .ok_or(VaultOpsError::NotInitialized)?,
        };
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &to,
            &amount,
        );

        // Debit the reward pool.
        let pool = balance::get_reward_pool_balance(&env);
        balance::set_reward_pool_balance(&env, pool.saturating_sub(amount));

        env.events().publish(
            (symbol_short!("rwd_surp"), &admin_addr, &to),
            (amount, env.ledger().sequence()),
        );
        Ok(())
    }
}
