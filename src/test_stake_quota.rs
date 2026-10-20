#![cfg(test)]
//! Tests for the stake-weighted operation quota (issue #339).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, String,
};

use crate::{
    balance,
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
    alice: Address,
    bob: Address,
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
        let bob = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            alice,
            bob,
        }
    }

    /// Seed a staker's position directly (no `stake()` entrypoint currently
    /// exists on `main` — see the module-level "Known gap" note).
    fn seed_position(&self, user: &Address, shares: i128, total_shares: i128, total_deposited: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_shares(&self.env, user, shares);
            balance::set_total_shares(&self.env, total_shares);
            balance::set_total_deposited(&self.env, total_deposited);
        });
    }
}

#[test]
fn unconfigured_pool_has_no_quota_gate() {
    let f = Fixture::new();
    assert_eq!(f.vault.get_quota_allowance(&f.alice), 0);
}

#[test]
fn large_staker_gets_more_quota() {
    let f = Fixture::new();
    f.vault.set_quota_config(&100, &1_000);
    f.seed_position(&f.alice, 8_000, 10_000, 10_000); // 80% of pool
    f.seed_position(&f.bob, 1_000, 10_000, 10_000); // 10% of pool

    let alice_allowance = f.vault.get_quota_allowance(&f.alice);
    let bob_allowance = f.vault.get_quota_allowance(&f.bob);
    assert!(alice_allowance > bob_allowance);
    assert_eq!(alice_allowance, 80);
    assert_eq!(bob_allowance, 10);
}

#[test]
fn zero_stake_gets_minimum_quota_of_one() {
    let f = Fixture::new();
    f.vault.set_quota_config(&100, &1_000);
    f.seed_position(&f.alice, 8_000, 10_000, 10_000);
    // Bob has no position at all.
    assert_eq!(f.vault.get_quota_allowance(&f.bob), 1);
}

#[test]
fn quota_consumed_on_content_submission() {
    let f = Fixture::new();
    f.vault.set_quota_config(&1, &1_000);
    f.seed_position(&f.alice, 5_000, 10_000, 10_000);

    assert_eq!(f.vault.get_quota_remaining(&f.alice), 1);
    f.vault
        .submit_content(&f.alice, &String::from_str(&f.env, "hash-1"));
    assert_eq!(f.vault.get_quota_remaining(&f.alice), 0);

    let result = f
        .vault
        .try_submit_content(&f.alice, &String::from_str(&f.env, "hash-2"));
    assert!(result.is_err());
}

#[test]
fn quota_resets_on_epoch_boundary() {
    let f = Fixture::new();
    f.vault.set_quota_config(&1, &500);
    f.seed_position(&f.alice, 5_000, 10_000, 10_000);

    f.vault
        .submit_content(&f.alice, &String::from_str(&f.env, "hash-1"));
    assert_eq!(f.vault.get_quota_remaining(&f.alice), 0);

    set_ledger(&f.env, 1_501);
    assert_eq!(f.vault.get_quota_remaining(&f.alice), 1);
    f.vault
        .submit_content(&f.alice, &String::from_str(&f.env, "hash-2"));
    assert_eq!(f.vault.get_quota_remaining(&f.alice), 0);
}
