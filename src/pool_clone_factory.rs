//! Pool clone factory (issue #412).
//!
//! Lets the admin deploy a fresh, independent instance of this same pool
//! contract from within a running pool, instead of redeploying bytecode by
//! hand for every new pool. The new instance is initialized immediately
//! with the supplied configuration and registered as a sibling pool in this
//! contract's league (`add_league_pool`, cross-pool registry from issue
//! #373 / `performance_league_table.rs`), so it's immediately visible to
//! cross-pool queries.
//!
//! # Wiring
//!
//! `deploy_clone` needs the Wasm hash of the code to deploy. Soroban gives
//! a contract no way to read back its own currently-executing Wasm hash, so
//! the caller supplies it (this contract's own hash, already uploaded to
//! the network via `env.deployer().upload_contract_wasm()`).
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw
//! `Symbol`-keyed storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Bytes, BytesN, Env, Symbol, Vec};

use crate::admin;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance key: addresses of every clone deployed by this instance.
const CLONES_KEY: Symbol = symbol_short!("pcf_cln");

/// Configuration for a newly deployed pool clone (issue #412).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CloneConfig {
    pub stake_token: Address,
    pub reward_token: Address,
    pub reward_rate_bps: i128,
    pub admin: Address,
}

fn get_clones(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&CLONES_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn record_clone(env: &Env, clone: &Address) {
    let mut clones = get_clones(env);
    clones.push_back(clone.clone());
    env.storage().instance().set(&CLONES_KEY, &clones);
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Deploys a fresh instance of this pool contract using `wasm_hash` as
    /// the template, initializes it with `config`, and registers it as a
    /// sibling pool in this contract's league. Admin only. `salt` must be
    /// unique per deployment â€” reusing one for the same admin/template
    /// produces the same deterministic address and reverts.
    ///
    /// `config.reward_token` is recorded in the `clone_deployed` event for
    /// off-chain bookkeeping; like this contract, the deployed clone has no
    /// public entrypoint to independently configure a reward token distinct
    /// from its stake token, so it isn't set on the new instance directly.
    ///
    /// Returns the deployed contract's address.
    pub fn deploy_clone(
        env: Env,
        wasm_hash: BytesN<32>,
        config: CloneConfig,
        salt: Bytes,
    ) -> Result<Address, VaultError> {
        admin::require_admin(&env)?;

        let salt_hash = env.crypto().sha256(&salt);
        let new_pool = env.deployer().with_current_contract(salt_hash).deploy(wasm_hash);

        let reward_rate_bps: u32 = config
            .reward_rate_bps
            .clamp(0, u32::MAX as i128)
            .try_into()
            .unwrap_or(0);
        VaultContractClient::new(&env, &new_pool).initialize(
            &config.admin,
            &config.stake_token,
            &reward_rate_bps,
            &None,
            &None,
        );

        crate::pool_clone_factory::record_clone(&env, &new_pool);
        Self::add_league_pool(env.clone(), new_pool.clone())?;

        env.events().publish(
            (symbol_short!("pcf_dep"),),
            (
                new_pool.clone(),
                config.stake_token,
                config.reward_token,
                config.admin,
                env.ledger().sequence(),
            ),
        );

        Ok(new_pool)
    }

    /// Addresses of every clone deployed by this instance via `deploy_clone`.
    pub fn get_deployed_clones(env: Env) -> Vec<Address> {
        crate::pool_clone_factory::get_clones(&env)
    }
}















