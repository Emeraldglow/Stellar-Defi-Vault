//! Vault Extensions for Issues #538, #539, #540, #541:
//! - Issue #538: Read-only contract version query via `get_contract_version()`
//! - Issue #539: Per-token-class fee override for multi-token pools
//! - Issue #540: Scheduled reward-rate ramp via `set_rate_ramp()`
//! - Issue #541: Deposit receipt memo support via `deposit_with_memo()`

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::vault::{VaultContract, VaultContractClient, CONTRACT_VERSION};

const RAMP_KEY: Symbol = symbol_short!("rate_rmp");
const FEE_OVERRIDE_KEY: Symbol = symbol_short!("tok_fee");

/// Configuration for a scheduled reward-rate ramp (issue #540).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RateRamp {
    pub start_rate_bps: u32,
    pub target_rate_bps: u32,
    pub start_ledger: u32,
    pub duration_ledgers: u32,
}

/// Token-specific fee override layered on top of pool-wide defaults (issue #539).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TokenFeeOverride {
    pub deposit_fee_bps: Option<u32>,
    pub unstake_fee_bps: Option<u32>,
}

// ── Rate ramp storage helpers (issue #540) ──────────────────────────────────

pub fn get_rate_ramp(env: &Env) -> Option<RateRamp> {
    env.storage().instance().get(&RAMP_KEY)
}

pub fn set_rate_ramp_storage(env: &Env, ramp: &RateRamp) {
    env.storage().instance().set(&RAMP_KEY, ramp);
}

pub fn clear_rate_ramp(env: &Env) {
    env.storage().instance().remove(&RAMP_KEY);
}

/// Compute the effective reward rate at the current ledger.
/// If a ramp is active, calculates linear interpolation between start and target.
/// If no ramp is active, returns the pool's base reward rate.
pub fn compute_current_rate(env: &Env) -> u32 {
    if let Some(ramp) = get_rate_ramp(env) {
        let current_ledger = env.ledger().sequence();
        if current_ledger <= ramp.start_ledger {
            return ramp.start_rate_bps;
        }
        let elapsed = current_ledger.saturating_sub(ramp.start_ledger) as u64;
        let duration = ramp.duration_ledgers as u64;
        if crate::ledger_boundary::duration_elapsed(current_ledger, ramp.start_ledger, ramp.duration_ledgers) {
            return ramp.target_rate_bps;
        }
        if ramp.target_rate_bps >= ramp.start_rate_bps {
            let diff = (ramp.target_rate_bps - ramp.start_rate_bps) as u64;
            let delta = (diff * elapsed) / duration;
            ramp.start_rate_bps + (delta as u32)
        } else {
            let diff = (ramp.start_rate_bps - ramp.target_rate_bps) as u64;
            let delta = (diff * elapsed) / duration;
            ramp.start_rate_bps - (delta as u32)
        }
    } else {
        balance::get_reward_rate_bps(env)
    }
}

// ── Per-token fee override storage helpers (issue #539) ──────────────────────

fn token_fee_key(token: &Address) -> (Symbol, Address) {
    (FEE_OVERRIDE_KEY, token.clone())
}

pub fn get_token_fee_override_storage(env: &Env, token: &Address) -> Option<TokenFeeOverride> {
    env.storage().persistent().get(&token_fee_key(token))
}

pub fn set_token_fee_override_storage(env: &Env, token: &Address, fee_override: &TokenFeeOverride) {
    env.storage().persistent().set(&token_fee_key(token), fee_override);
}

pub fn clear_token_fee_override_storage(env: &Env, token: &Address) {
    env.storage().persistent().remove(&token_fee_key(token));
}

/// Effective unstake fee for `token`: returns the token-specific override if set,
/// otherwise falls back to the pool-wide default.
pub fn get_effective_unstake_fee_bps(env: &Env, token: &Address) -> u32 {
    // Issue #703: no fee is charged during a fee holiday.
    if crate::vault_extensions_701_704::is_fee_holiday_active(env) {
        return 0;
    }
    if let Some(override_entry) = get_token_fee_override_storage(env, token) {
        if let Some(bps) = override_entry.unstake_fee_bps {
            return bps;
        }
    }
    balance::get_unstake_fee_bps(env)
}

/// Effective deposit fee for `token`: returns the token-specific override if set,
/// otherwise returns 0 (the global default deposit fee).
pub fn get_effective_deposit_fee_bps(env: &Env, token: &Address) -> u32 {
    // Issue #703: no fee is charged during a fee holiday.
    if crate::vault_extensions_701_704::is_fee_holiday_active(env) {
        return 0;
    }
    if let Some(override_entry) = get_token_fee_override_storage(env, token) {
        if let Some(bps) = override_entry.deposit_fee_bps {
            return bps;
        }
    }
    0
}

// ── Contract Implementation ──────────────────────────────────────────────────

#[contractimpl]
impl VaultContract {
    // ── Issue #541: Deposit receipt memo support ─────────────────────────────

    /// Deposit tokens into the vault with an optional reference memo (issue #541).
    /// The memo is capped at 64 bytes and included in the emitted `deposit_completed`
    /// event without being stored in persistent state.
    pub fn deposit_with_memo(
        env: Env,
        depositor: Address,
        amount: i128,
        memo: String,
    ) -> Result<i128, VaultError> {
        depositor.require_auth();
        if memo.len() > 64 {
            return Err(VaultError::MessageTooLong);
        }
        let shares_minted = VaultContract::stake(env.clone(), depositor.clone(), amount)?;
        events::deposit_completed(
            &env,
            &depositor,
            amount,
            shares_minted,
            &memo,
            env.ledger().sequence(),
        );
        Ok(shares_minted)
    }

    // ── Issue #540: Scheduled reward-rate ramp ───────────────────────────────

    /// Admin: begin a linear reward-rate interpolation from current rate to
    /// `target_bps` over `ramp_duration_ledgers` (issue #540).
    pub fn set_rate_ramp(
        env: Env,
        admin: Address,
        target_bps: u32,
        ramp_duration_ledgers: u32,
    ) -> Result<(), VaultError> {
        let stored_admin = admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        if ramp_duration_ledgers == 0 {
            return Err(VaultError::InvalidRate);
        }
        if target_bps > balance::MAX_RATE_BPS {
            return Err(VaultError::RateTooHigh);
        }

        let start_rate_bps = compute_current_rate(&env);
        let start_ledger = env.ledger().sequence();

        let ramp = RateRamp {
            start_rate_bps,
            target_rate_bps: target_bps,
            start_ledger,
            duration_ledgers: ramp_duration_ledgers,
        };
        set_rate_ramp_storage(&env, &ramp);

        events::rate_ramp_started(
            &env,
            start_rate_bps,
            target_bps,
            ramp_duration_ledgers,
            start_ledger,
        );

        Ok(())
    }

    /// Read-only query: returns the current rate, interpolating linearly if a
    /// rate ramp is currently active (issue #540).
    pub fn get_current_rate(env: Env) -> u32 {
        compute_current_rate(&env)
    }

    /// Admin: cancel an active rate ramp, freezing the rate at its current
    /// interpolated value (issue #540).
    pub fn cancel_rate_ramp(env: Env, admin: Address) -> Result<(), VaultError> {
        let stored_admin = admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        if let Some(_) = get_rate_ramp(&env) {
            let frozen_rate = compute_current_rate(&env);
            balance::set_reward_rate_bps(&env, frozen_rate);
            clear_rate_ramp(&env);
            Ok(())
        } else {
            Err(VaultError::NoCampaignActive)
        }
    }

    /// Complete an active rate ramp once its duration has elapsed, settling the
    /// rate to `target_bps` permanently and emitting `rate_ramp_completed` (issue #540).
    pub fn complete_rate_ramp(env: Env) -> Result<u32, VaultError> {
        let ramp = get_rate_ramp(&env).ok_or(VaultError::NoCampaignActive)?;
        let current_ledger = env.ledger().sequence();
        if !crate::ledger_boundary::duration_elapsed(current_ledger, ramp.start_ledger, ramp.duration_ledgers) {
            return Err(VaultError::InvalidRate);
        }
        let target = ramp.target_rate_bps;
        balance::set_reward_rate_bps(&env, target);
        clear_rate_ramp(&env);
        events::rate_ramp_completed(&env, target, current_ledger);
        Ok(target)
    }

    // ── Issue #539: Per-token fee override ───────────────────────────────────

    /// Admin: set or clear a token-specific fee override (issue #539).
    /// If both `deposit_fee_bps` and `unstake_fee_bps` are `None`, the override
    /// is cleared and default fees apply.
    pub fn set_token_fee_override(
        env: Env,
        admin: Address,
        token: Address,
        deposit_fee_bps: Option<u32>,
        unstake_fee_bps: Option<u32>,
    ) -> Result<(), VaultError> {
        let stored_admin = admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        if let Some(bps) = unstake_fee_bps {
            if bps > 500 {
                return Err(VaultError::UnstakeFeeTooHigh);
            }
        }
        if let Some(bps) = deposit_fee_bps {
            if bps > 10_000 {
                return Err(VaultError::InvalidPenaltyBps);
            }
        }

        if deposit_fee_bps.is_none() && unstake_fee_bps.is_none() {
            clear_token_fee_override_storage(&env, &token);
        } else {
            let entry = TokenFeeOverride {
                deposit_fee_bps,
                unstake_fee_bps,
            };
            set_token_fee_override_storage(&env, &token, &entry);
        }

        events::token_fee_override_set(
            &env,
            &token,
            deposit_fee_bps,
            unstake_fee_bps,
            env.ledger().sequence(),
        );

        Ok(())
    }

    /// Read-only query: return the configured fee override for `token` as
    /// `(Option<deposit_fee_bps>, Option<unstake_fee_bps>)` (issue #539).
    pub fn get_token_fee_override(
        env: Env,
        token: Address,
    ) -> (Option<u32>, Option<u32>) {
        if let Some(entry) = get_token_fee_override_storage(&env, &token) {
            (entry.deposit_fee_bps, entry.unstake_fee_bps)
        } else {
            (None, None)
        }
    }

    /// Read-only query: effective unstake fee bps for `token`, checking token override
    /// before falling back to the global default (issue #539).
    pub fn get_effective_unstake_fee_bps(env: Env, token: Address) -> u32 {
        get_effective_unstake_fee_bps(&env, &token)
    }

    /// Read-only query: effective deposit fee bps for `token`, checking token override
    /// before falling back to the global default (issue #539).
    pub fn get_effective_deposit_fee_bps(env: Env, token: Address) -> u32 {
        get_effective_deposit_fee_bps(&env, &token)
    }

    // ── Issue #538: Read-only contract version query ─────────────────────────

    /// Read-only contract version query returning the build-time `CONTRACT_VERSION` (issue #538).
    pub fn get_contract_version(env: Env) -> String {
        String::from_str(&env, CONTRACT_VERSION)
    }
}
