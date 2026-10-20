#![cfg(test)]
//! Tests for the reward waterfall (issue #341).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env, Symbol, TryFromVal,
};

use crate::{
    balance,
    reward_waterfall::RewardType,
    vault::{VaultContract, VaultContractClient},
};

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    token_admin: token::StellarAssetClient<'a>,
    admin: Address,
    alice: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
        });

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);

        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            token_admin,
            admin,
            alice,
        }
    }

    /// Seed the reward pool and fund the contract with the tokens to back it.
    fn fund_reward_pool(&self, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_reward_pool_balance(&self.env, amount);
        });
        self.token_admin.mint(&self.vault_id, &amount);
    }

    fn set_accrued(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_accrued_reward(&self.env, user, amount);
        });
    }
}

#[test]
fn default_waterfall_is_base_rate_first() {
    let f = Fixture::new();
    let order = f.vault.get_reward_waterfall();
    assert_eq!(order.get(0), Some(RewardType::BaseRate));
}

#[test]
fn admin_can_set_waterfall_order() {
    let f = Fixture::new();
    let order = soroban_sdk::vec![
        &f.env,
        RewardType::ReferralBonus,
        RewardType::BaseRate,
        RewardType::ValidatorBonus,
        RewardType::CampaignBoost,
        RewardType::AnniversaryBonus,
    ];
    f.vault.set_reward_waterfall(&order);
    assert_eq!(f.vault.get_reward_waterfall(), order);
}

#[test]
fn rejects_duplicate_types_in_waterfall() {
    let f = Fixture::new();
    let order = soroban_sdk::vec![&f.env, RewardType::BaseRate, RewardType::BaseRate];
    let result = f.vault.try_set_reward_waterfall(&order);
    assert!(result.is_err());
}

#[test]
fn sufficient_balance_pays_all_types() {
    let f = Fixture::new();
    f.set_accrued(&f.alice, 100);
    f.vault.credit_reward(&f.alice, &RewardType::ValidatorBonus, &50);
    f.vault.credit_reward(&f.alice, &RewardType::CampaignBoost, &25);
    f.fund_reward_pool(1_000);

    let total = f.vault.calc_total_reward(&f.alice);
    assert_eq!(total, 175);

    let paid = f.vault.claim_via_waterfall(&f.alice);
    assert_eq!(paid, 175);
    assert_eq!(f.vault.calc_total_reward(&f.alice), 0);
}

#[test]
fn insufficient_balance_pays_in_priority_order() {
    let f = Fixture::new();
    // BaseRate first by default; give alice 100 base + 100 validator bonus,
    // but only fund the pool with 100 — only BaseRate should get paid.
    f.set_accrued(&f.alice, 100);
    f.vault.credit_reward(&f.alice, &RewardType::ValidatorBonus, &100);
    f.fund_reward_pool(100);

    let paid = f.vault.claim_via_waterfall(&f.alice);
    assert_eq!(paid, 100);

    let breakdown = f.vault.get_reward_breakdown(&f.alice);
    let validator_remaining = breakdown
        .iter()
        .find(|(t, _)| *t == RewardType::ValidatorBonus)
        .map(|(_, amt)| amt)
        .unwrap();
    assert_eq!(validator_remaining, 100);
}

#[test]
fn waterfall_order_is_respected_when_reordered() {
    let f = Fixture::new();
    // Put ValidatorBonus ahead of BaseRate; with a pool that only covers the
    // validator bonus, BaseRate should be the one left unpaid.
    let order = soroban_sdk::vec![
        &f.env,
        RewardType::ValidatorBonus,
        RewardType::BaseRate,
        RewardType::CampaignBoost,
        RewardType::AnniversaryBonus,
        RewardType::ReferralBonus,
    ];
    f.vault.set_reward_waterfall(&order);

    f.set_accrued(&f.alice, 100);
    f.vault.credit_reward(&f.alice, &RewardType::ValidatorBonus, &50);
    f.fund_reward_pool(50);

    let paid = f.vault.claim_via_waterfall(&f.alice);
    assert_eq!(paid, 50);

    let breakdown = f.vault.get_reward_breakdown(&f.alice);
    let base_remaining = breakdown
        .iter()
        .find(|(t, _)| *t == RewardType::BaseRate)
        .map(|(_, amt)| amt)
        .unwrap();
    assert_eq!(base_remaining, 100);
}

#[test]
fn partial_payment_emits_event() {
    let f = Fixture::new();
    f.set_accrued(&f.alice, 100);
    f.vault.credit_reward(&f.alice, &RewardType::ValidatorBonus, &100);
    f.fund_reward_pool(100);

    f.vault.claim_via_waterfall(&f.alice);

    let events = f.env.events().all();
    let found = events.iter().any(|(topics, _data)| match topics.get(0) {
        Some(val) => Symbol::try_from_val(&f.env, &val)
            .map(|t| t == Symbol::new(&f.env, "rw_part"))
            .unwrap_or(false),
        None => false,
    });
    assert!(found, "expected partial_reward_paid event");
}
