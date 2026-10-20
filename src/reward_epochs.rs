//! Reward epochs with a per-epoch emission budget (issue #700).
//!
//! Reward distribution is divided into fixed-length epochs of `length_ledgers`
//! ledgers, each with an explicit emission `budget`. Every reward accrual
//! recorded during an epoch counts against that epoch's budget, and accrual
//! beyond it is cut off, so total emissions over any epoch are bounded and
//! predictable.
//!
//! # Unused budget
//!
//! Unspent budget is **discarded**, not rolled forward: each epoch starts with
//! exactly `budget`, however much the previous one emitted. This keeps the
//! bound on any epoch fixed at `budget`; a roll-forward would let one epoch
//! emit several budgets' worth after a quiet period.
//!
//! # How accrual is attributed
//!
//! Rewards are credited to users lazily (on stake, unstake, claim, ...), so
//! an accrual is charged to the epoch in which it is *recorded*, even when the
//! user earned it over ledgers that span a boundary. Read-only pending views
//! do not apply the budget. The epoch rolls, and `epoch_rolled` is emitted,
//! on the first accrual after a boundary rather than at the boundary itself,
//! since a contract cannot act at a ledger height on its own. An epoch with
//! no accruals therefore never emits an event; `get_current_epoch()` always
//! reports the epoch in force at the current ledger.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! instance storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultFeature6Error;
use crate::vault::VaultContractClient;
use crate::VaultContract;

const CONFIG_KEY: Symbol = symbol_short!("ep_cfg");
const STATE_KEY: Symbol = symbol_short!("ep_state");

/// Epoch length and emission budget.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EpochConfig {
    /// Length of an epoch, in ledgers.
    pub length_ledgers: u32,
    /// Maximum reward that may accrue in one epoch.
    pub budget_per_epoch: i128,
}

/// The epoch in force and how much it has emitted.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EpochState {
    /// Epoch id, starting at 0 when the config is first set.
    pub id: u32,
    /// Ledger at which the epoch started.
    pub start: u32,
    /// Reward accrued so far in the epoch.
    pub emitted: i128,
}

fn get_config(env: &Env) -> Option<EpochConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

fn get_state(env: &Env) -> Option<EpochState> {
    env.storage().instance().get(&STATE_KEY)
}

/// `state` advanced to the epoch containing ledger `now`, and whether it moved.
/// Skipped epochs (no accruals for a whole epoch or more) are counted in `id`.
fn rolled(config: &EpochConfig, state: &EpochState, now: u32) -> (EpochState, bool) {
    let elapsed = now.saturating_sub(state.start);
    if elapsed < config.length_ledgers {
        return (state.clone(), false);
    }
    let epochs = elapsed / config.length_ledgers;
    (
        EpochState {
            id: state.id.saturating_add(epochs),
            start: state
                .start
                .saturating_add(epochs.saturating_mul(config.length_ledgers)),
            emitted: 0,
        },
        true,
    )
}

/// Clamp a reward accrual of `amount` to what is left of the current epoch's
/// budget, record it as emitted, and return the amount allowed. Returns
/// `amount` unchanged when no epoch config is set.
pub fn apply_budget(env: &Env, amount: i128) -> i128 {
    let (config, stored) = match (get_config(env), get_state(env)) {
        (Some(config), Some(stored)) => (config, stored),
        _ => return amount,
    };
    let (mut state, did_roll) = rolled(&config, &stored, env.ledger().sequence());
    if did_roll {
        env.events().publish(
            (Symbol::new(env, "epoch_rolled"),),
            (stored.id, state.id, stored.emitted, config.budget_per_epoch),
        );
    }
    let allowed = amount.min((config.budget_per_epoch - state.emitted).max(0));
    state.emitted += allowed;
    env.storage().instance().set(&STATE_KEY, &state);
    allowed
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Sets the epoch length and per-epoch emission budget, and starts a new
    /// epoch at the current ledger. Unspent budget is discarded at each epoch
    /// boundary (see the module docs). The epoch id keeps counting up if a
    /// config already existed.
    ///
    /// # Authorization
    /// `admin` must sign and be the pool admin.
    ///
    /// # Errors
    /// - `InvalidEpochConfig`: `length_ledgers == 0` or `budget_per_epoch < 0`.
    pub fn set_epoch_config(
        env: Env,
        admin: Address,
        length_ledgers: u32,
        budget_per_epoch: i128,
    ) -> Result<(), VaultFeature6Error> {
        admin::require_admin_as(&env, &admin)?;
        if length_ledgers == 0 || budget_per_epoch < 0 {
            return Err(VaultFeature6Error::InvalidEpochConfig);
        }
        let id = get_state(&env).map(|s| s.id.saturating_add(1)).unwrap_or(0);
        env.storage().instance().set(
            &CONFIG_KEY,
            &EpochConfig {
                length_ledgers,
                budget_per_epoch,
            },
        );
        env.storage().instance().set(
            &STATE_KEY,
            &EpochState {
                id,
                start: env.ledger().sequence(),
                emitted: 0,
            },
        );
        Ok(())
    }

    /// The epoch in force at the current ledger as
    /// `(id, ends_at, emitted, budget)`, where `ends_at` is the ledger at
    /// which the next epoch starts. All zeros when no epoch config is set.
    pub fn get_current_epoch(env: Env) -> (u32, u32, i128, i128) {
        match (get_config(&env), get_state(&env)) {
            (Some(config), Some(state)) => {
                let (state, _) = rolled(&config, &state, env.ledger().sequence());
                (
                    state.id,
                    state.start.saturating_add(config.length_ledgers),
                    state.emitted,
                    config.budget_per_epoch,
                )
            }
            _ => (0, 0, 0, 0),
        }
    }
}
