#![cfg(test)]
use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env};
use crate::{VaultContract, VaultContractClient};

#[test]
fn test_issues_490_493() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);
    
    // 492
    client.set_fee_recipient(&admin, &Address::generate(&env));
    assert!(client.get_fee_recipient().is_some());
    
    // 493
    client.pause_deposits(&admin);
    let (p_dep, p_with) = client.get_pause_state();
    assert!(p_dep);
    assert!(!p_with);
}
