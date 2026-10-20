//! Issue #589: storage TTL management.
//!
//! Soroban archives storage entries whose TTL runs out. `extend_contract_ttl`
//! bumps the vault's instance storage (admin, token, totals, config) so the
//! contract itself never becomes inaccessible, and `deposit`/`withdraw` do the
//! same as a best-effort side effect. Per-user persistent entries are not
//! bumped here.
//!
//! The bump only pays for an extension when the remaining TTL is below the
//! configured threshold, so calling it while the TTL is already high is
//! close to free.
//!
//! `DataKey` is at Soroban's 50-variant cap, so the threshold uses a raw
//! `Symbol` key.

use soroban_sdk::{contracterror, contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::vault::{VaultContract, VaultContractClient};

const THRESHOLD_KEY: Symbol = symbol_short!("ttl_thr");

/// Threshold used until the admin sets one: about 30 days of ledgers.
pub const DEFAULT_TTL_THRESHOLD: u32 = 518_400;

/// Errors for the TTL entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TtlError {
    Unauthorized = 1,
    NotInitialized = 2,
    /// The threshold passed to `set_ttl_extension_threshold` was zero.
    InvalidThreshold = 3,
}

/// The configured threshold in ledgers.
pub fn threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&THRESHOLD_KEY)
        .unwrap_or(DEFAULT_TTL_THRESHOLD)
}

/// Extends the instance TTL to the configured threshold when it has fallen
/// below it. The target is clamped to the network's maximum TTL so this can
/// never fail, which keeps it safe to call at the end of `deposit`/`withdraw`.
pub fn bump(env: &Env) {
    let target = threshold(env).min(env.storage().max_ttl());
    env.storage().instance().extend_ttl(target, target);
}

#[contractimpl]
impl VaultContract {
    /// Anyone (keepers included): extend the vault's instance storage TTL to
    /// the configured threshold if it is currently below it.
    pub fn extend_contract_ttl(env: Env, caller: Address) {
        caller.require_auth();
        bump(&env);
    }

    /// Admin: set the TTL threshold in ledgers used by `extend_contract_ttl`
    /// and the automatic bumps. Must be greater than zero.
    pub fn set_ttl_extension_threshold(
        env: Env,
        admin: Address,
        ledgers: u32,
    ) -> Result<(), TtlError> {
        admin.require_auth();
        if admin != admin::get_admin(&env).map_err(|_| TtlError::NotInitialized)? {
            return Err(TtlError::Unauthorized);
        }
        if ledgers == 0 {
            return Err(TtlError::InvalidThreshold);
        }
        env.storage().instance().set(&THRESHOLD_KEY, &ledgers);
        Ok(())
    }

    /// Read-only: the TTL threshold in ledgers.
    pub fn get_ttl_extension_threshold(env: Env) -> u32 {
        threshold(&env)
    }
}
