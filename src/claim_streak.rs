//! Consecutive-claim streak bonus (issue #755).
//!
//! Rewards stakers who claim at least once in every reward period with a small
//! bonus, encouraging regular claim behaviour instead of long dormant periods
//! followed by one large claim.
//!
//! # Streak rules
//!
//! Ledgers are grouped into fixed-length periods of `period_ledgers`:
//!
//! ```text
//! period = current_ledger / period_ledgers
//! ```
//!
//! - The first claim ever starts the streak at `1`.
//! - A claim in the period immediately after the previous one increments the
//!   streak by one, up to `max_streak`.
//! - A claim that skips a period resets the streak to `1`.
//! - A second claim in the same period leaves the streak unchanged and pays no
//!   additional bonus, so a user cannot farm the multiplier by claiming twice.
//!
//! The bonus paid on a claim is `min(streak, max_streak) *
//! bonus_bps_per_streak` basis points of the reward being claimed, so it is
//! naturally capped by `max_streak`.
//!
//! # Reward pool
//!
//! The bonus is funded from the reward pool alongside the normal reward: the
//! entrypoint debits `reward_pool_balance` by the total payout and transfers
//! it from the vault. If the pool cannot cover the reward plus the bonus the
//! claim reverts with `InsufficientRewardPool`.
//!
//! # Claim cooldown
//!
//! Maintaining a streak only requires one successful claim per period. The
//! period is measured in ledgers independently of any per-claim cooldown, so a
//! cooldown cannot prevent a user from claiming in *some* ledger of each
//! period as long as `period_ledgers` is at least as long as the cooldown.
//! Operators should configure `period_ledgers` to be greater than or equal to
//! the claim rate limit / anti-dump cooldown they enable elsewhere.
//!
//! # Wiring
//!
//! Like `anti_dump_claim_cooldown.rs` and `epoch_reward_cap.rs`, this module
//! exposes its own claim entrypoint (`claim_with_streak_bonus`) rather than
//! editing `vault.rs`'s existing `claim()`, keeping the streak opt-in per
//! caller.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this module uses raw
//! `Symbol`-keyed storage, matching `balance.rs` and the other feature modules:
//!
//! - instance `stk_cfg` -> [`StreakConfig`]
//! - persistent `(stk_pos, user)` -> [`StreakState`]

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::vault::{VaultContract, VaultContractArgs, VaultContractClient};

/// Instance key for the admin-configured streak terms.
const CONFIG_KEY: Symbol = symbol_short!("stk_cfg");
/// Persistent key prefix for a user's streak state, keyed by
/// `(STATE_KEY, user)`.
const STATE_KEY: Symbol = symbol_short!("stk_pos");

/// Basis-point denominator used when converting the streak multiplier into a
/// reward amount.
const BPS_DENOMINATOR: i128 = 10_000;

/// Largest per-streak bonus the admin may configure (100% of the claim per
/// streak level). Prevents a misconfiguration from paying unbounded bonuses.
pub const MAX_BONUS_BPS_PER_STREAK: u32 = 10_000;

/// Largest streak a user can reach (and the largest cap the admin may set).
pub const MAX_STREAK_CAP: u32 = 1_000;

/// Admin-configured claim-streak terms (issue #755).
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreakConfig {
    /// Length of one reward period, in ledgers. Must be non-zero.
    pub period_ledgers: u32,
    /// Bonus per streak level, in basis points of the claimed reward.
    pub bonus_bps_per_streak: u32,
    /// Streak level at which the bonus stops growing.
    pub max_streak: u32,
}

/// Per-user claim-streak state.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreakState {
    /// Ledger of the user's most recent claim through this module.
    pub last_claim_ledger: u32,
    /// Period index of the user's most recent claim.
    pub last_period: u32,
    /// Current consecutive-period streak, already capped at `max_streak`.
    pub streak: u32,
}

fn get_config(env: &Env) -> Option<StreakConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

fn set_config(env: &Env, config: &StreakConfig) {
    env.storage().instance().set(&CONFIG_KEY, config);
}

fn get_state(env: &Env, user: &Address) -> Option<StreakState> {
    env.storage().persistent().get(&(STATE_KEY, user.clone()))
}

fn set_state(env: &Env, user: &Address, state: &StreakState) {
    env.storage()
        .persistent()
        .set(&(STATE_KEY, user.clone()), state);
}

fn period_of(ledger: u32, period_ledgers: u32) -> u32 {
    ledger / period_ledgers
}

/// Returns the streak after a claim in `current_period` and whether that claim
/// is eligible for a bonus.
///
/// A claim in the same period as the previous one neither advances nor resets
/// the streak, and pays no second bonus for that period.
fn next_streak(
    previous_streak: u32,
    last_period: u32,
    current_period: u32,
    max_streak: u32,
) -> (u32, bool) {
    if previous_streak == 0 {
        (1, true)
    } else if current_period <= last_period {
        // Same period as the previous claim (already counted), or the ledger
        // clock moved backwards: keep the streak as-is and pay no new bonus.
        (previous_streak, false)
    } else if current_period == last_period.saturating_add(1) {
        (previous_streak.saturating_add(1).min(max_streak), true)
    } else {
        (1, true)
    }
}

fn bonus_bps_for(streak: u32, eligible: bool, config: &StreakConfig) -> u32 {
    if eligible {
        streak
            .min(config.max_streak)
            .saturating_mul(config.bonus_bps_per_streak)
    } else {
        0
    }
}

#[contractimpl]
impl VaultContract {
    /// Admin: configure the claim-streak program (issue #755).
    ///
    /// `period_ledgers` is the length of one reward period in ledgers,
    /// `bonus_bps_per_streak` is the bonus per streak level in basis points of
    /// the claimed reward (`0` disables the bonus while still tracking
    /// streaks), and `max_streak` caps the bonus multiplier.
    ///
    /// # Authorization
    /// `admin` must sign and be the configured admin.
    ///
    /// # Errors
    /// - `ZeroAmount`: `period_ledgers` or `max_streak` is zero.
    /// - `InvalidRate`: `bonus_bps_per_streak` exceeds
    ///   [`MAX_BONUS_BPS_PER_STREAK`] or `max_streak` exceeds
    ///   [`MAX_STREAK_CAP`].
    /// - `Unauthorized` / `NotInitialized`: admin check failed.
    pub fn set_streak_config(
        env: Env,
        admin: Address,
        period_ledgers: u32,
        bonus_bps_per_streak: u32,
        max_streak: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;

        if period_ledgers == 0 || max_streak == 0 {
            return Err(VaultError::ZeroAmount);
        }
        if bonus_bps_per_streak > MAX_BONUS_BPS_PER_STREAK || max_streak > MAX_STREAK_CAP {
            return Err(VaultError::InvalidRate);
        }

        set_config(
            &env,
            &StreakConfig {
                period_ledgers,
                bonus_bps_per_streak,
                max_streak,
            },
        );

        env.events().publish(
            (symbol_short!("stk_set"),),
            (
                period_ledgers,
                bonus_bps_per_streak,
                max_streak,
                env.ledger().sequence(),
            ),
        );
        Ok(())
    }

    /// Read-only: the current streak configuration, if it has been set.
    pub fn get_streak_config(env: Env) -> Option<StreakConfig> {
        get_config(&env)
    }

    /// Read-only: `user`'s current claim streak (`0` before the first claim).
    ///
    /// The result is clamped to the configured `max_streak`, so lowering the
    /// cap takes effect immediately.
    pub fn get_claim_streak(env: Env, user: Address) -> u32 {
        match (get_state(&env, &user), get_config(&env)) {
            (Some(state), Some(config)) => state.streak.min(config.max_streak),
            (Some(state), None) => state.streak,
            _ => 0,
        }
    }

    /// Read-only: the bonus, in basis points of the claimed reward, that a
    /// claim by `user` at the current ledger would earn. `0` when the user
    /// already claimed in this period or the program is not configured.
    pub fn get_claim_streak_bonus_bps(env: Env, user: Address) -> u32 {
        let config = match get_config(&env) {
            Some(config) => config,
            None => return 0,
        };
        let state = get_state(&env, &user);
        let previous_streak = state.as_ref().map(|s| s.streak).unwrap_or(0);
        // Derive the previous period from the committed ledger rather than the
        // stored index, so an admin changing `period_ledgers` cannot make an
        // unchanged claim look like a skipped period.
        let last_period = state
            .as_ref()
            .map(|s| period_of(s.last_claim_ledger, config.period_ledgers))
            .unwrap_or(0);
        let (streak, eligible) = next_streak(
            previous_streak,
            last_period,
            period_of(env.ledger().sequence(), config.period_ledgers),
            config.max_streak,
        );
        bonus_bps_for(streak, eligible, &config)
    }

    /// Claim `user`'s accrued reward with the consecutive-period streak bonus
    /// included (issue #755).
    ///
    /// Records the claim for streak purposes, then pays the accrued reward plus
    /// any bonus from the reward pool. A claim in the same period as the
    /// previous one pays the reward but no second bonus. Returns the bonus
    /// amount transferred (`0` when the claim did not earn one).
    ///
    /// # Authorization
    /// `user` must sign.
    ///
    /// # Errors
    /// - `NotInitialized`: the streak program has not been configured.
    /// - `InsufficientRewardPool`: the pool cannot cover reward plus bonus.
    /// - `ArithmeticError`: reward + bonus overflowed.
    pub fn claim_with_streak_bonus(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let config = get_config(&env).ok_or(VaultError::NotInitialized)?;
        let accrued = balance::get_accrued_reward(&env, &user);
        if accrued <= 0 {
            return Ok(0);
        }

        let now = env.ledger().sequence();
        let current_period = period_of(now, config.period_ledgers);
        let state = get_state(&env, &user);
        let previous_streak = state.as_ref().map(|s| s.streak).unwrap_or(0);
        let last_period = state
            .as_ref()
            .map(|s| period_of(s.last_claim_ledger, config.period_ledgers))
            .unwrap_or(0);

        let (streak, eligible) = next_streak(
            previous_streak,
            last_period,
            current_period,
            config.max_streak,
        );
        let bonus_bps = bonus_bps_for(streak, eligible, &config);
        let bonus = accrued
            .checked_mul(bonus_bps as i128)
            .and_then(|value| value.checked_div(BPS_DENOMINATOR))
            .ok_or(VaultError::ArithmeticError)?;

        let total = accrued
            .checked_add(bonus)
            .ok_or(VaultError::ArithmeticError)?;
        let pool = balance::get_reward_pool_balance(&env);
        if pool < total {
            return Err(VaultError::InsufficientRewardPool);
        }

        set_state(
            &env,
            &user,
            &StreakState {
                last_claim_ledger: now,
                last_period: current_period,
                streak,
            },
        );

        balance::set_reward_pool_balance(&env, pool - total);
        balance::set_accrued_reward(&env, &user, 0);
        balance::add_total_rewards_paid(&env, total);

        let token_addr = match balance::get_reward_token(&env) {
            Some(reward_token) => reward_token,
            None => Self::token_address(&env)?,
        };
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &user,
            &total,
        );

        events::claimed(&env, &user, total, now);
        env.events()
            .publish((symbol_short!("stk_clm"), user), (streak, bonus, now));

        Ok(bonus)
    }
}
