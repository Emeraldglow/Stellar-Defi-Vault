#![cfg(test)]
//! Tests for the consecutive-claim streak bonus (issue #755).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
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
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        // The vault holds the tokens the streak claim pays out from.
        token::StellarAssetClient::new(&env, &token_addr).mint(&vault_id, &1_000_000);
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

    fn configure(&self, period_ledgers: u32, bonus_bps: u32, max_streak: u32) {
        self.vault
            .set_streak_config(&self.admin, &period_ledgers, &bonus_bps, &max_streak);
    }

    /// Give `user` a claimable reward and fund the reward pool directly.
    fn seed_claim(&self, user: &Address, accrued: i128, pool: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_accrued_reward(&self.env, user, accrued);
            balance::set_reward_pool_balance(&self.env, pool);
        });
    }
}

#[test]
fn streak_increments_on_consecutive_period_claims() {
    let f = Fixture::new();
    // 10% bonus per streak level, periods of 100 ledgers, cap of 5.
    f.configure(100, 1_000, 5);

    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 10); // 1 * 10% of 100
    assert_eq!(f.vault.get_claim_streak(&f.alice), 1);

    set_ledger(&f.env, 1_100);
    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 20); // 2 * 10% of 100
    assert_eq!(f.vault.get_claim_streak(&f.alice), 2);

    set_ledger(&f.env, 1_200);
    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 30); // 3 * 10% of 100
    assert_eq!(f.vault.get_claim_streak(&f.alice), 3);
}

#[test]
fn streak_resets_when_a_period_is_skipped() {
    let f = Fixture::new();
    f.configure(100, 1_000, 5);

    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 10);
    assert_eq!(f.vault.get_claim_streak(&f.alice), 1);

    // Jump two periods ahead: period 11 is skipped.
    set_ledger(&f.env, 1_200);
    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 10);
    assert_eq!(f.vault.get_claim_streak(&f.alice), 1);
}

#[test]
fn bonus_is_capped_by_max_streak() {
    let f = Fixture::new();
    // 10% per level but the multiplier stops at 2.
    f.configure(100, 1_000, 2);

    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 10);

    set_ledger(&f.env, 1_100);
    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 20);

    set_ledger(&f.env, 1_200);
    f.seed_claim(&f.alice, 100, 1_000_000);
    // Streak and bonus stop growing once the cap is reached.
    assert_eq!(f.vault.get_claim_streak(&f.alice), 2);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 20);

    set_ledger(&f.env, 1_300);
    assert_eq!(f.vault.get_claim_streak_bonus_bps(&f.alice), 2_000);
}

#[test]
fn second_claim_in_same_period_does_not_double_count() {
    let f = Fixture::new();
    f.configure(100, 1_000, 5);

    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 10);
    assert_eq!(f.vault.get_claim_streak(&f.alice), 1);

    // Same period: reward is still paid, but the streak and bonus are not.
    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 0);
    assert_eq!(f.vault.get_claim_streak(&f.alice), 1);

    // Next period resumes the streak exactly once.
    set_ledger(&f.env, 1_100);
    f.seed_claim(&f.alice, 100, 1_000_000);
    assert_eq!(f.vault.claim_with_streak_bonus(&f.alice), 20);
    assert_eq!(f.vault.get_claim_streak(&f.alice), 2);
}

#[test]
fn claim_requires_a_configured_program() {
    let f = Fixture::new();
    f.seed_claim(&f.alice, 100, 1_000_000);

    let result = f.vault.try_claim_with_streak_bonus(&f.alice);
    assert!(result.is_err());
    assert_eq!(f.vault.get_claim_streak(&f.alice), 0);
}

#[test]
fn only_admin_can_configure_the_streak() {
    let f = Fixture::new();
    let result = f
        .vault
        .try_set_streak_config(&f.alice, &100_u32, &1_000_u32, &5_u32);
    assert!(result.is_err());
    assert!(f.vault.get_streak_config().is_none());
}

#[test]
fn invalid_config_is_rejected() {
    let f = Fixture::new();

    assert!(f
        .vault
        .try_set_streak_config(&f.admin, &0_u32, &1_000_u32, &5_u32)
        .is_err());
    assert!(f
        .vault
        .try_set_streak_config(&f.admin, &100_u32, &1_000_u32, &0_u32)
        .is_err());
    assert!(f
        .vault
        .try_set_streak_config(&f.admin, &100_u32, &10_001_u32, &5_u32)
        .is_err());
}

#[test]
fn claim_reverts_when_pool_cannot_cover_bonus() {
    let f = Fixture::new();
    f.configure(100, 1_000, 5);

    // Exactly enough for the reward, but not the 10% bonus.
    f.seed_claim(&f.alice, 100, 100);

    let result = f.vault.try_claim_with_streak_bonus(&f.alice);
    assert!(result.is_err());
    // State is unchanged on failure.
    assert_eq!(f.vault.get_claim_streak(&f.alice), 0);
}

#[test]
fn streak_is_tracked_per_user() {
    let f = Fixture::new();
    f.configure(100, 1_000, 5);

    f.seed_claim(&f.alice, 100, 1_000_000);
    f.vault.claim_with_streak_bonus(&f.alice);

    set_ledger(&f.env, 1_100);
    // Bob has never claimed, so his streak starts at 1 independently.
    f.seed_claim(&f.bob, 100, 1_000_000);
    f.vault.claim_with_streak_bonus(&f.bob);

    assert_eq!(f.vault.get_claim_streak(&f.alice), 1);
    assert_eq!(f.vault.get_claim_streak(&f.bob), 1);
}
