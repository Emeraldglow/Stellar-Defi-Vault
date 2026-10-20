//! Personalized pool comfort score for a user's risk profile (issue #399).
//!
//! Purely informational â€” never affects staking mechanics. Compares the
//! pool's current configuration against a user's stated risk preferences
//! and reports a 0-100 suitability score with a flag per mismatch.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

const RISK_PROFILE_KEY: Symbol = symbol_short!("cs_prof");
const LOCK_DAYS_KEY: Symbol = symbol_short!("cs_lockd");
const SLASH_RISK_KEY: Symbol = symbol_short!("cs_slash");
const AUDITED_KEY: Symbol = symbol_short!("cs_audit");

const SCORE_MAX: u32 = 100;
const PENALTY_PER_FLAG: u32 = 20;

/// A user's stated risk preferences for this pool.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserRiskProfile {
    pub max_lock_days: u32,
    pub min_apy_bps: u32,
    pub max_fee_bps: u32,
    pub requires_no_slash: bool,
    pub requires_audited: bool,
}

/// Result of `get_comfort_score`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ComfortScore {
    pub score: u32,
    pub lock_flag: bool,
    pub apy_flag: bool,
    pub fee_flag: bool,
    pub slash_flag: bool,
    pub audit_flag: bool,
}

fn get_profile(env: &Env, user: &Address) -> Option<UserRiskProfile> {
    env.storage()
        .persistent()
        .get(&(RISK_PROFILE_KEY, user.clone()))
}

fn set_profile(env: &Env, user: &Address, profile: &UserRiskProfile) {
    env.storage()
        .persistent()
        .set(&(RISK_PROFILE_KEY, user.clone()), profile);
}

fn get_pool_lock_days(env: &Env) -> u32 {
    env.storage().instance().get(&LOCK_DAYS_KEY).unwrap_or(0)
}

fn get_pool_has_slash_risk(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&SLASH_RISK_KEY)
        .unwrap_or(false)
}

fn get_pool_audited(env: &Env) -> bool {
    env.storage().instance().get(&AUDITED_KEY).unwrap_or(false)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Set the caller's risk profile for this pool. Requires the user's own auth.
    pub fn set_risk_profile(
        env: Env,
        user: Address,
        profile: UserRiskProfile,
    ) -> Result<(), VaultError> {
        user.require_auth();
        set_profile(&env, &user, &profile);
        Ok(())
    }

    /// Read-only query: `user`'s stored risk profile, if any.
    pub fn get_risk_profile(env: Env, user: Address) -> Option<UserRiskProfile> {
        get_profile(&env, &user)
    }

    /// Admin-set pool lock period in days, compared against a profile's
    /// `max_lock_days`. Defaults to 0 (no lock) when never configured.
    pub fn set_pool_lock_days(env: Env, days: u32) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        env.storage().instance().set(&LOCK_DAYS_KEY, &days);
        Ok(())
    }

    /// Admin-set flag: whether this pool carries slashing risk, compared
    /// against a profile's `requires_no_slash`.
    pub fn set_pool_slash_risk(env: Env, has_slash_risk: bool) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        env.storage()
            .instance()
            .set(&SLASH_RISK_KEY, &has_slash_risk);
        Ok(())
    }

    /// Admin-set flag: whether this pool has been audited (issue #399's
    /// `set_pool_audited`).
    pub fn set_pool_audited(env: Env, audited: bool) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        env.storage().instance().set(&AUDITED_KEY, &audited);
        Ok(())
    }

    /// Rate this pool's suitability for `user`'s stored risk profile.
    /// Returns the max score (100) with no flags when the user has never
    /// called `set_risk_profile`.
    pub fn get_comfort_score(env: Env, user: Address) -> ComfortScore {
        let profile = match get_profile(&env, &user) {
            Some(p) => p,
            None => {
                return ComfortScore {
                    score: SCORE_MAX,
                    lock_flag: false,
                    apy_flag: false,
                    fee_flag: false,
                    slash_flag: false,
                    audit_flag: false,
                };
            }
        };

        let lock_flag =
            profile.max_lock_days > 0 && get_pool_lock_days(&env) > profile.max_lock_days;
        let apy_flag = balance::get_reward_rate_bps(&env) < profile.min_apy_bps;
        let fee_flag = balance::get_unstake_fee_bps(&env) > profile.max_fee_bps;
        let slash_flag = profile.requires_no_slash && get_pool_has_slash_risk(&env);
        let audit_flag = profile.requires_audited && !get_pool_audited(&env);

        let mut flags_triggered: u32 = 0;
        if lock_flag {
            flags_triggered += 1;
        }
        if apy_flag {
            flags_triggered += 1;
        }
        if fee_flag {
            flags_triggered += 1;
        }
        if slash_flag {
            flags_triggered += 1;
        }
        if audit_flag {
            flags_triggered += 1;
        }

        let score = SCORE_MAX.saturating_sub(flags_triggered.saturating_mul(PENALTY_PER_FLAG));

        ComfortScore {
            score,
            lock_flag,
            apy_flag,
            fee_flag,
            slash_flag,
            audit_flag,
        }
    }
}















