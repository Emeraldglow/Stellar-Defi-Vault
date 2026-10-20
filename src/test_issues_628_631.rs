#![cfg(test)]

extern crate std;

use soroban_sdk::{testutils::Address as _, token, vec, Address, Env, Vec};

use crate::{
    errors::VaultError,
    storage::PauseReason,
    vault::{VaultContract, VaultContractClient},
};

fn setup(
    env: &Env,
) -> (
    VaultContractClient<'_>,
    Address,
    Address,
    token::StellarAssetClient<'_>,
) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let token_address = env.register_stellar_asset_contract(admin.clone());
    let token_admin = token::StellarAssetClient::new(env, &token_address);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(env, &vault_id);
    vault.initialize(&admin, &token_address, &0, &None, &None);
    (vault, admin, token_address, token_admin)
}

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger()
        .with_mut(|ledger| ledger.sequence_number = sequence);
}

#[test]
fn manual_unpause_clears_pending_scheduled_unpause() {
    let env = Env::default();
    let (vault, _admin, _token, _token_admin) = setup(&env);
    let target = env.ledger().sequence() + 100;

    vault.pause_until(
        &target,
        &PauseReason::Other,
        &soroban_sdk::String::from_str(&env, "maintenance"),
    );
    assert_eq!(vault.get_scheduled_unpause(), Some(target));

    vault.unpause();
    assert_eq!(vault.get_scheduled_unpause(), None);
    assert!(!vault.is_paused());

    set_ledger(&env, target);
    assert!(!vault.is_paused());
}

#[test]
fn second_pause_until_overwrites_the_previous_schedule() {
    let env = Env::default();
    let (vault, _admin, _token, _token_admin) = setup(&env);
    let first_target = env.ledger().sequence() + 100;
    let second_target = first_target + 100;

    vault.pause_until(
        &first_target,
        &PauseReason::Other,
        &soroban_sdk::String::from_str(&env, "first"),
    );
    vault.pause_until(
        &second_target,
        &PauseReason::Other,
        &soroban_sdk::String::from_str(&env, "second"),
    );

    assert_eq!(vault.get_scheduled_unpause(), Some(second_target));
    set_ledger(&env, first_target);
    assert!(vault.is_paused());
    set_ledger(&env, second_target);
    assert!(!vault.is_paused());
}

#[test]
fn repeated_small_deposit_withdraw_cycles_cannot_extract_value_from_rounding() {
    let env = Env::default();
    let (vault, admin, token_address, token_admin) = setup(&env);
    let user = Address::generate(&env);
    token_admin.mint(&user, &1_300);
    token_admin.mint(&admin, &333);

    vault.deposit(&user, &1_000, &None);
    vault.add_yield(&admin, &333);
    let initial_user_balance = token::Client::new(&env, &token_address).balance(&user);

    for _ in 0..100 {
        vault.deposit(&user, &3, &None);
        vault.withdraw(&user, &2);
    }

    let final_user_balance = token::Client::new(&env, &token_address).balance(&user);
    assert!(final_user_balance <= initial_user_balance);
}

#[test]
fn reward_waterfall_accepts_limit_and_rejects_one_over() {
    let env = Env::default();
    let (vault, _admin, _token, _token_admin) = setup(&env);
    let at_limit = vec![
        &env,
        crate::reward_waterfall::RewardType::BaseRate,
        crate::reward_waterfall::RewardType::ValidatorBonus,
        crate::reward_waterfall::RewardType::CampaignBoost,
        crate::reward_waterfall::RewardType::AnniversaryBonus,
        crate::reward_waterfall::RewardType::ReferralBonus,
    ];
    assert!(vault.try_set_reward_waterfall(&at_limit).is_ok());

    let over_limit = vec![
        &env,
        crate::reward_waterfall::RewardType::BaseRate,
        crate::reward_waterfall::RewardType::ValidatorBonus,
        crate::reward_waterfall::RewardType::CampaignBoost,
        crate::reward_waterfall::RewardType::AnniversaryBonus,
        crate::reward_waterfall::RewardType::ReferralBonus,
        crate::reward_waterfall::RewardType::BaseRate,
    ];
    assert_eq!(
        vault.try_set_reward_waterfall(&over_limit),
        Err(Ok(VaultError::BatchTooLarge))
    );
}

#[test]
fn batch_position_query_accepts_limit_and_rejects_one_over() {
    let env = Env::default();
    let (vault, _admin, _token, _token_admin) = setup(&env);
    let mut at_limit = Vec::new(&env);
    for _ in 0..20 {
        at_limit.push_back(Address::generate(&env));
    }
    assert!(vault.try_batch_position_query(&at_limit).is_ok());

    at_limit.push_back(Address::generate(&env));
    assert_eq!(
        vault.try_batch_position_query(&at_limit),
        Err(Ok(VaultError::BatchTooLarge))
    );
}
