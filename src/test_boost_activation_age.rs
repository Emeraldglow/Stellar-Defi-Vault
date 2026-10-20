#![cfg(test)]
//! Tests for the boost-activation minimum age gate (issue #401).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

use crate::vault::{VaultContract, VaultContractClient};

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

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            alice,
        }
    }

    fn set_staked_at(&self, user: &Address, ledger: u32) {
        self.env.as_contract(&self.vault_id, || {
            self.env.storage().persistent().set(
                &crate::storage::DataKey::StakedAtLedger(user.clone()),
                &ledger,
            );
        });
    }
}

#[test]
fn new_position_gets_no_boost() {
    let f = Fixture::new();
    f.vault.set_boost_activation_minimum_age(&1_000);
    f.set_staked_at(&f.alice, 1_000);

    // Only 10 ledgers old — well under the 1000-ledger minimum.
    set_ledger(&f.env, 1_010);
    assert!(!f.vault.is_boost_eligible(&f.alice));
    assert_eq!(f.vault.get_ledgers_until_boost(&f.alice), 990);
}

#[test]
fn position_past_threshold_gets_boost() {
    let f = Fixture::new();
    f.vault.set_boost_activation_minimum_age(&1_000);
    f.set_staked_at(&f.alice, 1_000);

    set_ledger(&f.env, 2_500);
    assert!(f.vault.is_boost_eligible(&f.alice));
    assert_eq!(f.vault.get_ledgers_until_boost(&f.alice), 0);
}

#[test]
fn minimum_age_zero_always_boosts() {
    let f = Fixture::new();
    // Default minimum age is 0 (disabled) — never configured here.
    f.set_staked_at(&f.alice, 1_000);
    set_ledger(&f.env, 1_001);
    assert!(f.vault.is_boost_eligible(&f.alice));
    assert_eq!(f.vault.get_ledgers_until_boost(&f.alice), 0);
}

#[test]
fn boost_activated_event_fires_once() {
    let f = Fixture::new();
    f.vault.set_boost_activation_minimum_age(&1_000);
    f.set_staked_at(&f.alice, 1_000);
    set_ledger(&f.env, 2_500);

    assert!(f.vault.check_boost_activation(&f.alice));
    let events_after_first = f.env.events().all().len();

    // Second call: still eligible, but the activation event must not fire again.
    assert!(f.vault.check_boost_activation(&f.alice));
    let events_after_second = f.env.events().all().len();

    assert_eq!(events_after_first, events_after_second);
}
