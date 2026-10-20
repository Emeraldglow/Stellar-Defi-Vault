//! Keeper-triggered sub-pool rebalancing within admin bounds.
//!
//! `sub_pool_delegation.rs` lets a sub-pool's sub-admin tune its own reward
//! rate, and lets a staker move their own position between pools, but
//! nothing reallocates *capacity* across pools as utilization drifts over
//! time. This module adds that: the admin sets a `[min_capacity,
//! max_capacity]` band for each sub-pool, and any registered keeper
//! (`keeper_registry.rs`) can shift capacity from an under-utilized pool
//! into a constrained one, strictly within those admin-set bounds and a
//! configurable per-call cap and cooldown.
//!
//! # What moves, and what doesn't
//! A rebalance only ever adjusts a `SubPool`'s `max_capacity` (the hard cap
//! on how much it may hold) — it never touches `total_staked` or any
//! user's own position. No tokens move and no user's reward checkpoint is
//! disturbed; the vault's existing capital stays exactly where its stakers
//! put it. This keeps rebalancing bookkeeping-only, like
//! `move_between_sub_pools`, and leaves `sub_pool_delegation.rs`'s own
//! invariant (`total_staked` is the sum of user positions) untouched.
//!
//! # Bounds enforced
//! - The source pool's capacity may never drop below its own
//!   `total_staked` (can't cap a pool below what's already parked there)
//!   or below its admin-configured `min_capacity`.
//! - The destination pool's capacity may never exceed its admin-configured
//!   `max_capacity`.
//! - Per call, the shifted amount is capped by the admin-configured
//!   `max_rebalance_amount` (`0` = unlimited).
//! - Calls are throttled by an admin-configured cooldown tracked globally
//!   (`0` = no throttle), so a single compromised or misbehaving keeper
//!   can't thrash the allocation.
//! - The caller must be a currently-active registered keeper
//!   (`keeper_registry::is_registered`); a successful rebalance also counts
//!   toward that keeper's on-chain track record.
//!
//! # Storage
//! `DataKey`/`VaultExtError` both sit at Soroban's 50-variant cap, so this
//! uses raw `Symbol`-keyed storage and its own error type, matching
//! `ttl_management.rs`/`user_ttl.rs`.

use soroban_sdk::{contracterror, contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::vault::keeper_registry;
use crate::vault::sub_pool_delegation;
use crate::vault::VaultContract;

/// Instance key prefix: `(BOUNDS_KEY, sub_pool_id)` -> `(min_capacity, max_capacity)`.
const BOUNDS_KEY: Symbol = symbol_short!("rb_bound");
/// Instance key: max amount of capacity a single rebalance call may shift. `0` = unlimited.
const MAX_AMOUNT_KEY: Symbol = symbol_short!("rb_max");
/// Instance key: minimum ledgers required between rebalance calls. `0` = no throttle.
const COOLDOWN_KEY: Symbol = symbol_short!("rb_cool");
/// Instance key: the ledger of the last successful rebalance.
const LAST_LEDGER_KEY: Symbol = symbol_short!("rb_last");

/// Errors for the sub-pool rebalancing entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RebalanceError {
    NotInitialized = 1,
    ZeroAmount = 2,
    PositionNotFound = 3,
    /// `from_id == to_id`.
    SamePool = 4,
    /// Caller is not a currently-active registered keeper.
    KeeperNotRegistered = 5,
    /// `amount` exceeds the configured per-call cap, or over/underflows a bound.
    AmountTooLarge = 6,
    /// Called before `cooldown_ledgers` have elapsed since the last rebalance.
    CooldownActive = 7,
    /// Shifting `amount` out of the source pool would drop its capacity
    /// below its own `total_staked` or its configured `min_capacity`.
    SourceBoundViolated = 8,
    /// Shifting `amount` into the destination pool would push its capacity
    /// above its configured `max_capacity`.
    DestinationBoundViolated = 9,
    /// No rebalance bounds have been configured yet for one of the pools.
    BoundsNotConfigured = 10,
    /// `min_capacity > max_capacity`, or either is negative.
    InvalidBounds = 11,
}

fn get_bounds(env: &Env, sub_pool_id: u32) -> Option<(i128, i128)> {
    env.storage().instance().get(&(BOUNDS_KEY, sub_pool_id))
}

fn set_bounds(env: &Env, sub_pool_id: u32, min_capacity: i128, max_capacity: i128) {
    env.storage()
        .instance()
        .set(&(BOUNDS_KEY, sub_pool_id), &(min_capacity, max_capacity));
}

/// The configured per-call cap on how much capacity a single rebalance may
/// shift. `0` means unlimited.
pub fn max_rebalance_amount(env: &Env) -> i128 {
    env.storage().instance().get(&MAX_AMOUNT_KEY).unwrap_or(0)
}

/// The configured minimum number of ledgers required between rebalance
/// calls. `0` means no throttle.
pub fn cooldown_ledgers(env: &Env) -> u32 {
    env.storage().instance().get(&COOLDOWN_KEY).unwrap_or(0)
}

/// The ledger of the last successful rebalance, `0` if none has ever happened.
pub fn last_rebalance_ledger(env: &Env) -> u32 {
    env.storage().instance().get(&LAST_LEDGER_KEY).unwrap_or(0)
}

#[contractimpl]
impl VaultContract {
    /// Admin: set the `[min_capacity, max_capacity]` band that
    /// `rebalance_sub_pool_capacity` must keep `sub_pool_id`'s capacity
    /// within. Does not itself change the pool's *current* `max_capacity`
    /// (set by `create_sub_pool`) — only the range a future rebalance may
    /// move it within.
    pub fn set_sub_pool_rebalance_bounds(
        env: Env,
        sub_pool_id: u32,
        min_capacity: i128,
        max_capacity: i128,
    ) -> Result<(), RebalanceError> {
        admin::require_admin(&env).map_err(|_| RebalanceError::NotInitialized)?;
        if min_capacity < 0 || max_capacity < min_capacity {
            return Err(RebalanceError::InvalidBounds);
        }
        if sub_pool_delegation::get_sub_pool_capacity_state(&env, sub_pool_id).is_none() {
            return Err(RebalanceError::PositionNotFound);
        }
        set_bounds(&env, sub_pool_id, min_capacity, max_capacity);
        env.events().publish(
            (symbol_short!("rb_bnd"),),
            (sub_pool_id, min_capacity, max_capacity, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Admin: configure the per-call cap on how much capacity a single
    /// rebalance may shift (`0` = unlimited) and the minimum number of
    /// ledgers required between rebalance calls (`0` = no throttle).
    pub fn set_sub_pool_rebalance_limits(
        env: Env,
        max_amount: i128,
        cooldown: u32,
    ) -> Result<(), RebalanceError> {
        admin::require_admin(&env).map_err(|_| RebalanceError::NotInitialized)?;
        if max_amount < 0 {
            return Err(RebalanceError::ZeroAmount);
        }
        env.storage().instance().set(&MAX_AMOUNT_KEY, &max_amount);
        env.storage().instance().set(&COOLDOWN_KEY, &cooldown);
        Ok(())
    }

    /// Keeper-triggered: shift `amount` of *capacity* (never staked
    /// principal) from `from_id` to `to_id`, within the admin-configured
    /// bounds for both pools, the admin-configured per-call cap, and the
    /// admin-configured cooldown since the last rebalance. No tokens move
    /// and no user position changes.
    pub fn rebalance_sub_pool_capacity(
        env: Env,
        keeper: Address,
        from_id: u32,
        to_id: u32,
        amount: i128,
    ) -> Result<(), RebalanceError> {
        keeper.require_auth();
        if !keeper_registry::is_registered(&env, &keeper) {
            return Err(RebalanceError::KeeperNotRegistered);
        }
        if amount <= 0 {
            return Err(RebalanceError::ZeroAmount);
        }
        if from_id == to_id {
            return Err(RebalanceError::SamePool);
        }

        let max_amount = max_rebalance_amount(&env);
        if max_amount > 0 && amount > max_amount {
            return Err(RebalanceError::AmountTooLarge);
        }

        let cooldown = cooldown_ledgers(&env);
        let now = env.ledger().sequence();
        if cooldown > 0 {
            let last = last_rebalance_ledger(&env);
            if last > 0 && now.saturating_sub(last) < cooldown {
                return Err(RebalanceError::CooldownActive);
            }
        }

        let (from_min, _from_max_bound) =
            get_bounds(&env, from_id).ok_or(RebalanceError::BoundsNotConfigured)?;
        let (_to_min, to_max_bound) =
            get_bounds(&env, to_id).ok_or(RebalanceError::BoundsNotConfigured)?;

        let (from_staked, from_capacity) =
            sub_pool_delegation::get_sub_pool_capacity_state(&env, from_id)
                .ok_or(RebalanceError::PositionNotFound)?;
        let (_to_staked, to_capacity) =
            sub_pool_delegation::get_sub_pool_capacity_state(&env, to_id)
                .ok_or(RebalanceError::PositionNotFound)?;

        let new_from_capacity = from_capacity
            .checked_sub(amount)
            .ok_or(RebalanceError::AmountTooLarge)?;
        if new_from_capacity < from_staked || new_from_capacity < from_min {
            return Err(RebalanceError::SourceBoundViolated);
        }

        let new_to_capacity = to_capacity
            .checked_add(amount)
            .ok_or(RebalanceError::AmountTooLarge)?;
        if new_to_capacity > to_max_bound {
            return Err(RebalanceError::DestinationBoundViolated);
        }

        sub_pool_delegation::set_sub_pool_max_capacity(&env, from_id, new_from_capacity)
            .map_err(|_| RebalanceError::PositionNotFound)?;
        sub_pool_delegation::set_sub_pool_max_capacity(&env, to_id, new_to_capacity)
            .map_err(|_| RebalanceError::PositionNotFound)?;

        env.storage().instance().set(&LAST_LEDGER_KEY, &now);
        keeper_registry::record_keeper_action(&env, &keeper, 0);

        env.events().publish(
            (symbol_short!("rb_done"), keeper),
            (from_id, to_id, amount, now),
        );

        Ok(())
    }

    /// Read-only: the `[min_capacity, max_capacity]` rebalance band
    /// configured for `sub_pool_id`, if any.
    pub fn get_sub_pool_rebalance_bounds(env: Env, sub_pool_id: u32) -> Option<(i128, i128)> {
        get_bounds(&env, sub_pool_id)
    }

    /// Read-only: the per-call cap on how much capacity a single rebalance
    /// may shift. `0` means unlimited.
    pub fn get_max_rebalance_amount(env: Env) -> i128 {
        max_rebalance_amount(&env)
    }

    /// Read-only: the minimum number of ledgers required between rebalance
    /// calls. `0` means no throttle.
    pub fn get_rebalance_cooldown_ledgers(env: Env) -> u32 {
        cooldown_ledgers(&env)
    }

    /// Read-only: the ledger of the last successful rebalance, `0` if none
    /// has ever happened.
    pub fn get_last_rebalance_ledger(env: Env) -> u32 {
        last_rebalance_ledger(&env)
    }
}
