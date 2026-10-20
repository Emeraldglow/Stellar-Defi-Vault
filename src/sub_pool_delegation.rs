//! Sub-pool delegation (issue #454).
//!
//! Tiered fund management within one contract: up to 5 named sub-pools each
//! with a delegated sub-admin, independent capacity and reward rate.
//!
//! ## Reward settlement
//!
//! Each user×pool pair carries a lightweight time-weighted reward checkpoint:
//! - `(SUB_RWCK_KEY, user, pool_id)` → `u32` last-settled ledger
//! - `(SUB_RWAC_KEY, user, pool_id)` → `i128` accrued-but-unclaimed reward
//!
//! `settle_sub_pool_rewards` is called automatically on every state-changing
//! operation (stake, unstake, **move**) so that the settled amount always
//! reflects rewards earned up to the current ledger against the position that
//! was active.
//!
//! ## Move semantics
//!
//! `move_between_sub_pools` is an atomic bookkeeping-only operation:
//! tokens are already inside the contract, so no transfer is needed.
//! The source pool's rewards are settled before the position shrinks; the
//! destination's rewards are settled before the position grows.

use soroban_sdk::{
    contractimpl, contracttype, symbol_short, token, Address, Env, String, Symbol, Vec,
};

use crate::admin;
use crate::balance;
use crate::errors::VaultExtError;
use crate::storage::DataKey;
use crate::vault::VaultContract;

// ── Storage keys ─────────────────────────────────────────────────────────────

const SUB_POOLS_KEY: Symbol = symbol_short!("sub_pool");
const SUB_POOL_COUNT_KEY: Symbol = symbol_short!("sub_cnt");
/// Per-user per-pool staked amount:  `(key, user, pool_id)` → `i128`
const SUB_POS_KEY: Symbol = symbol_short!("sub_pos");
/// Per-user per-pool reward checkpoint ledger: `(key, user, pool_id)` → `u32`
const SUB_RWCK_KEY: Symbol = symbol_short!("sub_rwck");
/// Per-user per-pool accrued (unsettled) reward: `(key, user, pool_id)` → `i128`
const SUB_RWAC_KEY: Symbol = symbol_short!("sub_rwac");

/// Stellar ledgers per year at ~5 s/ledger — used for annualised rate math.
const LEDGERS_PER_YEAR: u32 = 6_307_200;

// ── Data types ────────────────────────────────────────────────────────────────

/// One sub-pool managed by a delegated sub-admin.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SubPool {
    pub id: u32,
    pub name: String,
    pub sub_admin: Address,
    /// Annualised reward rate in basis points (≤ main pool rate).
    pub reward_rate_bps: i128,
    pub total_staked: i128,
    /// Hard capacity cap; moves/stakes that would exceed this are rejected.
    pub max_capacity: i128,
}

// ── Private storage helpers ───────────────────────────────────────────────────

fn get_sub_pools(env: &Env) -> Vec<SubPool> {
    env.storage()
        .instance()
        .get(&SUB_POOLS_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_sub_pools(env: &Env, pools: &Vec<SubPool>) {
    env.storage().instance().set(&SUB_POOLS_KEY, pools);
}

fn get_next_sub_pool_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&SUB_POOL_COUNT_KEY)
        .unwrap_or(0)
}

fn set_next_sub_pool_id(env: &Env, id: u32) {
    env.storage().instance().set(&SUB_POOL_COUNT_KEY, &id);
}

fn get_sub_pool_position(env: &Env, user: &Address, sub_pool_id: u32) -> i128 {
    env.storage()
        .persistent()
        .get(&(SUB_POS_KEY, user.clone(), sub_pool_id))
        .unwrap_or(0)
}

fn set_sub_pool_position(env: &Env, user: &Address, sub_pool_id: u32, amount: i128) {
    if amount == 0 {
        env.storage()
            .persistent()
            .remove(&(SUB_POS_KEY, user.clone(), sub_pool_id));
    } else {
        env.storage()
            .persistent()
            .set(&(SUB_POS_KEY, user.clone(), sub_pool_id), &amount);
    }
}

fn get_sub_pool_reward_checkpoint(env: &Env, user: &Address, sub_pool_id: u32) -> u32 {
    env.storage()
        .persistent()
        .get(&(SUB_RWCK_KEY, user.clone(), sub_pool_id))
        .unwrap_or_else(|| env.ledger().sequence())
}

fn set_sub_pool_reward_checkpoint(env: &Env, user: &Address, sub_pool_id: u32, ledger: u32) {
    env.storage()
        .persistent()
        .set(&(SUB_RWCK_KEY, user.clone(), sub_pool_id), &ledger);
}

fn get_sub_pool_accrued(env: &Env, user: &Address, sub_pool_id: u32) -> i128 {
    env.storage()
        .persistent()
        .get(&(SUB_RWAC_KEY, user.clone(), sub_pool_id))
        .unwrap_or(0)
}

fn set_sub_pool_accrued(env: &Env, user: &Address, sub_pool_id: u32, amount: i128) {
    if amount == 0 {
        env.storage()
            .persistent()
            .remove(&(SUB_RWAC_KEY, user.clone(), sub_pool_id));
    } else {
        env.storage()
            .persistent()
            .set(&(SUB_RWAC_KEY, user.clone(), sub_pool_id), &amount);
    }
}

fn find_sub_pool_index(pools: &Vec<SubPool>, id: u32) -> Option<u32> {
    for i in 0..pools.len() {
        if pools.get(i).unwrap().id == id {
            return Some(i);
        }
    }
    None
}

/// Crate-internal: `sub_pool_id`'s current `(total_staked, max_capacity)`,
/// if it exists. Used by keeper-triggered rebalancing
/// (`sub_pool_rebalancing.rs`) to read a pool's current utilization without
/// exposing the whole `SubPool` mutation surface.
pub(crate) fn get_sub_pool_capacity_state(env: &Env, sub_pool_id: u32) -> Option<(i128, i128)> {
    let pools = get_sub_pools(env);
    let idx = find_sub_pool_index(&pools, sub_pool_id)?;
    let pool = pools.get(idx).unwrap();
    Some((pool.total_staked, pool.max_capacity))
}

/// Crate-internal: overwrites `sub_pool_id`'s `max_capacity` in place.
/// Used by keeper-triggered rebalancing (`sub_pool_rebalancing.rs`) to
/// shift capacity between pools without touching `total_staked` or any
/// user's own position.
pub(crate) fn set_sub_pool_max_capacity(
    env: &Env,
    sub_pool_id: u32,
    new_capacity: i128,
) -> Result<(), VaultExtError> {
    let mut pools = get_sub_pools(env);
    let idx = find_sub_pool_index(&pools, sub_pool_id).ok_or(VaultExtError::PositionNotFound)?;
    let mut pool = pools.get(idx).unwrap();
    pool.max_capacity = new_capacity;
    pools.set(idx, pool);
    set_sub_pools(env, &pools);
    Ok(())
}

// ── Reward settlement helper ──────────────────────────────────────────────────

/// Settle time-weighted rewards for `user` in `sub_pool_id` up to the current
/// ledger, using the caller-supplied `position` (the amount staked at the
/// *start* of the interval, before any pending mutation).
///
/// Updates the per-user accrued balance and advances the checkpoint.
/// Returns the newly accrued amount for this interval (for event data).
///
/// Formula:  reward = position × rate_bps × elapsed_ledgers
///                    ─────────────────────────────────────
///                         10 000 × LEDGERS_PER_YEAR
///
/// Integer-division rounding always rounds down (staker-conservative).
fn settle_sub_pool_rewards(
    env: &Env,
    user: &Address,
    sub_pool_id: u32,
    pool_rate_bps: i128,
    position: i128,
) -> i128 {
    let now = env.ledger().sequence();
    let checkpoint = get_sub_pool_reward_checkpoint(env, user, sub_pool_id);

    let new_reward = if position > 0 && pool_rate_bps > 0 && now > checkpoint {
        let elapsed = (now - checkpoint) as i128;
        // reward = position * rate_bps * elapsed / (10_000 * LEDGERS_PER_YEAR)
        position
            .saturating_mul(pool_rate_bps)
            .saturating_mul(elapsed)
            / (10_000_i128 * LEDGERS_PER_YEAR as i128)
    } else {
        0
    };

    if new_reward > 0 {
        let prior = get_sub_pool_accrued(env, user, sub_pool_id);
        set_sub_pool_accrued(env, user, sub_pool_id, prior.saturating_add(new_reward));
    }

    // Always advance checkpoint so the next interval starts from now.
    set_sub_pool_reward_checkpoint(env, user, sub_pool_id, now);

    new_reward
}

// ── Contract entrypoints ──────────────────────────────────────────────────────

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    // ── Admin ────────────────────────────────────────────────────────────────

    /// Admin: create a named sub-pool with a delegated sub-admin.
    pub fn create_sub_pool(
        env: Env,
        admin: Address,
        name: String,
        sub_admin: Address,
        reward_rate_bps: i128,
        max_capacity: i128,
    ) -> Result<u32, VaultExtError> {
        admin.require_auth();
        admin::require_admin(&env)?;
        if max_capacity <= 0 {
            return Err(VaultExtError::ZeroAmount);
        }
        if reward_rate_bps < 0 {
            return Err(VaultExtError::ZeroAmount);
        }
        let main_rate = balance::get_reward_rate_bps(&env) as i128;
        if main_rate > 0 && reward_rate_bps > main_rate {
            return Err(VaultExtError::InvalidFeeAllocation);
        }
        let mut pools = get_sub_pools(&env);
        if pools.len() >= 5 {
            return Err(VaultExtError::TooManyProposals);
        }
        let id = get_next_sub_pool_id(&env);
        let pool = SubPool {
            id,
            name: name.clone(),
            sub_admin: sub_admin.clone(),
            reward_rate_bps,
            total_staked: 0,
            max_capacity,
        };
        pools.push_back(pool);
        set_sub_pools(&env, &pools);
        set_next_sub_pool_id(&env, id + 1);
        env.events().publish(
            (symbol_short!("sub_crt"),),
            (id, name, sub_admin, reward_rate_bps, env.ledger().sequence()),
        );
        Ok(id)
    }

    // ── User operations ───────────────────────────────────────────────────────

    /// Stake `amount` into a specific sub-pool.
    ///
    /// Settles any rewards accrued before the new deposit increases the position.
    pub fn stake_into_sub_pool(
        env: Env,
        user: Address,
        sub_pool_id: u32,
        amount: i128,
    ) -> Result<(), VaultExtError> {
        user.require_auth();
        if amount <= 0 {
            return Err(VaultExtError::ZeroAmount);
        }

        let mut pools = get_sub_pools(&env);
        let idx = find_sub_pool_index(&pools, sub_pool_id)
            .ok_or(VaultExtError::PositionNotFound)?;
        let mut pool = pools.get(idx).unwrap();

        // Capacity check before touching any state.
        if pool.total_staked.saturating_add(amount) > pool.max_capacity {
            return Err(VaultExtError::InvalidFeeAllocation);
        }

        // Settle rewards on the pre-deposit position.
        let prior_position = get_sub_pool_position(&env, &user, sub_pool_id);
        settle_sub_pool_rewards(&env, &user, sub_pool_id, pool.reward_rate_bps, prior_position);

        // Pull tokens from user.
        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(VaultExtError::NotInitialized)?;
        token::Client::new(&env, &token_addr).transfer(
            &user,
            &env.current_contract_address(),
            &amount,
        );

        // Update pool aggregate and user position.
        pool.total_staked = pool.total_staked.saturating_add(amount);
        pools.set(idx, pool);
        set_sub_pools(&env, &pools);
        set_sub_pool_position(&env, &user, sub_pool_id, prior_position.saturating_add(amount));

        env.events().publish(
            (symbol_short!("sub_stk"), user),
            (sub_pool_id, amount, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Unstake `amount` from a specific sub-pool, returning tokens to the user.
    ///
    /// Settles rewards before reducing the position.
    pub fn unstake_from_sub_pool(
        env: Env,
        user: Address,
        sub_pool_id: u32,
        amount: i128,
    ) -> Result<(), VaultExtError> {
        user.require_auth();
        if amount <= 0 {
            return Err(VaultExtError::ZeroAmount);
        }

        let cur = get_sub_pool_position(&env, &user, sub_pool_id);
        if cur < amount {
            return Err(VaultExtError::PositionNotFound);
        }

        let mut pools = get_sub_pools(&env);
        let idx = find_sub_pool_index(&pools, sub_pool_id)
            .ok_or(VaultExtError::PositionNotFound)?;
        let mut pool = pools.get(idx).unwrap();

        // Settle rewards on the pre-withdrawal position.
        settle_sub_pool_rewards(&env, &user, sub_pool_id, pool.reward_rate_bps, cur);

        // Update accounting.
        pool.total_staked = pool.total_staked.saturating_sub(amount);
        pools.set(idx, pool);
        set_sub_pools(&env, &pools);
        set_sub_pool_position(&env, &user, sub_pool_id, cur - amount);

        // Return tokens.
        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(VaultExtError::NotInitialized)?;
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &user,
            &amount,
        );

        env.events().publish(
            (symbol_short!("sub_unstk"), user),
            (sub_pool_id, amount, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Move `amount` of stake from `from_id` to `to_id` atomically.
    ///
    /// No token transfer occurs — the tokens are already held by the contract.
    ///
    /// # Behaviour
    /// - Rewards accrued in **both** pools are settled against the positions
    ///   that existed before the move (source at `cur_from`, destination at
    ///   `cur_to`), so neither side loses earned rewards.
    /// - The destination's `max_capacity` is enforced.
    /// - Moving to the same pool reverts with `ZeroAmount`.
    ///
    /// # Errors
    /// - `ZeroAmount` — `amount ≤ 0` or `from_id == to_id`.
    /// - `PositionNotFound` — unknown pool id or insufficient source position.
    /// - `InvalidFeeAllocation` — destination would exceed `max_capacity`.
    pub fn move_between_sub_pools(
        env: Env,
        user: Address,
        from_id: u32,
        to_id: u32,
        amount: i128,
    ) -> Result<(), VaultExtError> {
        user.require_auth();

        if amount <= 0 {
            return Err(VaultExtError::ZeroAmount);
        }
        if from_id == to_id {
            return Err(VaultExtError::ZeroAmount);
        }

        // ── Validate source position ──────────────────────────────────────
        let cur_from = get_sub_pool_position(&env, &user, from_id);
        if cur_from < amount {
            return Err(VaultExtError::PositionNotFound);
        }

        // ── Load pool list once ───────────────────────────────────────────
        let mut pools = get_sub_pools(&env);
        let from_idx = find_sub_pool_index(&pools, from_id)
            .ok_or(VaultExtError::PositionNotFound)?;
        let to_idx = find_sub_pool_index(&pools, to_id)
            .ok_or(VaultExtError::PositionNotFound)?;

        // Snapshot both pools before mutating.
        let mut from_pool = pools.get(from_idx).unwrap();
        let mut to_pool = pools.get(to_idx).unwrap();

        // ── Capacity check on destination ─────────────────────────────────
        let cur_to = get_sub_pool_position(&env, &user, to_id);
        if to_pool.total_staked.saturating_add(amount) > to_pool.max_capacity {
            return Err(VaultExtError::InvalidFeeAllocation);
        }

        // ── Settle rewards on both sides before changing positions ────────
        // Source: settle against `cur_from` (the position about to shrink).
        settle_sub_pool_rewards(
            &env,
            &user,
            from_id,
            from_pool.reward_rate_bps,
            cur_from,
        );
        // Destination: settle against `cur_to` (the position about to grow).
        settle_sub_pool_rewards(
            &env,
            &user,
            to_id,
            to_pool.reward_rate_bps,
            cur_to,
        );

        // ── Mutate pool aggregates ────────────────────────────────────────
        from_pool.total_staked = from_pool.total_staked.saturating_sub(amount);
        to_pool.total_staked = to_pool.total_staked.saturating_add(amount);
        pools.set(from_idx, from_pool);
        pools.set(to_idx, to_pool);
        set_sub_pools(&env, &pools);

        // ── Mutate user positions ─────────────────────────────────────────
        set_sub_pool_position(&env, &user, from_id, cur_from - amount);
        set_sub_pool_position(&env, &user, to_id, cur_to.saturating_add(amount));

        // ── Event ─────────────────────────────────────────────────────────
        env.events().publish(
            (symbol_short!("sub_move"), user),
            (from_id, to_id, amount, env.ledger().sequence()),
        );
        Ok(())
    }

    // ── Sub-admin ─────────────────────────────────────────────────────────────

    /// Sub-admin (or vault admin) can adjust the reward rate of a sub-pool.
    /// Rate must be ≤ the main pool's reward rate.
    pub fn set_sub_pool_rate(
        env: Env,
        caller: Address,
        sub_pool_id: u32,
        rate_bps: i128,
    ) -> Result<(), VaultExtError> {
        caller.require_auth();
        let is_admin = admin::get_admin(&env)
            .map(|a| a == caller)
            .unwrap_or(false);
        let mut pools = get_sub_pools(&env);
        let idx = find_sub_pool_index(&pools, sub_pool_id)
            .ok_or(VaultExtError::PositionNotFound)?;
        let mut pool = pools.get(idx).unwrap();
        if !is_admin && pool.sub_admin != caller {
            return Err(VaultExtError::Unauthorized);
        }
        if rate_bps < 0 {
            return Err(VaultExtError::ZeroAmount);
        }
        let main_rate = balance::get_reward_rate_bps(&env) as i128;
        if main_rate > 0 && rate_bps > main_rate {
            return Err(VaultExtError::InvalidFeeAllocation);
        }
        pool.reward_rate_bps = rate_bps;
        pools.set(idx, pool);
        set_sub_pools(&env, &pools);
        env.events().publish(
            (symbol_short!("sub_rate"), caller),
            (sub_pool_id, rate_bps, env.ledger().sequence()),
        );
        Ok(())
    }

    // ── Read-only queries ─────────────────────────────────────────────────────

    /// Read-only: a single sub-pool by id.
    pub fn get_sub_pool(env: Env, id: u32) -> Option<SubPool> {
        let pools = get_sub_pools(&env);
        for i in 0..pools.len() {
            let p = pools.get(i).unwrap();
            if p.id == id {
                return Some(p);
            }
        }
        None
    }

    /// Read-only: all sub-pools.
    pub fn get_all_sub_pools(env: Env) -> Vec<SubPool> {
        get_sub_pools(&env)
    }

    /// Read-only: `user`'s staked amount in `sub_pool_id`.
    pub fn get_sub_pool_position(env: Env, user: Address, sub_pool_id: u32) -> i128 {
        get_sub_pool_position(&env, &user, sub_pool_id)
    }

    /// Read-only: rewards already settled (accrued but not yet claimed) for
    /// `user` in `sub_pool_id`.  Does **not** include rewards since the last
    /// checkpoint — call `calc_sub_pool_pending` for the live estimate.
    pub fn get_sub_pool_accrued(env: Env, user: Address, sub_pool_id: u32) -> i128 {
        get_sub_pool_accrued(&env, &user, sub_pool_id)
    }

    /// Read-only: estimated pending reward since the last checkpoint (not yet
    /// settled into the accrued balance).
    pub fn calc_sub_pool_pending(env: Env, user: Address, sub_pool_id: u32) -> i128 {
        let pools = get_sub_pools(&env);
        let idx = match find_sub_pool_index(&pools, sub_pool_id) {
            Some(i) => i,
            None => return 0,
        };
        let pool = pools.get(idx).unwrap();
        let position = get_sub_pool_position(&env, &user, sub_pool_id);
        if position == 0 || pool.reward_rate_bps == 0 {
            return 0;
        }
        let now = env.ledger().sequence();
        let checkpoint = get_sub_pool_reward_checkpoint(&env, &user, sub_pool_id);
        if now <= checkpoint {
            return 0;
        }
        let elapsed = (now - checkpoint) as i128;
        position
            .saturating_mul(pool.reward_rate_bps)
            .saturating_mul(elapsed)
            / (10_000_i128 * LEDGERS_PER_YEAR as i128)
    }
}
