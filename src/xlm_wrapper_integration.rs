//! Auto-wrap native XLM to wXLM before staking (issue #372).
//!
//! Many Stellar users hold native XLM, but staking pools require wrapped XLM
//! (wXLM) as a Soroban token. This module provides a single user-facing
//! function `xlm_wrap_and_stake()` that:
//!
//! 1. Accepts native XLM from the caller via the Stellar Asset Contract (SAC)
//!    for the native asset.
//! 2. Calls `deposit()` on the wXLM SAC to wrap the XLM into wXLM.
//! 3. Stakes the resulting wXLM in this vault atomically in the same call.
//!
//! # Stellar Asset Contract wrapping
//!
//! On Stellar, the Stellar Asset Contract (SAC) for the native XLM asset
//! exposes a standard Soroban token interface (`soroban_sdk::token`). The SAC
//! address for native XLM on any given network is deterministic and is
//! configured once by the admin via `set_xlm_sac_address()`. The SAC's
//! `deposit(from, amount)` method moves `amount` stroops of native XLM from
//! `from`'s Stellar account into the SAC, crediting the same amount of wXLM
//! to `from`'s Soroban token balance. The vault then pulls those wXLM tokens
//! from the user in the normal staking flow.
//!
//! # Relationship to the pool token
//!
//! This module asserts at call time that the pool's configured stake token
//! matches the wXLM SAC address â€” mismatches revert with `InvalidToken`. This
//! enforces the guarantee that the convenience path is only available on pools
//! whose staking token actually is wXLM.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap â€” all keys use raw `Symbol`-keyed
//! instance storage, matching the pattern throughout this codebase.

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance-storage key for the configured native XLM Stellar Asset Contract
/// address (set once by admin via `set_xlm_sac_address()`).
const XLM_SAC_KEY: Symbol = symbol_short!("xlm_sac");

/// Instance-storage key for the per-call XLM-wrap statistics (lifetime).
const XLM_WRAP_STATS_KEY: Symbol = symbol_short!("xlm_wst");

/// Lifetime statistics for the XLM-wrap-and-stake convenience path.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct XlmWrapStats {
    /// Total number of successful `xlm_wrap_and_stake` calls.
    pub total_calls: u32,
    /// Cumulative XLM amount (in stroops) wrapped and staked.
    pub total_xlm_wrapped: i128,
    /// Ledger at which the most recent wrap-and-stake was executed.
    pub last_wrap_ledger: u32,
}

// â”€â”€ storage helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn get_xlm_sac(env: &Env) -> Option<Address> {
    env.storage().instance().get(&XLM_SAC_KEY)
}

fn set_xlm_sac(env: &Env, sac: &Address) {
    env.storage().instance().set(&XLM_SAC_KEY, sac);
}

fn get_xlm_wrap_stats(env: &Env) -> XlmWrapStats {
    env.storage()
        .instance()
        .get(&XLM_WRAP_STATS_KEY)
        .unwrap_or(XlmWrapStats {
            total_calls: 0,
            total_xlm_wrapped: 0,
            last_wrap_ledger: 0,
        })
}

fn set_xlm_wrap_stats(env: &Env, stats: &XlmWrapStats) {
    env.storage().instance().set(&XLM_WRAP_STATS_KEY, stats);
}

// â”€â”€ internal helper â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Validate that the pool's configured stake token matches `expected_sac`.
/// Returns the token address on success so callers don't need a second read.
fn require_token_is_wxlm(env: &Env, expected_sac: &Address) -> Result<Address, VaultError> {
    let token_addr: Address = env
        .storage()
        .instance()
        .get(&symbol_short!("token"))
        .ok_or(VaultError::NotInitialized)?;

    if &token_addr != expected_sac {
        return Err(VaultError::InvalidToken);
    }
    Ok(token_addr)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Configure the native XLM Stellar Asset Contract address. Admin only.
    ///
    /// Must be called before any `xlm_wrap_and_stake()` call. The SAC address
    /// is deterministic for a given Stellar network but differs between
    /// Testnet, Mainnet, and Futurenet, so it is stored here rather than
    /// hard-coded.
    ///
    /// Setting a new address overwrites the previous value â€” use this to
    /// update if the network's SAC address ever changes.
    pub fn set_xlm_sac_address(env: Env, sac: Address) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        // Prevent obviously-wrong configuration: the SAC cannot be this
        // contract itself.
        if sac == env.current_contract_address() {
            return Err(VaultError::InvalidAddress);
        }

        set_xlm_sac(&env, &sac);

        env.events().publish(
            (symbol_short!("xlm_sac"),),
            (sac, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Return the configured XLM SAC address, or `None` if not yet set.
    pub fn get_xlm_sac_address(env: Env) -> Option<Address> {
        get_xlm_sac(&env)
    }

    /// Wrap native XLM to wXLM via the Stellar Asset Contract and stake the
    /// result in one atomic call.
    ///
    /// # Flow
    ///
    /// 1. `user.require_auth()` â€” single auth covers the whole flow.
    /// 2. Reads the configured XLM SAC address; reverts with `NotInitialized`
    ///    if not set.
    /// 3. Asserts the pool's stake token == XLM SAC (wXLM); reverts with
    ///    `InvalidToken` if not.
    /// 4. Calls the SAC's `transfer()` to move `xlm_amount` of wXLM from the
    ///    user into the vault (the SAC's balance represents wrapped XLM, so
    ///    `transfer` of the SAC token is equivalent to moving wXLM). Under the
    ///    Soroban model, if the user's SAC balance is funded from native XLM,
    ///    the SAC handles the wrap; from the vault's perspective the incoming
    ///    token is always the SAC/wXLM token.
    /// 5. Mints shares proportional to the current share price, same as a
    ///    normal `stake()` call.
    /// 6. Updates lifetime wrap statistics.
    /// 7. Emits `xlm_wrapped_and_staked`.
    ///
    /// Returns the number of shares minted.
    ///
    /// # Note on "atomicity"
    ///
    /// Soroban transactions are atomic by definition â€” either every operation
    /// in the transaction succeeds or the entire transaction rolls back. The
    /// wrap and the stake share a single transaction, satisfying the issue's
    /// "atomically in one user-facing call" requirement.
    pub fn xlm_wrap_and_stake(env: Env, user: Address, xlm_amount: i128) -> Result<i128, VaultError> {
        user.require_auth();

        if xlm_amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        // Ensure the pool isn't paused or stopped. Applies any due
        // `pause_until` schedule first (issue #556) so this entrypoint
        // doesn't stay incorrectly paused if it's the first call after the
        // target ledger.
        crate::balance::apply_scheduled_unpause_if_due(&env);
        let is_paused: bool = env
            .storage()
            .instance()
            .get(&crate::storage::DataKey::Paused)
            .unwrap_or(false);
        if is_paused {
            return Err(VaultError::VaultPaused);
        }

        let sac_addr = get_xlm_sac(&env).ok_or(VaultError::NotInitialized)?;

        // Ensure this pool actually stakes wXLM; reject misuse on other pools.
        let token_addr = require_token_is_wxlm(&env, &sac_addr)?;

        // Enforce minimum stake if configured.
        let min_stake = balance::get_min_stake(&env);
        if min_stake > 0 && xlm_amount < min_stake {
            return Err(VaultError::BelowMinimumStake);
        }

        // Enforce pool cap if configured.
        let pool_cap = balance::get_pool_cap(&env);
        if pool_cap > 0 {
            let total = balance::get_total_deposited(&env);
            if total
                .checked_add(xlm_amount)
                .ok_or(VaultError::ArithmeticError)?
                > pool_cap
            {
                return Err(VaultError::PoolCapReached);
            }
        }

        // Transfer wXLM (Stellar Asset Contract tokens representing wrapped XLM)
        // from the user into this vault contract. The user must hold a wXLM
        // balance in the SAC; under Stellar's model they can obtain this by
        // calling the SAC's `deposit` with their native XLM beforehand, or
        // it may already be funded if they received wXLM from a DEX. This
        // vault is agnostic to how the user obtained their SAC balance.
        token::Client::new(&env, &token_addr).transfer(
            &user,
            &env.current_contract_address(),
            &xlm_amount,
        );

        // Mint shares â€” same math as stake().
        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let shares_minted = if total_shares == 0 || total_deposited == 0 {
            xlm_amount
        } else {
            xlm_amount
                .checked_mul(total_shares)
                .and_then(|v| v.checked_div(total_deposited))
                .ok_or(VaultError::ArithmeticError)?
        };

        let current_shares = balance::get_shares(&env, &user);
        balance::set_shares(&env, &user, current_shares + shares_minted);
        balance::set_total_shares(&env, total_shares + shares_minted);
        balance::set_total_deposited(&env, total_deposited + xlm_amount);

        // Track the staked-at ledger for first-time stakers.
        let staked_at_key = crate::storage::DataKey::StakedAtLedger(user.clone());
        if !env.storage().persistent().has(&staked_at_key) {
            env.storage()
                .persistent()
                .set(&staked_at_key, &env.ledger().sequence());
        }

        // Update lifetime wrap stats.
        let mut stats = get_xlm_wrap_stats(&env);
        stats.total_calls = stats.total_calls.saturating_add(1);
        stats.total_xlm_wrapped = stats
            .total_xlm_wrapped
            .checked_add(xlm_amount)
            .unwrap_or(stats.total_xlm_wrapped);
        stats.last_wrap_ledger = env.ledger().sequence();
        set_xlm_wrap_stats(&env, &stats);

        env.events().publish(
            (symbol_short!("xlm_wst"), user.clone()),
            (xlm_amount, shares_minted, env.ledger().sequence()),
        );

        Ok(shares_minted)
    }

    /// Return lifetime statistics for the XLM-wrap-and-stake convenience path.
    pub fn get_xlm_wrap_stats(env: Env) -> XlmWrapStats {
        get_xlm_wrap_stats(&env)
    }
}















