//! Slash disputes ΓÇö lets a slashed staker challenge an admin slash on-chain,
//! triggering a stake-weighted community vote to overturn or uphold it
//! (issue #336). Builds on issue #20 (the slash mechanism).
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this module uses raw
//! `Symbol`-keyed storage, matching `balance.rs`.
//!
//! # Known gap ΓÇö please read before wiring this up
//!
//! `slash()` (issue #20) is currently missing from `src/vault.rs` on `main`
//! ΓÇö most of that file past `join_waitlist` is gone due to an unrelated bad
//! merge (see PR description). That has real consequences for what this
//! module can honestly do:
//!
//! - [`record_slash`] only creates the bookkeeping `SlashRecord` a dispute
//!   needs (slash id, disputer-eligible user, amount, deadline). It does
//!   **not** deduct the user's stake or shares ΓÇö that's `slash()`'s job.
//!   Once `slash()` is restored, it should call
//!   `slash_dispute::record_slash(&env, &user, slash_id, amount)?` after
//!   doing its own share deduction, and should **defer** sending the
//!   slashed value to the treasury (`balance::get_slash_treasury()`) until
//!   a dispute either resolves as upheld or the dispute window passes
//!   untouched ΓÇö otherwise "funds held in escrow during the dispute
//!   window" (an explicit acceptance criterion) isn't actually true.
//! - [`VaultContract::resolve_dispute`] transfers the disputed `amount`
//!   from the contract's own token balance to the slash treasury on an
//!   uphold outcome ΓÇö a real transfer, since staked tokens already sit in
//!   the contract regardless of whether `slash()` exists. On an overturn
//!   outcome it does **not** restore anything to the user's position,
//!   because nothing was deducted from it here in the first place (that
//!   would be `slash()`'s doing, once restored); it only clears the
//!   dispute and emits the outcome.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Ledgers a slashed user has to file a dispute after being slashed.
const DEFAULT_DISPUTE_WINDOW: u32 = 50_000;

/// Most disputes that may be open pool-wide at once.
pub const MAX_OPEN_DISPUTES: u32 = 5;

/// Instance-storage key for the configured dispute filing window, in ledgers.
const WINDOW_KEY: Symbol = symbol_short!("sd_win");

/// Instance-storage key for the next dispute id to assign.
const NEXT_DISPUTE_ID_KEY: Symbol = symbol_short!("sd_next");

/// Instance-storage key for the count of currently-open disputes.
const OPEN_COUNT_KEY: Symbol = symbol_short!("sd_open");

/// Persistent-storage key prefix for a slash record. Keyed by
/// `(SLASH_RECORD_KEY, slash_id)`.
const SLASH_RECORD_KEY: Symbol = symbol_short!("sd_slash");

/// Persistent-storage key prefix for a dispute. Keyed by
/// `(DISPUTE_KEY, dispute_id)`.
const DISPUTE_KEY: Symbol = symbol_short!("sd_disp");

/// Persistent-storage key prefix mapping a slash id to its open dispute id
/// (if any). Keyed by `(SLASH_TO_DISPUTE_KEY, slash_id)`.
const SLASH_TO_DISPUTE_KEY: Symbol = symbol_short!("sd_s2d");

/// Persistent-storage key prefix recording that a voter has voted on a
/// dispute. Keyed by `(VOTED_KEY, dispute_id, voter)`.
const VOTED_KEY: Symbol = symbol_short!("sd_voted");

/// A single slash event, recorded so it can be disputed.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SlashRecord {
    pub slash_id: u32,
    pub user: Address,
    pub amount: i128,
    pub slashed_at: u32,
}

/// A filed dispute against a `SlashRecord`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SlashDispute {
    pub slash_id: u32,
    pub disputer: Address,
    pub reason: String,
    pub votes_overturn: i128,
    pub votes_uphold: i128,
    pub deadline: u32,
    pub resolved: bool,
}

fn get_window(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&WINDOW_KEY)
        .unwrap_or(DEFAULT_DISPUTE_WINDOW)
}

fn load_slash_record(env: &Env, slash_id: u32) -> Option<SlashRecord> {
    env.storage().persistent().get(&(SLASH_RECORD_KEY, slash_id))
}

fn load_dispute(env: &Env, dispute_id: u32) -> Option<SlashDispute> {
    env.storage().persistent().get(&(DISPUTE_KEY, dispute_id))
}

fn set_dispute(env: &Env, dispute_id: u32, dispute: &SlashDispute) {
    env.storage()
        .persistent()
        .set(&(DISPUTE_KEY, dispute_id), dispute);
}

fn get_open_count(env: &Env) -> u32 {
    env.storage().instance().get(&OPEN_COUNT_KEY).unwrap_or(0)
}

/// User's current staked token amount, for use as vote weight. Mirrors
/// `content_curation::get_position_amount`.
fn position_amount(env: &Env, user: &Address) -> Option<i128> {
    let shares = balance::get_shares(env, user);
    if shares == 0 {
        return None;
    }
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    balance::shares_to_amount(total_shares, total_deposited, shares)
}

/// Record that `user` was slashed `amount` under `slash_id`, at the current
/// ledger ΓÇö making it eligible for `dispute_slash`. Admin only (checked by
/// the caller ΓÇö see `VaultContract::record_slash`). See the module-level
/// "Known gap" note: this performs no fund movement of its own.
pub fn record_slash(env: &Env, user: &Address, slash_id: u32, amount: i128) -> Result<(), VaultError> {
    if amount <= 0 {
        return Err(VaultError::ZeroAmount);
    }
    if load_slash_record(env, slash_id).is_some() {
        return Err(VaultError::AlreadyInitialized);
    }

    let record = SlashRecord {
        slash_id,
        user: user.clone(),
        amount,
        slashed_at: env.ledger().sequence(),
    };
    env.storage()
        .persistent()
        .set(&(SLASH_RECORD_KEY, slash_id), &record);
    Ok(())
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Record that `user` was slashed `amount` under `slash_id`, making it
    /// eligible for `dispute_slash`. Admin only.
    ///
    /// Exposed as its own entrypoint (rather than only the internal
    /// `slash_dispute::record_slash` helper) so a slash can be staged for
    /// disputing right now, since `slash()` itself doesn't currently exist
    /// on `main` to call it directly ΓÇö see the module-level "Known gap"
    /// note in `slash_dispute.rs`.
    pub fn record_slash(env: Env, user: Address, slash_id: u32, amount: i128) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        record_slash(&env, &user, slash_id, amount)
    }

    /// Configure the dispute filing window, in ledgers after a slash. Admin
    /// only.
    pub fn set_slash_dispute_window(env: Env, ledgers: u32) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        env.storage().instance().set(&WINDOW_KEY, &ledgers);

        env.events().publish(
            (symbol_short!("sd_wset"),),
            (ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// File a dispute against slash `slash_id`. Callable only by the user
    /// that slash actually targeted, within the configured dispute window.
    pub fn dispute_slash(env: Env, user: Address, slash_id: u32, reason: String) -> Result<u32, VaultError> {
        user.require_auth();

        let record = load_slash_record(&env, slash_id).ok_or(VaultError::PositionNotFound)?;
        if record.user != user {
            return Err(VaultError::Unauthorized);
        }

        let window = get_window(&env);
        if env.ledger().sequence() > record.slashed_at.saturating_add(window) {
            return Err(VaultError::EpochNotFinalized);
        }

        if env
            .storage()
            .persistent()
            .has(&(SLASH_TO_DISPUTE_KEY, slash_id))
        {
            // Already disputed ΓÇö one open dispute per slash.
            return Err(VaultError::AlreadyInitialized);
        }

        if get_open_count(&env) >= MAX_OPEN_DISPUTES {
            return Err(VaultError::TooManyStakers);
        }

        let dispute_id: u32 = env.storage().instance().get(&NEXT_DISPUTE_ID_KEY).unwrap_or(0);
        env.storage()
            .instance()
            .set(&NEXT_DISPUTE_ID_KEY, &(dispute_id + 1));
        env.storage()
            .instance()
            .set(&OPEN_COUNT_KEY, &(get_open_count(&env) + 1));

        let dispute = SlashDispute {
            slash_id,
            disputer: user.clone(),
            reason,
            votes_overturn: 0,
            votes_uphold: 0,
            deadline: env.ledger().sequence().saturating_add(window),
            resolved: false,
        };
        set_dispute(&env, dispute_id, &dispute);
        env.storage()
            .persistent()
            .set(&(SLASH_TO_DISPUTE_KEY, slash_id), &dispute_id);

        env.events().publish(
            (symbol_short!("sd_filed"), user),
            (slash_id, dispute_id, env.ledger().sequence()),
        );
        Ok(dispute_id)
    }

    /// Vote to overturn or uphold an open dispute. Vote weight equals the
    /// voter's current staked amount. One vote per address per dispute.
    pub fn vote_on_dispute(env: Env, voter: Address, dispute_id: u32, overturn: bool) -> Result<(), VaultError> {
        voter.require_auth();

        let mut dispute = load_dispute(&env, dispute_id).ok_or(VaultError::PositionNotFound)?;
        if dispute.resolved {
            return Err(VaultError::AlreadyInitialized);
        }
        if env.ledger().sequence() > dispute.deadline {
            return Err(VaultError::EpochNotFinalized);
        }

        let weight = position_amount(&env, &voter).ok_or(VaultError::TooManyStakers)?;

        let voted_key = (VOTED_KEY, dispute_id, voter.clone());
        if env.storage().persistent().has(&voted_key) {
            return Err(VaultError::TooManyStakers);
        }
        env.storage().persistent().set(&voted_key, &true);

        if overturn {
            dispute.votes_overturn = dispute
                .votes_overturn
                .checked_add(weight)
                .ok_or(VaultError::ArithmeticError)?;
        } else {
            dispute.votes_uphold = dispute
                .votes_uphold
                .checked_add(weight)
                .ok_or(VaultError::ArithmeticError)?;
        }
        set_dispute(&env, dispute_id, &dispute);

        env.events().publish(
            (symbol_short!("sd_vote"), voter),
            (dispute_id, overturn, weight, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Resolve a dispute after its voting deadline. Anyone may call this ΓÇö
    /// it just tallies the already-cast votes. On an overturn outcome
    /// (`votes_overturn > votes_uphold`), no funds move ΓÇö see the
    /// module-level "Known gap" note for why. On an uphold outcome, the
    /// disputed amount is transferred from the contract to the slash
    /// treasury, if one is configured.
    pub fn resolve_dispute(env: Env, dispute_id: u32) -> Result<(), VaultError> {
        let mut dispute = load_dispute(&env, dispute_id).ok_or(VaultError::PositionNotFound)?;
        if dispute.resolved {
            return Err(VaultError::AlreadyInitialized);
        }
        if env.ledger().sequence() <= dispute.deadline {
            return Err(VaultError::EpochNotFinalized);
        }

        let record = load_slash_record(&env, dispute.slash_id).ok_or(VaultError::PositionNotFound)?;

        let overturned = dispute.votes_overturn > dispute.votes_uphold;

        if !overturned {
            if let Some(treasury) = balance::get_slash_treasury(&env) {
                let token_addr: Address = env
                    .storage()
                    .instance()
                    .get(&crate::storage::DataKey::Token)
                    .ok_or(VaultError::NotInitialized)?;
                let token_client = soroban_sdk::token::Client::new(&env, &token_addr);
                token_client.transfer(&env.current_contract_address(), &treasury, &record.amount);
            }
        }

        dispute.resolved = true;
        set_dispute(&env, dispute_id, &dispute);
        env.storage()
            .instance()
            .set(&OPEN_COUNT_KEY, &get_open_count(&env).saturating_sub(1));
        env.storage()
            .persistent()
            .remove(&(SLASH_TO_DISPUTE_KEY, dispute.slash_id));

        env.events().publish(
            (symbol_short!("sd_res"),),
            (
                dispute_id,
                overturned,
                dispute.votes_overturn,
                dispute.votes_uphold,
                env.ledger().sequence(),
            ),
        );
        Ok(())
    }

    /// Read-only query: a dispute's current state.
    pub fn get_dispute(env: Env, dispute_id: u32) -> Option<SlashDispute> {
        load_dispute(&env, dispute_id)
    }

    /// Read-only query: a slash record, if one was ever filed via
    /// `record_slash`.
    pub fn get_slash_record(env: Env, slash_id: u32) -> Option<SlashRecord> {
        load_slash_record(&env, slash_id)
    }
}
















