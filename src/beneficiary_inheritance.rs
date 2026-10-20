//! Beneficiary designation with an inactivity trigger (issue #707).
//!
//! An owner names a beneficiary and an inactivity period. If the owner makes
//! no contract interaction for that many ledgers, the beneficiary may take
//! over the whole position with `claim_inheritance`.
//!
//! Unlike an emergency contact, this moves funds: the primary position, its
//! accrued rewards and any split positions all go to the beneficiary.
//!
//! # What counts as an owner interaction
//!
//! Deposits and withdrawals (through the activity log), `claim`,
//! `claim_partial`, `lock_position`, `split_position`, `merge_positions` and
//! `set_beneficiary` itself. Each resets the clock via [`touch`].
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! persistent storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::balance;
use crate::errors::VaultSafetyError;
use crate::vault::{VaultContract, VaultContractClient};

/// Persistent-storage key prefix: `(BENEFICIARY_KEY, owner) -> Beneficiary`.
const BENEFICIARY_KEY: Symbol = symbol_short!("benef");

/// A designated beneficiary and the owner's inactivity clock.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct Beneficiary {
    pub beneficiary: Address,
    pub inactivity_ledgers: u32,
    pub last_interaction: u32,
}

fn get_record(env: &Env, user: &Address) -> Option<Beneficiary> {
    env.storage()
        .persistent()
        .get(&(BENEFICIARY_KEY, user.clone()))
}

/// Reset `user`'s inactivity clock. A no-op for users without a beneficiary,
/// so the common path costs one storage read.
pub(crate) fn touch(env: &Env, user: &Address) {
    if let Some(mut record) = get_record(env, user) {
        record.last_interaction = env.ledger().sequence();
        env.storage()
            .persistent()
            .set(&(BENEFICIARY_KEY, user.clone()), &record);
    }
}

#[contractimpl]
impl VaultContract {
    /// Name `beneficiary` to take over `user`'s position after
    /// `inactivity_ledgers` ledgers without an owner interaction (issue #707).
    /// Replaces any previous designation and starts the clock now.
    ///
    /// # Authorization
    /// `user` must sign.
    ///
    /// # Errors
    /// - `ZeroAmount`: `inactivity_ledgers` is 0.
    /// - `InvalidBeneficiary`: `beneficiary` is `user`.
    ///
    /// # Events
    /// - `ben_set` with the beneficiary, period and ledger.
    pub fn set_beneficiary(
        env: Env,
        user: Address,
        beneficiary: Address,
        inactivity_ledgers: u32,
    ) -> Result<(), VaultSafetyError> {
        user.require_auth();
        if inactivity_ledgers == 0 {
            return Err(VaultSafetyError::ZeroAmount);
        }
        if beneficiary == user {
            return Err(VaultSafetyError::InvalidBeneficiary);
        }
        let ledger = env.ledger().sequence();
        env.storage().persistent().set(
            &(BENEFICIARY_KEY, user.clone()),
            &Beneficiary {
                beneficiary: beneficiary.clone(),
                inactivity_ledgers,
                last_interaction: ledger,
            },
        );
        env.events().publish(
            (symbol_short!("ben_set"), user),
            (beneficiary, inactivity_ledgers, ledger),
        );
        Ok(())
    }

    /// Remove `user`'s beneficiary designation (issue #707).
    ///
    /// # Authorization
    /// `user` must sign.
    ///
    /// # Errors
    /// - `NoBeneficiary`: none is set.
    ///
    /// # Events
    /// - `ben_clr` with the ledger.
    pub fn clear_beneficiary(env: Env, user: Address) -> Result<(), VaultSafetyError> {
        user.require_auth();
        if get_record(&env, &user).is_none() {
            return Err(VaultSafetyError::NoBeneficiary);
        }
        env.storage()
            .persistent()
            .remove(&(BENEFICIARY_KEY, user.clone()));
        env.events()
            .publish((symbol_short!("ben_clr"), user), (env.ledger().sequence(),));
        Ok(())
    }

    /// Read-only: `user`'s beneficiary designation and inactivity clock
    /// (issue #707).
    pub fn get_beneficiary(env: Env, user: Address) -> Option<Beneficiary> {
        get_record(&env, &user)
    }

    /// Take over `user`'s whole position as their designated beneficiary,
    /// once `user` has made no interaction for `inactivity_ledgers` ledgers
    /// (issue #707). The primary position, its accrued rewards and any split
    /// positions move to `beneficiary`, and the designation is consumed.
    ///
    /// # Authorization
    /// `beneficiary` must sign.
    ///
    /// # Errors
    /// - `NoBeneficiary`: `user` has no designation.
    /// - `Unauthorized`: `beneficiary` is not the one `user` named.
    /// - `InactivityPeriodNotElapsed`: the inactivity period has not passed.
    /// - `PositionNotFound`: `user` holds no position.
    /// - `RecipientAlreadyStaking`: `beneficiary` already holds a position.
    /// - `VaultPaused`: the pool is paused.
    ///
    /// # Events
    /// - `pos_xf_rw` (`events::position_transferred_with_rewards`) and
    ///   `ben_claim` with the beneficiary and ledger.
    pub fn claim_inheritance(
        env: Env,
        beneficiary: Address,
        user: Address,
    ) -> Result<(), VaultSafetyError> {
        beneficiary.require_auth();
        let record = get_record(&env, &user).ok_or(VaultSafetyError::NoBeneficiary)?;
        if record.beneficiary != beneficiary {
            return Err(VaultSafetyError::Unauthorized);
        }
        let claimable_at = record
            .last_interaction
            .checked_add(record.inactivity_ledgers)
            .ok_or(VaultSafetyError::ArithmeticError)?;
        if !crate::ledger_boundary::is_reached(env.ledger().sequence(), claimable_at) {
            return Err(VaultSafetyError::InactivityPeriodNotElapsed);
        }

        Self::move_position_with_rewards(&env, &user, &beneficiary)?;

        // The primary position is now the beneficiary's: fix the staker
        // registry (remove first so the registered count ends up correct).
        Self::remove_from_staker_list(&env, &user);
        balance::register_staker(&env, &beneficiary);

        // Split positions are part of the full position.
        let splits = balance::get_split_positions(&env, &user);
        if !splits.is_empty() {
            let mut moved = balance::get_split_positions(&env, &beneficiary);
            for position in splits.iter() {
                moved.push_back(position);
            }
            balance::set_split_positions(&env, &beneficiary, &moved);
            balance::set_split_positions(&env, &user, &Vec::new(&env));
        }

        env.storage()
            .persistent()
            .remove(&(BENEFICIARY_KEY, user.clone()));
        env.events().publish(
            (symbol_short!("ben_claim"), user),
            (beneficiary, env.ledger().sequence()),
        );
        Ok(())
    }
}
