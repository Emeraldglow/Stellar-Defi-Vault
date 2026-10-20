//! Burn milestone tracker (issue #452).
//!
//! Celebrates deflationary token burn achievements when cumulative burns
//! cross configurable thresholds. Emits celebratory events and stores
//! permanent records.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultExtError;
use crate::vault::VaultContract;

const BURN_THRESHOLDS_KEY: Symbol = symbol_short!("burn_thr");
const BURN_REACHED_KEY: Symbol = symbol_short!("burn_hit");

fn get_thresholds(env: &Env) -> Vec<i128> {
    env.storage()
        .instance()
        .get(&BURN_THRESHOLDS_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_thresholds(env: &Env, thresholds: &Vec<i128>) {
    env.storage().instance().set(&BURN_THRESHOLDS_KEY, thresholds);
}

fn get_reached(env: &Env) -> Vec<bool> {
    env.storage()
        .instance()
        .get(&BURN_REACHED_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_reached(env: &Env, reached: &Vec<bool>) {
    env.storage().instance().set(&BURN_REACHED_KEY, reached);
}

/// Internal helper to check and emit milestones after a burn.
pub fn check_burn_milestones(env: &Env, burn_that_triggered: i128) {
    let thresholds = get_thresholds(env);
    if thresholds.is_empty() {
        return;
    }
    // total burned is sum of tokens burned + fees burned
    let total_tokens = balance::get_total_tokens_burned(env);
    let total_fees = balance::get_fees_burned(env);
    let total_burned = total_tokens.saturating_add(total_fees);
    let mut reached = get_reached(env);
    // Ensure reached len matches thresholds len (init if mismatch)
    if reached.len() != thresholds.len() {
        let mut new_reached = Vec::new(env);
        for _ in 0..thresholds.len() {
            new_reached.push_back(false);
        }
        // preserve existing where possible
        let min_len = if reached.len() < thresholds.len() { reached.len() } else { thresholds.len() };
        for i in 0..min_len {
            new_reached.set(i, reached.get(i).unwrap());
        }
        reached = new_reached;
    }
    let ledger = env.ledger().sequence();
    let mut changed = false;
    for i in 0..thresholds.len() {
        let thr = thresholds.get(i).unwrap();
        let is_reached = reached.get(i).unwrap();
        if !is_reached && total_burned >= thr {
            reached.set(i, true);
            changed = true;
            // burn_milestone_reached event: (threshold, total_burned, burn_that_triggered, ledger)
            env.events().publish(
                (symbol_short!("burn_ms"),),
                (thr, total_burned, burn_that_triggered, ledger),
            );
        }
    }
    if changed {
        set_reached(env, &reached);
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Admin sets burn milestones. Max 10, ascending order required.
    pub fn set_burn_milestones(
        env: Env,
        admin: Address,
        thresholds: Vec<i128>,
    ) -> Result<(), VaultExtError> {
        admin.require_auth();
        admin::require_admin(&env)?;
        if thresholds.len() > 10 {
            return Err(VaultExtError::TooManyMilestones);
        }
        // Check ascending order (strictly increasing) and positive
        let mut prev: Option<i128> = None;
        for thr in thresholds.iter() {
            if thr <= 0 {
                return Err(VaultExtError::InvalidFeeAllocation);
            }
            if let Some(p) = prev {
                if thr <= p {
                    return Err(VaultExtError::InvalidFeeAllocation);
                }
            }
            prev = Some(thr);
        }
        set_thresholds(&env, &thresholds);
        // Reset reached flags to false for new thresholds
        let mut reached = Vec::new(&env);
        for _ in 0..thresholds.len() {
            reached.push_back(false);
        }
        // If total already exceeds thresholds, we keep as not yet celebrated until next burn
        // (spec says fires on crossing, so don't auto-celebrate here)
        set_reached(&env, &reached);
        Ok(())
    }

    /// Read-only query: threshold + whether reached
    pub fn get_burn_milestones(env: Env) -> Vec<(i128, bool)> {
        let thresholds = get_thresholds(&env);
        let reached = get_reached(&env);
        let mut out = Vec::new(&env);
        for i in 0..thresholds.len() {
            let thr = thresholds.get(i).unwrap();
            let is_reached = if i < reached.len() {
                reached.get(i).unwrap()
            } else {
                false
            };
            out.push_back((thr, is_reached));
        }
        out
    }

    /// Next uncelebrated threshold
    pub fn get_next_burn_milestone(env: Env) -> Option<i128> {
        let thresholds = get_thresholds(&env);
        let reached = get_reached(&env);
        for i in 0..thresholds.len() {
            let is_reached = if i < reached.len() {
                reached.get(i).unwrap()
            } else {
                false
            };
            if !is_reached {
                return Some(thresholds.get(i).unwrap());
            }
        }
        None
    }

    /// Total burned (sourced from tot_burn + fbb_brn)
    pub fn get_total_burned(env: Env) -> i128 {
        let a = balance::get_total_tokens_burned(&env);
        let b = balance::get_fees_burned(&env);
        a.saturating_add(b)
    }

    /// Manual burn for milestone testing: admin burns `amount` of reward tokens.
    /// Increments total_burned counter and checks milestones.
    pub fn burn_tokens(env: Env, admin: Address, amount: i128) -> Result<i128, VaultExtError> {
        admin.require_auth();
        admin::require_admin(&env)?;
        if amount <= 0 {
            return Err(VaultExtError::ZeroAmount);
        }
        balance::add_tokens_burned(&env, amount);
        check_burn_milestones(&env, amount);
        Ok(amount)
    }
}
