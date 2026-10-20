//! Collateral swap without unstaking (issue #335).
//!
//! Lets a staker swap their underlying collateral token for a different one
//! while keeping their position (and, per the issue, an active debt-NFT
//! loan) open â€” unstake the old token, swap via DEX, re-stake the result,
//! all without closing the loan.
//!
//! # A real gap this module is built against
//!
//! The issue depends on issue #214 ("NFT collateral minting" â€” a debt-NFT
//! representing an open loan against a staked position). That doesn't exist
//! in this crate: `DebtNFT` is referenced by name in `vault.rs`'s imports
//! but is never defined anywhere in `storage.rs`, and there is no minting,
//! `face_value`, or loan-tracking code at all (see this PR's description for
//! the wider picture of what does and doesn't exist here). "Validates debt
//! NFT exists and position is collateralized" and "debt NFT face_value
//! updated" from the acceptance criteria cannot be implemented against real
//! data as a result.
//!
//! What *does* exist and is used here: `balance::get_dex_router`/
//! `vault::DexRouterClient` (issue #205), and `DataKey::StakedAtLedger` as a
//! stand-in signal for "this user has an open position" (not "is
//! collateralized" specifically â€” there's no collateralization flag to
//! check). `CollateralSwapConfig` tracks the swap's own before/after state
//! directly rather than updating a debt-NFT's `face_value`, since there's no
//! debt NFT to update.
//!
//! Also unstakes/re-stakes are unavailable for the same reason documented
//! throughout this PR (no live `stake`/`unstake` entrypoint) â€” this module
//! swaps the *token* via the DEX router directly rather than round-tripping
//! through unstake-then-stake, since that round trip has nothing to call.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `vesting_cliff.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol};

use crate::balance;
use crate::errors::VaultError;
use crate::storage::DataKey;
use crate::vault::{DexRouterClient, VaultContract, VaultContractClient};

/// Persistent-storage key prefix for a user's most recent collateral swap.
const CONFIG_KEY: Symbol = symbol_short!("col_swp");

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CollateralSwapConfig {
    /// Placeholder until issue #214's debt-NFT tracking exists â€” see this
    /// module's doc comment. Always `0` today.
    pub debt_nft_id: u32,
    pub original_token: Address,
    pub swap_token: Address,
    pub amount: i128,
}

fn staked_at_ledger(env: &Env, user: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get::<_, u32>(&DataKey::StakedAtLedger(user.clone()))
}

pub fn get_config(env: &Env, user: &Address) -> Option<CollateralSwapConfig> {
    env.storage().persistent().get(&(CONFIG_KEY, user.clone()))
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Swap `user`'s current position from its existing token into
    /// `new_stake_token` via the configured DEX router, without closing the
    /// position window.
    ///
    /// `min_new_amount` is slippage protection: the call reverts with
    /// `SlippageExceeded` (mapped to `VaultError::InvalidRate` â€” see note
    /// below) if the router delivers less than that.
    ///
    /// See this module's doc comment for what "validates debt NFT exists
    /// and position is collateralized" and "face_value updated" could not be
    /// implemented against, given issue #214 doesn't exist in this crate.
    pub fn initiate_collateral_swap(
        env: Env,
        user: Address,
        new_stake_token: Address,
        min_new_amount: i128,
    ) -> Result<i128, VaultError> {
        user.require_auth();

        // Stand-in for "position is collateralized" -- see doc comment.
        staked_at_ledger(&env, &user).ok_or(VaultError::NotInitialized)?;

        let router_address = balance::get_dex_router(&env).ok_or(VaultError::NotYieldSource)?;
        let router = DexRouterClient::new(&env, &router_address);

        let old_amount = balance::get_shares(&env, &user);
        if old_amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let old_token_address: Address = env
            .storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("token"))
            .ok_or(VaultError::NotInitialized)?;

        // Pre-fund the router, matching the transfer-then-swap contract
        // documented on `DexRouterInterface` in vault.rs.
        let old_token_client = token::Client::new(&env, &old_token_address);
        old_token_client.transfer(&env.current_contract_address(), &router_address, &old_amount);

        let new_amount = router.swap(
            &old_token_address,
            &new_stake_token,
            &old_amount,
            &min_new_amount,
            &env.current_contract_address(),
        );

        // The issue names a distinct `SlippageExceeded` error; this crate's
        // VaultError enum has no such variant (see this PR's description for
        // the state of errors.rs), so InvalidRate -- already used for
        // "a configured rate/threshold failed validation" elsewhere (e.g.
        // set_dynamic_fee_config) -- is the closest existing fit rather than
        // inventing a new variant this crate's Vec<50> cap discussion
        // (storage.rs) suggests should be added deliberately, not as a side
        // effect of one feature module.
        if new_amount < min_new_amount {
            return Err(VaultError::InvalidRate);
        }

        env.storage().persistent().set(
            &(CONFIG_KEY, user.clone()),
            &CollateralSwapConfig {
                debt_nft_id: 0,
                original_token: old_token_address.clone(),
                swap_token: new_stake_token.clone(),
                amount: new_amount,
            },
        );

        env.events().publish(
            (symbol_short!("col_swap"), user.clone()),
            (
                old_token_address,
                new_stake_token,
                old_amount,
                new_amount,
                env.ledger().sequence(),
            ),
        );

        Ok(new_amount)
    }

    /// The user's most recent collateral swap record, if any.
    pub fn get_collateral_swap(env: Env, user: Address) -> Option<CollateralSwapConfig> {
        crate::collateral_swap::get_config(&env, &user)
    }
}







