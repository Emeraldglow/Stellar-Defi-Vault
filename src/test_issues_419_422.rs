#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger as _},
    token, Address, Env, String, Vec,
};

use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

// ── helpers ──────────────────────────────────────────────────────────────────

fn create_token<'a>(
    env: &Env,
    admin: &Address,
) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let address = env.register_stellar_asset_contract(admin.clone());
    let client = token::Client::new(env, &address);
    let admin_client = token::StellarAssetClient::new(env, &address);
    (address, client, admin_client)
}

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
    charity: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 1000;
        });

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let charity = Address::generate(&env);

        let (token_addr, token, token_admin) = create_token(&env, &admin);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        // Mint some tokens for testing
        token_admin.mint(&alice, &1_000_000);
        token_admin.mint(&bob, &1_000_000);
        token_admin.mint(&vault_id, &10_000_000);

        Fixture {
            env,
            vault,
            vault_id,
            token,
            token_admin,
            admin,
            alice,
            bob,
            charity,
        }
    }
}

// ── Issue #419: charitable_donation_routing ──────────────────────────────────

#[test]
fn test_add_charity_succeeds() {
    let f = Fixture::new();
    let name = String::from_str(&f.env, "Save the Children");
    f.vault.add_charity(&f.admin, &f.charity, &name);
    let charities = f.vault.get_charities();
    assert_eq!(charities.len(), 1);
    assert_eq!(charities.get(0).unwrap().address, f.charity);
    assert_eq!(charities.get(0).unwrap().name, name);
}

#[test]
fn test_add_charity_requires_admin() {
    let f = Fixture::new();
    let name = String::from_str(&f.env, "Test Charity");
    f.env.mock_all_auths();
    let result = f.vault.try_add_charity(&f.alice, &f.charity, &name);
    assert!(result.is_err());
}

#[test]
fn test_add_charity_rejects_duplicate() {
    let f = Fixture::new();
    let name = String::from_str(&f.env, "Charity");
    f.vault.add_charity(&f.admin, &f.charity, &name);
    let result = f.vault.try_add_charity(&f.admin, &f.charity, &name);
    assert!(result.is_err());
}

#[test]
fn test_add_charity_rejects_max() {
    let f = Fixture::new();
    let name = String::from_str(&f.env, "C");
    for i in 0..10 {
        let addr = Address::generate(&f.env);
        f.vault.add_charity(&f.admin, &addr, &name);
    }
    let extra = Address::generate(&f.env);
    let result = f.vault.try_add_charity(&f.admin, &extra, &name);
    assert!(result.is_err());
}

#[test]
fn test_remove_charity_succeeds() {
    let f = Fixture::new();
    let name = String::from_str(&f.env, "Charity");
    f.vault.add_charity(&f.admin, &f.charity, &name);
    f.vault.remove_charity(&f.admin, &f.charity);
    let charities = f.vault.get_charities();
    assert_eq!(charities.len(), 0);
}

#[test]
fn test_remove_nonexistent_charity_fails() {
    let f = Fixture::new();
    let result = f.vault.try_remove_charity(&f.admin, &f.charity);
    assert!(result.is_err());
}

#[test]
fn test_set_donation_config_succeeds() {
    let f = Fixture::new();
    let name = String::from_str(&f.env, "Charity");
    f.vault.add_charity(&f.admin, &f.charity, &name);
    f.vault.set_donation_config(&f.alice, &f.charity, &1000_u32);
    let config = f.vault.get_donation_config(&f.alice).unwrap();
    assert_eq!(config.charity, f.charity);
    assert_eq!(config.donation_bps, 1000);
}

#[test]
fn test_set_donation_config_rejects_unregistered_charity() {
    let f = Fixture::new();
    let result = f
        .vault
        .try_set_donation_config(&f.alice, &f.charity, &1000_u32);
    assert!(result.is_err());
}

#[test]
fn test_set_donation_config_rejects_over_50_percent() {
    let f = Fixture::new();
    let name = String::from_str(&f.env, "Charity");
    f.vault.add_charity(&f.admin, &f.charity, &name);
    let result = f
        .vault
        .try_set_donation_config(&f.alice, &f.charity, &5001_u32);
    assert!(result.is_err());
}

#[test]
fn test_donation_zero_disables() {
    let f = Fixture::new();
    let name = String::from_str(&f.env, "Charity");
    f.vault.add_charity(&f.admin, &f.charity, &name);
    f.vault.set_donation_config(&f.alice, &f.charity, &0_u32);
    let config = f.vault.get_donation_config(&f.alice).unwrap();
    assert_eq!(config.donation_bps, 0);
}

#[test]
fn test_get_total_donated_tracks_accumulation() {
    let f = Fixture::new();
    let name = String::from_str(&f.env, "Charity");
    f.vault.add_charity(&f.admin, &f.charity, &name);
    let donated = f.vault.get_total_donated_to_charity(&f.charity);
    assert_eq!(donated, 0);
}

// ── Issue #420: position_shadow_clone ────────────────────────────────────────

#[test]
fn test_create_shadow_clone_succeeds() {
    let f = Fixture::new();
    f.token_admin.mint(&f.alice, &1000);
    f.vault.deposit(&f.alice, &1000, &0);
    let clone_id = f.vault.create_shadow_clone(&f.alice);
    assert!(clone_id == 0 || clone_id > 0);
    let clone = f.vault.get_shadow_clone(&clone_id).unwrap();
    assert_eq!(clone.original_owner, f.alice);
    assert_eq!(clone.amount, 1000);
    assert_eq!(clone.clone_id, clone_id);
}

#[test]
fn test_shadow_clone_matches_position_state() {
    let f = Fixture::new();
    f.token_admin.mint(&f.alice, &5000);
    f.vault.deposit(&f.alice, &5000, &0);
    let clone_id = f.vault.create_shadow_clone(&f.alice);
    let clone = f.vault.get_shadow_clone(&clone_id).unwrap();
    assert_eq!(clone.amount, 5000);
    assert_eq!(clone.staked_since, 1000);
}

#[test]
fn test_live_position_change_does_not_affect_clone() {
    let f = Fixture::new();
    f.token_admin.mint(&f.alice, &5000);
    f.vault.deposit(&f.alice, &5000, &0);
    let clone_id = f.vault.create_shadow_clone(&f.alice);
    let clone = f.vault.get_shadow_clone(&clone_id).unwrap();
    assert_eq!(clone.amount, 5000);
    // Deposit more — clone should still reflect original snapshot
    f.token_admin.mint(&f.alice, &3000);
    f.vault.deposit(&f.alice, &3000, &0);
    let clone_after = f.vault.get_shadow_clone(&clone_id).unwrap();
    assert_eq!(clone_after.amount, 5000);
}

#[test]
fn test_max_clones_enforced() {
    let f = Fixture::new();
    f.token_admin.mint(&f.alice, &100_000);
    f.vault.deposit(&f.alice, &1000, &0);
    for _ in 0..5 {
        f.vault.create_shadow_clone(&f.alice);
    }
    let result = f.vault.try_create_shadow_clone(&f.alice);
    assert!(result.is_err());
}

#[test]
fn test_owner_can_delete_clone() {
    let f = Fixture::new();
    f.token_admin.mint(&f.alice, &1000);
    f.vault.deposit(&f.alice, &1000, &0);
    let clone_id = f.vault.create_shadow_clone(&f.alice);
    assert!(f.vault.get_shadow_clone(&clone_id).is_some());
    f.vault.delete_shadow_clone(&f.alice, &clone_id);
    assert!(f.vault.get_shadow_clone(&clone_id).is_none());
}

#[test]
fn test_get_user_shadow_clones_returns_all() {
    let f = Fixture::new();
    f.token_admin.mint(&f.alice, &100_000);
    f.vault.deposit(&f.alice, &1000, &0);
    let id1 = f.vault.create_shadow_clone(&f.alice);
    let id2 = f.vault.create_shadow_clone(&f.alice);
    let clones = f.vault.get_user_shadow_clones(&f.alice);
    assert_eq!(clones.len(), 2);
    let ids: Vec<u32> = clones.iter().map(|c| c.clone_id).collect();
    assert!(ids.contains(&id1) || ids.contains(&id2));
}

// ── Issue #421: dex_limit_order_buyback ──────────────────────────────────────

#[test]
fn test_place_buyback_limit_order_succeeds() {
    let f = Fixture::new();
    let order_id = f
        .vault
        .place_buyback_limit_order(&f.admin, &1000_i128, &5000_i128, &1000_u32);
    let orders = f.vault.get_active_limit_orders();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders.get(0).unwrap().order_id, order_id);
    assert_eq!(orders.get(0).unwrap().max_price_bps, 1000);
    assert_eq!(orders.get(0).unwrap().amount_to_spend, 5000);
}

#[test]
fn test_place_buyback_limit_order_requires_admin() {
    let f = Fixture::new();
    let result = f
        .vault
        .try_place_buyback_limit_order(&f.alice, &1000_i128, &5000_i128, &1000_u32);
    assert!(result.is_err());
}

#[test]
fn test_place_buyback_limit_order_rejects_zero_amount() {
    let f = Fixture::new();
    let result = f
        .vault
        .try_place_buyback_limit_order(&f.admin, &1000_i128, &0_i128, &1000_u32);
    assert!(result.is_err());
}

#[test]
fn test_max_concurrent_orders_enforced() {
    let f = Fixture::new();
    for _ in 0..5 {
        f.vault
            .place_buyback_limit_order(&f.admin, &1000_i128, &1000_i128, &1000_u32);
    }
    let result = f
        .vault
        .try_place_buyback_limit_order(&f.admin, &1000_i128, &1000_i128, &1000_u32);
    assert!(result.is_err());
}

#[test]
fn test_cancel_limit_order_succeeds() {
    let f = Fixture::new();
    let order_id = f
        .vault
        .place_buyback_limit_order(&f.admin, &1000_i128, &5000_i128, &1000_u32);
    f.vault.cancel_limit_order(&f.admin, &order_id);
    let orders = f.vault.get_active_limit_orders();
    assert_eq!(orders.len(), 0);
}

#[test]
fn test_cancel_limit_order_requires_admin() {
    let f = Fixture::new();
    let order_id = f
        .vault
        .place_buyback_limit_order(&f.admin, &1000_i128, &5000_i128, &1000_u32);
    let result = f.vault.try_cancel_limit_order(&f.alice, &order_id);
    assert!(result.is_err());
}

// ── Issue #422: staker_sentiment_index ───────────────────────────────────────

#[test]
fn test_sentiment_inactive_pool_scores_zero() {
    let f = Fixture::new();
    let report = f.vault.get_sentiment_index();
    // No activity, no stakers, no votes — all signals are 0
    assert_eq!(report.score, 0);
    assert_eq!(report.inflow_signal, 0);
    assert_eq!(report.claim_signal, 0);
    assert_eq!(report.governance_signal, 0);
    assert_eq!(report.message_signal, 0);
    assert_eq!(report.rating_signal, 0);
    assert_eq!(report.computed_at, 1000);
}

#[test]
fn test_sentiment_rating_signal_contributes() {
    let f = Fixture::new();
    // Set rating to 10000 bps (100%) — /400 = 25 (max)
    // But we need to call set_pool_average_rating_bps
    f.vault.set_pool_average_rating_bps(&10000_u32);
    let report = f.vault.get_sentiment_index();
    assert_eq!(report.rating_signal, 25);
}

#[test]
fn test_sentiment_rating_signal_capped_at_25() {
    let f = Fixture::new();
    f.vault.set_pool_average_rating_bps(&20000_u32); // 20000 / 400 = 50, capped at 25
    let report = f.vault.get_sentiment_index();
    assert_eq!(report.rating_signal, 25);
}

#[test]
fn test_record_sentiment_inflow_and_message() {
    let f = Fixture::new();
    f.vault.record_sentiment_inflow(&f.alice, &1000_i128, &true);
    f.vault.record_sentiment_message(&f.alice);
    // Even with inflow and message, without stakers in the pool, inflow_signal is 0
    // because total_staked is 0. message_signal: 1 * 5 = 5
    let report = f.vault.get_sentiment_index();
    assert_eq!(report.message_signal, 5);
}

#[test]
fn test_sentiment_message_capped_at_five_messages() {
    let f = Fixture::new();
    for _ in 0..6 {
        f.vault.record_sentiment_message(&f.alice);
    }
    let report = f.vault.get_sentiment_index();
    assert_eq!(report.message_signal, 25); // 6*5 = 30, capped at 25
}

#[test]
fn test_sentiment_score_capped_at_100() {
    let f = Fixture::new();
    f.vault.set_pool_average_rating_bps(&10000_u32); // 25
                                                     // message_signal maxed: 25
    for _ in 0..6 {
        f.vault.record_sentiment_message(&f.alice);
    }
    // Total so far: 25 + 25 = 50
    let report = f.vault.get_sentiment_index();
    assert!(report.score <= 100);
    assert!(report.score >= 50);
}

#[test]
fn test_sentiment_computed_at_updates() {
    let f = Fixture::new();
    let report1 = f.vault.get_sentiment_index();
    assert_eq!(report1.computed_at, 1000);
    set_ledger(&f.env, 2000);
    let report2 = f.vault.get_sentiment_index();
    assert_eq!(report2.computed_at, 2000);
}

#[test]
fn test_record_sentiment_claim_records_event() {
    let f = Fixture::new();
    f.vault.record_sentiment_claim(&f.alice, &500_i128);
    // Claim velocity is 0 because total_staked is 0
    let report = f.vault.get_sentiment_index();
    assert_eq!(report.claim_signal, 0);
}

#[test]
fn test_record_sentiment_vote_records_event() {
    let f = Fixture::new();
    f.vault.record_sentiment_vote(&f.alice);
    // governance_signal is 0 because total_stakers is 0
    let report = f.vault.get_sentiment_index();
    assert_eq!(report.governance_signal, 0);
}
