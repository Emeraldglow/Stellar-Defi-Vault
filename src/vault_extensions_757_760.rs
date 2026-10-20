//! Vault extensions for issues #757, #758, #759, and #760.
//!
//! - Issue #757: Referral statistics and claimable referral earnings balance.
//! - Issue #758: Vote-escrow style reward boost by locking the reward token.
//! - Issue #759: Gauge voting to direct emissions across sub-pools.
//! - Issue #760: Vote receipt query `get_vote()` and `has_voted()`.

use soroban_sdk::{
    contractimpl, symbol_short, token, Address, Env, Symbol, Vec,
};

use crate::admin;
use crate::balance;
use crate::errors::{VaultBoostGaugeError, VaultError};
use crate::events;
use crate::storage::{ReferralStats, VeBoostLock, VoteReceipt};
use crate::vault::VaultContract;

const VE_CONFIG_KEY: Symbol = symbol_short!("ve_cfg");
const VE_LOCK_KEY: Symbol = symbol_short!("ve_lock");
const GAUGE_VOTERS_KEY: Symbol = symbol_short!("g_voters");
const GAUGE_VOTE_KEY: Symbol = symbol_short!("gauge_vt");
const GAUGE_OVERRIDE_KEY: Symbol = symbol_short!("g_ovr");
const GAUGE_BOUND_KEY: Symbol = symbol_short!("g_bnd");

const DEFAULT_MAX_VE_DURATION: u32 = 100_000;
const DEFAULT_MAX_VE_BOOST_BPS: u32 = 5_000; // +50% max boost
const DEFAULT_VE_AMOUNT_SCALE: i128 = 10_000; // token scale for max boost calculation

fn get_ve_config(env: &Env) -> (u32, u32, i128) {
    env.storage()
        .instance()
        .get(&VE_CONFIG_KEY)
        .unwrap_or((DEFAULT_MAX_VE_DURATION, DEFAULT_MAX_VE_BOOST_BPS, DEFAULT_VE_AMOUNT_SCALE))
}

fn get_gauge_voters(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&GAUGE_VOTERS_KEY)
        .unwrap_or(Vec::new(env))
}

fn add_gauge_voter(env: &Env, voter: &Address) {
    let mut voters = get_gauge_voters(env);
    let mut exists = false;
    for i in 0..voters.len() {
        if &voters.get(i).unwrap() == voter {
            exists = true;
            break;
        }
    }
    if !exists {
        voters.push_back(voter.clone());
        env.storage().instance().set(&GAUGE_VOTERS_KEY, &voters);
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    // ── Issue #757: Referral statistics and claimable earnings balance ─────────

    /// Read-only query for referral statistics of `referrer`.
    pub fn get_referral_stats(env: Env, referrer: Address) -> ReferralStats {
        balance::get_referral_stats(&env, &referrer)
    }

    /// Claim accrued referral earnings. Requires authorization from `referrer`.
    /// Zeroes the claimable balance upon payout.
    pub fn claim_referral_earnings(
        env: Env,
        referrer: Address,
    ) -> Result<i128, VaultBoostGaugeError> {
        referrer.require_auth();

        let mut stats = balance::get_referral_stats(&env, &referrer);
        let claimable = stats.earned.saturating_sub(stats.claimed);
        if claimable <= 0 {
            return Ok(0);
        }

        let reward_token = Self::reward_token_address(&env)?;
        let client = token::Client::new(&env, &reward_token);
        let pool_address = env.current_contract_address();

        if client.balance(&pool_address) < claimable {
            return Err(VaultBoostGaugeError::InsufficientRewardPool);
        }

        client.transfer(&pool_address, &referrer, &claimable);

        stats.claimed = stats
            .claimed
            .checked_add(claimable)
            .ok_or(VaultBoostGaugeError::ArithmeticError)?;
        balance::set_referral_stats(&env, &referrer, &stats);

        events::referral_earnings_claimed(
            &env,
            &referrer,
            claimable,
            env.ledger().sequence(),
        );

        Ok(claimable)
    }

    // ── Issue #758: Vote-escrow style reward boost ────────────────────────────

    /// Admin: configure vote-escrow boost parameters.
    pub fn set_ve_boost_config(
        env: Env,
        admin: Address,
        max_duration_ledgers: u32,
        max_boost_bps: u32,
        amount_scale: i128,
    ) -> Result<(), VaultBoostGaugeError> {
        admin::require_admin_as(&env, &admin)?;
        if max_duration_ledgers == 0 {
            return Err(VaultBoostGaugeError::InvalidDuration);
        }
        if max_boost_bps == 0 || max_boost_bps > 10_000 {
            return Err(VaultBoostGaugeError::InvalidBounds);
        }
        if amount_scale <= 0 {
            return Err(VaultBoostGaugeError::ZeroAmount);
        }

        env.storage().instance().set(
            &VE_CONFIG_KEY,
            &(max_duration_ledgers, max_boost_bps, amount_scale),
        );
        Ok(())
    }

    /// Lock reward tokens for `duration_ledgers` to receive a decaying reward boost.
    pub fn lock_for_boost(
        env: Env,
        user: Address,
        amount: i128,
        duration_ledgers: u32,
    ) -> Result<(), VaultBoostGaugeError> {
        user.require_auth();

        if amount <= 0 {
            return Err(VaultBoostGaugeError::ZeroAmount);
        }
        if duration_ledgers == 0 {
            return Err(VaultBoostGaugeError::InvalidDuration);
        }

        let (max_duration, _, _) = get_ve_config(&env);
        if duration_ledgers > max_duration {
            return Err(VaultBoostGaugeError::DurationTooLong);
        }

        let reward_token = Self::reward_token_address(&env)?;
        let client = token::Client::new(&env, &reward_token);
        client.transfer(&user, &env.current_contract_address(), &amount);

        let current_ledger = env.ledger().sequence();
        let unlock_ledger = current_ledger.saturating_add(duration_ledgers);

        let lock = VeBoostLock {
            amount,
            unlock_ledger,
            duration_ledgers,
        };

        env.storage()
            .persistent()
            .set(&(VE_LOCK_KEY, user.clone()), &lock);

        events::ve_boost_locked(&env, &user, amount, duration_ledgers, unlock_ledger);
        Ok(())
    }

    /// Read-only query for current vote-escrow boost in basis points.
    /// Decays linearly to zero at expiry.
    pub fn get_ve_boost_bps(env: Env, user: Address) -> u32 {
        let lock: Option<VeBoostLock> = env
            .storage()
            .persistent()
            .get(&(VE_LOCK_KEY, user.clone()));

        let lock = match lock {
            Some(l) => l,
            None => return 0,
        };

        let current_ledger = env.ledger().sequence();
        if current_ledger >= lock.unlock_ledger {
            return 0;
        }

        let remaining = lock.unlock_ledger - current_ledger;
        let (max_duration, max_boost_bps, amount_scale) = get_ve_config(&env);

        let numerator = lock
            .amount
            .saturating_mul(remaining as i128)
            .saturating_mul(max_boost_bps as i128);
        let denominator = amount_scale.saturating_mul(max_duration as i128);

        if denominator <= 0 {
            return 0;
        }

        let raw_boost = numerator / denominator;
        raw_boost.min(max_boost_bps as i128).max(0) as u32
    }

    /// Unlock locked reward tokens after lock duration has expired.
    pub fn unlock_boost(env: Env, user: Address) -> Result<i128, VaultBoostGaugeError> {
        user.require_auth();

        let lock: Option<VeBoostLock> = env
            .storage()
            .persistent()
            .get(&(VE_LOCK_KEY, user.clone()));

        let lock = lock.ok_or(VaultBoostGaugeError::PositionNotFound)?;
        let current_ledger = env.ledger().sequence();

        if current_ledger < lock.unlock_ledger {
            return Err(VaultBoostGaugeError::LockNotExpired);
        }

        let reward_token = Self::reward_token_address(&env)?;
        let client = token::Client::new(&env, &reward_token);
        client.transfer(&env.current_contract_address(), &user, &lock.amount);

        env.storage()
            .persistent()
            .remove(&(VE_LOCK_KEY, user.clone()));

        events::ve_boost_unlocked(&env, &user, lock.amount, current_ledger);
        Ok(lock.amount)
    }

    // ── Issue #759: Gauge voting for emission distribution ────────────────────

    /// Cast gauge vote allocating emissions across sub-pools (allocations sum to 10000 bps).
    pub fn vote_gauge(
        env: Env,
        user: Address,
        allocations: Vec<(u32, u32)>,
    ) -> Result<(), VaultBoostGaugeError> {
        user.require_auth();

        let mut total_bps = 0u32;
        for i in 0..allocations.len() {
            let (_, bps) = allocations.get(i).unwrap();
            total_bps = total_bps
                .checked_add(bps)
                .ok_or(VaultBoostGaugeError::ArithmeticError)?;
        }

        if total_bps != 10_000 {
            return Err(VaultBoostGaugeError::InvalidAllocation);
        }

        env.storage()
            .persistent()
            .set(&(GAUGE_VOTE_KEY, user.clone()), &allocations);

        add_gauge_voter(&env, &user);
        events::gauge_voted(&env, &user, env.ledger().sequence());
        Ok(())
    }

    /// Admin: set min/max emission weight bounds for a sub-pool.
    pub fn set_gauge_bounds(
        env: Env,
        admin: Address,
        sub_pool_id: u32,
        min_bps: u32,
        max_bps: u32,
    ) -> Result<(), VaultBoostGaugeError> {
        admin::require_admin_as(&env, &admin)?;
        if min_bps > max_bps || max_bps > 10_000 {
            return Err(VaultBoostGaugeError::InvalidBounds);
        }
        env.storage()
            .instance()
            .set(&(GAUGE_BOUND_KEY, sub_pool_id), &(min_bps, max_bps));
        Ok(())
    }

    /// Admin: override gauge weights with a fixed allocation.
    pub fn set_gauge_override(
        env: Env,
        admin: Address,
        overrides: Option<Vec<(u32, u32)>>,
    ) -> Result<(), VaultBoostGaugeError> {
        admin::require_admin_as(&env, &admin)?;
        if let Some(ref ovr) = overrides {
            let mut sum = 0u32;
            for i in 0..ovr.len() {
                sum = sum
                    .checked_add(ovr.get(i).unwrap().1)
                    .ok_or(VaultBoostGaugeError::ArithmeticError)?;
            }
            if sum != 10_000 {
                return Err(VaultBoostGaugeError::InvalidAllocation);
            }
            env.storage().instance().set(&GAUGE_OVERRIDE_KEY, ovr);
        } else {
            env.storage().instance().remove(&GAUGE_OVERRIDE_KEY);
        }
        Ok(())
    }

    /// Read-only query for current gauge weights across sub-pools.
    pub fn get_gauge_weights(env: Env) -> Vec<(u32, u32)> {
        // Check for admin override first
        let manual_override: Option<Vec<(u32, u32)>> =
            env.storage().instance().get(&GAUGE_OVERRIDE_KEY);
        if let Some(ovr) = manual_override {
            return ovr;
        }

        let voters = get_gauge_voters(&env);
        let mut pool_votes: Vec<(u32, i128)> = Vec::new(&env);
        let mut total_vote_power: i128 = 0;

        for i in 0..voters.len() {
            let voter = voters.get(i).unwrap();
            let shares = balance::get_shares(&env, &voter);
            if shares <= 0 {
                continue;
            }

            let allocations: Option<Vec<(u32, u32)>> =
                env.storage().persistent().get(&(GAUGE_VOTE_KEY, voter));
            if let Some(alloc) = allocations {
                for j in 0..alloc.len() {
                    let (pool_id, bps) = alloc.get(j).unwrap();
                    let power = shares.saturating_mul(bps as i128);
                    total_vote_power = total_vote_power.saturating_add(power);

                    let mut found = false;
                    for k in 0..pool_votes.len() {
                        let (pid, cur_p) = pool_votes.get(k).unwrap();
                        if pid == pool_id {
                            pool_votes.set(k, (pid, cur_p.saturating_add(power)));
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        pool_votes.push_back((pool_id, power));
                    }
                }
            }
        }

        let mut results: Vec<(u32, u32)> = Vec::new(&env);
        if total_vote_power <= 0 {
            return results;
        }

        for i in 0..pool_votes.len() {
            let (pool_id, power) = pool_votes.get(i).unwrap();
            let mut weight_bps = ((power.saturating_mul(10_000)) / total_vote_power) as u32;

            // Apply bounds if configured
            let bounds: Option<(u32, u32)> =
                env.storage().instance().get(&(GAUGE_BOUND_KEY, pool_id));
            if let Some((min_b, max_b)) = bounds {
                weight_bps = weight_bps.max(min_b).min(max_b);
            }

            results.push_back((pool_id, weight_bps));
        }

        results
    }

    // ── Issue #760: Vote receipt query ────────────────────────────────────────

    /// Read-only query for a user's vote receipt on a proposal.
    pub fn get_vote(env: Env, proposal_id: u32, voter: Address) -> Option<VoteReceipt> {
        balance::get_vote_receipt(&env, proposal_id, &voter)
    }

    /// Read-only check whether `voter` has voted on `proposal_id`.
    pub fn has_voted(env: Env, proposal_id: u32, voter: Address) -> bool {
        balance::has_voted(&env, proposal_id, &voter)
            || balance::get_vote_receipt(&env, proposal_id, &voter).is_some()
    }

    /// Change a previously cast vote on an active proposal.
    pub fn change_vote(
        env: Env,
        user: Address,
        proposal_id: u32,
        support: bool,
    ) -> Result<(), VaultBoostGaugeError> {
        user.require_auth();

        let mut proposal = balance::get_proposal(&env, proposal_id)
            .ok_or(VaultBoostGaugeError::ProposalNotFound)?;

        if proposal.enacted {
            return Err(VaultBoostGaugeError::ProposalEnacted);
        }
        if env.ledger().sequence() > proposal.ends_at {
            return Err(VaultBoostGaugeError::VotingEnded);
        }

        let old_receipt = balance::get_vote_receipt(&env, proposal_id, &user)
            .ok_or(VaultBoostGaugeError::NotVoted)?;

        if old_receipt.support != support {
            if old_receipt.support {
                proposal.votes_for = proposal.votes_for.saturating_sub(old_receipt.weight);
                proposal.votes_against = proposal
                    .votes_against
                    .checked_add(old_receipt.weight)
                    .ok_or(VaultBoostGaugeError::ArithmeticError)?;
            } else {
                proposal.votes_against = proposal.votes_against.saturating_sub(old_receipt.weight);
                proposal.votes_for = proposal
                    .votes_for
                    .checked_add(old_receipt.weight)
                    .ok_or(VaultBoostGaugeError::ArithmeticError)?;
            }

            balance::set_proposal(&env, proposal_id, &proposal);

            let new_receipt = VoteReceipt {
                support,
                weight: old_receipt.weight,
                ledger: env.ledger().sequence(),
            };
            balance::set_vote_receipt(&env, proposal_id, &user, &new_receipt);

            events::proposal_vote_changed(
                &env,
                &user,
                proposal_id,
                support,
                old_receipt.weight,
                env.ledger().sequence(),
            );
        }

        Ok(())
    }
}
