#![cfg(test)]
//! Tests for the pool-insights / runway-guard / admin-recovery issue batch.

use crate::admin_recovery::{AdminRecoveryProposal, ADMIN_RECOVERY_DELAY_LEDGERS};
use crate::errors::VaultOpsError;
use crate::pool_insights::RoundingDirection;
use crate::runway_guard::MIN_RUNWAY_LEDGERS_FLOOR;
use crate::vault::{VaultContract, VaultContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

fn setup(env: &Env) -> (VaultContractClient<'_>, Address, Address) {
    env.mock_all_auths();
    let vault_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(env, &vault_id);
    let admin = Address::generate(env);
    let token = env.register_stellar_asset_contract(admin.clone());
    client.initialize(&admin, &token, &0_u32, &None::<u32>, &None::<u32>);
    (client, admin, token)
}

fn mint(env: &Env, token: &Address, to: &Address, amount: i128) {
    token::StellarAssetClient::new(env, token).mint(to, &amount);
}

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| li.sequence_number = sequence);
}

// ── Issue: pool summary ──────────────────────────────────────────────────────

#[test]
fn pool_summary_aggregates_pool_state() {
    let env = Env::default();
    let (client, _admin, token) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    mint(&env, &token, &alice, 1_000);
    mint(&env, &token, &bob, 500);

    client.stake(&alice, &1_000, &0);
    client.stake(&bob, &500, &0);

    let summary = client.get_pool_summary();
    assert_eq!(summary.total_deposited, 1_500);
    assert_eq!(summary.depositor_count, 2);
    assert_eq!(summary.current_rate_bps, 0);
    // No pool cap configured, so utilisation is reported as zero.
    assert_eq!(summary.utilization_bps, 0);
    // Matches the individual query.
    assert_eq!(summary.reward_pool_balance, client.get_reward_pool_balance());
}

// ── Issue: rounding-policy transparency ──────────────────────────────────────

#[test]
fn rounding_policy_documents_floor_behaviour() {
    let env = Env::default();
    let (client, _admin, _token) = setup(&env);

    let policy = client.get_rounding_policy();
    assert_eq!(policy.deposit, RoundingDirection::Down);
    assert_eq!(policy.withdraw, RoundingDirection::Down);
    assert_eq!(policy.preview_redeem, RoundingDirection::Down);
}

#[test]
fn rounding_policy_matches_observed_conversion() {
    let env = Env::default();
    let (client, _admin, token) = setup(&env);

    let alice = Address::generate(&env);
    mint(&env, &token, &alice, 1_000);
    client.stake(&alice, &1_000, &0);
    // 1:1 first deposit, and preview_redeem floors to the same amount.
    assert_eq!(client.preview_redeem(&1_000), 1_000);
    assert_eq!(client.preview_redeem(&999), 999);
}

// ── Issue: reward-runway guard ───────────────────────────────────────────────

#[test]
fn projected_runway_matches_formula() {
    let env = Env::default();
    let (client, admin, token) = setup(&env);

    let alice = Address::generate(&env);
    mint(&env, &token, &alice, 1_000);
    client.stake(&alice, &1_000, &0);
    mint(&env, &token, &admin, 1_000);
    client.fund_reward_pool(&admin, &1_000);

    // rate 0 => infinite runway.
    assert_eq!(client.get_projected_runway(), u32::MAX);

    client.set_reward_rate_bps(&500);
    // runway = 1000 * 10_000 * 6_307_200 / (500 * 1000) = 126_144_000.
    assert_eq!(client.get_projected_runway(), 126_144_000);
}

#[test]
fn rate_change_within_runway_succeeds_and_short_runway_reverts() {
    let env = Env::default();
    let (client, admin, token) = setup(&env);

    let alice = Address::generate(&env);
    mint(&env, &token, &alice, 1_000);
    client.stake(&alice, &1_000, &0);
    mint(&env, &token, &admin, 1_000);
    client.fund_reward_pool(&admin, &1_000);

    // A one-day minimum is comfortably satisfied.
    client.set_min_runway_ledgers(&admin, &MIN_RUNWAY_LEDGERS_FLOOR);
    client.set_reward_rate_bps(&500);
    assert_eq!(client.get_projected_runway(), 126_144_000);

    // Raise the minimum above the projected runway: the next change reverts.
    client.set_min_runway_ledgers(&admin, &(126_144_000 + 1));
    let result = client.try_set_reward_rate_bps(&500);
    assert_eq!(result, Err(Ok(VaultOpsError::InsufficientRunway)));
}

#[test]
fn set_min_runway_rejects_below_floor_and_non_admin() {
    let env = Env::default();
    let (client, admin, _token) = setup(&env);

    let result = client.try_set_min_runway_ledgers(&admin, &(MIN_RUNWAY_LEDGERS_FLOOR - 1));
    assert_eq!(result, Err(Ok(VaultOpsError::InvalidRunway)));

    let stranger = Address::generate(&env);
    let result = client.try_set_min_runway_ledgers(&stranger, &MIN_RUNWAY_LEDGERS_FLOOR);
    assert_eq!(result, Err(Ok(VaultOpsError::Unauthorized)));
}

// ── Issue: time-delayed admin recovery ───────────────────────────────────────

#[test]
fn recovery_is_blocked_before_delay_then_executes() {
    let env = Env::default();
    let (client, admin, _token) = setup(&env);

    let proposer = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.propose_admin_recovery(&proposer, &new_admin);
    let stored: AdminRecoveryProposal = client.get_admin_recovery_proposal().unwrap();
    assert_eq!(stored.new_admin, new_admin);

    // Before the delay elapses, execution is rejected and admin is unchanged.
    let result = client.try_execute_admin_recovery();
    assert_eq!(result, Err(Ok(VaultOpsError::RecoveryDelayNotElapsed)));
    assert_eq!(client.get_admin(), admin);

    set_ledger(&env, ADMIN_RECOVERY_DELAY_LEDGERS + 1);
    client.execute_admin_recovery();
    assert_eq!(client.get_admin(), new_admin);
    assert!(client.get_admin_recovery_proposal().is_none());
}

#[test]
fn live_admin_can_cancel_recovery() {
    let env = Env::default();
    let (client, admin, _token) = setup(&env);

    let proposer = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.propose_admin_recovery(&proposer, &new_admin);
    client.cancel_admin_recovery(&admin);
    assert!(client.get_admin_recovery_proposal().is_none());

    // A cancelled proposal can no longer be executed.
    set_ledger(&env, ADMIN_RECOVERY_DELAY_LEDGERS + 1);
    let result = client.try_execute_admin_recovery();
    assert_eq!(result, Err(Ok(VaultOpsError::RecoveryNotPending)));
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn only_one_recovery_proposal_at_a_time() {
    let env = Env::default();
    let (client, _admin, _token) = setup(&env);

    let proposer = Address::generate(&env);
    client.propose_admin_recovery(&proposer, &Address::generate(&env));
    let result = client.try_propose_admin_recovery(&proposer, &Address::generate(&env));
    assert_eq!(result, Err(Ok(VaultOpsError::RecoveryAlreadyPending)));
}
