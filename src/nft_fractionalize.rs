//! NFT receipt fractionalization.
//!
//! Splits a staker's NFT receipt into N fungible fraction tokens, each
//! representing a proportional claim on the underlying staking position.
//! Enables secondary market trading of staking positions.
//!
//! While fractionalized, the original staker cannot unstake â€” the position is
//! locked until reconstruction burns all fraction tokens and restores the NFT.
//!
//! # Storage
//!
//! Uses raw `Symbol`-keyed persistent storage, matching `balance.rs` and
//! `commitment.rs`, since `DataKey` is at Soroban's 50-variant cap.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Minimum number of fractions per NFT.
pub const MIN_FRACTIONS: u32 = 2;

/// Maximum number of fractions per NFT.
pub const MAX_FRACTIONS: u32 = 1000;

/// Persistent-storage key: whether a user's NFT is fractionalized.
/// Keyed by `(FR_LOCKED_KEY, user)`.
const FR_LOCKED_KEY: Symbol = symbol_short!("fr_lock");

/// Persistent-storage key: metadata for a fractionalized NFT.
/// Keyed by `(FR_META_KEY, owner)`.
const FR_META_KEY: Symbol = symbol_short!("fr_meta");

/// Persistent-storage key: fraction token holders for a fractionalized NFT.
/// Keyed by `(FR_HOLDERS_KEY, owner)`.
const FR_HOLDERS_KEY: Symbol = symbol_short!("fr_hold");

/// Persistent-storage key: fraction balance.
/// Keyed by `(FR_BAL_KEY, owner, holder)`.
const FR_BAL_KEY: Symbol = symbol_short!("fr_bal");

// â”€â”€ storage helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn is_locked(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&(FR_LOCKED_KEY, user.clone()))
        .unwrap_or(false)
}

/// Public check: whether a user's position is locked due to fractionalization.
/// Used by `do_unstake` to reject unstakes while fractions are outstanding.
pub fn is_position_locked(env: &Env, user: &Address) -> bool {
    is_locked(env, user)
}

fn set_locked(env: &Env, user: &Address, locked: bool) {
    env.storage()
        .persistent()
        .set(&(FR_LOCKED_KEY, user.clone()), &locked);
}

fn get_meta(env: &Env, owner: &Address) -> Option<FractionalizedNFT> {
    env.storage()
        .persistent()
        .get(&(FR_META_KEY, owner.clone()))
}

fn set_meta(env: &Env, owner: &Address, meta: &FractionalizedNFT) {
    env.storage()
        .persistent()
        .set(&(FR_META_KEY, owner.clone()), meta);
}

fn remove_meta(env: &Env, owner: &Address) {
    env.storage()
        .persistent()
        .remove(&(FR_META_KEY, owner.clone()));
}

fn get_holders(env: &Env, owner: &Address) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&(FR_HOLDERS_KEY, owner.clone()))
        .unwrap_or(Vec::new(env))
}

fn set_holders(env: &Env, owner: &Address, holders: &Vec<Address>) {
    env.storage()
        .persistent()
        .set(&(FR_HOLDERS_KEY, owner.clone()), holders);
}

fn remove_holders(env: &Env, owner: &Address) {
    env.storage()
        .persistent()
        .remove(&(FR_HOLDERS_KEY, owner.clone()));
}

fn get_frac_balance(env: &Env, owner: &Address, holder: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&(FR_BAL_KEY, owner.clone(), holder.clone()))
        .unwrap_or(0)
}

fn set_frac_balance(env: &Env, owner: &Address, holder: &Address, amount: u32) {
    env.storage()
        .persistent()
        .set(&(FR_BAL_KEY, owner.clone(), holder.clone()), &amount);
}

fn remove_frac_balance(env: &Env, owner: &Address, holder: &Address) {
    env.storage()
        .persistent()
        .remove(&(FR_BAL_KEY, owner.clone(), holder.clone()));
}

/// Metadata for a fractionalized NFT.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct FractionalizedNFT {
    pub owner: Address,
    pub total_fractions: u32,
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Fractionalize an NFT receipt into `num_fractions` fungible tokens.
    ///
    /// The NFT must exist and not already be fractionalized. `num_fractions`
    /// must be between 2 and 1000. The caller's position is locked â€” they
    /// cannot unstake until all fractions are returned via `reconstruct_nft`.
    pub fn fractionalize_nft(
        env: Env,
        user: Address,
        num_fractions: u32,
    ) -> Result<(), VaultError> {
        user.require_auth();

        if num_fractions < MIN_FRACTIONS || num_fractions > MAX_FRACTIONS {
            return Err(VaultError::InvalidRate);
        }

        // Must have an active staking position.
        let shares = balance::get_shares(&env, &user);
        if shares == 0 {
            return Err(VaultError::PositionNotFound);
        }

        // Must not already be fractionalized.
        if is_locked(&env, &user) {
            return Err(VaultError::AlreadyInitialized);
        }

        // Lock the position.
        set_locked(&env, &user, true);

        let meta = FractionalizedNFT {
            owner: user.clone(),
            total_fractions: num_fractions,
        };
        set_meta(&env, &user, &meta);

        // All fractions start with the owner.
        let mut holders: Vec<Address> = Vec::new(&env);
        holders.push_back(user.clone());
        set_holders(&env, &user, &holders);
        set_frac_balance(&env, &user, &user, num_fractions);

        env.events().publish(
            (symbol_short!("fr_frac"), user),
            (num_fractions, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Transfer fraction tokens from the caller to `to`.
    ///
    /// Both caller and `to` must be registered holders. The caller must have
    /// at least `amount` fraction tokens.
    pub fn transfer_fractions(
        env: Env,
        owner: Address,
        to: Address,
        amount: u32,
    ) -> Result<(), VaultError> {
        let from = owner.clone();
        from.require_auth();

        if amount == 0 {
            return Err(VaultError::ZeroAmount);
        }

        if !is_locked(&env, &owner) {
            return Err(VaultError::PositionNotFound);
        }

        let from_bal = get_frac_balance(&env, &owner, &from);
        if from_bal < amount {
            return Err(VaultError::InsufficientShares);
        }

        // Update balances.
        let new_from = from_bal - amount;
        if new_from == 0 {
            remove_frac_balance(&env, &owner, &from);
        } else {
            set_frac_balance(&env, &owner, &from, new_from);
        }

        let to_bal = get_frac_balance(&env, &owner, &to);
        set_frac_balance(&env, &owner, &to, to_bal + amount);

        // Register `to` as a holder if not already present.
        let mut holders = get_holders(&env, &owner);
        let mut already_holder = false;
        for h in holders.iter() {
            if h == to {
                already_holder = true;
                break;
            }
        }
        if !already_holder {
            holders.push_back(to);
            set_holders(&env, &owner, &holders);
        }

        Ok(())
    }

    /// Reconstruct the NFT by burning all fraction tokens.
    ///
    /// All fractions must have been returned to the original owner. The NFT is
    /// restored and the position is unlocked, allowing unstaking again.
    pub fn reconstruct_nft(env: Env, user: Address) -> Result<(), VaultError> {
        user.require_auth();

        if !is_locked(&env, &user) {
            return Err(VaultError::PositionNotFound);
        }

        let meta = get_meta(&env, &user).ok_or(VaultError::PositionNotFound)?;

        // Verify all fractions are back with the owner.
        let owner_bal = get_frac_balance(&env, &user, &user);
        if owner_bal != meta.total_fractions {
            return Err(VaultError::InsufficientShares);
        }

        // Clean up all storage.
        let holders = get_holders(&env, &user);
        for holder in holders.iter() {
            remove_frac_balance(&env, &user, &holder);
        }
        remove_holders(&env, &user);
        remove_meta(&env, &user);
        set_locked(&env, &user, false);

        env.events().publish(
            (symbol_short!("fr_recn"), user),
            (meta.total_fractions, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only query: check if a user's position is locked due to
    /// fractionalization.
    pub fn is_position_fractionalized(env: Env, user: Address) -> bool {
        is_locked(&env, &user)
    }

    /// Read-only query: return fraction balance for a holder of the given
    /// owner's fractionalized NFT.
    pub fn get_fraction_balance(env: Env, owner: Address, holder: Address) -> u32 {
        get_frac_balance(&env, &owner, &holder)
    }

    /// Read-only query: return the list of current fraction holders for the
    /// given owner's fractionalized NFT.
    pub fn get_fraction_holders(env: Env, owner: Address) -> Vec<Address> {
        get_holders(&env, &owner)
    }
}















