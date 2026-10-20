//! Issue #590: recover foreign tokens sent straight to the vault address.
//!
//! Admin only. Sweeps the vault's entire balance of `token` to `to`, unless
//! `token` is the vault's stake token or its reward token: sweeping those
//! would take user funds, so it reverts with `CannotSweepVaultToken`.

use soroban_sdk::{contracterror, contractimpl, token, Address, Env, Symbol};

use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient};
use crate::{admin, balance};

/// Errors for `sweep_foreign_tokens`.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SweepError {
    Unauthorized = 1,
    NotInitialized = 2,
    /// `token` is the vault's stake token or reward token.
    CannotSweepVaultToken = 3,
}

#[contractimpl]
impl VaultContract {
    /// Admin: send the vault's full balance of a foreign `token` to `to` and
    /// return the amount swept. A zero balance is a no-op that returns `0`
    /// (no transfer, no event). Emits `foreign_tokens_swept` as
    /// `(token, amount, to, ledger)`.
    pub fn sweep_foreign_tokens(
        env: Env,
        admin: Address,
        token: Address,
        to: Address,
    ) -> Result<i128, SweepError> {
        admin.require_auth();
        if admin != admin::get_admin(&env).map_err(|_| SweepError::NotInitialized)? {
            return Err(SweepError::Unauthorized);
        }
        let stake_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(SweepError::NotInitialized)?;
        if token == stake_token || balance::get_reward_token(&env) == Some(token.clone()) {
            return Err(SweepError::CannotSweepVaultToken);
        }

        let client = token::Client::new(&env, &token);
        let amount = client.balance(&env.current_contract_address());
        if amount <= 0 {
            return Ok(0);
        }
        client.transfer(&env.current_contract_address(), &to, &amount);
        env.events().publish(
            (Symbol::new(&env, "foreign_tokens_swept"),),
            (token, amount, to, env.ledger().sequence()),
        );
        Ok(amount)
    }
}
