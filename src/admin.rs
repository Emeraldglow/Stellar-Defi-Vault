use crate::errors::VaultError;
use crate::storage::{AdminKey, DataKey, StorageKey};
use soroban_sdk::{symbol_short, Address, Env, Symbol, Vec};

const ADMIN_RENOUNCED_KEY: Symbol = symbol_short!("adm_none");
const PENDING_ADMIN_KEY: Symbol = symbol_short!("pend_adm");

pub fn require_admin_as(env: &Env, claimed_admin: &Address) -> Result<(), VaultError> {
    let admin = get_admin(env)?;
    if &admin != claimed_admin {
        return Err(VaultError::Unauthorized);
    }
    admin.require_auth();
    Ok(())
}

/// Reads the currently proposed pending administrator, if any (Issue #738).
pub fn get_pending_admin(env: &Env) -> Option<Address> {
    env.storage()
        .instance()
        .get(&StorageKey::Admin(AdminKey::PendingAdmin))
        .or_else(|| env.storage().instance().get(&PENDING_ADMIN_KEY))
}

/// Sets the pending administrator proposal (Issue #738).
pub fn set_pending_admin(env: &Env, pending: &Address) {
    env.storage()
        .instance()
        .set(&StorageKey::Admin(AdminKey::PendingAdmin), pending);
    env.storage().instance().set(&PENDING_ADMIN_KEY, pending);
}

/// Clears any active pending administrator proposal (Issue #738).
pub fn clear_pending_admin(env: &Env) {
    env.storage()
        .instance()
        .remove(&StorageKey::Admin(AdminKey::PendingAdmin));
    env.storage().instance().remove(&PENDING_ADMIN_KEY);
}

/// Writes the admin key. When this replaces an existing, different admin
/// (a mid-flight handover via admin recovery or any other transfer path),
/// every still-pending timelocked admin action is invalidated — see
/// [`invalidate_pending_timelocked_actions`].
pub fn set_admin(env: &Env, admin: &Address) {
    let previous: Option<Address> = env.storage().instance().get(&DataKey::Admin);
    env.storage().instance().set(&DataKey::Admin, admin);
    clear_pending_admin(env);
    if let Some(previous) = previous {
        if &previous != admin {
            invalidate_pending_timelocked_actions(env, &previous, admin);
        }
    }
}

/// Timelocked admin actions are bound to the admin that queued them. On an
/// admin change, all of them are dropped so that:
/// - an outgoing admin's stale action cannot execute under the new admin's
///   watch (the new admin would otherwise only have to call `execute_*`, or
///   miss the queue entirely), and
/// - the new admin does not silently inherit intentions it never signed off
///   on.
///
/// Covers the #195 `queue_action` queue and the #503 `queue_admin_action`
/// queue (both removed), and the #455 config-change announcements (marked
/// cancelled, so the public record of what was announced is preserved). The
/// new admin must re-queue anything it still wants, restarting the delay.
/// Emits `adm_inval` with the per-queue counts.
fn invalidate_pending_timelocked_actions(env: &Env, old_admin: &Address, new_admin: &Address) {
    let queued = crate::balance::get_pending_actions(env);
    let queued_count = queued.len();
    if queued_count > 0 {
        crate::balance::set_pending_actions(env, &Vec::new(env));
    }
    let timelocked_count = crate::vault::vault_extensions_502_505::clear_queued_admin_actions(env);
    let announced_count = crate::time_locked_admin_proposal::cancel_open_proposals(env);

    env.events().publish(
        (symbol_short!("adm_inval"), old_admin.clone()),
        (new_admin.clone(), queued_count, timelocked_count, announced_count),
    );
}

pub fn get_admin(env: &Env) -> Result<Address, VaultError> {
    if is_renounced(env) {
        return Err(VaultError::NoAdmin);
    }
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(VaultError::NotInitialized)
}

pub fn is_renounced(env: &Env) -> bool {
    env.storage().instance().get(&ADMIN_RENOUNCED_KEY).unwrap_or(false)
}

/// Permanently removes primary and emergency admin authority.
pub fn renounce(env: &Env, expected_admin: &Address) -> Result<(), VaultError> {
    let current_admin = get_admin(env)?;
    if &current_admin != expected_admin {
        return Err(VaultError::Unauthorized);
    }
    expected_admin.require_auth();
    env.storage().instance().set(&ADMIN_RENOUNCED_KEY, &true);
    env.storage().instance().remove(&DataKey::Admin);
    env.storage().instance().remove(&symbol_short!("emg_adm"));
    clear_pending_admin(env);
    Ok(())
}

/// Sets a new emergency admin address.
/// CAN ONLY be called by the primary admin.
pub fn set_emergency_admin(env: &Env, emergency_admin: &Address) -> Result<(), VaultError> {
    // Enforce requirement 4: Only the primary admin can designate another emergency admin
    let primary_admin = get_admin(env)?;
    primary_admin.require_auth();

    if emergency_admin == &env.current_contract_address() {
        return Err(VaultError::InvalidAddress);
    }

    env.storage().instance().set(&symbol_short!("emg_adm"), emergency_admin);

    // Enforce requirement 5: Emit event: emergency_admin_set
    env.events().publish(
        (symbol_short!("emer_set"),),
        emergency_admin.clone(),
    );

    Ok(())
}

pub fn get_emergency_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("emg_adm"))
}

/// Revokes the current emergency admin address.
/// CAN ONLY be called by the primary admin.
pub fn clear_emergency_admin(env: &Env) -> Result<(), VaultError> {
    // Only the primary admin can clear/revoke an emergency admin
    let primary_admin = get_admin(env)?;
    primary_admin.require_auth();

    if env.storage().instance().has(&symbol_short!("emg_adm")) {
        env.storage().instance().remove(&symbol_short!("emg_adm"));

        // Enforce requirement 5: Emit event: emergency_admin_revoked
        env.events().publish(
            (symbol_short!("emer_rvk"),),
            (),
        );
    }

    Ok(())
}

pub fn require_admin(env: &Env) -> Result<(), VaultError> {
    let admin = get_admin(env)?;
    admin.require_auth();
    Ok(())
}

/// Validates that the active transaction context is authorized by either 
/// the primary administrator or the secondary crisis administrator.
pub fn require_admin_or_emergency_admin(env: &Env, caller: &Address) -> Result<(), VaultError> {
    let primary = get_admin(env)?;
    if caller == &primary {
        primary.require_auth();
        return Ok(());
    }

    if let Some(emergency) = get_emergency_admin(env) {
        if caller == &emergency {
            emergency.require_auth();
            return Ok(());
        }
    }

    // Falls back to NotAuthorized/Unauthorized variant present in your error definitions
    Err(VaultError::Unauthorized)
}
