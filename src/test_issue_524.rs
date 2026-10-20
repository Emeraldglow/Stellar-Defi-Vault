#![cfg(test)]
//! Tests for issue #524: configurable reward payout token.
//!
//! Covers the three behaviours the issue asks for: a pool that never configures
//! a reward token pays in the deposit token exactly as before, a configured
//! reward token is what gets paid out and what `fund_reward_pool` pulls, and
//! funding with anything other than the configured reward token reverts.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::balance;
use crate::errors::VaultError;
use crate::vault::{VaultContract, VaultContractClient};

fn create_token<'a>(
    env: &Env,
    admin: &Address,
) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let address = env.register_stellar_asset_contract(admin.clone());
    let client = token::Client::new(env, &address);
    let admin_client = token::StellarAssetClient::new(env, &address);
    (address, client, admin_client)
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    admin: Address,
    alice: Address,
    /// The deposit (stake) token.
    stake_addr: Address,
    stake: token::Client<'a>,
    stake_admin: token::StellarAssetClient<'a>,
    /// A second, independent token used as the configured reward token.
    reward_addr: Address,
    reward: token::Client<'a>,
    reward_admin: token::StellarAssetClient<'a>,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 1000;
        });

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);

        let (stake_addr, stake, stake_admin) = create_token(&env, &admin);
        let (reward_addr, reward, reward_admin) = create_token(&env, &admin);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &stake_addr, &0_u32, &None, &None);

        stake_admin.mint(&alice, &1_000_000);

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            alice,
            stake_addr,
            stake,
            stake_admin,
            reward_addr,
            reward,
            reward_admin,
        }
    }

    /// Gives `alice` an open position, so a claim has someone to pay.
    fn staked_user(&self) -> Address {
        self.vault.deposit(&self.alice, &100, &0);
        self.alice.clone()
    }

    /// Seeds a pending reward directly into contract storage (tests only),
    /// mirroring how other suites stand in for streaming accrual.
    fn seed_accrued_reward(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_accrued_reward(&self.env, user, amount);
        });
    }

    fn pending_reward(&self, user: &Address) -> i128 {
        self.env.as_contract(&self.vault_id, || {
            balance::get_accrued_reward(&self.env, user)
        })
    }

    fn reward_pool_balance(&self) -> i128 {
        self.env.as_contract(&self.vault_id, || {
            balance::get_reward_pool_balance(&self.env)
        })
    }

    /// Configures the second token as the reward payout asset.
    fn configure_reward_token(&self) {
        self.vault.set_reward_token(&self.admin, &self.reward_addr);
    }
}

// ── Default: no reward token configured ─────────────────────────────────────

#[test]
fn reward_token_defaults_to_the_deposit_token() {
    let f = Fixture::new();
    assert_eq!(f.vault.get_stake_token(), f.stake_addr);
    assert_eq!(
        f.vault.get_reward_token(),
        f.stake_addr,
        "an unconfigured pool must report the deposit token as its reward token"
    );
}

#[test]
fn claim_pays_in_the_deposit_token_when_none_is_configured() {
    let f = Fixture::new();
    let alice = f.staked_user();

    // Only the deposit token is held by the vault, so a reward-token payout
    // would have nothing to pay from.
    f.stake_admin.mint(&f.vault.address, &500);
    f.seed_accrued_reward(&alice, 100);

    let before = f.stake.balance(&alice);
    let claimed = f.vault.claim(&alice);

    assert_eq!(claimed, 100);
    assert_eq!(f.stake.balance(&alice), before + 100);
    assert_eq!(
        f.reward.balance(&alice),
        0,
        "the second token must be untouched when it is not the reward token"
    );
}

// ── Configured reward token ─────────────────────────────────────────────────

#[test]
fn set_reward_token_updates_the_query() {
    let f = Fixture::new();
    f.configure_reward_token();
    assert_eq!(f.vault.get_reward_token(), f.reward_addr);
    assert_eq!(
        f.vault.get_stake_token(),
        f.stake_addr,
        "the deposit token is unaffected by the reward-token choice"
    );
}

#[test]
fn set_reward_token_can_be_reset_to_the_deposit_token() {
    let f = Fixture::new();
    f.configure_reward_token();
    f.vault.set_reward_token(&f.admin, &f.stake_addr);
    assert_eq!(f.vault.get_reward_token(), f.stake_addr);
}

#[test]
fn set_reward_token_rejects_a_non_admin_caller() {
    let f = Fixture::new();
    let intruder = Address::generate(&f.env);
    // Real auth (not mocked) so the intruder cannot sign for the admin.
    f.env.set_auths(&[]);

    let res = f.vault.try_set_reward_token(&intruder, &f.reward_addr);
    assert!(res.is_err());
    assert_eq!(f.vault.get_reward_token(), f.stake_addr);
}

#[test]
fn set_reward_token_rejects_the_contract_address() {
    let f = Fixture::new();
    let res = f.vault.try_set_reward_token(&f.admin, &f.vault.address);
    assert_eq!(res, Err(Ok(VaultError::InvalidAddress)));
    assert_eq!(f.vault.get_reward_token(), f.stake_addr);
}

#[test]
fn claim_pays_in_the_configured_reward_token() {
    let f = Fixture::new();
    let alice = f.staked_user();
    f.configure_reward_token();

    // The vault holds the reward token, and no extra deposit token.
    f.reward_admin.mint(&f.vault.address, &500);
    f.seed_accrued_reward(&alice, 100);

    let stake_before = f.stake.balance(&alice);
    let claimed = f.vault.claim(&alice);

    assert_eq!(claimed, 100);
    assert_eq!(
        f.reward.balance(&alice),
        100,
        "the reward must be paid in the configured reward token"
    );
    assert_eq!(
        f.stake.balance(&alice),
        stake_before,
        "the deposit token balance must be untouched by a reward payout"
    );
    assert_eq!(f.pending_reward(&alice), 0);
}

#[test]
fn stake_and_claim_pays_in_the_configured_reward_token() {
    let f = Fixture::new();
    let alice = f.staked_user();
    f.configure_reward_token();
    f.reward_admin.mint(&f.vault.address, &500);
    f.seed_accrued_reward(&alice, 100);

    let claimed = f.vault.stake_and_claim(&alice, &10);

    assert_eq!(claimed, 100);
    assert_eq!(f.reward.balance(&alice), 100);
}

#[test]
fn claim_pays_in_the_reward_token_after_it_is_reconfigured() {
    let f = Fixture::new();
    let alice = f.staked_user();
    f.reward_admin.mint(&f.vault.address, &500);

    f.configure_reward_token();
    f.seed_accrued_reward(&alice, 100);
    f.vault.claim(&alice);
    assert_eq!(f.reward.balance(&alice), 100);

    // Back to the deposit token: the next claim must switch assets with it.
    f.vault.set_reward_token(&f.admin, &f.stake_addr);
    f.stake_admin.mint(&f.vault.address, &500);
    f.seed_accrued_reward(&alice, 40);
    let claimed = f.vault.claim(&alice);

    assert_eq!(claimed, 40);
    assert_eq!(f.reward.balance(&alice), 100, "no second reward payout");
    assert_eq!(f.stake.balance(&alice), 1_000_000 - 100 + 40);
}

// ── Funding the reward pool in the configured token ─────────────────────────

#[test]
fn fund_reward_pool_pulls_the_configured_reward_token() {
    let f = Fixture::new();
    f.configure_reward_token();
    f.reward_admin.mint(&f.admin, &1_000);

    let stake_before = f.stake.balance(&f.admin);
    f.vault.fund_reward_pool(&f.admin, &400);

    assert_eq!(f.reward.balance(&f.vault.address), 400);
    assert_eq!(f.reward.balance(&f.admin), 600);
    assert_eq!(
        f.stake.balance(&f.admin),
        stake_before,
        "funding must not touch the deposit token once rewards are decoupled"
    );
    assert_eq!(f.reward_pool_balance(), 400);
}

#[test]
fn fund_reward_pool_pulls_the_deposit_token_when_unset() {
    let f = Fixture::new();
    f.stake_admin.mint(&f.admin, &1_000);

    f.vault.fund_reward_pool(&f.admin, &250);

    assert_eq!(f.stake.balance(&f.vault.address), 250);
    assert_eq!(f.stake.balance(&f.admin), 750);
    assert_eq!(f.reward.balance(&f.vault.address), 0);
    assert_eq!(f.reward_pool_balance(), 250);
}

#[test]
fn funding_reverts_when_the_funder_only_holds_the_deposit_token() {
    let f = Fixture::new();
    f.configure_reward_token();
    // The admin holds deposit tokens but none of the configured reward token,
    // so the funding transfer cannot be sourced.
    f.stake_admin.mint(&f.admin, &1_000);

    let res = f.vault.try_fund_reward_pool(&f.admin, &400);
    assert!(res.is_err());
    assert_eq!(f.reward_pool_balance(), 0);
    assert_eq!(f.stake.balance(&f.vault.address), 0);
}

// ── Partial claims ──────────────────────────────────────────────────────────

#[test]
fn claim_partial_pays_the_configured_reward_token_and_leaves_the_rest() {
    let f = Fixture::new();
    let alice = f.staked_user();
    f.configure_reward_token();
    f.reward_admin.mint(&f.vault.address, &500);
    f.seed_accrued_reward(&alice, 300);

    let claimed = f.vault.claim_partial(&alice, &120);

    assert_eq!(claimed, 120);
    assert_eq!(f.reward.balance(&alice), 120);
    assert_eq!(f.stake.balance(&alice), 1_000_000 - 100);
    assert_eq!(
        f.pending_reward(&alice),
        180,
        "the unclaimed remainder stays pending"
    );

    // The remainder is still claimable, and still in the reward token.
    assert_eq!(f.vault.claim_partial(&alice, &180), 180);
    assert_eq!(f.reward.balance(&alice), 300);
    assert_eq!(f.pending_reward(&alice), 0);
}

#[test]
fn claim_partial_defaults_to_the_deposit_token() {
    let f = Fixture::new();
    let alice = f.staked_user();
    f.stake_admin.mint(&f.vault.address, &500);
    f.seed_accrued_reward(&alice, 300);

    assert_eq!(f.vault.claim_partial(&alice, &120), 120);
    assert_eq!(f.stake.balance(&alice), 1_000_000 - 100 + 120);
    assert_eq!(f.reward.balance(&alice), 0);
}

#[test]
fn claim_partial_rejects_more_than_the_pending_reward() {
    let f = Fixture::new();
    let alice = f.staked_user();
    f.configure_reward_token();
    f.reward_admin.mint(&f.vault.address, &500);
    f.seed_accrued_reward(&alice, 100);

    let res = f.vault.try_claim_partial(&alice, &101);
    assert_eq!(res, Err(Ok(VaultError::InsufficientRewardPool)));
    assert_eq!(f.pending_reward(&alice), 100, "state is left untouched");
    assert_eq!(f.reward.balance(&alice), 0);
}

#[test]
fn claim_partial_rejects_non_positive_amounts() {
    let f = Fixture::new();
    let alice = f.staked_user();
    f.seed_accrued_reward(&alice, 100);

    assert_eq!(
        f.vault.try_claim_partial(&alice, &0),
        Err(Ok(VaultError::ZeroAmount))
    );
    assert_eq!(
        f.vault.try_claim_partial(&alice, &-5),
        Err(Ok(VaultError::ZeroAmount))
    );
    assert_eq!(f.pending_reward(&alice), 100);
}
