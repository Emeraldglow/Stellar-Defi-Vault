//! Issue #489: multi-token vault support.
//!
//! The token set at `initialize` keeps using `deposit`/`stake` with the
//! existing global share accounting. On top of that the admin can approve up to
//! `MAX_SUPPORTED_TOKENS` additional deposit tokens; each gets its own
//! `TokenPool` (shares and deposits), so the share price is computed per token
//! and never mixes assets. Soroban has no function overloading, so the
//! token-aware entrypoints are `deposit_token` / `withdraw_token`.
//!
//! `DataKey` is at Soroban's 50-variant cap, so storage uses raw tuple keys.

use soroban_sdk::{
    contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, Vec,
};

use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient};
use crate::{admin, balance};

/// Most additional deposit tokens the vault will support.
pub const MAX_SUPPORTED_TOKENS: u32 = 10;

const TOKENS_KEY: Symbol = symbol_short!("sup_toks");

/// Share accounting for one supported token.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenPool {
    pub total_shares: i128,
    pub total_deposited: i128,
}

/// Errors for the multi-token entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum MultiTokenError {
    Unauthorized = 1,
    NotInitialized = 2,
    ZeroAmount = 3,
    UnsupportedToken = 4,
    /// The token is already supported, or is the vault's primary token.
    TokenNotAddable = 5,
    TooManySupportedTokens = 6,
    /// The token's pool still holds deposits.
    PoolNotEmpty = 7,
    InsufficientShares = 8,
    ArithmeticError = 9,
    /// The pool is paused, stopped, or shutting down.
    DepositsClosed = 10,
}

fn tokens(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&TOKENS_KEY)
        .unwrap_or(Vec::new(env))
}

fn pool_key(token: &Address) -> (Symbol, Address) {
    (symbol_short!("tpool"), token.clone())
}

fn shares_key(token: &Address, user: &Address) -> (Symbol, Address, Address) {
    (symbol_short!("tp_sh"), token.clone(), user.clone())
}

fn load_pool(env: &Env, token: &Address) -> TokenPool {
    env.storage()
        .persistent()
        .get(&pool_key(token))
        .unwrap_or(TokenPool {
            total_shares: 0,
            total_deposited: 0,
        })
}

fn require_admin_caller(env: &Env, admin: &Address) -> Result<(), MultiTokenError> {
    admin.require_auth();
    if *admin != admin::get_admin(env).map_err(|_| MultiTokenError::NotInitialized)? {
        return Err(MultiTokenError::Unauthorized);
    }
    Ok(())
}

#[contractimpl]
impl VaultContract {
    /// Admin: approve `token` for `deposit_token`. At most 10 tokens; the
    /// primary token and duplicates are rejected.
    pub fn add_supported_token(
        env: Env,
        admin: Address,
        token: Address,
    ) -> Result<(), MultiTokenError> {
        require_admin_caller(&env, &admin)?;
        let primary: Option<Address> = env.storage().instance().get(&DataKey::Token);
        let mut list = tokens(&env);
        if primary == Some(token.clone()) || list.contains(&token) {
            return Err(MultiTokenError::TokenNotAddable);
        }
        if list.len() >= MAX_SUPPORTED_TOKENS {
            return Err(MultiTokenError::TooManySupportedTokens);
        }
        list.push_back(token);
        env.storage().instance().set(&TOKENS_KEY, &list);
        Ok(())
    }

    /// Admin: stop supporting `token`. Only allowed once its pool is empty.
    pub fn remove_supported_token(
        env: Env,
        admin: Address,
        token: Address,
    ) -> Result<(), MultiTokenError> {
        require_admin_caller(&env, &admin)?;
        let mut list = tokens(&env);
        let idx = list
            .first_index_of(&token)
            .ok_or(MultiTokenError::UnsupportedToken)?;
        let pool = load_pool(&env, &token);
        if pool.total_deposited != 0 || pool.total_shares != 0 {
            return Err(MultiTokenError::PoolNotEmpty);
        }
        list.remove(idx);
        env.storage().instance().set(&TOKENS_KEY, &list);
        Ok(())
    }

    /// Read-only: the additional supported deposit tokens (not the primary token).
    pub fn get_supported_tokens(env: Env) -> Vec<Address> {
        tokens(&env)
    }

    /// Read-only: the share accounting for a supported token.
    pub fn get_token_pool(env: Env, token: Address) -> TokenPool {
        load_pool(&env, &token)
    }

    /// Read-only: `user`'s shares in `token`'s pool.
    pub fn get_token_shares(env: Env, token: Address, user: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&shares_key(&token, &user))
            .unwrap_or(0)
    }

    /// Deposit `amount` of a supported `token`, minting shares in that token's
    /// own pool (1:1 for its first deposit). Returns the shares minted.
    pub fn deposit_token(
        env: Env,
        depositor: Address,
        token: Address,
        amount: i128,
    ) -> Result<i128, MultiTokenError> {
        depositor.require_auth();
        if amount <= 0 {
            return Err(MultiTokenError::ZeroAmount);
        }
        if !tokens(&env).contains(&token) {
            return Err(MultiTokenError::UnsupportedToken);
        }
        if VaultContract::is_paused(env.clone())
            || VaultContract::is_stopped(env.clone())
            || balance::get_sunset_exit_deadline(&env).is_some()
        {
            return Err(MultiTokenError::DepositsClosed);
        }
        let mut pool = load_pool(&env, &token);
        let shares = if pool.total_shares == 0 || pool.total_deposited == 0 {
            amount
        } else {
            amount
                .checked_mul(pool.total_shares)
                .and_then(|v| v.checked_div(pool.total_deposited))
                .ok_or(MultiTokenError::ArithmeticError)?
        };
        if shares == 0 {
            return Err(MultiTokenError::ZeroAmount);
        }
        token::Client::new(&env, &token).transfer(
            &depositor,
            &env.current_contract_address(),
            &amount,
        );
        pool.total_shares += shares;
        pool.total_deposited += amount;
        env.storage().persistent().set(&pool_key(&token), &pool);
        let held = Self::get_token_shares(env.clone(), token.clone(), depositor.clone());
        env.storage()
            .persistent()
            .set(&shares_key(&token, &depositor), &(held + shares));
        Ok(shares)
    }

    /// Burn `shares` of `token`'s pool and pay out their underlying amount.
    pub fn withdraw_token(
        env: Env,
        depositor: Address,
        token: Address,
        shares: i128,
    ) -> Result<i128, MultiTokenError> {
        depositor.require_auth();
        if shares <= 0 {
            return Err(MultiTokenError::ZeroAmount);
        }
        let held = Self::get_token_shares(env.clone(), token.clone(), depositor.clone());
        if shares > held {
            return Err(MultiTokenError::InsufficientShares);
        }
        let mut pool = load_pool(&env, &token);
        let amount = shares
            .checked_mul(pool.total_deposited)
            .and_then(|v| v.checked_div(pool.total_shares))
            .ok_or(MultiTokenError::ArithmeticError)?;
        pool.total_shares -= shares;
        pool.total_deposited -= amount;
        env.storage().persistent().set(&pool_key(&token), &pool);
        env.storage()
            .persistent()
            .set(&shares_key(&token, &depositor), &(held - shares));
        token::Client::new(&env, &token).transfer(
            &env.current_contract_address(),
            &depositor,
            &amount,
        );
        Ok(amount)
    }
}
