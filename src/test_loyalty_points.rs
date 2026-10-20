#![cfg(test)]
//! Tests for the non-transferable loyalty points ledger (issue #756).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
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
    admin: Address,
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
            admin,
            alice,
            bob,
        }
    }

    /// Seed a 1:1 position held since the current ledger.
    fn seed(&self, user: &Address, shares: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_shares(&self.env, user, shares);
            balance::set_total_shares(&self.env, balance::get_total_shares(&self.env) + shares);
            balance::set_total_deposited(
                &self.env,
                balance::get_total_deposited(&self.env) + shares,
            );
            balance::set_staked_at_ledger(&self.env, user, self.env.ledger().sequence());
        });
    }
}

#[test]
fn accrual_is_proportional_to_size_and_time() {
    let f = Fixture::new();
    f.vault.set_points_rate(&f.admin, &2_i128);

    f.seed(&f.alice, 100);
    f.seed(&f.bob, 200);

    // 100 ledgers at rate 2.
    set_ledger(&f.env, 1_100);
    assert_eq!(f.vault.get_points(&f.alice), 100 * 100 * 2);
    // Twice the position accrues twice the points.
    assert_eq!(f.vault.get_points(&f.bob), 200 * 100 * 2);

    // Twice the time accrues twice the points.
    set_ledger(&f.env, 1_200);
    assert_eq!(f.vault.get_points(&f.alice), 100 * 200 * 2);
    assert_eq!(f.vault.get_points(&f.bob), 200 * 200 * 2);
}

#[test]
fn no_accrual_when_rate_is_zero() {
    let f = Fixture::new();
    f.vault.set_points_rate(&f.admin, &0_i128);

    f.seed(&f.alice, 100);
    set_ledger(&f.env, 1_500);

    assert_eq!(f.vault.get_points(&f.alice), 0);
    assert_eq!(f.vault.get_points_rate(), 0);
}

#[test]
fn disabling_rate_stops_further_accrual() {
    let f = Fixture::new();
    f.vault.set_points_rate(&f.admin, &3_i128);
    f.seed(&f.alice, 100);

    set_ledger(&f.env, 1_100);
    let earned = f.vault.get_points(&f.alice);
    assert_eq!(earned, 100 * 100 * 3);

    // Disable accrual, then advance: the balance is frozen.
    f.vault.set_points_rate(&f.admin, &0_i128);
    set_ledger(&f.env, 1_500);
    assert_eq!(f.vault.get_points(&f.alice), earned);
}

#[test]
fn rate_change_only_applies_going_forward() {
    let f = Fixture::new();
    f.vault.set_points_rate(&f.admin, &2_i128);
    f.seed(&f.alice, 100);

    set_ledger(&f.env, 1_100);
    f.vault.set_points_rate(&f.admin, &4_i128);
    set_ledger(&f.env, 1_200);

    // 100 ledgers at rate 2, then 100 ledgers at rate 4.
    assert_eq!(f.vault.get_points(&f.alice), 100 * 100 * 2 + 100 * 100 * 4);
}

#[test]
fn no_position_earns_no_points() {
    let f = Fixture::new();
    f.vault.set_points_rate(&f.admin, &5_i128);
    set_ledger(&f.env, 2_000);

    assert_eq!(f.vault.get_points(&f.bob), 0);
}

#[test]
fn only_admin_can_set_the_rate() {
    let f = Fixture::new();
    let result = f.vault.try_set_points_rate(&f.alice, &1_i128);
    assert!(result.is_err());
}
