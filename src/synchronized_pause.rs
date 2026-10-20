//! Issue #398: synchronized pause - pause all sibling pools simultaneously
//!
//! Builds on Issue #260 (pool registry). In a multi-pool deployment, a security
//! incident affecting one pool may require pausing all sibling pools immediately.

use crate::errors::VaultOverflowError;
use crate::storage::PauseReason;
use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, String, Vec};

pub const MAX_SIBLING_POOLS: u32 = 10;

/// Result of synchronized pause operation
#[derive(Clone, Debug, PartialEq)]
#[soroban_sdk::contracttype]
pub struct SynchronizedPauseResult {
    pub pool_address: Address,
    pub success: bool,
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl crate::VaultContract {
    /// Pause all sibling pools in the pool registry simultaneously
    pub fn synchronized_pause(
        env: Env,
        admin: Address,
        reason: PauseReason,
        message: String,
    ) -> Result<Vec<SynchronizedPauseResult>, VaultOverflowError> {
        admin.require_auth();
        crate::admin::require_admin(&env, &admin)?;

        // Pause this pool first
        let pause_info = crate::storage::PauseInfo {
            reason: reason.clone(),
            message: message.clone(),
            paused_at: env.ledger().sequence(),
        };
        env.storage()
            .instance()
            .set(&symbol_short!("pause_inf"), &pause_info);
        env.storage()
            .instance()
            .set(&crate::storage::DataKey::Paused, &true);

        // Get sibling pools from registry
        let siblings: Option<Vec<Address>> = env
            .storage()
            .persistent()
            .get(&symbol_short!("sibl_pls"));

        let mut results = Vec::new(&env);
        let ledger = env.ledger().sequence();

        if let Some(sibling_list) = siblings {
            if sibling_list.len() > MAX_SIBLING_POOLS {
                return Err(VaultOverflowError::TooManyCompetitors);
            }

            let mut pools_paused = 0u32;
            let mut pools_failed = 0u32;

            for i in 0..sibling_list.len() {
                let sibling = sibling_list.get(i).unwrap();

                // Attempt to pause sibling pool - don't revert on failure
                let pause_result: Result<(), _> = env.try_invoke_contract(
                    &sibling,
                    &soroban_sdk::symbol_short!("pause_w_rs"),
                    (reason.clone(), message.clone()).into(),
                );

                let success = pause_result.is_ok();
                if success {
                    pools_paused += 1;
                } else {
                    pools_failed += 1;
                }

                results.push_back(SynchronizedPauseResult {
                    pool_address: sibling.clone(),
                    success,
                });
            }

            env.events().publish(
                (symbol_short!("sync_ps"), admin),
                (pools_paused, pools_failed, ledger),
            );
        }

        Ok(results)
    }

    /// Unpause all sibling pools in the pool registry simultaneously
    pub fn synchronized_unpause(
        env: Env,
        admin: Address,
    ) -> Result<Vec<SynchronizedPauseResult>, VaultOverflowError> {
        admin.require_auth();
        crate::admin::require_admin(&env, &admin)?;

        // Unpause this pool first
        env.storage()
            .instance()
            .remove(&symbol_short!("pause_inf"));
        env.storage()
            .instance()
            .set(&crate::storage::DataKey::Paused, &false);

        // Get sibling pools from registry
        let siblings: Option<Vec<Address>> = env
            .storage()
            .persistent()
            .get(&symbol_short!("sibl_pls"));

        let mut results = Vec::new(&env);
        let ledger = env.ledger().sequence();

        if let Some(sibling_list) = siblings {
            if sibling_list.len() > MAX_SIBLING_POOLS {
                return Err(VaultOverflowError::TooManyCompetitors);
            }

            let mut pools_unpaused = 0u32;
            let mut pools_failed = 0u32;

            for i in 0..sibling_list.len() {
                let sibling = sibling_list.get(i).unwrap();

                // Attempt to unpause sibling pool
                let unpause_result: Result<(), _> = env.try_invoke_contract(
                    &sibling,
                    &soroban_sdk::symbol_short!("unpause"),
                    Vec::new(&env),
                );

                let success = unpause_result.is_ok();
                if success {
                    pools_unpaused += 1;
                } else {
                    pools_failed += 1;
                }

                results.push_back(SynchronizedPauseResult {
                    pool_address: sibling.clone(),
                    success,
                });
            }

            env.events().publish(
                (symbol_short!("sync_unps"), admin),
                (pools_unpaused, pools_failed, ledger),
            );
        }

        Ok(results)
    }

    /// Admin-only: set the list of sibling pools for synchronized operations
    pub fn set_sibling_pools(
        env: Env,
        admin: Address,
        sibling_pools: Vec<Address>,
    ) -> Result<(), VaultOverflowError> {
        admin.require_auth();
        crate::admin::require_admin(&env, &admin)?;

        if sibling_pools.len() > MAX_SIBLING_POOLS {
            return Err(VaultOverflowError::TooManyCompetitors);
        }

        env.storage()
            .persistent()
            .set(&symbol_short!("sibl_pls"), &sibling_pools);

        Ok(())
    }

    /// Read-only: get the list of sibling pools
    pub fn get_sibling_pools(env: Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&symbol_short!("sibl_pls"))
            .unwrap_or(Vec::new(&env))
    }
}
