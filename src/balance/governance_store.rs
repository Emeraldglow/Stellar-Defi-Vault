//! Governance storage accessors.
//!
//! Handles proposals, voting weights, delegates, veto thresholds, and pending actions.

use crate::storage::{AdminProposal, GovernanceProposal, PendingAction};
use soroban_sdk::{symbol_short, Address, Env, Symbol, Vec};

// ── Vote weight mode ──────────────────────────────────────────────────────────

/// Pool-wide vote weight mode (snapshotted per proposal at creation time).
pub fn get_vote_weight_mode(env: &Env) -> VoteWeightMode {
    env.storage()
        .instance()
        .get(&symbol_short!("vw_mode"))
        .unwrap_or(VoteWeightMode::Linear)
}

pub fn set_vote_weight_mode(env: &Env, mode: VoteWeightMode) {
    env.storage().instance().set(&symbol_short!("vw_mode"), &mode);
}

// ── Proposal Vec store ────────────────────────────────────────────────────────

pub fn get_admin_proposals(env: &Env) -> Vec<AdminProposal> {
    env.storage()
        .instance()
        .get(&symbol_short!("adm_prop"))
        .unwrap_or(Vec::new(env))
}

pub fn set_admin_proposals(env: &Env, proposals: &Vec<AdminProposal>) {
    env.storage()
        .instance()
        .set(&symbol_short!("adm_prop"), proposals);
}

pub fn get_governance_proposals(env: &Env) -> Vec<GovernanceProposal> {
    env.storage()
        .instance()
        .get(&symbol_short!("gov_prop"))
        .unwrap_or(Vec::new(env))
}

pub fn set_governance_proposals(env: &Env, proposals: &Vec<GovernanceProposal>) {
    env.storage()
        .instance()
        .set(&symbol_short!("gov_prop"), proposals);
}

pub fn get_veto_threshold_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("veto_bps"))
        .unwrap_or(0)
}

pub fn set_veto_threshold_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("veto_bps"), &bps);
}

pub fn get_proposal_vetoer(env: &Env, proposal_id: u32) -> Option<Address> {
    let key = (symbol_short!("prop_vtr"), proposal_id);
    env.storage().instance().get(&key)
}

pub fn set_proposal_vetoer(env: &Env, proposal_id: u32, vetoer: &Address) {
    let key = (symbol_short!("prop_vtr"), proposal_id);
    env.storage().instance().set(&key, vetoer);
}

pub fn get_vote_delegate(env: &Env, user: &Address) -> Option<Address> {
    let key = (symbol_short!("v_del"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_vote_delegate(env: &Env, user: &Address, delegate: &Address) {
    let key = (symbol_short!("v_del"), user.clone());
    env.storage().persistent().set(&key, delegate);
}

pub fn remove_vote_delegate(env: &Env, user: &Address) {
    let key = (symbol_short!("v_del"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_delegated_weight_snapshot(env: &Env, user: &Address) -> i128 {
    let key = (symbol_short!("dw_snap"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_delegated_weight_snapshot(env: &Env, user: &Address, weight: i128) {
    let key = (symbol_short!("dw_snap"), user.clone());
    env.storage().persistent().set(&key, &weight);
}

pub fn remove_delegated_weight_snapshot(env: &Env, user: &Address) {
    let key = (symbol_short!("dw_snap"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_delegated_vote_weight(env: &Env, delegate: &Address) -> i128 {
    let key = (symbol_short!("d_v_wt"), delegate.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_delegated_vote_weight(env: &Env, delegate: &Address, weight: i128) {
    let key = (symbol_short!("d_v_wt"), delegate.clone());
    env.storage().persistent().set(&key, &weight);
}

pub fn get_pending_actions(env: &Env) -> Vec<PendingAction> {
    env.storage()
        .instance()
        .get(&symbol_short!("p_action"))
        .unwrap_or(Vec::new(env))
}

pub fn set_pending_actions(env: &Env, actions: &Vec<PendingAction>) {
    env.storage()
        .instance()
        .set(&symbol_short!("p_action"), actions);
}

pub fn get_contract_delegate(
    env: &Env,
    user: &Address,
    contract: &Address,
) -> Option<crate::storage::ContractDelegate> {
    let key = (symbol_short!("cdel"), user.clone(), contract.clone());
    env.storage().persistent().get(&key)
}

pub fn set_contract_delegate(
    env: &Env,
    user: &Address,
    contract: &Address,
    delegate: &crate::storage::ContractDelegate,
) {
    let key = (symbol_short!("cdel"), user.clone(), contract.clone());
    env.storage().persistent().set(&key, delegate);
}

pub fn remove_contract_delegate(env: &Env, user: &Address, contract: &Address) {
    let key = (symbol_short!("cdel"), user.clone(), contract.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_emergency_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&symbol_short!("emg_adm"))
}

pub fn set_emergency_admin(env: &Env, admin: &Address) {
    env.storage()
        .instance()
        .set(&symbol_short!("emg_adm"), admin);
}

pub fn get_proposal(env: &Env, id: u32) -> Option<GovernanceProposal> {
    let key = (Symbol::new(env, "prop"), id);
    env.storage().persistent().get(&key)
}

pub fn set_proposal(env: &Env, id: u32, proposal: &GovernanceProposal) {
    let key = (Symbol::new(env, "prop"), id);
    env.storage().persistent().set(&key, proposal);
}

pub fn has_voted(env: &Env, proposal_id: u32, voter: &Address) -> bool {
    let key = (Symbol::new(env, "voted"), proposal_id, voter.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn set_voted(env: &Env, proposal_id: u32, voter: &Address) {
    let key = (Symbol::new(env, "voted"), proposal_id, voter.clone());
    env.storage().persistent().set(&key, &true);
}

pub fn get_vote_receipt(
    env: &Env,
    proposal_id: u32,
    voter: &Address,
) -> Option<crate::storage::VoteReceipt> {
    let key = (symbol_short!("vt_rcpt"), proposal_id, voter.clone());
    env.storage().persistent().get(&key)
}

pub fn set_vote_receipt(
    env: &Env,
    proposal_id: u32,
    voter: &Address,
    receipt: &crate::storage::VoteReceipt,
) {
    let key = (symbol_short!("vt_rcpt"), proposal_id, voter.clone());
    env.storage().persistent().set(&key, receipt);
}
