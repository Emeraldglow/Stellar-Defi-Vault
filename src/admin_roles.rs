//! Issue #426: treasurer / pauser / rater roles alongside the single admin.
//!
//! The stored admin keeps every permission and is the only account that can
//! assign roles. Until `set_roles` is called, every role resolves to the admin.
//! The existing admin entrypoints take no caller address, so they stay
//! admin-only; the role-gated entrypoints below take an explicit `caller` (with
//! a `role_` prefix where the existing name is taken).
//!
//! `DataKey` is at Soroban's 50-variant cap, so storage uses raw `Symbol` keys.

use soroban_sdk::{
    contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, String, Symbol,
};

use crate::storage::{BrandingConfig, DataKey, PauseReason};
use crate::vault::{
    VaultContract, VaultContractClient, MAX_BRANDING_LOGO_LEN, MAX_BRANDING_NAME_LEN,
    MAX_BRANDING_TWITTER_LEN, MAX_BRANDING_URL_LEN,
};
use crate::{admin, balance, events};

const ROLES_KEY: Symbol = symbol_short!("adm_roles");
const POOL_NAME_KEY: Symbol = symbol_short!("pool_nm");
const CONTACT_KEY: Symbol = symbol_short!("emg_cont");

/// The three delegated role holders.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRoles {
    pub treasurer: Address,
    pub pauser: Address,
    pub rater: Address,
}

/// Errors for role management and the role-gated entrypoints.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RoleError {
    /// `set_roles` was called by someone other than the stored admin.
    Unauthorized = 1,
    /// The caller is neither the admin nor the holder of the required role.
    UnauthorizedRole = 2,
    NotInitialized = 3,
    InvalidValue = 4,
    InsufficientTreasury = 5,
    /// The underlying pause/unpause was rejected (stopped pool, re-pause cooldown).
    OperationFailed = 6,
}

#[derive(Copy, Clone)]
enum Role {
    Treasurer,
    Pauser,
    Rater,
}

fn load_roles(env: &Env) -> Result<AdminRoles, RoleError> {
    if let Some(roles) = env.storage().instance().get(&ROLES_KEY) {
        return Ok(roles);
    }
    let a = admin::get_admin(env).map_err(|_| RoleError::NotInitialized)?;
    Ok(AdminRoles {
        treasurer: a.clone(),
        pauser: a.clone(),
        rater: a,
    })
}

/// Requires `caller`'s auth and that they are the admin or hold `role`.
fn authorize(env: &Env, caller: &Address, role: Role) -> Result<(), RoleError> {
    caller.require_auth();
    if *caller == admin::get_admin(env).map_err(|_| RoleError::NotInitialized)? {
        return Ok(());
    }
    let roles = load_roles(env)?;
    let holder = match role {
        Role::Treasurer => roles.treasurer,
        Role::Pauser => roles.pauser,
        Role::Rater => roles.rater,
    };
    if *caller == holder {
        Ok(())
    } else {
        Err(RoleError::UnauthorizedRole)
    }
}

fn emit_role_assigned(env: &Env, role_type: &str, assignee: &Address) {
    env.events().publish(
        (Symbol::new(env, "role_assigned"),),
        (
            String::from_str(env, role_type),
            assignee.clone(),
            env.ledger().sequence(),
        ),
    );
}

#[contractimpl]
impl VaultContract {
    /// Original admin only: assign the treasurer, pauser and rater. Emits
    /// `role_assigned` as `(role_type, assignee, ledger)` for each.
    pub fn set_roles(
        env: Env,
        admin: Address,
        treasurer: Address,
        pauser: Address,
        rater: Address,
    ) -> Result<(), RoleError> {
        admin.require_auth();
        if admin != admin::get_admin(&env).map_err(|_| RoleError::NotInitialized)? {
            return Err(RoleError::Unauthorized);
        }
        env.storage().instance().set(
            &ROLES_KEY,
            &AdminRoles {
                treasurer: treasurer.clone(),
                pauser: pauser.clone(),
                rater: rater.clone(),
            },
        );
        emit_role_assigned(&env, "treasurer", &treasurer);
        emit_role_assigned(&env, "pauser", &pauser);
        emit_role_assigned(&env, "rater", &rater);
        Ok(())
    }

    /// Read-only: the current role holders (all the admin before `set_roles`).
    pub fn get_roles(env: Env) -> Result<AdminRoles, RoleError> {
        load_roles(&env)
    }

    // ── Treasurer ───────────────────────────────────────────────────────────

    /// Treasurer/admin: set the reward APR in bps (same effect as `set_reward_rate_bps`).
    pub fn set_reward_rate(env: Env, caller: Address, rate_bps: u32) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Treasurer)?;
        if rate_bps > balance::MAX_RATE_BPS {
            return Err(RoleError::InvalidValue);
        }
        crate::vault_extensions_538_541::clear_rate_ramp(&env);
        let old_rate = balance::get_reward_rate_bps(&env);
        balance::set_reward_rate_bps(&env, rate_bps);
        balance::record_rate_change(&env, old_rate, rate_bps);
        Ok(())
    }

    /// Treasurer/admin: set the unstake fee in bps, max 500 (same as `set_unstake_fee_bps`).
    pub fn set_fee(env: Env, caller: Address, bps: u32) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Treasurer)?;
        // Routes through the ceiling check (MAX_FEE_BPS) and notice-period
        // gate: increases are queued; decreases apply immediately.
        crate::fee_change_notice::propose_fee_change(&env, bps)
            .map_err(|_| RoleError::InvalidValue)?;
        Ok(())
    }

    /// Treasurer/admin: pay `amount` out of the community treasury balance to `to`.
    pub fn withdraw_treasury(
        env: Env,
        caller: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Treasurer)?;
        let treasury = crate::community_treasury::get_community_treasury_balance(&env);
        if amount <= 0 {
            return Err(RoleError::InvalidValue);
        }
        if amount > treasury {
            return Err(RoleError::InsufficientTreasury);
        }
        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(RoleError::NotInitialized)?;
        token::Client::new(&env, &token_addr).transfer(&env.current_contract_address(), &to, &amount);
        crate::community_treasury::set_community_treasury_balance(&env, treasury - amount);
        Ok(())
    }

    /// Treasurer/admin: move `amount` from the caller's balance into the reward pool.
    pub fn supply_rewards(env: Env, caller: Address, amount: i128) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Treasurer)?;
        if amount <= 0 {
            return Err(RoleError::InvalidValue);
        }
        let token_addr: Address = match balance::get_reward_token(&env) {
            Some(token) => token,
            None => env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(RoleError::NotInitialized)?,
        };
        token::Client::new(&env, &token_addr).transfer(
            &caller,
            &env.current_contract_address(),
            &amount,
        );
        let pool = balance::get_reward_pool_balance(&env);
        balance::set_reward_pool_balance(&env, pool + amount);
        Ok(())
    }

    // ── Pauser ──────────────────────────────────────────────────────────────

    /// Pauser/admin: pause the pool (same as `pause`).
    pub fn role_pause(
        env: Env,
        caller: Address,
        reason: PauseReason,
        message: String,
    ) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Pauser)?;
        if crate::pause_grace_period::repause_blocked(&env) {
            return Err(RoleError::OperationFailed);
        }
        VaultContract::pause_by(&env, &caller, reason, message)
            .map_err(|_| RoleError::OperationFailed)
    }

    /// Pauser/admin: lift a pause (same as `unpause`).
    pub fn role_unpause(env: Env, caller: Address) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Pauser)?;
        VaultContract::unpause_by(&env, &caller).map_err(|_| RoleError::OperationFailed)
    }

    /// Pauser/admin: permanently stop the pool. Irreversible (same as `emergency_stop`).
    pub fn role_emergency_stop(env: Env, caller: Address) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Pauser)?;
        env.storage().instance().set(&DataKey::Stopped, &true);
        events::stopped(&env, &caller);
        Ok(())
    }

    /// Pauser/admin: reject new deposits and waive unstake fees until users exit
    /// by `exit_deadline` (same as `initiate_sunset`).
    pub fn graceful_shutdown(env: Env, caller: Address, exit_deadline: u32) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Pauser)?;
        balance::set_sunset_exit_deadline(&env, exit_deadline);
        events::sunset_initiated(&env, &caller, exit_deadline);
        Ok(())
    }

    // ── Rater ───────────────────────────────────────────────────────────────

    /// Rater/admin: set the pool display name (1-50 bytes).
    pub fn set_pool_name(env: Env, caller: Address, name: String) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Rater)?;
        if name.len() == 0 || name.len() > MAX_BRANDING_NAME_LEN {
            return Err(RoleError::InvalidValue);
        }
        env.storage().instance().set(&POOL_NAME_KEY, &name);
        Ok(())
    }

    /// Read-only: the pool name, `None` until one is set.
    pub fn get_pool_name(env: Env) -> Option<String> {
        env.storage().instance().get(&POOL_NAME_KEY)
    }

    /// Rater/admin: set the pool description, max 200 bytes (same as `set_pool_description`).
    pub fn role_set_pool_description(
        env: Env,
        caller: Address,
        description: String,
    ) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Rater)?;
        if description.len() > 200 {
            return Err(RoleError::InvalidValue);
        }
        balance::set_pool_description(&env, &description);
        events::description_updated(&env, &caller, &description);
        Ok(())
    }

    /// Rater/admin: set the pool branding (field length caps as in `BrandingConfig` docs).
    pub fn set_branding(env: Env, caller: Address, config: BrandingConfig) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Rater)?;
        if config.display_name.len() > MAX_BRANDING_NAME_LEN
            || config.logo_hash.len() > MAX_BRANDING_LOGO_LEN
            || config.website_url.len() > MAX_BRANDING_URL_LEN
            || config.twitter_handle.len() > MAX_BRANDING_TWITTER_LEN
        {
            return Err(RoleError::InvalidValue);
        }
        balance::set_branding(&env, &config);
        Ok(())
    }

    /// Rater/admin: set the emergency contact string (1-200 bytes).
    pub fn set_emergency_contact(
        env: Env,
        caller: Address,
        contact: String,
    ) -> Result<(), RoleError> {
        authorize(&env, &caller, Role::Rater)?;
        if contact.len() == 0 || contact.len() > 200 {
            return Err(RoleError::InvalidValue);
        }
        env.storage().instance().set(&CONTACT_KEY, &contact);
        Ok(())
    }

    /// Read-only: the emergency contact, `None` until one is set.
    pub fn get_emergency_contact(env: Env) -> Option<String> {
        env.storage().instance().get(&CONTACT_KEY)
    }
}
