#![cfg(test)]
//! Tests for the validator node reward integration.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env, Symbol, TryFromVal, Vec,
};

use crate::{
    errors::VaultError,
    vault::{VaultContract, VaultContractClient},
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
    validator: Address,
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
        let validator = Address::generate(&env);

        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_addr);
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);
        token_admin.mint(&alice, &10_000_000);
        token_admin.mint(&bob, &10_000_000);
        // Mint extra tokens to the contract for validator reward payouts.
        // The vault contract address isn't known until after register_contract,
        // but we can mint to admin and transfer later.

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        // Fund the contract with tokens for validator reward payouts.
        token_admin.mint(&vault_id, &10_000_000);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            token,
            token_admin,
            admin,
            alice,
            bob,
            validator,
        }
    }
}

// ── set_validator_node / get_validator_node ──────────────────────────────────

#[test]
fn admin_can_set_validator_node() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    let node = f.vault.get_validator_node();
    assert_eq!(node, Some(f.validator));
}

#[test]
fn validator_node_defaults_to_none() {
    let f = Fixture::new();
    assert!(f.vault.get_validator_node().is_none());
}

// ── deposit_validator_rewards ────────────────────────────────────────────────

#[test]
fn validator_can_deposit() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.deposit_validator_rewards(&f.validator, &1_000);

    assert_eq!(f.vault.get_validator_reward_pool(), 1_000);
}

#[test]
fn deposit_without_validator_set_reverts() {
    let f = Fixture::new();

    let result = f.vault.try_deposit_validator_rewards(&f.validator, &1_000);
    assert_eq!(result, Err(Ok(VaultError::NotInitialized)));
}

#[test]
fn non_validator_deposit_rejected() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    // alice is not the validator node
    let result = f.vault.try_deposit_validator_rewards(&f.alice, &1_000);
    assert_eq!(result, Err(Ok(VaultError::Unauthorized)));
}

#[test]
fn zero_deposit_rejected() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    let result = f.vault.try_deposit_validator_rewards(&f.validator, &0);
    assert_eq!(result, Err(Ok(VaultError::ZeroAmount)));
}

#[test]
fn negative_deposit_rejected() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    let result = f.vault.try_deposit_validator_rewards(&f.validator, &-100);
    assert_eq!(result, Err(Ok(VaultError::ZeroAmount)));
}

#[test]
fn multiple_deposits_accumulate() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.deposit_validator_rewards(&f.validator, &500);
    f.vault.deposit_validator_rewards(&f.validator, &300);

    assert_eq!(f.vault.get_validator_reward_pool(), 800);
}

// ── distribute_validator_rewards ─────────────────────────────────────────────

#[test]
fn distribution_proportional_to_stake() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    // alice stakes 7_500, bob stakes 2_500 => 75%/25% split.
    f.vault.stake(&f.alice, &7_500, &0);
    f.vault.stake(&f.bob, &2_500, &0);

    f.vault.deposit_validator_rewards(&f.validator, &10_000);
    f.vault.distribute_validator_rewards();

    // alice gets 75% = 7_500, bob gets 25% = 2_500.
    assert_eq!(f.vault.get_validator_reward_balance(&f.alice), 7_500);
    assert_eq!(f.vault.get_validator_reward_balance(&f.bob), 2_500);

    // Pool should be zeroed.
    assert_eq!(f.vault.get_validator_reward_pool(), 0);
}

#[test]
fn distribution_with_equal_stakes() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.stake(&f.alice, &5_000, &0);
    f.vault.stake(&f.bob, &5_000, &0);

    f.vault.deposit_validator_rewards(&f.validator, &1_000);
    f.vault.distribute_validator_rewards();

    assert_eq!(f.vault.get_validator_reward_balance(&f.alice), 500);
    assert_eq!(f.vault.get_validator_reward_balance(&f.bob), 500);
}

#[test]
fn distribution_with_no_stakers_reverts() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.deposit_validator_rewards(&f.validator, &1_000);

    let result = f.vault.try_distribute_validator_rewards();
    assert_eq!(result, Err(Ok(VaultError::PositionNotFound)));
}

#[test]
fn distribution_with_empty_pool_reverts() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.stake(&f.alice, &5_000, &0);

    let result = f.vault.try_distribute_validator_rewards();
    assert_eq!(result, Err(Ok(VaultError::ZeroAmount)));
}

#[test]
fn multiple_distributions_accumulate_balance() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.stake(&f.alice, &10_000, &0);

    f.vault.deposit_validator_rewards(&f.validator, &1_000);
    f.vault.distribute_validator_rewards();

    f.vault.deposit_validator_rewards(&f.validator, &2_000);
    f.vault.distribute_validator_rewards();

    // alice gets 100% of both rounds: 1_000 + 2_000 = 3_000.
    assert_eq!(f.vault.get_validator_reward_balance(&f.alice), 3_000);
}

// ── claim_validator_rewards ──────────────────────────────────────────────────

#[test]
fn user_claims_correct_share() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.stake(&f.alice, &5_000, &0);
    f.vault.stake(&f.bob, &5_000, &0);

    f.vault.deposit_validator_rewards(&f.validator, &1_000);
    f.vault.distribute_validator_rewards();

    let claimed = f.vault.claim_validator_rewards(&f.alice);
    assert_eq!(claimed, 500);

    // Balance should be zero after claim.
    assert_eq!(f.vault.get_validator_reward_balance(&f.alice), 0);
}

#[test]
fn claim_with_no_balance_reverts() {
    let f = Fixture::new();

    let result = f.vault.try_claim_validator_rewards(&f.alice);
    assert_eq!(result, Err(Ok(VaultError::NothingToWithdraw)));
}

#[test]
fn claim_transfers_tokens() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.stake(&f.alice, &10_000, &0);

    f.vault.deposit_validator_rewards(&f.validator, &1_000);
    f.vault.distribute_validator_rewards();

    let balance_before = f.token.balance(&f.alice);
    f.vault.claim_validator_rewards(&f.alice);
    let balance_after = f.token.balance(&f.alice);

    assert_eq!(balance_after - balance_before, 1_000);
}

#[test]
fn claim_twice_only_pays_once() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.stake(&f.alice, &10_000, &0);

    f.vault.deposit_validator_rewards(&f.validator, &1_000);
    f.vault.distribute_validator_rewards();

    f.vault.claim_validator_rewards(&f.alice);

    let result = f.vault.try_claim_validator_rewards(&f.alice);
    assert_eq!(result, Err(Ok(VaultError::NothingToWithdraw)));
}

// ── validator_rewards_distributed event ──────────────────────────────────────

#[test]
fn distribution_emits_event() {
    let f = Fixture::new();
    f.vault.set_validator_node(&f.validator);

    f.vault.stake(&f.alice, &10_000, &0);
    f.vault.deposit_validator_rewards(&f.validator, &1_000);
    f.vault.distribute_validator_rewards();

    let events = f.env.events().all();
    let found = events.iter().any(|(topics, _data)| {
        match topics.get(0) {
            Some(val) => Symbol::try_from_val(&f.env, &val)
                .map(|t| t == Symbol::new(&f.env, "vr_dist"))
                .unwrap_or(false),
            None => false,
        }
    });
    assert!(found, "expected validator_rewards_distributed event");
}
