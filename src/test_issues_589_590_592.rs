#![cfg(test)]
//! Tests for issues #589 (TTL management), #590 (`sweep_foreign_tokens`) and
//! #592 (`initialize` double-initialization guard).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::errors::VaultError;
use crate::foreign_token_sweep::SweepError;
use crate::ttl_management::TtlError;
use crate::vault::{VaultContract, VaultContractClient};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    admin: Address,
    token_addr: Address,
}

impl<'a> Fixture<'a> {
    /// `min_ttl` is the initial TTL of every new entry, so a small value lets
    /// a test move the ledger past it without extending anything.
    fn new(min_ttl: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = min_ttl;
            li.min_persistent_entry_ttl = min_ttl;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 1_000;
        });

        let admin = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            token_addr,
        }
    }

    fn set_ledger(&self, sequence: u32) {
        self.env.ledger().with_mut(|li| li.sequence_number = sequence);
    }

    fn mint(&self, token: &Address, to: &Address, amount: i128) {
        token::StellarAssetClient::new(&self.env, token).mint(to, &amount);
    }
}

// ── #592: initialize double-initialization guard ─────────────────────────────

#[test]
fn first_initialize_sets_the_expected_state() {
    let f = Fixture::new(10_000_000);
    assert_eq!(f.vault.get_admin(), f.admin);
    assert_eq!(f.vault.get_stake_token(), f.token_addr);
    assert_eq!(f.vault.get_reward_rate_bps(), 500);
}

#[test]
fn second_initialize_reverts_and_leaves_state_untouched() {
    let f = Fixture::new(10_000_000);
    let attacker = Address::generate(&f.env);
    let other_token = f.env.register_stellar_asset_contract(attacker.clone());

    let res = f
        .vault
        .try_initialize(&attacker, &other_token, &9_000_u32, &None, &None);
    assert_eq!(res, Err(Ok(VaultError::AlreadyInitialized)));

    assert_eq!(f.vault.get_admin(), f.admin);
    assert_eq!(f.vault.get_stake_token(), f.token_addr);
    assert_eq!(f.vault.get_reward_rate_bps(), 500);
}

// ── #590: sweep_foreign_tokens ───────────────────────────────────────────────

#[test]
fn sweeps_the_full_balance_of_an_unrelated_token() {
    let f = Fixture::new(10_000_000);
    let foreign = f.env.register_stellar_asset_contract(Address::generate(&f.env));
    let to = Address::generate(&f.env);
    f.mint(&foreign, &f.vault_id, 250);

    assert_eq!(f.vault.sweep_foreign_tokens(&f.admin, &foreign, &to), 250);

    let client = token::Client::new(&f.env, &foreign);
    assert_eq!(client.balance(&to), 250);
    assert_eq!(client.balance(&f.vault_id), 0);
}

#[test]
fn sweeping_the_vault_token_reverts_and_moves_nothing() {
    let f = Fixture::new(10_000_000);
    let to = Address::generate(&f.env);
    f.mint(&f.token_addr, &f.vault_id, 100);

    let res = f.vault.try_sweep_foreign_tokens(&f.admin, &f.token_addr, &to);
    assert_eq!(res, Err(Ok(SweepError::CannotSweepVaultToken)));

    let client = token::Client::new(&f.env, &f.token_addr);
    assert_eq!(client.balance(&f.vault_id), 100);
    assert_eq!(client.balance(&to), 0);
}

#[test]
fn sweeping_a_zero_balance_is_a_no_op() {
    let f = Fixture::new(10_000_000);
    let foreign = f.env.register_stellar_asset_contract(Address::generate(&f.env));
    let to = Address::generate(&f.env);

    assert_eq!(f.vault.sweep_foreign_tokens(&f.admin, &foreign, &to), 0);
    assert_eq!(token::Client::new(&f.env, &foreign).balance(&to), 0);
}

#[test]
fn only_the_admin_can_sweep() {
    let f = Fixture::new(10_000_000);
    let foreign = f.env.register_stellar_asset_contract(Address::generate(&f.env));
    let stranger = Address::generate(&f.env);
    f.mint(&foreign, &f.vault_id, 10);

    let res = f.vault.try_sweep_foreign_tokens(&stranger, &foreign, &stranger);
    assert_eq!(res, Err(Ok(SweepError::Unauthorized)));
}

// ── #589: TTL management ─────────────────────────────────────────────────────

#[test]
fn threshold_defaults_and_is_admin_settable() {
    let f = Fixture::new(10_000_000);
    assert_eq!(
        f.vault.get_ttl_extension_threshold(),
        crate::ttl_management::DEFAULT_TTL_THRESHOLD
    );
    f.vault.set_ttl_extension_threshold(&f.admin, &5_000_u32);
    assert_eq!(f.vault.get_ttl_extension_threshold(), 5_000);

    let stranger = Address::generate(&f.env);
    assert_eq!(
        f.vault.try_set_ttl_extension_threshold(&stranger, &5_000_u32),
        Err(Ok(TtlError::Unauthorized))
    );
    assert_eq!(
        f.vault.try_set_ttl_extension_threshold(&f.admin, &0_u32),
        Err(Ok(TtlError::InvalidThreshold))
    );
}

/// Control: with a 100-ledger initial TTL and no extension, the instance is
/// archived once the ledger moves past it.
#[test]
#[should_panic]
fn instance_expires_without_an_extension() {
    let f = Fixture::new(100);
    f.set_ledger(3_000);
    f.vault.get_reward_rate_bps();
}

#[test]
fn extend_contract_ttl_keeps_the_instance_alive_when_below_threshold() {
    let f = Fixture::new(100);
    f.vault.set_ttl_extension_threshold(&f.admin, &5_000_u32);
    f.vault.extend_contract_ttl(&Address::generate(&f.env));

    f.set_ledger(3_000);
    assert_eq!(f.vault.get_reward_rate_bps(), 500);
}

#[test]
fn extend_contract_ttl_is_a_no_op_when_already_above_threshold() {
    let f = Fixture::new(100);
    f.vault.set_ttl_extension_threshold(&f.admin, &5_000_u32);
    let keeper = Address::generate(&f.env);
    f.vault.extend_contract_ttl(&keeper);
    // Same ledger: the TTL is already at the threshold, so this must not fail.
    f.vault.extend_contract_ttl(&keeper);

    f.set_ledger(2_000);
    assert_eq!(f.vault.get_reward_rate_bps(), 500);
}

#[test]
fn deposit_bumps_the_instance_ttl() {
    let f = Fixture::new(100);
    f.vault.set_ttl_extension_threshold(&f.admin, &5_000_u32);
    let user = Address::generate(&f.env);
    f.mint(&f.token_addr, &user, 1_000);
    f.vault.deposit(&user, &1_000);

    f.set_ledger(3_000);
    assert_eq!(f.vault.get_reward_rate_bps(), 500);
}
