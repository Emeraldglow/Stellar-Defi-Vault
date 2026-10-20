//! Reward token audit trail (issue #467).
//!
//! Adds a detailed audit trail specifically for reward token flows — every
//! reward payment, buyback, treasury withdrawal, insurance payout, and top-up
//! recorded with full context including source, destination, amount, and
//! triggering function. Enables complete financial reconciliation of the
//! reward token reserve.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! persistent storage, matching `balance.rs` and other feature modules.
//!
//! Log is append-only and never truncated. Storage keys:
//! - Total count: `symbol_short!("aud_cnt")` -> `u64`
//! - Summary: `symbol_short!("aud_sum")` -> `(total_paid: i128, total_burned: i128, total_withdrawn: i128, total_topped_up: i128)`
//! - Paginated pages: `(symbol_short!("aud_pg"), page: u32)` -> `Vec<RewardTokenMovement>` (100 entries per page)

use soroban_sdk::{contracttype, symbol_short, Address, Env, String, Symbol, Vec};

const AUDIT_COUNT_KEY: Symbol = symbol_short!("aud_cnt");
const AUDIT_SUMMARY_KEY: Symbol = symbol_short!("aud_sum");
const AUDIT_PAGE_KEY: Symbol = symbol_short!("aud_pg");

/// 100 entries per page in persistent storage.
pub const AUDIT_PAGE_SIZE: u32 = 100;

/// Movement types for reward token audit trail.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum MovementType {
    RewardPaid,
    BuybackBurn,
    TreasuryWithdraw,
    InsurancePayout,
    ReserveTopUp,
    DeferredClaim,
    MutualInsurance,
}

/// Recorded context of a reward token movement.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RewardTokenMovement {
    pub movement_id: u64,
    pub movement_type: MovementType,
    pub from: Address,
    pub to: Address,
    pub amount: i128,
    pub triggered_by: String,
    pub ledger: u32,
}

/// Read total number of movements logged.
pub fn get_audit_log_count(env: &Env) -> u64 {
    env.storage().persistent().get(&AUDIT_COUNT_KEY).unwrap_or(0)
}

/// Read financial summary: (total_paid, total_burned, total_withdrawn, total_topped_up).
pub fn get_audit_log_summary(env: &Env) -> (i128, i128, i128, i128) {
    env.storage()
        .persistent()
        .get(&AUDIT_SUMMARY_KEY)
        .unwrap_or((0, 0, 0, 0))
}

/// Read a specific 0-indexed page of audit log entries (up to 100 entries).
pub fn get_audit_log_page(env: &Env, page: u32) -> Vec<RewardTokenMovement> {
    env.storage()
        .persistent()
        .get(&(AUDIT_PAGE_KEY, page))
        .unwrap_or_else(|| Vec::new(env))
}

/// Internal helper called to record every reward token movement with full context.
pub fn log_reward_movement(
    env: &Env,
    movement_type: MovementType,
    from: Address,
    to: Address,
    amount: i128,
    triggered_by: String,
) -> u64 {
    let current_count = get_audit_log_count(env);
    let movement_id = current_count.saturating_add(1);
    let ledger = env.ledger().sequence();

    let movement = RewardTokenMovement {
        movement_id,
        movement_type: movement_type.clone(),
        from: from.clone(),
        to: to.clone(),
        amount,
        triggered_by: triggered_by.clone(),
        ledger,
    };

    // 0-indexed page: movement 1..100 -> page 0; 101..200 -> page 1, etc.
    let page = ((movement_id - 1) / (AUDIT_PAGE_SIZE as u64)) as u32;
    let mut page_entries = get_audit_log_page(env, page);
    page_entries.push_back(movement);
    env.storage().persistent().set(&(AUDIT_PAGE_KEY, page), &page_entries);
    env.storage().persistent().set(&AUDIT_COUNT_KEY, &movement_id);

    // Update cumulative summary
    let (mut total_paid, mut total_burned, mut total_withdrawn, mut total_topped_up) =
        get_audit_log_summary(env);

    match movement_type {
        MovementType::RewardPaid | MovementType::DeferredClaim => {
            total_paid = total_paid.saturating_add(amount);
        }
        MovementType::BuybackBurn => {
            total_burned = total_burned.saturating_add(amount);
        }
        MovementType::TreasuryWithdraw => {
            total_withdrawn = total_withdrawn.saturating_add(amount);
        }
        MovementType::ReserveTopUp => {
            total_topped_up = total_topped_up.saturating_add(amount);
        }
        MovementType::InsurancePayout | MovementType::MutualInsurance => {
            // Logged as movements without mutating core summary buckets
        }
    }

    env.storage().persistent().set(
        &AUDIT_SUMMARY_KEY,
        &(total_paid, total_burned, total_withdrawn, total_topped_up),
    );

    movement_id
}
