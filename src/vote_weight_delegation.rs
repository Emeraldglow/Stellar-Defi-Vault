//! Vote-weight delegation (issue #519).
//!
//! Lets a staker delegate their governance vote weight (as read by
//! `vault.rs`'s `vote_weight_at`) to another address without moving their
//! staked funds — the standard "delegate your vote, keep your tokens"
//! governance pattern.
//!
//! A delegator may have at most one active delegate at a time; delegating
//! again replaces the previous delegate. A delegate accumulates a bounded
//! list of delegators ([`MAX_DELEGATORS_PER_DELEGATE`]) so
//! `effective_vote_weight_at` can sum their weight in one bounded pass.
//!
//! # Scope note
//!
//! Delegation here is a single flat hop (no re-delegation chains — a
//! delegate's *own* effective weight is still just their own stake plus
//! their direct delegators; it does not further include anyone who delegated
//! to one of their delegators). This module also does not timestamp when a
//! delegation started, so `effective_vote_weight_at(user, ledger)` applies
//! the *current* delegation graph to *historical* per-address weight
//! snapshots from `vote_weight_at` — it cannot reconstruct what the
//! delegation graph itself looked like at `ledger` in the past.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol, Vec};

use crate::errors::VaultFeature3Error;
use crate::vault::VaultContract;

/// Most delegators a single delegate may accumulate.
pub const MAX_DELEGATORS_PER_DELEGATE: u32 = 50;

/// Persistent key prefix: a delegator's chosen delegate. Keyed by
/// `(DELEGATE_KEY, delegator)`.
const DELEGATE_KEY: Symbol = symbol_short!("vwd_dele");
/// Persistent key prefix: a delegate's list of delegators. Keyed by
/// `(DELEGATORS_KEY, delegate)`.
const DELEGATORS_KEY: Symbol = symbol_short!("vwd_drs");

fn get_delegate(env: &Env, delegator: &Address) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&(DELEGATE_KEY, delegator.clone()))
}

fn set_delegate(env: &Env, delegator: &Address, delegate: &Address) {
    env.storage()
        .persistent()
        .set(&(DELEGATE_KEY, delegator.clone()), delegate);
}

fn remove_delegate(env: &Env, delegator: &Address) {
    env.storage()
        .persistent()
        .remove(&(DELEGATE_KEY, delegator.clone()));
}

fn get_delegators(env: &Env, delegate: &Address) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&(DELEGATORS_KEY, delegate.clone()))
        .unwrap_or(Vec::new(env))
}

fn set_delegators(env: &Env, delegate: &Address, delegators: &Vec<Address>) {
    env.storage()
        .persistent()
        .set(&(DELEGATORS_KEY, delegate.clone()), delegators);
}

fn remove_from_delegators(env: &Env, delegate: &Address, delegator: &Address) {
    let list = get_delegators(env, delegate);
    let mut updated = Vec::new(env);
    for entry in list.iter() {
        if &entry != delegator {
            updated.push_back(entry);
        }
    }
    set_delegators(env, delegate, &updated);
}

#[contractimpl]
impl VaultContract {
    /// Delegate the caller's governance vote weight to `delegate`. Replaces
    /// any existing delegation. Does not move staked funds.
    pub fn delegate_vote_weight(
        env: Env,
        delegator: Address,
        delegate: Address,
    ) -> Result<(), VaultFeature3Error> {
        delegator.require_auth();

        if delegator == delegate {
            return Err(VaultFeature3Error::SelfDelegationNotAllowed);
        }

        if let Some(existing) = crate::vote_weight_delegation::get_delegate(&env, &delegator) {
            if existing == delegate {
                return Ok(());
            }
            crate::vote_weight_delegation::remove_from_delegators(&env, &existing, &delegator);
        }

        let mut new_delegators =
            crate::vote_weight_delegation::get_delegators(&env, &delegate);
        if new_delegators.len() >= MAX_DELEGATORS_PER_DELEGATE {
            return Err(VaultFeature3Error::TooManyDelegators);
        }
        new_delegators.push_back(delegator.clone());
        crate::vote_weight_delegation::set_delegators(&env, &delegate, &new_delegators);
        crate::vote_weight_delegation::set_delegate(&env, &delegator, &delegate);

        env.events().publish(
            (symbol_short!("vwd_set"), delegator),
            (delegate, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Revoke the caller's active delegation, if any.
    pub fn revoke_vote_delegation(env: Env, delegator: Address) -> Result<(), VaultFeature3Error> {
        delegator.require_auth();

        let existing = crate::vote_weight_delegation::get_delegate(&env, &delegator)
            .ok_or(VaultFeature3Error::NoDelegationSet)?;
        crate::vote_weight_delegation::remove_from_delegators(&env, &existing, &delegator);
        crate::vote_weight_delegation::remove_delegate(&env, &delegator);

        env.events().publish(
            (symbol_short!("vwd_rvk"), delegator),
            (existing, env.ledger().sequence()),
        );
        Ok(())
    }

    /// The address `delegator` currently delegates to, if any.
    pub fn get_vote_delegate(env: Env, delegator: Address) -> Option<Address> {
        crate::vote_weight_delegation::get_delegate(&env, &delegator)
    }

    /// The addresses currently delegating their vote weight to `delegate`.
    pub fn get_vote_delegators(env: Env, delegate: Address) -> Vec<Address> {
        crate::vote_weight_delegation::get_delegators(&env, &delegate)
    }

    /// `user`'s effective governance vote weight at `ledger`: their own
    /// historical weight (zero if they have delegated it away) plus the
    /// historical weight of everyone currently delegating to them. See the
    /// module doc for the scope note on applying the current delegation
    /// graph to historical weight.
    pub fn effective_vote_weight_at(
        env: Env,
        user: Address,
        ledger: u32,
    ) -> Result<i128, VaultFeature3Error> {
        let own_weight = if crate::vote_weight_delegation::get_delegate(&env, &user).is_some() {
            0
        } else {
            VaultContract::vote_weight_at(env.clone(), user.clone(), ledger)?
        };

        let mut total = own_weight;
        let delegators = crate::vote_weight_delegation::get_delegators(&env, &user);
        for delegator in delegators.iter() {
            let weight = VaultContract::vote_weight_at(env.clone(), delegator, ledger)?;
            total = total
                .checked_add(weight)
                .ok_or(VaultFeature3Error::ArithmeticError)?;
        }
        Ok(total)
    }
}
