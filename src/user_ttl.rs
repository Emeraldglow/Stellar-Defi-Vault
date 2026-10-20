//! Per-user persistent-entry TTL management.
//!
//! `ttl_management::bump` keeps the vault's *instance* storage alive, but
//! each user's own entries live in Soroban *persistent* storage with their
//! own, independent TTL: a staker who deposits once and never interacts
//! again can have their position, reward-checkpoint, and settings entries
//! archived even while the vault's instance storage stays healthy. This
//! module extends those per-user entries on every deposit/withdraw/claim,
//! and exposes `bump_user_entries` so anyone (a keeper, a frontend, another
//! user) can refresh a dormant position on someone else's behalf.
//!
//! # Threshold / target
//! Mirrors `ttl_management.rs`: an entry whose remaining TTL has fallen
//! below the configured `threshold` (in ledgers) is extended out to the
//! configured `target` (also in ledgers, clamped to the network max). Both
//! are admin-configurable via `set_user_ttl_extension`, so calling the bump
//! while an entry's TTL is already high is close to free.
//!
//! # Entries covered
//! - Position: `ShareBalance`, `StakeHistory`, `StakedAtLedger`,
//!   `LastUnstakeLedger`, `UnbondingPosition`, `FrozenAt`, `FirstStakedAt`,
//!   `Restaked`, `VestingEntries`.
//! - Checkpoint: `RewardCheckpointLedger`, `LastClaimLedger`,
//!   `AccruedReward`, `RewardRemainder`.
//! - Settings: `Delegate`, `UserClaimWindow`, `Whitelisted`, `KycApproved`.
//!
//! A key the user never populated (e.g. no vesting entries, no delegate) is
//! simply skipped: extending the TTL of a nonexistent persistent entry
//! panics, so every key is checked with `has()` first, which also makes
//! `bump_entries`/`bump_user_entries` safe to call for any address at all,
//! including one that has never interacted with the vault.
//!
//! # Restoring an archived entry
//! If nobody bumps a dormant user before their TTL fully elapses, Soroban
//! archives their persistent entries. A returning user is not locked out,
//! but the recovery path depends on how they re-engage:
//! - Calling `stake()` again always works: it only ever *writes* those keys
//!   (`ShareBalance`, `StakedAtLedger`, etc.), and a write to an archived
//!   persistent key recreates it from scratch at the current TTL.
//! - Calling a read-then-write entrypoint such as `claim()` or `withdraw()`
//!   on an archived entry fails the transaction, because Soroban requires
//!   archived ledger entries to be restored *before* the transaction that
//!   touches them. The caller (the user, or anyone acting on their behalf)
//!   must first submit a restore-footprint transaction — via
//!   `RestoreFootprintTransactionData` through the Stellar RPC/CLI (e.g.
//!   `soroban contract restore` in the CLI) — naming the archived ledger
//!   keys for that address (position, checkpoint, and settings entries, as
//!   listed above). Once restored, the normal entrypoint call succeeds.
//! - `bump_user_entries` called regularly (e.g. by a keeper bot sweeping
//!   `AllStakers`) is the cheap alternative to ever reaching that state, and
//!   is the recommended default for keeping dormant positions alive.

use soroban_sdk::{contracterror, contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::storage::DataKey;
use crate::vault::VaultContract;

const THRESHOLD_KEY: Symbol = symbol_short!("utl_thr");
const TARGET_KEY: Symbol = symbol_short!("utl_tgt");

/// Threshold used until the admin sets one: about 30 days of ledgers.
pub const DEFAULT_USER_TTL_THRESHOLD: u32 = 518_400;
/// Target used until the admin sets one: about 60 days of ledgers.
pub const DEFAULT_USER_TTL_TARGET: u32 = 1_036_800;

/// Errors for the per-user TTL entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum UserTtlError {
    NotInitialized = 1,
    Unauthorized = 2,
    /// Either value passed to `set_user_ttl_extension` was zero.
    InvalidThreshold = 3,
}

/// The configured per-user TTL extension threshold, in ledgers.
pub fn threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&THRESHOLD_KEY)
        .unwrap_or(DEFAULT_USER_TTL_THRESHOLD)
}

/// The configured per-user TTL extension target, in ledgers.
pub fn target(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&TARGET_KEY)
        .unwrap_or(DEFAULT_USER_TTL_TARGET)
}

/// Extends `key`'s TTL to `tgt` if the entry exists and its remaining TTL is
/// currently below `thr`. A no-op for a key the user never wrote.
fn bump_if_present(env: &Env, key: &DataKey, thr: u32, tgt: u32) {
    let storage = env.storage().persistent();
    if storage.has(key) {
        storage.extend_ttl(key, thr, tgt);
    }
}

/// Extends the TTL of every persistent entry this contract keeps for `user`:
/// their position, reward-checkpoint, and settings entries. Safe to call for
/// any address, including one that has never interacted with the vault.
pub fn bump_entries(env: &Env, user: &Address) {
    let thr = threshold(env);
    let tgt = target(env).min(env.storage().max_ttl());

    // Position
    bump_if_present(env, &DataKey::ShareBalance(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::StakeHistory(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::StakedAtLedger(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::LastUnstakeLedger(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::UnbondingPosition(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::FrozenAt(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::FirstStakedAt(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::Restaked(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::VestingEntries(user.clone()), thr, tgt);

    // Checkpoint
    bump_if_present(env, &DataKey::RewardCheckpointLedger(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::LastClaimLedger(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::AccruedReward(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::RewardRemainder(user.clone()), thr, tgt);

    // Settings
    bump_if_present(env, &DataKey::Delegate(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::UserClaimWindow(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::Whitelisted(user.clone()), thr, tgt);
    bump_if_present(env, &DataKey::KycApproved(user.clone()), thr, tgt);
}

#[contractimpl]
impl VaultContract {
    /// Anyone (keepers included): extend `user`'s persistent position,
    /// checkpoint, and settings entries to the configured target, for any
    /// of them currently below the threshold. Keeps a dormant position from
    /// being archived without requiring `user`'s own signature.
    pub fn bump_user_entries(env: Env, user: Address) {
        crate::user_ttl::bump_entries(&env, &user);
    }

    /// Admin: set the threshold/target (in ledgers) used by
    /// `bump_user_entries` and the automatic per-user bumps on
    /// deposit/withdraw/claim. Both must be greater than zero.
    pub fn set_user_ttl_extension(
        env: Env,
        admin_addr: Address,
        threshold_ledgers: u32,
        target_ledgers: u32,
    ) -> Result<(), UserTtlError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env).map_err(|_| UserTtlError::NotInitialized)? {
            return Err(UserTtlError::Unauthorized);
        }
        if threshold_ledgers == 0 || target_ledgers == 0 {
            return Err(UserTtlError::InvalidThreshold);
        }
        env.storage()
            .instance()
            .set(&THRESHOLD_KEY, &threshold_ledgers);
        env.storage().instance().set(&TARGET_KEY, &target_ledgers);
        Ok(())
    }

    /// Read-only: the per-user TTL extension threshold, in ledgers.
    pub fn get_user_ttl_extension_threshold(env: Env) -> u32 {
        crate::user_ttl::threshold(&env)
    }

    /// Read-only: the per-user TTL extension target, in ledgers.
    pub fn get_user_ttl_extension_target(env: Env) -> u32 {
        crate::user_ttl::target(&env)
    }
}
