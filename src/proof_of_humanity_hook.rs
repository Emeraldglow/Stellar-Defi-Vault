//! Proof-of-humanity hook (issue #461).
//!
//! Integration hook for an external proof-of-humanity protocol (Proof of
//! Humanity, Worldcoin, BrightID, …). Verified human addresses stake under a
//! lower minimum with no fee surcharge; unverified addresses face a higher
//! minimum stake and a bps fee surcharge on top of existing fees. Keeps the
//! pool permissionless while raising the cost of Sybil farming.
//!
//! The oracle contract must implement `is_verified(address: Address) -> bool`.
//! When no oracle is configured, every address is treated as unverified
//! (conservative default). When the oracle is configured but the call fails,
//! `oracle_fallback_mode` decides: `Permissive` treats the address as verified,
//! `Restrictive` treats it as unverified.
//!
//! Enforcement is exposed through a dedicated `stake_verified()` entrypoint
//! (additive, leaving the base `stake()` untouched) that applies the correct
//! minimum and collects any surcharge into protocol fees before staking the
//! remainder.
//!
//! # Storage (`DataKey` is at Soroban's 50-variant cap — raw `Symbol` keys)
//!
//! - Config: `symbol_short!("hmn_cfg")` -> `HumanityVerificationConfig`
//! - Fallback mode: `symbol_short!("hmn_fbm")` -> `OracleFallbackMode`

use soroban_sdk::{
    contractclient, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
};

use crate::admin;
use crate::balance;
use crate::errors::VaultCampaignError;
use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient};

const CONFIG_KEY: Symbol = symbol_short!("hmn_cfg");
const FALLBACK_KEY: Symbol = symbol_short!("hmn_fbm");

/// Interface the external identity oracle must implement.
#[contractclient(name = "HumanityOracleClient")]
pub trait HumanityOracle {
    fn is_verified(env: Env, address: Address) -> bool;
}

/// How `is_verified_human()` behaves when the oracle call fails.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum OracleFallbackMode {
    /// Treat the address as verified (allow all).
    Permissive,
    /// Treat the address as unverified (conservative).
    Restrictive,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HumanityVerificationConfig {
    pub oracle: Address,
    pub verified_min_stake: i128,
    pub unverified_min_stake: i128,
    pub unverified_fee_surcharge_bps: u32,
}

/// Result of a humanity check, returned by `humanity_check()`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HumanityCheckResult {
    pub verified: bool,
    pub applied_min_stake: i128,
    pub fee_surcharge_bps: u32,
}

pub fn get_config(env: &Env) -> Option<HumanityVerificationConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

fn set_config(env: &Env, config: &HumanityVerificationConfig) {
    env.storage().instance().set(&CONFIG_KEY, config);
}

pub fn get_fallback_mode(env: &Env) -> OracleFallbackMode {
    env.storage()
        .instance()
        .get(&FALLBACK_KEY)
        .unwrap_or(OracleFallbackMode::Restrictive)
}

fn token_address(env: &Env) -> Result<Address, VaultCampaignError> {
    env.storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(VaultCampaignError::NotInitialized)
}

/// Queries the configured oracle for `user`. Returns `false` when no oracle is
/// configured; applies `oracle_fallback_mode` when the oracle call fails.
pub fn is_verified(env: &Env, user: &Address) -> bool {
    let config = match get_config(env) {
        Some(c) => c,
        None => return false,
    };
    let client = HumanityOracleClient::new(env, &config.oracle);
    match client.try_is_verified(user) {
        Ok(Ok(verified)) => verified,
        _ => matches!(get_fallback_mode(env), OracleFallbackMode::Permissive),
    }
}

/// The minimum stake that applies to `user` given their verification status.
/// `0` when no config is set (no restriction).
pub fn applied_min_stake(env: &Env, user: &Address) -> i128 {
    match get_config(env) {
        Some(config) => {
            if is_verified(env, user) {
                config.verified_min_stake
            } else {
                config.unverified_min_stake
            }
        }
        None => 0,
    }
}

/// The fee surcharge in bps that applies to `user`. `0` for verified addresses
/// and when no config is set.
pub fn fee_surcharge_bps(env: &Env, user: &Address) -> u32 {
    match get_config(env) {
        Some(config) => {
            if is_verified(env, user) {
                0
            } else {
                config.unverified_fee_surcharge_bps
            }
        }
        None => 0,
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #461: admin registers the external verification oracle. Preserves
    /// any previously configured thresholds; new registrations default them to
    /// zero (configure via `set_humanity_config`).
    pub fn set_humanity_oracle(
        env: Env,
        admin_addr: Address,
        oracle_contract: Address,
    ) -> Result<(), VaultCampaignError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        let config = match get_config(&env) {
            Some(mut c) => {
                c.oracle = oracle_contract.clone();
                c
            }
            None => HumanityVerificationConfig {
                oracle: oracle_contract.clone(),
                verified_min_stake: 0,
                unverified_min_stake: 0,
                unverified_fee_surcharge_bps: 0,
            },
        };
        set_config(&env, &config);

        env.events()
            .publish((symbol_short!("hmn_orc"),), oracle_contract);
        Ok(())
    }

    /// Issue #461: admin configures the verified/unverified minimum stakes and
    /// the unverified fee surcharge. Requires an oracle to have been registered.
    pub fn set_humanity_config(
        env: Env,
        admin_addr: Address,
        verified_min_stake: i128,
        unverified_min_stake: i128,
        unverified_fee_surcharge_bps: u32,
    ) -> Result<(), VaultCampaignError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        if verified_min_stake < 0 || unverified_min_stake < 0 {
            return Err(VaultCampaignError::InvalidHumanityConfig);
        }
        if unverified_fee_surcharge_bps > 10_000 {
            return Err(VaultCampaignError::InvalidHumanityConfig);
        }

        let mut config = get_config(&env).ok_or(VaultCampaignError::OracleNotConfigured)?;
        config.verified_min_stake = verified_min_stake;
        config.unverified_min_stake = unverified_min_stake;
        config.unverified_fee_surcharge_bps = unverified_fee_surcharge_bps;
        set_config(&env, &config);
        Ok(())
    }

    /// Issue #461: admin sets the behaviour used when the oracle call fails.
    pub fn set_oracle_fallback_mode(
        env: Env,
        admin_addr: Address,
        mode: OracleFallbackMode,
    ) -> Result<(), VaultCampaignError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;
        env.storage().instance().set(&FALLBACK_KEY, &mode);
        Ok(())
    }

    /// Issue #461: is `user` a verified human per the configured oracle?
    pub fn is_verified_human(env: Env, user: Address) -> bool {
        is_verified(&env, &user)
    }

    /// Issue #461: read the current verification config.
    pub fn get_humanity_config(env: Env) -> Option<HumanityVerificationConfig> {
        get_config(&env)
    }

    /// Issue #461: read the current oracle fallback mode.
    pub fn get_oracle_fallback_mode(env: Env) -> OracleFallbackMode {
        get_fallback_mode(&env)
    }

    /// Issue #461: check `user` against the oracle and emit
    /// `humanity_verification_checked`. Read-only apart from the event.
    pub fn humanity_check(env: Env, user: Address) -> HumanityCheckResult {
        let verified = is_verified(&env, &user);
        let applied = applied_min_stake(&env, &user);
        let surcharge = fee_surcharge_bps(&env, &user);
        env.events().publish(
            (symbol_short!("hmn_chk"), user),
            (verified, applied, env.ledger().sequence()),
        );
        HumanityCheckResult {
            verified,
            applied_min_stake: applied,
            fee_surcharge_bps: surcharge,
        }
    }

    /// Issue #461: stake `amount` with the proof-of-humanity gate applied.
    ///
    /// Enforces `verified_min_stake` for verified humans and
    /// `unverified_min_stake` for everyone else, and — for unverified addresses
    /// — deducts `unverified_fee_surcharge_bps` of `amount` into protocol fees
    /// on top of any existing fees before the remainder is staked. Emits
    /// `humanity_verification_checked`. Returns the shares minted.
    pub fn stake_verified(
        env: Env,
        user: Address,
        amount: i128,
    ) -> Result<i128, VaultCampaignError> {
        user.require_auth();
        if amount <= 0 {
            return Err(VaultCampaignError::ZeroAmount);
        }

        let verified = is_verified(&env, &user);
        let min_stake = applied_min_stake(&env, &user);
        let surcharge_bps = fee_surcharge_bps(&env, &user);
        let current_ledger = env.ledger().sequence();

        env.events().publish(
            (symbol_short!("hmn_chk"), user.clone()),
            (verified, min_stake, current_ledger),
        );

        if amount < min_stake {
            return Err(VaultCampaignError::BelowHumanityMinStake);
        }

        let token_addr = token_address(&env)?;
        let token_client = token::Client::new(&env, &token_addr);
        let contract = env.current_contract_address();

        let surcharge = amount.saturating_mul(surcharge_bps as i128) / 10_000;
        let stake_amount = amount - surcharge;

        if surcharge > 0 {
            token_client.transfer(&user, &contract, &surcharge);
            balance::add_protocol_fee_collected(&env, surcharge);
            // Issue #754: recycle a configured share of the surcharge into the
            // reward pool before community-treasury routing.
            let recyclable_surcharge = surcharge
                - crate::vault::fee_recycle::recycle_fee(&env, surcharge)
                    .map_err(|_| VaultCampaignError::ArithmeticError)?;
            crate::community_treasury::route_fee_revenue(&env, recyclable_surcharge)?;
        }

        token_client.transfer(&user, &contract, &stake_amount);
        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let shares_minted = if total_shares == 0 || total_deposited == 0 {
            stake_amount
        } else {
            balance::amount_to_shares(total_shares, total_deposited, stake_amount)
                .ok_or(VaultCampaignError::ArithmeticError)?
        };

        let current_shares = balance::get_shares(&env, &user);
        balance::set_shares(&env, &user, current_shares + shares_minted);
        balance::set_total_shares(&env, total_shares + shares_minted);
        balance::set_total_deposited(&env, total_deposited + stake_amount);

        Ok(shares_minted)
    }
}
