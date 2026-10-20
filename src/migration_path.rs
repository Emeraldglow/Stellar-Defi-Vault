//! Contract migration path for vault v2 upgrades.
//!
//! Provides `set_migration_target` (admin registers successor contract) and
//! `migrate_position` (atomically moves a user's full position to the new vault
//! without a manual withdraw-then-redeposit).
//!
//! # Cross-contract interface
//!
//! The successor vault must implement `MigrationVaultInterface`:
//!   `migrate_in(user: Address, amount: i128) -> i128`
//!
//! `migrate_in` is called after this vault has already transferred the tokens
//! to the new vault, so it must mint shares for `user` proportional to
//! `amount` without re-pulling tokens from the user's wallet.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap; a raw `Symbol`-keyed instance
//! entry is used instead (matching the established pattern in this crate).

use soroban_sdk::{contractimpl, symbol_short, token, Address, Env};

use crate::{
    admin,
    balance,
    errors::VaultError,
    storage::DataKey,
    VaultContract,
};

const MIGRATION_TARGET_KEY: soroban_sdk::Symbol = symbol_short!("mig_tgt");

/// Minimal interface for the successor vault contract.
///
/// `migrate_in` is called by this vault after it has transferred `amount`
/// tokens directly to the new vault. The new vault must mint shares for
/// `user` without pulling tokens again and return the number of shares minted.
#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "MigrationVaultClient")]
pub trait MigrationVaultInterface {
    fn migrate_in(env: Env, user: Address, amount: i128) -> i128;
}

/// Store the migration target address. Returns `None` if unset.
pub fn get_target(env: &Env) -> Option<Address> {
    env.storage().instance().get(&MIGRATION_TARGET_KEY)
}

fn store_target(env: &Env, target: &Address) {
    env.storage().instance().set(&MIGRATION_TARGET_KEY, target);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Admin: register the successor vault contract address.
    ///
    /// Once set, users may call `migrate_position` to move their position
    /// atomically. Overwrites any previously registered target.
    pub fn set_migration_target(
        env: Env,
        admin: Address,
        new_vault: Address,
    ) -> Result<(), VaultError> {
        admin.require_auth();
        if admin != crate::admin::get_admin(&env)? {
            return Err(VaultError::Unauthorized);
        }
        if new_vault == env.current_contract_address() {
            return Err(VaultError::InvalidAddress);
        }
        store_target(&env, &new_vault);
        env.events().publish(
            (symbol_short!("mig_set"),),
            new_vault,
        );
        Ok(())
    }

    /// Read-only: the currently registered migration target, or `None`.
    pub fn get_migration_target(env: Env) -> Option<Address> {
        get_target(&env)
    }

    /// Move the caller's full position from this vault to the registered
    /// successor vault in a single transaction.
    ///
    /// 1. The user's shares are converted to the underlying token amount.
    /// 2. Share and deposit accounting on this vault is updated (position closed).
    /// 3. Tokens are transferred directly from this vault to the new vault.
    /// 4. The new vault's `migrate_in(user, amount)` is called to mint shares.
    ///
    /// Reverts with `NotInitialized` when no migration target has been set,
    /// `PositionNotFound` when the user has no active position, and
    /// `ArithmeticError` on share conversion overflow.
    pub fn migrate_position(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let new_vault = get_target(&env).ok_or(VaultError::NotInitialized)?;

        let shares = balance::get_shares(&env, &user);
        if shares == 0 {
            return Err(VaultError::PositionNotFound);
        }

        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let amount = balance::shares_to_amount(total_shares, total_deposited, shares)
            .ok_or(VaultError::ArithmeticError)?;

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(VaultError::NotInitialized)?;

        // Close the position on this vault.
        balance::set_shares(&env, &user, 0);
        balance::set_total_shares(&env, total_shares - shares);
        balance::set_total_deposited(&env, total_deposited - amount);

        // Push tokens directly to the new vault, then call migrate_in.
        let token_client = token::Client::new(&env, &token_addr);
        token_client.transfer(&env.current_contract_address(), &new_vault, &amount);

        let new_vault_client = MigrationVaultClient::new(&env, &new_vault);
        let minted_shares = new_vault_client.migrate_in(&user, &amount);

        env.events().publish(
            (symbol_short!("migrated"),),
            (user, amount),
        );
        Ok(minted_shares)
    }
}
