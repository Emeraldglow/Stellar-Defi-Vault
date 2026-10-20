#![cfg(test)]
//! Tests for the transfer cooldown (issue #340).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

use crate::{
    transfer_cooldown,
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
    admin: Address,
    recipient: Address,
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
        let recipient = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            recipient,
        }
    }

    fn record_transfer(&self, at_ledger: u32) {
        set_ledger(&self.env, at_ledger);
        self.env.as_contract(&self.vault_id, || {
            transfer_cooldown::record_transfer_received(&self.env, &self.recipient);
        });
    }
}

#[test]
fn cooldown_zero_disables_the_check() {
    let f = Fixture::new();
    // Default cooldown is 0.
    f.record_transfer(1_000);
    assert!(f.vault.try_check_transfer_cooldown(&f.recipient).is_ok());
}

#[test]
fn recipient_cannot_unstake_during_cooldown() {
    let f = Fixture::new();
    f.vault.set_transfer_cooldown(&1_000);
    f.record_transfer(1_000);

    set_ledger(&f.env, 1_500);
    let result = f.vault.try_check_transfer_cooldown(&f.recipient);
    assert!(result.is_err());
    assert_eq!(f.vault.get_transfer_cooldown_remaining(&f.recipient), 500);
}

#[test]
fn cooldown_expires_correctly() {
    let f = Fixture::new();
    f.vault.set_transfer_cooldown(&1_000);
    f.record_transfer(1_000);

    set_ledger(&f.env, 2_001);
    assert_eq!(f.vault.get_transfer_cooldown_remaining(&f.recipient), 0);
    assert!(f.vault.try_check_transfer_cooldown(&f.recipient).is_ok());
}

#[test]
fn sender_unaffected_after_transfer() {
    let f = Fixture::new();
    f.vault.set_transfer_cooldown(&1_000);
    // Sender never received a transfer, so they have no recorded cooldown.
    assert_eq!(f.vault.get_transfer_cooldown_remaining(&f.admin), 0);
    assert!(f.vault.try_check_transfer_cooldown(&f.admin).is_ok());
}

#[test]
fn clearing_removes_the_cooldown() {
    let f = Fixture::new();
    f.vault.set_transfer_cooldown(&1_000);
    f.record_transfer(1_000);

    f.env.as_contract(&f.vault_id, || {
        transfer_cooldown::clear_transfer_received(&f.env, &f.recipient);
    });

    set_ledger(&f.env, 1_100);
    assert_eq!(f.vault.get_transfer_cooldown_remaining(&f.recipient), 0);
}
