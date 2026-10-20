// Issue #534 — per-position custom reward multiplier
//
// Lets the admin apply a one-off reward multiplier to a specific user's
// position (distinct from the general boost schedule), useful for honoring
// manual grants, bug bounty rewards, or migration compensation.

use crate::admin;
use crate::errors::VaultError;
use soroban_sdk::{symbol_short, Address, Env};

/// Storage: persistent per-user tuple (multiplier_bps: u32, expires_at: Option<u32>).
/// Key is a raw Symbol "posmul" — each user gets their own persistent entry
/// keyed by address (matching the pattern used by other per-user persistent
/// keys in balance.rs).

/// Admin-only: set a custom reward multiplier for a specific user's position.
///
/// `multiplier_bps` — e.g. 15000 = 1.5x (150% of base rate).
/// `expires_at` — optional ledger number after which the multiplier reverts to 1x.
pub fn set(
    env: &Env,
    admin_addr: &Address,
    user: &Address,
    multiplier_bps: u32,
    expires_at: Option<u32>,
) -> Result<(), VaultError> {
    admin_addr.require_auth();
    admin::require_admin(env)?;

    if multiplier_bps == 0 {
        return Err(VaultError::ZeroAmount);
    }

    let data = (multiplier_bps, expires_at);
    env.storage()
        .persistent()
        .set(&soroban_sdk::Symbol::new(env, "posmul"), &(user.clone(), data));

    env.events().publish(
        (symbol_short!("pos_mul"), admin_addr, user),
        (multiplier_bps, expires_at, env.ledger().sequence()),
    );

    Ok(())
}

/// Read-only query: get the position multiplier for a user, if set and not expired.
/// Returns (multiplier_bps, expires_at) or None.
pub fn get(env: &Env, user: &Address) -> Option<(u32, Option<u32>)> {
    let stored: Option<(Address, (u32, Option<u32>))> = env
        .storage()
        .persistent()
        .get(&soroban_sdk::Symbol::new(env, "posmul"));

    match stored {
        Some((addr, (mult, exp))) if addr == *user => {
            if let Some(expiry) = exp {
                if env.ledger().sequence() > expiry {
                    return None;
                }
            }
            Some((mult, exp))
        }
        _ => None,
    }
}

/// Returns the effective multiplier in bps for a user.
/// 10_000 = 1x (no custom multiplier or expired).
pub fn get_effective_multiplier_bps(env: &Env, user: &Address) -> u32 {
    match get(env, user) {
        Some((mult, _)) => mult,
        None => 10_000,
    }
}
