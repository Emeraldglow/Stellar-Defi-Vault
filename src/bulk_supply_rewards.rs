//! Bulk reward-pool top-up (issue #334).
//!
//! Lets several reward suppliers top up the reward pool in a single
//! transaction instead of each calling a top-up function separately.
//! Processes each entry independently â€” one supplier failing (not
//! whitelisted or insufficient allowance) does not revert the others, and
//! `bulk_supply_rewards` never reverts as a whole for that reason.
//!
//! # Storage
//!
//! Reuses `balance::get_reward_pool_balance`/`set_reward_pool_balance` and
//! the existing per-user `Whitelisted` allowlist â€” no new storage keys.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Vec};

use crate::balance;
use crate::errors::VaultError;
use crate::storage::DataKey;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Most suppliers a single `bulk_supply_rewards` call may process.
pub const MAX_BULK_SUPPLY_ENTRIES: u32 = 10;

/// One supplier's contribution in a `bulk_supply_rewards` call.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BulkSupplyEntry {
    pub supplier: Address,
    pub amount: i128,
}

/// Outcome of processing one `BulkSupplyEntry`: the supplier, whether it
/// succeeded, and the amount actually credited (0 on failure).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BulkSupplyResult {
    pub supplier: Address,
    pub success: bool,
    pub amount: i128,
}

fn is_whitelisted(env: &Env, supplier: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::Whitelisted(supplier.clone()))
        .unwrap_or(false)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Process up to `MAX_BULK_SUPPLY_ENTRIES` reward top-ups in one
    /// transaction. Each `supplier` must already be whitelisted and must have
    /// pre-approved a token allowance (via the standard SEP-41 `approve`) for
    /// at least their `amount`, since this call does not carry their
    /// signature. Reverts with `BatchTooLarge` if more than
    /// `MAX_BULK_SUPPLY_ENTRIES` entries are supplied; otherwise never
    /// reverts â€” per-entry failures are reported in the returned vector.
    pub fn bulk_supply_rewards(
        env: Env,
        entries: Vec<BulkSupplyEntry>,
    ) -> Result<Vec<BulkSupplyResult>, VaultError> {
        if entries.len() > MAX_BULK_SUPPLY_ENTRIES {
            return Err(VaultError::BatchTooLarge);
        }

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(VaultError::NotInitialized)?;
        let token_client = soroban_sdk::token::Client::new(&env, &token_addr);

        let mut results = Vec::new(&env);
        let mut total_added: i128 = 0;

        for entry in entries.iter() {
            if entry.amount <= 0 || !is_whitelisted(&env, &entry.supplier) {
                results.push_back(BulkSupplyResult {
                    supplier: entry.supplier.clone(),
                    success: false,
                    amount: 0,
                });
                continue;
            }

            let allowance = token_client.allowance(&entry.supplier, &env.current_contract_address());
            if allowance < entry.amount {
                results.push_back(BulkSupplyResult {
                    supplier: entry.supplier.clone(),
                    success: false,
                    amount: 0,
                });
                continue;
            }

            token_client.transfer_from(
                &env.current_contract_address(),
                &entry.supplier,
                &env.current_contract_address(),
                &entry.amount,
            );

            total_added = total_added.saturating_add(entry.amount);
            results.push_back(BulkSupplyResult {
                supplier: entry.supplier.clone(),
                success: true,
                amount: entry.amount,
            });

            env.events().publish(
                (symbol_short!("rw_sup"), entry.supplier.clone()),
                (entry.amount, env.ledger().sequence()),
            );
        }

        if total_added > 0 {
            let pool = balance::get_reward_pool_balance(&env);
            balance::set_reward_pool_balance(&env, pool + total_added);
        }

        Ok(results)
    }
}















