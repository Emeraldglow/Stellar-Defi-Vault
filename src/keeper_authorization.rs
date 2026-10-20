//! Per-user keeper authorization (issue #698).
//!
//! A user can authorize up to three specific keeper addresses to trigger
//! `claim` or `compound` for them. This is different from `keeper_registry`
//! (which keepers the admin approves) and from the public `compound()` (which
//! anyone can call once the user has opted in): here the *user* decides who
//! may act for them.
//!
//! A keeper only triggers the action. `claim_for()` pays the reward to the
//! user exactly as the user's own `claim()` would, and `compound_for()`
//! reinvests it into the user's own position, so no funds ever route to the
//! keeper. Revoking a keeper takes effect immediately.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! persistent storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol, Vec};

use crate::errors::{VaultFeature5Error, VaultFeature6Error};
use crate::vault::VaultContractClient;
use crate::VaultContract;

const KEEPERS_KEY: Symbol = symbol_short!("kpr_auth");

/// Maximum number of keepers a single user can authorize at once.
pub const MAX_KEEPERS_PER_USER: u32 = 3;

fn get_keepers(env: &Env, user: &Address) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&(KEEPERS_KEY, user.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

fn require_authorized(
    env: &Env,
    user: &Address,
    keeper: &Address,
) -> Result<(), VaultFeature6Error> {
    if get_keepers(env, user).contains(keeper) {
        Ok(())
    } else {
        Err(VaultFeature6Error::KeeperNotAuthorized)
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Authorizes `keeper` to call `claim_for()` and `compound_for()` for
    /// `user`. Authorizing an already-authorized keeper is a no-op.
    ///
    /// # Authorization
    /// `user` must sign.
    ///
    /// # Errors
    /// - `KeeperLimitReached`: `user` already has three authorized keepers.
    ///
    /// # Events
    /// - `kpr_auth` when a keeper is newly authorized.
    pub fn authorize_keeper(
        env: Env,
        user: Address,
        keeper: Address,
    ) -> Result<(), VaultFeature6Error> {
        user.require_auth();
        let mut keepers = get_keepers(&env, &user);
        if keepers.contains(&keeper) {
            return Ok(());
        }
        if keepers.len() >= MAX_KEEPERS_PER_USER {
            return Err(VaultFeature6Error::KeeperLimitReached);
        }
        keepers.push_back(keeper.clone());
        env.storage()
            .persistent()
            .set(&(KEEPERS_KEY, user.clone()), &keepers);
        env.events()
            .publish((symbol_short!("kpr_auth"), user, keeper), ());
        Ok(())
    }

    /// Removes `keeper`'s authorization for `user`, effective immediately.
    /// Revoking a keeper that is not authorized is a no-op.
    ///
    /// # Authorization
    /// `user` must sign.
    ///
    /// # Events
    /// - `kpr_rvk` when an authorization is removed.
    pub fn revoke_keeper(env: Env, user: Address, keeper: Address) {
        user.require_auth();
        let keepers = get_keepers(&env, &user);
        if let Some(index) = keepers.first_index_of(&keeper) {
            let mut keepers = keepers;
            keepers.remove(index);
            env.storage()
                .persistent()
                .set(&(KEEPERS_KEY, user.clone()), &keepers);
            env.events()
                .publish((symbol_short!("kpr_rvk"), user, keeper), ());
        }
    }

    /// Whether `keeper` is currently authorized by `user`.
    pub fn is_authorized_keeper(env: Env, user: Address, keeper: Address) -> bool {
        get_keepers(&env, &user).contains(&keeper)
    }

    /// Claims `user`'s accrued rewards on their behalf. The reward is paid to
    /// `user`, never to `keeper`. Returns the amount paid.
    ///
    /// # Authorization
    /// `keeper` must sign and must be authorized by `user`.
    ///
    /// # Errors
    /// - `KeeperNotAuthorized`: `keeper` is not authorized by `user`.
    /// - Errors from the claim path (`InsufficientRewardPool`, ...).
    ///
    /// # Events
    /// - `claim_for` plus the usual `claimed` event.
    pub fn claim_for(
        env: Env,
        keeper: Address,
        user: Address,
    ) -> Result<i128, VaultFeature6Error> {
        keeper.require_auth();
        require_authorized(&env, &user, &keeper)?;
        let reward = VaultContract::claim_for_user(&env, &user)?;
        env.events()
            .publish((symbol_short!("claim_for"), keeper, user), reward);
        Ok(reward)
    }

    /// Compounds `user`'s accrued rewards into their position on their
    /// behalf, using the existing `compound()`. The user must have enabled
    /// auto-compounding.
    ///
    /// # Authorization
    /// `keeper` must sign and must be authorized by `user`.
    ///
    /// # Errors
    /// - `KeeperNotAuthorized`: `keeper` is not authorized by `user`.
    /// - `AutoCompoundNotEnabled`: `user` has not opted into auto-compounding.
    /// - `ArithmeticError`: share conversion overflow.
    pub fn compound_for(
        env: Env,
        keeper: Address,
        user: Address,
    ) -> Result<(), VaultFeature6Error> {
        keeper.require_auth();
        require_authorized(&env, &user, &keeper)?;
        VaultContract::compound(env, user).map_err(|err| match err {
            VaultFeature5Error::AutoCompoundNotEnabled => {
                VaultFeature6Error::AutoCompoundNotEnabled
            }
            _ => VaultFeature6Error::ArithmeticError,
        })
    }
}
