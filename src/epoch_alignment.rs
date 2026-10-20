//! Calendar-style epoch alignment (issue #342).
//!
//! Builds on issue #37's epoch-based distribution: instead of an epoch
//! "starting" whenever `set_epoch_mode` happens to be called, epochs are
//! pinned to predictable ledger boundaries â€” `anchor_ledger + N *
//! epoch_length` â€” so reward timing is auditable across restarts and doesn't
//! drift with when an admin happened to flip a switch.
//!
//! # Wiring
//!
//! This crate has no live reward-distribution entrypoint to hook an automatic
//! epoch-transition trigger into yet (see this PR's description for the full
//! picture), so â€” matching the level of integration already established by
//! `vesting_cliff.rs`/`price_oracle.rs` â€” this module is entirely
//! computation-from-the-current-ledger: there's no stored "current epoch"
//! that needs advancing, `get_current_epoch_number` derives it fresh every
//! call, which is what makes epoch transitions automatic with no admin
//! trigger in the first place.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `vesting_cliff.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance-storage key for the epoch alignment config.
const CONFIG_KEY: Symbol = symbol_short!("epc_algn");

/// Persistent-storage key prefix recording the last epoch number `user`
/// interacted in, so `epoch_boundary_crossed` fires at most once per user
/// per boundary crossing rather than on every call.
const LAST_EPOCH_KEY: Symbol = symbol_short!("epc_last");

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EpochAlignment {
    pub anchor_ledger: u32,
    pub epoch_length: u32,
}

/// The current alignment config, if one has been set.
pub fn get_alignment(env: &Env) -> Option<EpochAlignment> {
    env.storage().instance().get(&CONFIG_KEY)
}

/// The epoch number the current ledger falls in, under `alignment`.
///
/// `(current_ledger - anchor_ledger) / epoch_length`, per the issue's
/// formula. Ledgers before the anchor are clamped to epoch 0 rather than
/// underflowing â€” an anchor set in the future (e.g. scheduling a new
/// alignment ahead of time) reports epoch 0 until it's reached.
fn epoch_number_at(alignment: &EpochAlignment, ledger: u32) -> u32 {
    if alignment.epoch_length == 0 {
        return 0;
    }
    ledger
        .saturating_sub(alignment.anchor_ledger)
        .checked_div(alignment.epoch_length)
        .unwrap_or(0)
}

/// The first ledger of `epoch_number`, under `alignment`.
fn epoch_start_ledger(alignment: &EpochAlignment, epoch_number: u32) -> u32 {
    alignment
        .anchor_ledger
        .saturating_add(epoch_number.saturating_mul(alignment.epoch_length))
}

/// The last ledger of `epoch_number` (inclusive), under `alignment`.
fn epoch_end_ledger(alignment: &EpochAlignment, epoch_number: u32) -> u32 {
    epoch_start_ledger(alignment, epoch_number)
        .saturating_add(alignment.epoch_length)
        .saturating_sub(1)
}

/// Emit `epoch_boundary_crossed` the first time `user` interacts after the
/// epoch number has advanced since their last recorded interaction, then
/// remember the new epoch number. No-op when no alignment is configured.
///
/// Mirrors `vesting_cliff::maybe_emit_cliff_unlocked`'s lazy-emission
/// approach: a contract can't wake itself at a ledger height, so this is the
/// closest thing to "on boundary" available without an off-chain keeper.
pub fn maybe_emit_boundary_crossed(env: &Env, user: &Address) {
    let alignment = match get_alignment(env) {
        Some(a) => a,
        None => return,
    };

    let current_epoch = epoch_number_at(&alignment, env.ledger().sequence());
    let key = (LAST_EPOCH_KEY, user.clone());
    let last_epoch: Option<u32> = env.storage().persistent().get(&key);

    match last_epoch {
        Some(old_epoch) if old_epoch != current_epoch => {
            env.storage().persistent().set(&key, &current_epoch);
            env.events().publish(
                (symbol_short!("epc_cross"), user.clone()),
                (old_epoch, current_epoch, env.ledger().sequence()),
            );
        }
        Some(_) => {}
        None => {
            // First interaction ever seen for this user under alignment â€”
            // record the baseline without emitting; there's no prior epoch
            // to have "crossed" from.
            env.storage().persistent().set(&key, &current_epoch);
        }
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Set calendar-style epoch alignment. Admin only.
    ///
    /// Per the issue notes, this does not retroactively affect rewards
    /// already distributed in past epochs â€” it only changes how future
    /// epoch numbers/boundaries are computed from this point forward.
    pub fn set_epoch_alignment(
        env: Env,
        anchor_ledger: u32,
        epoch_length: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        if epoch_length == 0 {
            return Err(VaultError::InvalidRate);
        }

        let alignment = EpochAlignment {
            anchor_ledger,
            epoch_length,
        };
        env.storage().instance().set(&CONFIG_KEY, &alignment);

        let admin_addr = admin::get_admin(&env)?;
        env.events().publish(
            (symbol_short!("epc_algn_"), admin_addr),
            (anchor_ledger, epoch_length, env.ledger().sequence()),
        );
        Ok(())
    }

    /// The current alignment config `(anchor_ledger, epoch_length)`, if set.
    pub fn get_epoch_alignment(env: Env) -> Option<(u32, u32)> {
        get_alignment(&env).map(|a| (a.anchor_ledger, a.epoch_length))
    }

    /// The epoch number the current ledger falls in. `0` when no alignment
    /// is configured (matches the anchor-in-the-future clamp for
    /// consistency, rather than an arbitrary sentinel).
    pub fn get_current_epoch_number(env: Env) -> u32 {
        match get_alignment(&env) {
            Some(a) => epoch_number_at(&a, env.ledger().sequence()),
            None => 0,
        }
    }

    /// The first ledger of `epoch_number`. `0` when no alignment is set.
    pub fn get_epoch_start_ledger(env: Env, epoch_number: u32) -> u32 {
        match get_alignment(&env) {
            Some(a) => epoch_start_ledger(&a, epoch_number),
            None => 0,
        }
    }

    /// The last ledger of `epoch_number` (inclusive). `0` when no alignment
    /// is set.
    pub fn get_epoch_end_ledger(env: Env, epoch_number: u32) -> u32 {
        match get_alignment(&env) {
            Some(a) => epoch_end_ledger(&a, epoch_number),
            None => 0,
        }
    }
}















