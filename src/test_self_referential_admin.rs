#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Env,
};
use crate::errors::VaultError;
use crate::storage::FeeRecipient;
use crate::vault::VaultContract;
use crate::VaultContractClient;

fn setup_test_vault(env: &Env) -> (VaultContractClient, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let token = Address::generate(env);
    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(env, &contract_id);

    client.initialize(&admin, &token, &1000, &None, &None);

    (client, contract_id, admin, token)
}

#[test]
fn test_transfer_admin_to_self_is_blocked() {
    let env = Env::default();
    let (client, contract_id, _admin, _token) = setup_test_vault(&env);

    // Attempting to set admin to the contract's own address must fail
    let res = client.try_transfer_admin(&contract_id);
    assert_eq!(res, Err(Ok(VaultError::InvalidAddress)));
}

#[test]
fn test_transfer_admin_to_valid_address_succeeds() {
    let env = Env::default();
    let (client, _contract_id, admin, _token) = setup_test_vault(&env);
    assert_eq!(client.get_admin(), admin);

    let new_admin = Address::generate(&env);
    let res = client.try_transfer_admin(&new_admin);
    assert!(res.is_ok());
    assert_eq!(client.get_admin(), new_admin);
}

#[test]
fn test_set_emergency_admin_to_self_is_blocked() {
    let env = Env::default();
    let (client, contract_id, admin, _token) = setup_test_vault(&env);

    // Attempting to set emergency admin to contract's own address must fail
    let res = client.try_set_emergency_admin(&admin, &contract_id);
    assert_eq!(res, Err(Ok(VaultError::InvalidAddress)));
}

#[test]
fn test_set_emergency_admin_to_valid_address_succeeds() {
    let env = Env::default();
    let (client, _contract_id, admin, _token) = setup_test_vault(&env);

    let emergency_admin = Address::generate(&env);
    let res = client.try_set_emergency_admin(&admin, &emergency_admin);
    assert!(res.is_ok());
}

#[test]
fn test_self_referential_fee_recipient_is_allowed() {
    let env = Env::default();
    let (client, contract_id, admin, _token) = setup_test_vault(&env);

    // Fee recipient pointed at the vault itself is allowed because fee distributions
    // simply remain in the contract's own token reserves.
    client.set_fee_recipient(&admin, &contract_id);
    assert_eq!(client.get_fee_recipient(), Some(contract_id.clone()));
}

#[test]
fn test_self_referential_fee_recipients_list_is_allowed() {
    let env = Env::default();
    let (client, contract_id, _admin, _token) = setup_test_vault(&env);

    // Proportional fee recipient list configured with the vault itself
    let recipients = vec![
        &env,
        FeeRecipient {
            recipient: contract_id.clone(),
            share_bps: 10_000,
        },
    ];

    let res = client.try_set_fee_recipients(&recipients);
    assert!(res.is_ok());

    let stored = client.get_fee_recipients();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored.get(0).unwrap().recipient, contract_id);
}
