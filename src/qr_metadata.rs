//! Stake receipt QR metadata (issue #324).
//!
//! Read-only aggregator returning a structured payload representing a user's
//! staking position, formatted for QR code generation by mobile wallets and
//! frontend apps. Enables offline presentation of staking credentials without
//! a real-time contract query per field. Pure read â€” no state changes.

use soroban_sdk::{contractimpl, contracttype, Address, Env, String};

use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;
use crate::vault::{ CONTRACT_VERSION};

/// Structured staking-position payload for QR code generation.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct QRMetadata {
    pub contract_address: Address,
    pub user: Address,
    pub staked_amount: i128,
    pub staked_since: u32,
    pub pending_reward: i128,
    pub pool_name: String,
    pub version: String,
    pub generated_at: u32,
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Returns `user`'s current staking position formatted for QR code
    /// generation. Reverts with `PositionNotFound` if the user has no open
    /// position. No auth required â€” this is a read-only query.
    ///
    /// `pool_name` is always empty since this contract has no pool-naming
    /// feature configured; `version` is `CONTRACT_VERSION`.
    pub fn get_qr_metadata(env: Env, user: Address) -> Result<QRMetadata, VaultError> {
        let shares = balance::get_shares(&env, &user);
        if shares == 0 {
            return Err(VaultError::PositionNotFound);
        }

        let staked_amount = balance::shares_to_amount(
            balance::get_total_shares(&env),
            balance::get_total_deposited(&env),
            shares,
        )
        .unwrap_or(0);

        let staked_since: u32 = env
            .storage()
            .persistent()
            .get(&crate::storage::DataKey::StakedAtLedger(user.clone()))
            .unwrap_or(0);

        let pending_reward = balance::get_accrued_reward(&env, &user);

        Ok(QRMetadata {
            contract_address: env.current_contract_address(),
            user: user.clone(),
            staked_amount,
            staked_since,
            pending_reward,
            pool_name: String::from_str(&env, ""),
            version: String::from_str(&env, CONTRACT_VERSION),
            generated_at: env.ledger().sequence(),
        })
    }
}









