//! Reward claim receipts for accounting and tax export.
//!
//! Every paid claim gets a sequential, per-user `claim_id` and a structured
//! `claim_receipt` event, so off-chain tooling can reconstruct a user's full
//! reward-claim history unambiguously from the event log alone, without
//! having to disambiguate same-ledger claims by transaction order.
//!
//! Intentionally events-only: no per-receipt data is kept in contract
//! storage, only the monotonic counter needed to assign the next id.
//!
//! # Event shape
//! Topics: `(Symbol("clm_rcpt"), user: Address)` — matching this crate's
//! existing convention (e.g. `events::claimed`) of keying events by the
//! subject address in the topics.
//! Data: `(claim_id: u32, amount: i128, reward_token: Address, ledger: u32,
//! destination: Address)`.
//!
//! # Storage
//! `DataKey` sits at Soroban's 50-variant cap, so the per-user counter uses
//! a raw `Symbol`-tuple key, matching `balance.rs`/`position_heartbeat.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::vault::VaultContract;

const CLAIM_COUNT_KEY: Symbol = symbol_short!("clm_cnt");

fn get_count(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&(CLAIM_COUNT_KEY, user.clone()))
        .unwrap_or(0)
}

fn set_count(env: &Env, user: &Address, count: u32) {
    env.storage()
        .persistent()
        .set(&(CLAIM_COUNT_KEY, user.clone()), &count);
}

/// Allocates the next monotonic `claim_id` for `user` (1-based: the first
/// receipt is id `1`), emits the `claim_receipt` event, and persists the
/// new count. Call exactly once per *paid* claim — after the final payout
/// `amount`, `reward_token`, and `destination` are known — from each
/// full/partial claim entrypoint. Returns the assigned `claim_id`.
pub fn record_receipt(
    env: &Env,
    user: &Address,
    amount: i128,
    reward_token: &Address,
    destination: &Address,
) -> u32 {
    let claim_id = get_count(env, user).saturating_add(1);
    set_count(env, user, claim_id);

    env.events().publish(
        (symbol_short!("clm_rcpt"), user.clone()),
        (
            claim_id,
            amount,
            reward_token.clone(),
            env.ledger().sequence(),
            destination.clone(),
        ),
    );

    claim_id
}

#[contractimpl]
impl VaultContract {
    /// Read-only: the number of claim receipts issued to `user` so far
    /// (equivalently, the `claim_id` of their most recent claim, or `0` if
    /// they have never been paid a claim).
    pub fn get_claim_count(env: Env, user: Address) -> u32 {
        crate::claim_receipts::get_count(&env, &user)
    }
}
