//! NFT-triggered position exit (issue #410).
//!
//! Builds on the stake-receipt NFT contract (`nft.rs`, issue #26): lets a
//! receipt holder exit their staking position by burning the NFT itself,
//! instead of calling `unstake`/`unstake_all` directly.
//!
//! # Adapting to the actual receipt contract
//!
//! The issue's acceptance criteria describes `redeem_nft(user, nft_id: u32)`
//! verifying ownership via `owner_of(nft_id)` and burning a specific token
//! id. `StakeReceiptNFT` (`nft.rs`) doesn't work that way: it's a
//! non-transferable receipt keyed directly by holder `Address` (`mint`/
//! `burn`/`has_receipt` all take a `user: Address`, not a token id), with no
//! `nft_id` or `owner_of` concept at all. `burn_and_redeem` is written
//! against that real interface instead: the caller redeems *their own*
//! receipt, which is exactly the receipt tied to their address â€” there is no
//! separate id to look up, and "wrong owner" is rejected the same way every
//! other entrypoint here rejects it, via `user.require_auth()` plus
//! requiring a receipt/position to exist for that address.
//!
//! # Storage
//!
//! Reuses `balance::get_nft_contract` / `set_nft_contract` (issue #40),
//! which already exist but were never wired to a public entrypoint â€” added
//! here since `burn_and_redeem` cannot function without a way to register
//! the receipt contract.

use soroban_sdk::{contractimpl, symbol_short, token, Address, Env};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::nft::StakeReceiptNFTClient;
use crate::storage::DataKey;
use crate::VaultContract;
use crate::vault::VaultContractClient;

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Register the stake-receipt NFT contract address. Admin only.
    ///
    /// Required before `burn_and_redeem` can be used â€” mirrors how
    /// `nft_client.initialize(&vault_id)` on the receipt contract's side
    /// registers this vault as its minter.
    pub fn set_nft_contract(env: Env, nft_addr: Address) -> Result<(), VaultError> {
        let admin_addr = admin::get_admin(&env)?;
        admin_addr.require_auth();

        balance::set_nft_contract(&env, &nft_addr);
        events::admin_action_set_nft_contract(&env, &admin_addr, &nft_addr);
        Ok(())
    }

    /// Read-only query: the registered stake-receipt NFT contract, if any.
    pub fn get_nft_contract(env: Env) -> Option<Address> {
        balance::get_nft_contract(&env)
    }

    /// Burn the caller's stake-receipt NFT and fully exit their staking
    /// position in the same call (issue #410).
    ///
    /// Pending reward is settled first â€” exactly like the regular unstake
    /// flow â€” then the full principal is returned, and finally the receipt
    /// is burned. Returns the total token amount returned to the user.
    ///
    /// Reverts with `NotInitialized` if no NFT contract has been registered
    /// via `set_nft_contract`, and with `PositionNotFound` if the caller
    /// holds no receipt or has no active position.
    pub fn burn_and_redeem(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let nft_addr = balance::get_nft_contract(&env).ok_or(VaultError::NotInitialized)?;
        let nft_client = StakeReceiptNFTClient::new(&env, &nft_addr);
        if !nft_client.has_receipt(&user) {
            return Err(VaultError::PositionNotFound);
        }

        let shares = balance::get_shares(&env, &user);
        if shares <= 0 {
            return Err(VaultError::PositionNotFound);
        }

        // Respect the pool's lock-up period, same as the regular unstake
        // flow. `guardian approval` (also named in the issue) has no
        // implementation anywhere else in this contract to defer to, so
        // there is nothing to enforce there.
        let lock_period: u32 = env.storage().instance().get(&DataKey::LockPeriod).unwrap_or(0);
        if lock_period > 0 {
            let staked_at: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::StakedAtLedger(user.clone()))
                .unwrap_or(0);
            if env.ledger().sequence() < staked_at.saturating_add(lock_period) {
                return Err(VaultError::InvalidRate);
            }
        }

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(VaultError::NotInitialized)?;
        let token_client = token::Client::new(&env, &token_addr);

        // Settle any pending reward before the position is closed, same
        // ordering as the rest of the unstake flow.
        let rewards_claimed = balance::get_accrued_reward(&env, &user);
        if rewards_claimed > 0 {
            balance::set_accrued_reward(&env, &user, 0);
            let total_paid = balance::get_total_rewards_paid(&env);
            balance::set_total_rewards_paid(&env, total_paid + rewards_claimed);
            token_client.transfer(&env.current_contract_address(), &user, &rewards_claimed);
        }

        // Return the full position.
        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let amount_returned = balance::shares_to_amount(total_shares, total_deposited, shares)
            .ok_or(VaultError::ArithmeticError)?;

        balance::set_shares(&env, &user, 0);
        balance::set_total_shares(&env, total_shares - shares);
        balance::set_total_deposited(&env, total_deposited - amount_returned);
        if amount_returned > 0 {
            token_client.transfer(&env.current_contract_address(), &user, &amount_returned);
        }

        // Burn the receipt last, once the exit has fully settled.
        nft_client.burn(&user);

        env.events().publish(
            (symbol_short!("nft_rdm"), user),
            (amount_returned, rewards_claimed, env.ledger().sequence()),
        );
        Ok(amount_returned)
    }
}
















