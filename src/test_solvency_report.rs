#![cfg(test)]
//! Tests for the solvency report (issue #753).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::{
    balance,
    vault::{VaultContract, VaultContractClient},
};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    alice: Address,
    bob: Address,
    token_admin: token::StellarAssetClient<'a>,
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
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        Fixture {
            env,
            vault,
            vault_id,
            alice,
            bob,
            token_admin,
        }
    }

    /// Fund the vault with real tokens, as stakers and reward funders would.
    fn mint_to_vault(&self, amount: i128) {
        self.token_admin.mint(&self.vault_id, &amount);
    }

    /// Set the internal accounting without moving tokens, so tests can create
    /// solvent, balanced, and deficit token balances independently.
    fn seed(&self, principal: i128, pool: i128, alice_accrued: i128, bob_accrued: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_total_deposited(&self.env, principal);
            balance::set_reward_pool_balance(&self.env, pool);
            balance::set_accrued_reward(&self.env, &self.alice, alice_accrued);
            balance::set_accrued_reward(&self.env, &self.bob, bob_accrued);
        });
    }
}

#[test]
fn report_defaults_to_zero_before_initialization() {
    let env = Env::default();
    env.mock_all_auths();
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);

    let report = vault.get_solvency_report();
    assert_eq!(report.token_balance, 0);
    assert_eq!(report.total_principal, 0);
    assert_eq!(report.reward_pool, 0);
    assert_eq!(report.accrued_unclaimed, 0);
    assert_eq!(report.surplus_or_deficit, 0);
}

#[test]
fn report_is_exactly_balanced_when_assets_match_liabilities() {
    let f = Fixture::new();
    f.seed(1_000, 0, 200, 0);
    f.mint_to_vault(1_200);

    let report = f.vault.get_solvency_report();
    assert_eq!(report.token_balance, 1_200);
    assert_eq!(report.total_principal, 1_000);
    assert_eq!(report.accrued_unclaimed, 200);
    assert_eq!(report.surplus_or_deficit, 0);
}

#[test]
fn report_shows_surplus_when_assets_exceed_liabilities() {
    let f = Fixture::new();
    f.seed(1_000, 500, 200, 0);
    f.mint_to_vault(2_000);

    let report = f.vault.get_solvency_report();
    assert_eq!(report.token_balance, 2_000);
    assert_eq!(report.total_principal, 1_000);
    assert_eq!(report.reward_pool, 500);
    assert_eq!(report.accrued_unclaimed, 200);
    assert_eq!(report.surplus_or_deficit, 800);
}

#[test]
fn report_shows_deficit_when_assets_fall_short() {
    let f = Fixture::new();
    f.seed(1_000, 0, 200, 0);
    f.mint_to_vault(900);

    let report = f.vault.get_solvency_report();
    assert_eq!(report.token_balance, 900);
    assert_eq!(report.total_principal, 1_000);
    assert_eq!(report.accrued_unclaimed, 200);
    assert_eq!(report.surplus_or_deficit, -300);
}

#[test]
fn direct_donation_increases_reported_surplus() {
    let f = Fixture::new();
    f.seed(1_000, 0, 0, 0);
    f.mint_to_vault(1_000);

    let before = f.vault.get_solvency_report();
    assert_eq!(before.token_balance, 1_000);
    assert_eq!(before.surplus_or_deficit, 0);

    // A donation bypasses every internal counter, so only the token balance
    // and the derived surplus may move.
    f.mint_to_vault(250);

    let after = f.vault.get_solvency_report();
    assert_eq!(after.token_balance, 1_250);
    assert_eq!(after.total_principal, before.total_principal);
    assert_eq!(after.reward_pool, before.reward_pool);
    assert_eq!(after.accrued_unclaimed, before.accrued_unclaimed);
    assert_eq!(after.surplus_or_deficit, 250);
}

#[test]
fn principal_and_reward_funds_are_reported_separately() {
    let f = Fixture::new();
    f.seed(1_000, 400, 100, 0);
    f.mint_to_vault(1_500);

    let report = f.vault.get_solvency_report();
    assert_eq!(report.total_principal, 1_000);
    assert_eq!(report.reward_pool, 400);
    assert_eq!(report.accrued_unclaimed, 100);
    // The surplus is the unaccrued reward budget above principal and accrued
    // rewards, not a second copy of the reward pool.
    assert_eq!(report.surplus_or_deficit, 400);
}

#[test]
fn accrued_accumulator_tracks_accruals_and_claims() {
    let f = Fixture::new();
    f.env.as_contract(&f.vault_id, || {
        balance::set_accrued_reward(&f.env, &f.alice, 300);
        balance::set_accrued_reward(&f.env, &f.bob, 200);
        assert_eq!(balance::get_total_accrued_unclaimed(&f.env), 500);

        // A claim clears the user's accrued balance.
        balance::set_accrued_reward(&f.env, &f.alice, 0);
        assert_eq!(balance::get_total_accrued_unclaimed(&f.env), 200);

        // New accrual via the `add` helper moves the total too.
        balance::add_accrued_reward(&f.env, &f.bob, 50);
        assert_eq!(balance::get_total_accrued_unclaimed(&f.env), 250);
    });

    let report = f.vault.get_solvency_report();
    assert_eq!(report.accrued_unclaimed, 250);
}

#[test]
fn accrued_accumulator_is_idempotent_for_same_value() {
    let f = Fixture::new();
    f.env.as_contract(&f.vault_id, || {
        balance::set_accrued_reward(&f.env, &f.alice, 300);
        balance::set_accrued_reward(&f.env, &f.alice, 300);
        assert_eq!(balance::get_total_accrued_unclaimed(&f.env), 300);
    });
}

#[test]
fn donation_can_cure_a_deficit() {
    let f = Fixture::new();
    f.seed(1_000, 0, 500, 0);
    f.mint_to_vault(1_000);

    let deficit = f.vault.get_solvency_report();
    assert_eq!(deficit.surplus_or_deficit, -500);

    f.mint_to_vault(500);
    let balanced = f.vault.get_solvency_report();
    assert_eq!(balanced.surplus_or_deficit, 0);

    f.mint_to_vault(1);
    let recovered = f.vault.get_solvency_report();
    assert_eq!(recovered.surplus_or_deficit, 1);
}

#[test]
fn reward_pool_does_not_change_principal() {
    let f = Fixture::new();
    f.seed(1_000, 0, 0, 0);
    f.mint_to_vault(1_000);

    f.env.as_contract(&f.vault_id, || {
        balance::add_reward_pool_balance(&f.env, 300);
    });

    let report = f.vault.get_solvency_report();
    assert_eq!(report.total_principal, 1_000);
    assert_eq!(report.reward_pool, 300);
    assert_eq!(report.surplus_or_deficit, 0);
}
