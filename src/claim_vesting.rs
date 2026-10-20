//! Issue #511: optional linear vesting for claimed rewards.
//!
//! Instead of paying a claim out in full, `claim_vesting` locks the claimed
//! reward into a per-user linear vesting schedule that unlocks gradually over
//! an admin-configured number of ledgers, reducing sell pressure on the
//! reward token. Unlocked rewards are paid with `release_vested`.
//!
//! Vesting is off by default (`duration == 0`); in that case `claim_vesting`
//! behaves exactly like `claim`. The plain `claim` entrypoint is unaffected.
//!
//! Claim-time deductions (bug-bounty contribution and claim fee) are applied
//! when the reward enters the schedule, so the schedule only holds what the
//! user will actually receive.
//!
//! Each user has at most one schedule. A new `claim_vesting` first pays out
//! whatever has already unlocked, then merges the still-locked remainder with
//! the new reward into a fresh schedule starting at the current ledger.
//!
//! # Storage
//!
//! - `symbol_short!("cv_dur")` -> `u32` (instance): vesting duration in ledgers
//! - `(symbol_short!("cv_sched"), Address)` -> `ClaimVestingSchedule` (persistent)

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, String, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::{VaultAccessError, VaultError};
use crate::events;
use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient, STELLAR_LEDGERS_PER_YEAR};

const DURATION_KEY: Symbol = symbol_short!("cv_dur");
const SCHEDULE_PREFIX: Symbol = symbol_short!("cv_sched");

/// Longest vesting duration the admin may configure (about one year).
pub const MAX_CLAIM_VESTING_LEDGERS: u32 = STELLAR_LEDGERS_PER_YEAR;

/// A user's linear reward-vesting schedule.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimVestingSchedule {
    /// Total reward locked into this schedule.
    pub total: i128,
    /// Portion of `total` already paid out.
    pub released: i128,
    /// Ledger the schedule started unlocking.
    pub start_ledger: u32,
    /// Ledgers over which `total` unlocks linearly.
    pub duration: u32,
}

fn schedule_key(user: &Address) -> (Symbol, Address) {
    (SCHEDULE_PREFIX, user.clone())
}

pub fn read_duration(env: &Env) -> u32 {
    env.storage().instance().get(&DURATION_KEY).unwrap_or(0)
}

pub fn read_schedule(env: &Env, user: &Address) -> Option<ClaimVestingSchedule> {
    env.storage().persistent().get(&schedule_key(user))
}

fn write_schedule(env: &Env, user: &Address, schedule: &ClaimVestingSchedule) {
    env.storage().persistent().set(&schedule_key(user), schedule);
}

fn clear_schedule(env: &Env, user: &Address) {
    env.storage().persistent().remove(&schedule_key(user));
}

/// Amount of `schedule.total` unlocked as of `ledger`.
fn vested_amount(schedule: &ClaimVestingSchedule, ledger: u32) -> i128 {
    let elapsed = ledger.saturating_sub(schedule.start_ledger);
    if schedule.duration == 0 || elapsed >= schedule.duration {
        return schedule.total;
    }
    schedule
        .total
        .checked_mul(elapsed as i128)
        .map(|v| v / schedule.duration as i128)
        // Overflow is only possible for absurd totals; fall back to the
        // slower but overflow-safe split.
        .unwrap_or_else(|| {
            (schedule.total / schedule.duration as i128) * elapsed as i128
        })
}

/// Unlocked but not yet paid-out amount.
fn releasable_amount(schedule: &ClaimVestingSchedule, ledger: u32) -> i128 {
    vested_amount(schedule, ledger)
        .saturating_sub(schedule.released)
        .max(0)
}

fn reward_token(env: &Env) -> Result<Address, VaultError> {
    env.storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(VaultError::NotInitialized)
}

/// Pays out everything currently unlocked for `user` and updates (or clears)
/// their schedule. Returns the amount paid.
fn release(env: &Env, user: &Address) -> Result<i128, VaultError> {
    let mut schedule = match read_schedule(env, user) {
        Some(s) => s,
        None => return Ok(0),
    };
    let amount = releasable_amount(&schedule, env.ledger().sequence());
    if amount > 0 {
        schedule.released = schedule
            .released
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticError)?;
        token::Client::new(env, &reward_token(env)?).transfer(
            &env.current_contract_address(),
            user,
            &amount,
        );
        env.events()
            .publish((symbol_short!("vest_rel"), user.clone()), amount);
    }
    if schedule.released >= schedule.total {
        clear_schedule(env, user);
    } else {
        write_schedule(env, user, &schedule);
    }
    Ok(amount)
}

#[contractimpl]
impl VaultContract {
    /// Admin: set the linear vesting period (ledgers) applied by
    /// `claim_vesting`. `0` disables vesting (claims pay instantly). Changing
    /// the duration only affects schedules created or merged afterwards.
    pub fn set_claim_vesting_duration(
        env: Env,
        admin_addr: Address,
        ledgers: u32,
    ) -> Result<(), VaultAccessError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env)? {
            return Err(VaultAccessError::Unauthorized);
        }
        if ledgers > MAX_CLAIM_VESTING_LEDGERS {
            return Err(VaultAccessError::InvalidVestingDuration);
        }
        env.storage().instance().set(&DURATION_KEY, &ledgers);
        Ok(())
    }

    /// Read-only: the configured claim vesting period (`0` = disabled).
    pub fn get_claim_vesting_duration(env: Env) -> u32 {
        read_duration(&env)
    }

    /// Claim accrued rewards into a linear vesting schedule.
    ///
    /// With vesting disabled this is identical to `claim` and returns the
    /// amount paid. With vesting enabled it first releases anything already
    /// unlocked, then locks the net reward (after bounty contribution and
    /// claim fee) and returns the amount newly added to the schedule.
    pub fn claim_vesting(env: Env, staker: Address) -> Result<i128, VaultAccessError> {
        let duration = read_duration(&env);
        if duration == 0 {
            return Ok(Self::claim(env, staker)?);
        }
        staker.require_auth();

        let accrued = balance::get_accrued_reward(&env, &staker);
        if accrued <= 0 || crate::peg_stabilization::emissions_halted(&env) {
            return Ok(0);
        }

        balance::set_accrued_reward(&env, &staker, 0);
        let total_paid = balance::get_total_rewards_paid(&env);
        balance::set_total_rewards_paid(
            &env,
            total_paid
                .checked_add(accrued)
                .ok_or(VaultAccessError::ArithmeticError)?,
        );

        let bounty =
            crate::stake_funded_bug_bounty::deduct_bounty_contribution(&env, &staker, accrued);
        let after_bounty = accrued.saturating_sub(bounty);
        let net = crate::claim_fee::apply_claim_fee(&env, &staker, after_bounty)?;
        if net <= 0 {
            return Ok(0);
        }

        // Pay out whatever already unlocked, then fold the locked remainder
        // into a fresh schedule together with the new reward.
        release(&env, &staker)?;
        let locked_remainder = read_schedule(&env, &staker)
            .map(|s| s.total.saturating_sub(s.released))
            .unwrap_or(0);
        let total = locked_remainder
            .checked_add(net)
            .ok_or(VaultAccessError::ArithmeticError)?;
        let now = env.ledger().sequence();
        write_schedule(
            &env,
            &staker,
            &ClaimVestingSchedule {
                total,
                released: 0,
                start_ledger: now,
                duration,
            },
        );

        crate::reward_token_audit_trail::log_reward_movement(
            &env,
            crate::reward_token_audit_trail::MovementType::RewardPaid,
            env.current_contract_address(),
            staker.clone(),
            net,
            String::from_str(&env, "claim_vesting"),
        );
        balance::set_last_claim_action_ledger(&env, &staker, now);
        env.events()
            .publish((symbol_short!("vest_add"), staker.clone()), (net, total, duration));
        events::claimed(&env, &staker, net, now);
        Ok(net)
    }

    /// Pay out all currently unlocked vested rewards. Returns the amount paid.
    pub fn release_vested(env: Env, staker: Address) -> Result<i128, VaultAccessError> {
        staker.require_auth();
        Ok(release(&env, &staker)?)
    }

    /// Read-only: the user's active vesting schedule, if any.
    pub fn get_claim_vesting(env: Env, user: Address) -> Option<ClaimVestingSchedule> {
        read_schedule(&env, &user)
    }

    /// Read-only: vested rewards `release_vested` would pay right now.
    pub fn get_releasable_vested(env: Env, user: Address) -> i128 {
        read_schedule(&env, &user)
            .map(|s| releasable_amount(&s, env.ledger().sequence()))
            .unwrap_or(0)
    }
}
