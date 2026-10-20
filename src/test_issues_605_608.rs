#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, Env, Symbol, TryFromVal, Vec,
};

use crate::{
    admin,
    errors::{PublicApiError, VaultError},
    vault::{VaultContract, VaultContractClient},
};

fn admin_fixture() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault_id = env.register_contract(None, VaultContract);
    env.as_contract(&vault_id, || admin::set_admin(&env, &admin));
    (env, vault_id, admin)
}

#[test]
fn admin_renouncement_is_irreversible_and_emits_event() {
    let (env, vault_id, admin) = admin_fixture();
    let client = VaultContractClient::new(&env, &vault_id);

    client.set_reward_rate_bps(&1_000);
    client.renounce_admin(&admin);

    assert_eq!(client.try_get_admin(), Err(Ok(VaultError::NoAdmin)));
    assert_eq!(
        client.try_set_reward_rate_bps(&500),
        Err(Ok(VaultError::NoAdmin))
    );
    assert_eq!(
        client.try_renounce_admin(&admin),
        Err(Ok(VaultError::NoAdmin))
    );

    // Re-initialization cannot recreate an admin after renouncement.
    let token = Address::generate(&env);
    assert_eq!(
        client.try_initialize(&admin, &token, &0, &None, &None),
        Err(Ok(VaultError::AlreadyInitialized))
    );

    let (_, topics, _) = env.events().all().last().unwrap();
    let topic = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
    assert_eq!(topic, Symbol::new(&env, "admin_renounced"));
}

#[test]
fn validation_failures_are_typed_results() {
    let (env, vault_id, admin) = admin_fixture();
    let client = VaultContractClient::new(&env, &vault_id);

    let recipients = Vec::from_array(&env, [Address::generate(&env)]);
    let invalid_shares = Vec::from_array(&env, [9_999_u32]);
    assert_eq!(
        client.try_set_treasury_split(&admin, &recipients, &invalid_shares),
        Err(Ok(PublicApiError::InvalidAllocation))
    );

    assert_eq!(
        client.try_execute_admin_action(&admin, &999),
        Err(Ok(PublicApiError::ActionNotFound))
    );
}
