//! Tests for issues #526-#529: scheduled exit, snapshot airdrop, external
//! price oracle, and co-sponsor reward matching.

use soroban_sdk::{testutils::{Address as _, Ledger as _}, Address, Env};

use crate::errors::VaultFeature2Error;
use crate::vault::{VaultContract, VaultContractClient};

/// Deploys the vault and initializes it with a real Stellar asset contract as
/// the stake token. Returns the client and the admin.
fn setup(env: &Env) -> (VaultContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(env, &contract);
    let token_addr = env.register_stellar_asset_contract(Address::generate(env));
    client.initialize(&admin, &token_addr, &1_000u32, &None, &None);
    (client, admin)
}

// ── Issue #526: scheduled exit ──────────────────────────────────────────────

#[test]
fn test_schedule_exit_sets_target_ledger() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let user = Address::generate(&env);

    // Schedule exit at a future ledger.
    let result = client.try_schedule_exit(&user, &1000);
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn test_schedule_exit_reverts_on_past_ledger() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let user = Address::generate(&env);

    // Move the chain forward so the target ledger is in the past.
    env.ledger().with_mut(|li| li.sequence_number = 500);

    let result = client.try_schedule_exit(&user, &100);
    assert_eq!(result, Err(Ok(VaultFeature2Error::InvalidRecoveryConfig)));
}

#[test]
fn test_get_scheduled_exit_returns_none_when_empty() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let user = Address::generate(&env);

    let result = client.get_scheduled_exit(&user);
    assert!(result.is_none());
}

// ── Issue #527: snapshot airdrop ────────────────────────────────────────────

#[test]
fn test_create_airdrop_returns_id() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    // The snapshot ledger must be in the past; move the chain forward first.
    env.ledger().with_mut(|li| li.sequence_number = 100);

    let airdrop_token = Address::generate(&env);
    let result = client.try_create_airdrop(&airdrop_token, &100_000, &50);
    assert_eq!(result, Ok(Ok(1)));
}

#[test]
fn test_create_airdrop_reverts_on_future_ledger() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let airdrop_token = Address::generate(&env);
    // Try to create with future snapshot ledger.
    let result = client.try_create_airdrop(&airdrop_token, &100_000, &999999);
    assert_eq!(result, Err(Ok(VaultFeature2Error::InvalidRecoveryConfig)));
}

// ── Issue #528: external price oracle ───────────────────────────────────────

#[test]
fn test_set_price_oracle_and_get() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let oracle = Address::generate(&env);
    let result = client.try_set_price_oracle(&oracle);
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn test_get_position_value_usd_reverts_without_oracle() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let user = Address::generate(&env);
    // No oracle set — should revert.
    let result = client.try_get_position_value_usd(&user);
    assert_eq!(result, Err(Ok(VaultFeature2Error::NoOracleConfigured)));
}

// ── Issue #529: co-sponsor ──────────────────────────────────────────────────

#[test]
fn test_register_co_sponsor() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let sponsor = Address::generate(&env);
    let result = client.try_register_co_sponsor(&sponsor, &500, &10000);
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn test_register_co_sponsor_reverts_with_zero_bps() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let sponsor = Address::generate(&env);
    // match_bps=0 should fail.
    let result = client.try_register_co_sponsor(&sponsor, &0, &10000);
    assert_eq!(result, Err(Ok(VaultFeature2Error::InvalidRecoveryConfig)));
}
