//! External price oracle integration for collateral valuation (issue #528).
//!
//! Registers an oracle contract implementing a standard price-feed interface
//! (e.g. Reflector or similar Stellar-ecosystem oracle) so external protocols
//! can value a position in a reference currency (e.g. USD) rather than only in
//! the underlying deposit token.
//!
//! # Storage
//!
//! Raw `Symbol`-keyed instance storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultFeature2Error;
use crate::balance;
use crate::vault::{VaultContract, VaultContractClient};

const ORACLE_KEY: Symbol = symbol_short!("ext_orcl");

/// Client for the standard price-feed interface the registered oracle must
/// implement: `fn get_price(token: Address) -> i128`, returning the price in
/// the smallest unit (e.g. cents). Only the generated client is called; the
/// trait itself just declares that client's shape.
#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "PriceOracleClient")]
pub trait PriceOracleInterface {
    fn get_price(env: Env, token: Address) -> i128;
}

/// Register an oracle contract implementing a standard price-feed interface.
pub fn set_oracle(env: &Env, oracle: &Address) {
    env.storage().instance().set(&ORACLE_KEY, oracle);
}

/// Read the registered oracle address, if any.
pub fn get_oracle(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ORACLE_KEY)
}

#[contractimpl]
impl VaultContract {
    /// Admin registers an oracle contract for collateral valuation.
    pub fn set_price_oracle(env: Env, oracle: Address) -> Result<(), VaultFeature2Error> {
        admin::require_admin(&env)?;

        set_oracle(&env, &oracle);

        env.events().publish(
            (symbol_short!("orcl_upd"),),
            (oracle, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Returns the user's position value in USD by combining the local
    /// share balance with the oracle's current price.
    /// Reverts with `NoOracleConfigured` if no oracle is set.
    pub fn get_position_value_usd(env: Env, user: Address) -> Result<i128, VaultFeature2Error> {
        let oracle_addr = get_oracle(&env)
            .ok_or(VaultFeature2Error::NoOracleConfigured)?;

        // Get the user's redeemable value in the vault's token.
        let shares = balance::get_shares(&env, &user);
        if shares <= 0 {
            return Ok(0);
        }

        let token_addr = VaultContract::token_address(&env)
            .map_err(|_| VaultFeature2Error::NotInitialized)?;

        let price_per_token: i128 = PriceOracleClient::new(&env, &oracle_addr)
            .get_price(&token_addr);

        // Calculate total value: shares * price_per_token.
        // Assumes price is in 7-decimal fixed point (same as the vault's token).
        let value_usd = shares
            .checked_mul(price_per_token)
            .ok_or(VaultFeature2Error::ArithmeticError)?;

        Ok(value_usd)
    }
}
