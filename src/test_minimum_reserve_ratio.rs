#![cfg(test)]
//! Tests for the minimum reward-reserve ratio floor (issue #405).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::{
    balance,
    crate::{VaultContract, VaultContractClient},
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
    token_client: token::Client<'a>,
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
        let token_client = token::Client::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            token_admin,
            token_client,
            alice,
        }
    }

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

    fn seed_staker(&self, user: &Address) {
        self.env.as_contract(&self.vault_id, || {
            let mut all = balance::get_all_stakers(&self.env);
            all.push_back(user.clone());
            balance::set_all_stakers(&self.env, &all);
        });
    }
}

#[test]
fn ratio_zero_disables_the_floor() {
    let f = Fixture::new();
    assert_eq!(f.vault.get_minimum_reserve_ratio_bps(), 0);
    assert!(f.vault.is_reserve_floor_met());
}

#[test]
fn claim_within_floor_pays_fully() {
    let f = Fixture::new();
    f.seed_staker(&f.alice);
    f.set_accrued(&f.alice, 100);
    // Obligations = 100; 20% floor = 20; funding 200 leaves plenty of room.
    f.vault.set_minimum_reserve_ratio_bps(&2_000);
    f.fund_reward_pool(200);

    let paid = f.vault.claim_with_reserve_floor(&f.alice);
    assert_eq!(paid, 100);
    assert_eq!(f.token_client.balance(&f.alice), 100);
    assert_eq!(f.vault.get_deferred_reward(&f.alice), None);
}

#[test]
fn claim_breaching_floor_is_capped_and_deferred() {
    let f = Fixture::new();
    f.seed_staker(&f.alice);
    f.set_accrued(&f.alice, 100);
    // Obligations = 100; 50% floor = 50; pool only holds 60, so only 10 is
    // payable now (60 - 50 floor) and 90 must be deferred.
    f.vault.set_minimum_reserve_ratio_bps(&5_000);
    f.fund_reward_pool(60);

    let paid = f.vault.claim_with_reserve_floor(&f.alice);
    assert_eq!(paid, 10);
    assert_eq!(f.token_client.balance(&f.alice), 10);

    let deferred = f.vault.get_deferred_reward(&f.alice).unwrap();
    assert_eq!(deferred.amount, 90);
}

#[test]
fn floor_met_reports_true_when_sufficient_reserves() {
    let f = Fixture::new();
    f.seed_staker(&f.alice);
    f.set_accrued(&f.alice, 100);
    f.vault.set_minimum_reserve_ratio_bps(&2_000);
    f.fund_reward_pool(200);

    assert!(f.vault.is_reserve_floor_met());
}

#[test]
fn floor_not_met_when_reserves_thin() {
    let f = Fixture::new();
    f.seed_staker(&f.alice);
    f.set_accrued(&f.alice, 1_000);
    f.vault.set_minimum_reserve_ratio_bps(&5_000);
    f.fund_reward_pool(10);

    assert!(!f.vault.is_reserve_floor_met());
}

