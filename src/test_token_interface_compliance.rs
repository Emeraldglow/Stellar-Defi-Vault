#![cfg(test)]

extern crate std;

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    Address, Env,
};

use crate::errors::VaultError;
use crate::vault::{VaultContract, VaultContractClient};

/// A dummy mock contract that does NOT implement SEP-41 (no decimals, no balance, etc.)
#[contract]
struct NonTokenContract;

#[contractimpl]
impl NonTokenContract {
    pub fn dummy_call(_env: Env) -> u32 {
        42
    }
}

/// A mock contract that has decimals() but panics / returns error
#[contract]
struct FaultyTokenContract;

#[contractimpl]
impl FaultyTokenContract {
    pub fn decimals(_env: Env) -> u32 {
        panic!("Decimals call fails unexpectedly");
    }
}

// ── Issue #598: SEP-41 Token Interface Compliance Tests ───────────────────────

#[test]
fn test_valid_sep41_token_passes_initialization() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(1_000);

    let admin = Address::generate(&env);
    // Standard SAC complies with SEP-41
    let token = env.register_stellar_asset_contract(admin.clone());

    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);

    let res = client.try_initialize(
        &admin,
        &token,
        &1_000_u32, // 10% APR
        &None,
        &None,
    );

    assert!(res.is_ok(), "Valid SEP-41 token should initialize successfully");
    assert_eq!(client.get_token(), token);
}

#[test]
fn test_non_contract_address_fails_with_invalid_token_interface() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(1_000);

    let admin = Address::generate(&env);
    // Raw generated address with no contract deployed at it
    let fake_token = Address::generate(&env);

    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);

    let res = client.try_initialize(
        &admin,
        &fake_token,
        &1_000_u32,
        &None,
        &None,
    );

    assert!(res.is_err(), "Non-contract address must fail initialization");
    assert_eq!(
        res.unwrap_err().unwrap(),
        VaultError::InvalidTokenInterface,
        "Must revert with InvalidTokenInterface"
    );
}

#[test]
fn test_contract_without_sep41_fails_with_invalid_token_interface() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(1_000);

    let admin = Address::generate(&env);
    // Contract exists, but does not implement SEP-41 decimals()
    let non_token = env.register_contract(None, NonTokenContract);

    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);

    let res = client.try_initialize(
        &admin,
        &non_token,
        &1_000_u32,
        &None,
        &None,
    );

    assert!(res.is_err(), "Contract without decimals() must fail");
    assert_eq!(
        res.unwrap_err().unwrap(),
        VaultError::InvalidTokenInterface
    );
}

#[test]
fn test_faulty_token_decimals_call_fails_with_invalid_token_interface() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(1_000);

    let admin = Address::generate(&env);
    let faulty_token = env.register_contract(None, FaultyTokenContract);

    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);

    let res = client.try_initialize(
        &admin,
        &faulty_token,
        &1_000_u32,
        &None,
        &None,
    );

    assert!(res.is_err(), "Faulty decimals() must fail");
    assert_eq!(
        res.unwrap_err().unwrap(),
        VaultError::InvalidTokenInterface
    );
}

#[test]
fn test_reward_token_sep41_compliance_check() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(1_000);

    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract(admin.clone());
    let non_token = env.register_contract(None, NonTokenContract);

    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);

    // Call __constructor with invalid reward_token
    let res = client.try___constructor(
        &admin,
        &token,
        &Some(non_token),
        &Some(500_u32),
        &None,
        &None,
    );

    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err().unwrap(),
        VaultError::InvalidTokenInterface,
        "Reward token failing SEP-41 must revert with InvalidTokenInterface"
    );
}
