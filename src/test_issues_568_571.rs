#![cfg(test)]
//! Tests for issues #568-#571:
//! - #568 unique depositor count cap
//! - #569 reward-claim gas rebate
//! - #570 deposit-side preview
//! - #571 indexing recommendations

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

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
    #[allow(dead_code)]
    token_addr: Address,
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
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
        let (token_addr, token, token_admin) = create_token(&env, &admin);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            token_addr,
            token,
            token_admin,
        }
    }

    fn funded_user(&self, amount: i128) -> Address {
        let user = Address::generate(&self.env);
        self.token_admin.mint(&user, &amount);
        user
    }

    fn back_vault_with_rewards(&self, amount: i128) {
        self.token_admin.mint(&self.vault.address, &amount);
    }

    /// Seeds a pending reward directly into contract storage (tests only).
    fn seed_accrued_reward(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            crate::balance::set_accrued_reward(&self.env, user, amount);
        });
    }

    /// Overwrites the pool's share/deposit counters to create a non-1:1 ratio
    /// (tests only).
    fn seed_share_ratio(&self, total_shares: i128, total_deposited: i128) {
        self.env.as_contract(&self.vault_id, || {
            crate::balance::set_total_shares(&self.env, total_shares);
            crate::balance::set_total_deposited(&self.env, total_deposited);
        });
    }
}

// ── Issue #568: unique depositor count cap ─────────────────────────────────

#[test]
fn depositor_cap_blocks_only_new_addresses() {
    let f = Fixture::new();
    let alice = f.funded_user(1_000);
    let bob = f.funded_user(1_000);
    let carol = f.funded_user(1_000);

    f.vault.set_max_depositor_count(&f.admin, &2_u32);
    assert_eq!(f.vault.get_max_depositor_count(), 2);

    f.vault.deposit(&alice, &100, &0);
    f.vault.deposit(&bob, &100, &0);
    assert_eq!(f.vault.get_depositor_count(), 2);

    // A brand-new address is rejected once the cap is met.
    let res = f.vault.try_deposit(&carol, &100, &None);
    assert_eq!(res, Err(Ok(VaultError::DepositorCapReached)));
    assert_eq!(f.vault.get_depositor_count(), 2);

    // Existing depositors can still add to their own position.
    f.vault.deposit(&alice, &50, &0);
    assert_eq!(f.vault.get_depositor_count(), 2);
}

#[test]
fn depositor_cap_zero_disables_the_limit() {
    let f = Fixture::new();
    f.vault.set_max_depositor_count(&f.admin, &0_u32);

    for _ in 0..3 {
        let user = f.funded_user(100);
        f.vault.deposit(&user, &10, &0);
    }
    assert_eq!(f.vault.get_depositor_count(), 3);
}

#[test]
fn stake_and_claim_honours_the_depositor_cap() {
    let f = Fixture::new();
    let alice = f.funded_user(1_000);
    let bob = f.funded_user(1_000);

    f.vault.set_max_depositor_count(&f.admin, &1_u32);
    f.vault.stake_and_claim(&alice, &100);

    let res = f.vault.try_stake_and_claim(&bob, &100);
    assert_eq!(res, Err(Ok(VaultError::DepositorCapReached)));
    assert_eq!(f.vault.get_depositor_count(), 1);
}

// ── Issue #569: reward-claim gas rebate ────────────────────────────────────

#[test]
fn gas_rebate_is_paid_when_the_pool_is_funded() {
    let f = Fixture::new();
    let alice = f.funded_user(1_000);
    f.vault.deposit(&alice, &100, &0);

    f.token_admin.mint(&f.admin, &60);
    f.vault.fund_gas_rebate_pool(&f.admin, &60);
    f.vault.set_gas_rebate_amount(&f.admin, &25);
    assert_eq!(f.vault.get_gas_rebate_pool(), 60);

    f.back_vault_with_rewards(200);
    f.seed_accrued_reward(&alice, 100);

    let before = f.token.balance(&alice);
    let claimed = f.vault.claim(&alice);
    // The claim's return value stays the reward; the rebate is paid alongside.
    assert_eq!(claimed, 100);
    assert_eq!(f.token.balance(&alice), before + 100 + 25);
    assert_eq!(f.vault.get_gas_rebate_pool(), 35);
}

#[test]
fn claim_succeeds_without_a_rebate_when_the_pool_is_empty() {
    let f = Fixture::new();
    let alice = f.funded_user(1_000);
    f.vault.deposit(&alice, &100, &0);

    // Rebate configured but never funded.
    f.vault.set_gas_rebate_amount(&f.admin, &25);
    assert_eq!(f.vault.get_gas_rebate_pool(), 0);

    f.back_vault_with_rewards(200);
    f.seed_accrued_reward(&alice, 100);

    let before = f.token.balance(&alice);
    let claimed = f.vault.claim(&alice);
    assert_eq!(claimed, 100);
    assert_eq!(f.token.balance(&alice), before + 100);
    assert_eq!(f.vault.get_gas_rebate_pool(), 0);
}

#[test]
fn gas_rebate_pool_depletes_one_claim_at_a_time() {
    let f = Fixture::new();
    let alice = f.funded_user(1_000);
    f.vault.deposit(&alice, &100, &0);

    f.token_admin.mint(&f.admin, &50);
    f.vault.fund_gas_rebate_pool(&f.admin, &50);
    f.vault.set_gas_rebate_amount(&f.admin, &25);
    f.back_vault_with_rewards(1_000);

    f.seed_accrued_reward(&alice, 100);
    f.vault.claim(&alice);
    assert_eq!(f.vault.get_gas_rebate_pool(), 25);

    f.seed_accrued_reward(&alice, 100);
    f.vault.claim(&alice);
    assert_eq!(f.vault.get_gas_rebate_pool(), 0);

    // A third claim with an empty pool still succeeds, just without a rebate.
    f.seed_accrued_reward(&alice, 100);
    let before = f.token.balance(&alice);
    let claimed = f.vault.claim(&alice);
    assert_eq!(claimed, 100);
    assert_eq!(f.token.balance(&alice), before + 100);
    assert_eq!(f.vault.get_gas_rebate_pool(), 0);
}

#[test]
fn zero_rebate_amount_disables_rebates() {
    let f = Fixture::new();
    let alice = f.funded_user(1_000);
    f.vault.deposit(&alice, &100, &0);

    f.token_admin.mint(&f.admin, &60);
    f.vault.fund_gas_rebate_pool(&f.admin, &60);
    // Default rebate amount is 0 = disabled.
    assert_eq!(f.vault.get_gas_rebate_amount(), 0);

    f.back_vault_with_rewards(200);
    f.seed_accrued_reward(&alice, 100);

    let before = f.token.balance(&alice);
    f.vault.claim(&alice);
    assert_eq!(f.token.balance(&alice), before + 100);
    assert_eq!(f.vault.get_gas_rebate_pool(), 60);
}

// ── Issue #570: deposit-side preview ───────────────────────────────────────

#[test]
fn preview_deposit_matches_a_real_deposit() {
    let f = Fixture::new();
    let alice = f.funded_user(10_000);

    // Empty pool: first deposit is 1:1.
    assert_eq!(f.vault.preview_deposit(&100), 100);
    let minted = f.vault.deposit(&alice, &100, &0);
    assert_eq!(minted, 100);

    // Seed a non-1:1 share ratio to prove the preview tracks the real math.
    f.seed_share_ratio(200, 100);

    let preview = f.vault.preview_deposit(&250);
    assert_eq!(preview, 500);
    let minted_again = f.vault.deposit(&alice, &250, &0);
    assert_eq!(preview, minted_again);
}

#[test]
fn preview_deposit_is_read_only_and_safe_for_bad_amounts() {
    let f = Fixture::new();
    let alice = f.funded_user(100);
    f.vault.deposit(&alice, &100, &0);

    let shares_before = f.vault.shares_of(&alice);
    assert_eq!(f.vault.preview_deposit(&0), 0);
    assert_eq!(f.vault.preview_deposit(&-5), 0);
    // No state changed.
    assert_eq!(f.vault.shares_of(&alice), shares_before);
}

// ── Issue #571: indexing recommendations ───────────────────────────────────

#[test]
fn indexing_recommendations_are_non_empty_and_stable() {
    let f = Fixture::new();
    let first = f.vault.get_indexing_recommendations();
    let second = f.vault.get_indexing_recommendations();

    assert!(first.len() > 0);
    assert_eq!(first, second);
}
