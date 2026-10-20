//! Vault Extensions for Issues #502, #503, #504, #505
//!
//! Issue #593: every ad-hoc string `panic!` in this module has been replaced
//! with a typed `VaultFeature5Error` return (stable numeric codes — see
//! `ERRORS.md`), and the admin gates now propagate `VaultError` via `?`
//! instead of `.unwrap()`-ing into an untyped host panic. The duplicate
//! `set_timelock_delay` entrypoint was dropped in favor of the canonical one
//! in `vault.rs` (issue #195); `queue_admin_action` reads that shared delay
//! via `balance::get_timelock_delay`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};
use crate::vault::{VaultContract, VaultContractClient};
use crate::storage::AdminAction;
use crate::admin;
use crate::balance;
use crate::errors::{VaultError, VaultFeature5Error};

// ----------------------------------------------------------------------------
// Issue #502: Withdrawal Queue
// ----------------------------------------------------------------------------
const WQ_ENABLED: Symbol = symbol_short!("wq_en");
const WQ_QUEUE: Symbol = symbol_short!("wq_q");

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct QueueEntry {
    pub user: Address,
    pub shares: i128,
    pub requested_at: u64,
}

pub fn is_withdrawal_queue_enabled(env: &Env) -> bool {
    env.storage().instance().get(&WQ_ENABLED).unwrap_or(false)
}

pub fn enqueue_withdrawal(env: &Env, user: Address, shares: i128) {
    let mut q: Vec<QueueEntry> = env.storage().instance().get(&WQ_QUEUE).unwrap_or(Vec::new(env));
    q.push_back(QueueEntry {
        user: user.clone(),
        shares,
        requested_at: env.ledger().timestamp(),
    });
    env.storage().instance().set(&WQ_QUEUE, &q);
    env.events().publish((symbol_short!("wq_queued"),), (user, shares));
}

// ----------------------------------------------------------------------------
// Issue #503: Admin Timelock
// ----------------------------------------------------------------------------
const TL_NEXT_ID: Symbol = symbol_short!("tl_nxt_id");
const TL_ACTIONS: Symbol = symbol_short!("tl_acts");

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedAction {
    pub action: AdminAction,
    pub executable_at: u32,
}

/// Drops every queued #503 action, returning how many were pending. Called by
/// `admin::set_admin` on an admin change so a previous admin's queue never
/// carries over.
pub(crate) fn clear_queued_admin_actions(env: &Env) -> u32 {
    let map: Option<soroban_sdk::Map<u32, QueuedAction>> = env.storage().instance().get(&TL_ACTIONS);
    match map {
        Some(map) if !map.is_empty() => {
            env.storage().instance().remove(&TL_ACTIONS);
            map.len()
        }
        _ => 0,
    }
}

// ----------------------------------------------------------------------------
// Issue #504: Auto Compound
// ----------------------------------------------------------------------------
// Issue #609 storage audit: this was previously a single `Map<Address, bool>`
// under one instance-storage key, which is per-user data misclassified as
// instance storage — every call rewrote the whole map, and the entry grows
// without bound as the depositor count grows (see STORAGE.md). Switched to a
// `(Symbol, Address)` tuple key per user under persistent storage, matching
// the convention used for every other per-user key in this codebase.
const AUTO_COMPOUND: Symbol = symbol_short!("auto_comp");

// ----------------------------------------------------------------------------
// Issue #505: Tokenize Position
// ----------------------------------------------------------------------------
const NFT_BALANCES: Symbol = symbol_short!("nft_bals"); // Map<u32, i128>
const NFT_OWNERS: Symbol = symbol_short!("nft_owns"); // Map<u32, Address>
const NEXT_NFT_ID: Symbol = symbol_short!("nft_nxt");

#[contractimpl]
impl VaultContract {
    // ------------------------------------------------------------------------
    // Issue #502: Withdrawal Queue
    // ------------------------------------------------------------------------
    pub fn enable_withdrawal_queue(env: Env, admin: Address, enabled: bool) -> Result<(), VaultError> {
        admin.require_auth();
        admin::require_admin(&env)?;
        env.storage().instance().set(&WQ_ENABLED, &enabled);
        Ok(())
    }

    pub fn process_withdrawal_queue(env: Env, admin: Address, max_entries: u32) -> Result<(), VaultError> {
        admin.require_auth();
        admin::require_admin(&env)?;
        let mut q: Vec<QueueEntry> = env.storage().instance().get(&WQ_QUEUE).unwrap_or(Vec::new(&env));
        let mut processed = 0;

        while processed < max_entries && q.len() > 0 {
            let entry = q.get(0).unwrap();
            match Self::do_unstake(&env, &entry.user, entry.shares) {
                Ok(_) => {
                    env.events().publish((symbol_short!("wq_proc"),), (entry.user.clone(), entry.shares));
                    q.remove(0);
                    processed += 1;
                },
                Err(_) => {
                    break;
                }
            }
        }
        env.storage().instance().set(&WQ_QUEUE, &q);
        Ok(())
    }

    pub fn get_queue_position(env: Env, user: Address) -> Option<u32> {
        let q: Vec<QueueEntry> = env.storage().instance().get(&WQ_QUEUE).unwrap_or(Vec::new(&env));
        for i in 0..q.len() {
            if q.get(i).unwrap().user == user {
                return Some(i);
            }
        }
        None
    }

    // ------------------------------------------------------------------------
    // Issue #503: Admin Timelock
    // ------------------------------------------------------------------------
    pub fn queue_admin_action(env: Env, admin: Address, action: AdminAction) -> Result<u32, VaultError> {
        admin.require_auth();
        admin::require_admin(&env)?;
        // The shared timelock delay configured via `set_timelock_delay`
        // (issue #195) governs this queue as well.
        let delay: u32 = balance::get_timelock_delay(&env);
        let action_id: u32 = env.storage().instance().get(&TL_NEXT_ID).unwrap_or(1);
        env.storage().instance().set(&TL_NEXT_ID, &(action_id + 1));

        let queued = QueuedAction {
            action: action.clone(),
            executable_at: env.ledger().sequence() + delay,
        };
        let mut map: soroban_sdk::Map<u32, QueuedAction> = env.storage().instance().get(&TL_ACTIONS).unwrap_or(soroban_sdk::Map::new(&env));
        map.set(action_id, queued);
        env.storage().instance().set(&TL_ACTIONS, &map);
        env.events().publish((symbol_short!("act_queue"),), (action_id, action));
        Ok(action_id)
    }

    pub fn execute_admin_action(env: Env, admin: Address, action_id: u32) -> Result<(), VaultFeature5Error> {
        admin.require_auth();
        admin::require_admin(&env)?;
        let mut map: soroban_sdk::Map<u32, QueuedAction> = env.storage().instance().get(&TL_ACTIONS).unwrap_or(soroban_sdk::Map::new(&env));
        match map.get(action_id) {
            Some(queued) => {
                if env.ledger().sequence() < queued.executable_at {
                    return Err(VaultFeature5Error::TimelockNotExpired);
                }
                env.events().publish((symbol_short!("act_exec"),), (action_id,));
                map.remove(action_id);
                env.storage().instance().set(&TL_ACTIONS, &map);
                Ok(())
            }
            None => Err(VaultFeature5Error::ActionNotFound),
        }
        Ok(())
    }

    pub fn cancel_admin_action(env: Env, admin: Address, action_id: u32) -> Result<(), VaultFeature5Error> {
        admin.require_auth();
        admin::require_admin(&env)?;
        let mut map: soroban_sdk::Map<u32, QueuedAction> = env.storage().instance().get(&TL_ACTIONS).unwrap_or(soroban_sdk::Map::new(&env));
        if map.contains_key(action_id) {
            map.remove(action_id);
            env.storage().instance().set(&TL_ACTIONS, &map);
            env.events().publish((symbol_short!("act_canc"),), (action_id,));
            Ok(())
        } else {
            Err(VaultFeature5Error::ActionNotFound)
        }
        Ok(())
    }

    // ------------------------------------------------------------------------
    // Issue #504: Auto Compound
    // ------------------------------------------------------------------------
    pub fn set_auto_compound(env: Env, user: Address, enabled: bool) {
        user.require_auth();
        env.storage()
            .persistent()
            .set(&(AUTO_COMPOUND, user.clone()), &enabled);
    }

    pub fn compound(env: Env, user: Address) -> Result<(), VaultFeature5Error> {
        // Issue #705: compounding re-stakes rewards, which is an inflow.
        if crate::vault::withdraw_only_mode::is_active(&env) {
            return Err(VaultFeature5Error::WithdrawOnlyMode);
        }
        let enabled: bool = env
            .storage()
            .persistent()
            .get(&(AUTO_COMPOUND, user.clone()))
            .unwrap_or(false);
        if !enabled {
            return Err(VaultFeature5Error::AutoCompoundNotEnabled);
        }

        let pending = balance::get_accrued_reward(&env, &user);
        if pending > 0 {
            balance::set_accrued_reward(&env, &user, 0);

            let total_shares = balance::get_total_shares(&env);
            let total_deposited = balance::get_total_deposited(&env);
            let shares = if total_deposited == 0 {
                pending
            } else {
                pending
                    .checked_mul(total_shares)
                    .and_then(|value| value.checked_div(total_deposited))
                    .ok_or(PublicApiError::ArithmeticError)?
            };

            balance::set_shares(&env, &user, balance::get_shares(&env, &user) + shares);
            balance::set_total_shares(&env, total_shares + shares);
            balance::set_total_deposited(&env, total_deposited + pending);

            env.events().publish((Symbol::new(&env, "compounded"),), (user, pending, shares, env.ledger().sequence()));
        }
        Ok(())
    }

    // ------------------------------------------------------------------------
    // Issue #505: Tokenize Position
    // ------------------------------------------------------------------------
    pub fn tokenize_position(env: Env, user: Address) -> Result<u32, VaultFeature5Error> {
        user.require_auth();
        let shares = balance::get_shares(&env, &user);
        if shares <= 0 {
            return Err(VaultFeature5Error::NoSharesToTokenize);
        }

        balance::set_shares(&env, &user, 0);

        let token_id: u32 = env.storage().instance().get(&NEXT_NFT_ID).unwrap_or(1);
        env.storage().instance().set(&NEXT_NFT_ID, &(token_id + 1));

        let mut owns: soroban_sdk::Map<u32, Address> = env.storage().instance().get(&NFT_OWNERS).unwrap_or(soroban_sdk::Map::new(&env));
        owns.set(token_id, user.clone());
        env.storage().instance().set(&NFT_OWNERS, &owns);

        let mut bals: soroban_sdk::Map<u32, i128> = env.storage().instance().get(&NFT_BALANCES).unwrap_or(soroban_sdk::Map::new(&env));
        bals.set(token_id, shares);
        env.storage().instance().set(&NFT_BALANCES, &bals);

        env.events().publish((symbol_short!("tok_pos"),), (user, token_id, shares));
        Ok(token_id)
    }

    pub fn redeem_position_nft(env: Env, user: Address, token_id: u32) -> Result<(), VaultFeature5Error> {
        user.require_auth();
        let mut owns: soroban_sdk::Map<u32, Address> = env.storage().instance().get(&NFT_OWNERS).unwrap_or(soroban_sdk::Map::new(&env));

        if owns.get(token_id) != Some(user.clone()) {
            return Err(VaultFeature5Error::NotNftOwner);
        }

        let mut bals: soroban_sdk::Map<u32, i128> = env.storage().instance().get(&NFT_BALANCES).unwrap_or(soroban_sdk::Map::new(&env));
        let shares = bals.get(token_id).unwrap_or(0);

        owns.remove(token_id);
        bals.remove(token_id);
        env.storage().instance().set(&NFT_OWNERS, &owns);
        env.storage().instance().set(&NFT_BALANCES, &bals);

        balance::set_shares(&env, &user, balance::get_shares(&env, &user) + shares);
        env.events().publish((Symbol::new(&env, "nft_redeem"),), (user, token_id, shares));
        Ok(())
    }
}
