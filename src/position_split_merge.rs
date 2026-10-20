//! Position splitting and merging by id (issue #706).
//!
//! A user's positions are addressed by id: id `0` is the primary position
//! (the user's share balance) and id `n >= 1` is entry `n - 1` of the
//! additive split-position list that `position_split()` (issue #209) already
//! maintains. This module adds the id-addressed operations on top of that
//! list rather than introducing a second storage model.
//!
//! Merging a position removes its list entry, so ids above a removed one
//! shift down by one; re-read `get_split_positions()` after a merge.
//!
//! Both operations settle pending rewards first, so the reward total is
//! unchanged by either of them, and both preserve the total share count.
//!
//! # Compatibility
//!
//! Lock status and boost tier both derive from a position's
//! `staked_at_ledger`, which a split copies to the new position. Two
//! positions are therefore compatible to merge exactly when their
//! `staked_at_ledger` matches.

use soroban_sdk::{contractimpl, Address, Env};

use crate::balance;
use crate::errors::VaultSafetyError;
use crate::events;
use crate::storage::{DataKey, StakePosition};
use crate::vault::{VaultContract, VaultContractClient};

/// `(amount, staked_at_ledger)` of the position `id` for `user`.
fn position_info(env: &Env, user: &Address, id: u32) -> Result<(i128, u32), VaultSafetyError> {
    if id == 0 {
        let shares = balance::get_shares(env, user);
        if shares <= 0 {
            return Err(VaultSafetyError::PositionNotFound);
        }
        let staked_at = env
            .storage()
            .persistent()
            .get::<_, u32>(&DataKey::StakedAtLedger(user.clone()))
            .unwrap_or(0);
        return Ok((shares, staked_at));
    }
    let position = balance::get_split_positions(env, user)
        .get(id - 1)
        .ok_or(VaultSafetyError::PositionNotFound)?;
    Ok((position.amount, position.staked_at_ledger))
}

#[contractimpl]
impl VaultContract {
    /// Split `shares` off position `position_id` into a new, independent
    /// position and return the new position's id (issue #706).
    ///
    /// The new position inherits the source's `staked_at_ledger`, so lock and
    /// boost status are preserved on both halves. Pending rewards are settled
    /// before the split. Counts against the per-user position cap.
    ///
    /// # Authorization
    /// `user` must sign.
    ///
    /// # Errors
    /// - `PositionNotFound`: `position_id` does not exist.
    /// - `InvalidSplitAmount`: `shares` is not strictly between 0 and the
    ///   source position's size.
    /// - `MaxPositionsReached`: the per-user position cap is reached.
    /// - Errors from reward accrual.
    ///
    /// # Events
    /// - `pos_split` (`events::position_split`).
    pub fn split_position(
        env: Env,
        user: Address,
        position_id: u32,
        shares: i128,
    ) -> Result<u32, VaultSafetyError> {
        user.require_auth();

        let (source_amount, staked_at_ledger) = position_info(&env, &user, position_id)?;
        if shares <= 0 || shares >= source_amount {
            return Err(VaultSafetyError::InvalidSplitAmount);
        }
        crate::vault_extensions_546_549::ensure_position_slot(&env, &user)?;

        // Settle pending rewards on the pre-split primary position.
        Self::accrue_rewards(&env, &user, balance::get_shares(&env, &user))?;
        crate::vault::beneficiary_inheritance::touch(&env, &user);

        let current_ledger = env.ledger().sequence();
        let remaining = source_amount - shares;
        let mut splits = balance::get_split_positions(&env, &user);
        if position_id == 0 {
            balance::set_shares(&env, &user, remaining);
            balance::set_last_claim_ledger(&env, &user, current_ledger);
        } else {
            splits.set(
                position_id - 1,
                StakePosition {
                    amount: remaining,
                    staked_at_ledger,
                    last_claim_ledger: current_ledger,
                },
            );
        }
        splits.push_back(StakePosition {
            amount: shares,
            staked_at_ledger,
            last_claim_ledger: current_ledger,
        });
        let new_id = splits.len();
        balance::set_split_positions(&env, &user, &splits);

        events::position_split(&env, &user, source_amount, shares);
        Ok(new_id)
    }

    /// Merge positions `id_a` and `id_b` into one and return the surviving
    /// id, the lower of the two (issue #706).
    ///
    /// Only positions with the same `staked_at_ledger` (and therefore the same
    /// lock and boost parameters) can be merged. Pending rewards are settled
    /// before the merge. Ids above the removed position shift down by one.
    ///
    /// # Authorization
    /// `user` must sign.
    ///
    /// # Errors
    /// - `PositionNotFound`: either id does not exist.
    /// - `IncompatiblePositions`: the ids are equal, or the positions' lock
    ///   and boost parameters differ.
    /// - `ArithmeticError`: overflow summing the positions.
    /// - Errors from reward accrual.
    ///
    /// # Events
    /// - `merge` (`events::positions_merged`) with the merged token amount.
    pub fn merge_positions(
        env: Env,
        user: Address,
        id_a: u32,
        id_b: u32,
    ) -> Result<u32, VaultSafetyError> {
        user.require_auth();

        if id_a == id_b {
            return Err(VaultSafetyError::IncompatiblePositions);
        }
        let (amount_a, staked_a) = position_info(&env, &user, id_a)?;
        let (amount_b, staked_b) = position_info(&env, &user, id_b)?;
        if staked_a != staked_b {
            return Err(VaultSafetyError::IncompatiblePositions);
        }
        let combined = amount_a
            .checked_add(amount_b)
            .ok_or(VaultSafetyError::ArithmeticError)?;

        // Settle pending rewards on the pre-merge primary position.
        Self::accrue_rewards(&env, &user, balance::get_shares(&env, &user))?;
        crate::vault::beneficiary_inheritance::touch(&env, &user);

        let (kept, removed) = if id_a < id_b { (id_a, id_b) } else { (id_b, id_a) };
        let current_ledger = env.ledger().sequence();
        let mut splits = balance::get_split_positions(&env, &user);
        if kept == 0 {
            balance::set_shares(&env, &user, combined);
            balance::set_last_claim_ledger(&env, &user, current_ledger);
        } else {
            splits.set(
                kept - 1,
                StakePosition {
                    amount: combined,
                    staked_at_ledger: staked_a,
                    last_claim_ledger: current_ledger,
                },
            );
        }
        splits.remove(removed - 1);
        balance::set_split_positions(&env, &user, &splits);

        let total_amount = balance::shares_to_amount(
            balance::get_total_shares(&env),
            balance::get_total_deposited(&env),
            combined,
        )
        .ok_or(VaultSafetyError::ArithmeticError)?;
        events::positions_merged(&env, &user, 2, total_amount);
        Ok(kept)
    }
}
