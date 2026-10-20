//! Burns, fees, buybacks, and slash accounting storage accessors.
//!
//! Handles burned fee accumulation, unstake fee reserves, penalty bps,
//! slashing treasury, flash stake fees, and withdrawal receipts.

use super::positions::assert_non_negative;
use crate::storage::{DataKey, FeeRecipient, FlashStakeReceipt, WithdrawalReceipt};
use soroban_sdk::{symbol_short, Address, Env, Vec};

pub fn get_fees_burned(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("fee_brnd"))
        .unwrap_or(0)
}

pub fn set_fees_burned(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&symbol_short!("fee_brnd"), &amount);
}

pub fn add_fees_burned(env: &Env, amount: i128) {
    let current = get_fees_burned(env);
    set_fees_burned(env, current.saturating_add(amount));
}

pub fn fee_buyback_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&symbol_short!("fee_bybk"))
        .unwrap_or(false)
}

pub fn set_fee_buyback_enabled(env: &Env, enabled: bool) {
    env.storage()
        .instance()
        .set(&symbol_short!("fee_bybk"), &enabled);
}

pub fn get_unstake_fee_reserve(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("fee_rsrv"))
        .unwrap_or(0)
}

pub fn set_unstake_fee_reserve(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&symbol_short!("fee_rsrv"), &amount);
}

pub fn add_unstake_fee_reserve(env: &Env, amount: i128) {
    let current = get_unstake_fee_reserve(env);
    set_unstake_fee_reserve(env, current.saturating_add(amount));
}

pub fn get_unstake_fee_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::UnstakeFeeBps)
        .unwrap_or(0)
}

pub fn set_unstake_fee_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&DataKey::UnstakeFeeBps, &bps);
}

pub fn get_early_exit_penalty_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::EarlyExitPenaltyBps)
        .unwrap_or(0)
}

pub fn set_early_exit_penalty_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&DataKey::EarlyExitPenaltyBps, &bps);
}

pub fn get_slash_treasury(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("slash_tr"))
}

pub fn set_slash_treasury(env: &Env, treasury: &Address) {
    env.storage()
        .instance()
        .set(&symbol_short!("slash_tr"), treasury);
}

pub fn get_fee_recipients(env: &Env) -> Vec<FeeRecipient> {
    env.storage()
        .instance()
        .get(&symbol_short!("fee_rcpt"))
        .unwrap_or(Vec::new(env))
}

pub fn set_fee_recipients(env: &Env, recipients: &Vec<FeeRecipient>) {
    env.storage()
        .instance()
        .set(&symbol_short!("fee_rcpt"), recipients);
}

pub fn get_lot_size(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("lot_size"))
        .unwrap_or(0)
}

pub fn set_lot_size(env: &Env, lot_size: i128) {
    assert_non_negative(env, lot_size);
    env.storage()
        .instance()
        .set(&symbol_short!("lot_size"), &lot_size);
}

pub fn get_flash_stake_fee_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("fl_fee"))
        .unwrap_or(0)
}

pub fn set_flash_stake_fee_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("fl_fee"), &bps);
}

pub fn next_flash_receipt_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .instance()
        .get(&symbol_short!("fl_nid"))
        .unwrap_or(0);
    let next = current.saturating_add(1);
    env.storage().instance().set(&symbol_short!("fl_nid"), &next);
    next
}

pub fn get_flash_receipt(env: &Env, receipt_id: u64) -> Option<FlashStakeReceipt> {
    let key = (symbol_short!("fl_rcpt"), receipt_id);
    env.storage().persistent().get(&key)
}

pub fn set_flash_receipt(env: &Env, receipt_id: u64, receipt: &FlashStakeReceipt) {
    let key = (symbol_short!("fl_rcpt"), receipt_id);
    env.storage().persistent().set(&key, receipt);
}

pub fn get_unstake_insurance_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("uns_ins"))
        .unwrap_or(0)
}

pub fn set_unstake_insurance_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("uns_ins"), &bps);
}

pub fn get_withdrawal_receipt_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&symbol_short!("wrc_cnt"))
        .unwrap_or(0)
}

pub fn set_withdrawal_receipt_counter(env: &Env, counter: u64) {
    env.storage()
        .instance()
        .set(&symbol_short!("wrc_cnt"), &counter);
}

pub fn get_withdrawal_receipt(env: &Env, receipt_id: u64) -> Option<WithdrawalReceipt> {
    let key = (symbol_short!("w_rcpt"), receipt_id);
    env.storage().persistent().get(&key)
}

pub fn set_withdrawal_receipt(env: &Env, receipt: &WithdrawalReceipt) {
    let key = (symbol_short!("w_rcpt"), receipt.receipt_id);
    env.storage().persistent().set(&key, receipt);
}

pub fn get_user_receipt_ids(env: &Env, user: &Address) -> Vec<u64> {
    let key = (symbol_short!("u_rcpt"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_user_receipt_ids(env: &Env, user: &Address, ids: &Vec<u64>) {
    let key = (symbol_short!("u_rcpt"), user.clone());
    env.storage().persistent().set(&key, ids);
}
