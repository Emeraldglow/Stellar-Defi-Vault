#![cfg(test)]
//! Tests for automatic fee recycling into the reward pool (issue #754).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger as _},
    Address, Env, Symbol, TryFromVal, Val, Vec,
};

use crate::{
    balance,
    vault::{fee_recycle::MAX_FEE_RECYCLE_BPS, VaultContract, VaultContractClient},
};

fn topic_matches(env: &Env, topics: &Vec<Val>, name: &str) -> bool {
    match topics.get(0) {
        Some(val) => Symbol::try_from_val(env, &val)
            .map(|topic| topic == Symbol::new(env, name))
            .unwrap_or(false),
        None => false,
    }
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

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            alice,
            bob,
        }
    }
}

#[test]
fn fee_recycle_bps_defaults_to_zero() {
    let f = Fixture::new();
    assert_eq!(f.vault.get_fee_recycle_bps(), 0);
    let (recycled, remainder) = f.vault.preview_fee_recycle(&10_000_i128);
    assert_eq!(recycled, 0);
    assert_eq!(remainder, 10_000);
}

#[test]
fn set_fee_recycle_bps_requires_admin() {
    let f = Fixture::new();
    assert!(f
        .vault
        .try_set_fee_recycle_bps(&f.alice, &5_000_u32)
        .is_err());
    assert_eq!(f.vault.get_fee_recycle_bps(), 0);
}

#[test]
fn set_fee_recycle_bps_rejects_above_max() {
    let f = Fixture::new();
    assert!(f
        .vault
        .try_set_fee_recycle_bps(&f.admin, &(MAX_FEE_RECYCLE_BPS + 1))
        .is_err());
    assert_eq!(f.vault.get_fee_recycle_bps(), 0);
}

#[test]
fn set_fee_recycle_bps_accepts_full_share() {
    let f = Fixture::new();
    f.vault.set_fee_recycle_bps(&f.admin, &MAX_FEE_RECYCLE_BPS);
    assert_eq!(f.vault.get_fee_recycle_bps(), MAX_FEE_RECYCLE_BPS);
}

#[test]
fn preview_recycles_configured_share() {
    let f = Fixture::new();
    f.vault.set_fee_recycle_bps(&f.admin, &5_000_u32);

    let (recycled, remainder) = f.vault.preview_fee_recycle(&15_000_i128);
    assert_eq!(recycled, 7_500);
    assert_eq!(remainder, 7_500);

    // Flooring: a tiny fee rounds down to zero rather than stealing principal.
    let (recycled, remainder) = f.vault.preview_fee_recycle(&1_i128);
    assert_eq!(recycled, 0);
    assert_eq!(remainder, 1);
}

#[test]
fn preview_zero_bps_preserves_fee() {
    let f = Fixture::new();
    f.vault.set_fee_recycle_bps(&f.admin, &0_u32);

    let (recycled, remainder) = f.vault.preview_fee_recycle(&15_000_i128);
    assert_eq!(recycled, 0);
    assert_eq!(remainder, 15_000);
}

#[test]
fn recycle_credits_pool_and_emits_event() {
    let f = Fixture::new();
    f.vault.set_fee_recycle_bps(&f.admin, &5_000_u32);
    f.env.events().all();

    let (recycled, pool_after) = f.env.as_contract(&f.vault_id, || {
        balance::set_reward_pool_balance(&f.env, 1_000);
        let recycled =
            crate::vault::fee_recycle::recycle_fee(&f.env, 10_000).expect("recycle should succeed");
        (recycled, balance::get_reward_pool_balance(&f.env))
    });

    assert_eq!(recycled, 5_000);
    assert_eq!(pool_after, 6_000);

    let events = f.env.events().all();
    let recycle_events: std::vec::Vec<_> = events
        .into_iter()
        .filter(|(_, topics, _)| topic_matches(&f.env, topics, "fees_recycled"))
        .collect();
    assert_eq!(recycle_events.len(), 1);

    let data = Vec::<Val>::try_from_val(&f.env, &recycle_events[0].2).unwrap();
    let amount = i128::try_from_val(&f.env, &data.get(0).unwrap()).unwrap();
    let new_pool = i128::try_from_val(&f.env, &data.get(1).unwrap()).unwrap();
    assert_eq!(amount, 5_000);
    assert_eq!(new_pool, 6_000);
}

#[test]
fn recycle_is_a_noop_when_disabled_or_empty() {
    let f = Fixture::new();
    let (recycled, pool_after) = f.env.as_contract(&f.vault_id, || {
        balance::set_reward_pool_balance(&f.env, 1_000);
        let recycled =
            crate::vault::fee_recycle::recycle_fee(&f.env, 10_000).expect("recycle should succeed");
        (recycled, balance::get_reward_pool_balance(&f.env))
    });

    assert_eq!(recycled, 0);
    assert_eq!(pool_after, 1_000);
}

#[test]
fn composes_with_insurance_and_treasury_split() {
    let f = Fixture::new();
    let gross_fee = 15_000_i128;
    let insurance_bps = 2_000_i128;
    let recycle_bps = 5_000_u32;

    f.vault.set_fee_recycle_bps(&f.admin, &recycle_bps);

    let recipients = Vec::from_array(&f.env, [f.alice.clone()]);
    let shares = Vec::from_array(&f.env, [10_000_u32]);
    f.vault.set_treasury_split(&f.admin, &recipients, &shares);

    // Documented order: insurance off the gross fee, then recycling off the
    // post-insurance remainder, then the treasury split on what is left.
    let insurance_share = gross_fee * insurance_bps / 10_000;
    let recyclable_fee = gross_fee - insurance_share;
    let (recycled, distributable_fee) = f.vault.preview_fee_recycle(&recyclable_fee);

    assert_eq!(insurance_share, 3_000);
    assert_eq!(recycled, 6_000);
    assert_eq!(distributable_fee, 6_000);

    let split = f.vault.get_treasury_split().expect("split configured");
    assert_eq!(split.bps_shares.get(0).unwrap(), 10_000);
    let recipient_amount = distributable_fee * split.bps_shares.get(0).unwrap() as i128 / 10_000;
    assert_eq!(recipient_amount, 6_000);
}

#[test]
fn zero_bps_preserves_full_treasury_amount() {
    let f = Fixture::new();
    let recipients = Vec::from_array(&f.env, [f.bob.clone()]);
    let shares = Vec::from_array(&f.env, [10_000_u32]);
    f.vault.set_treasury_split(&f.admin, &recipients, &shares);

    let (recycled, distributable_fee) = f.vault.preview_fee_recycle(&15_000_i128);
    assert_eq!(recycled, 0);
    assert_eq!(distributable_fee, 15_000);

    let split = f.vault.get_treasury_split().expect("split configured");
    let recipient_amount = distributable_fee * split.bps_shares.get(0).unwrap() as i128 / 10_000;
    assert_eq!(recipient_amount, 15_000);
}

#[test]
fn admin_can_disable_recycling_after_enabling_it() {
    let f = Fixture::new();
    f.vault.set_fee_recycle_bps(&f.admin, &7_500_u32);
    assert_eq!(f.vault.get_fee_recycle_bps(), 7_500);

    f.vault.set_fee_recycle_bps(&f.admin, &0_u32);
    assert_eq!(f.vault.get_fee_recycle_bps(), 0);
    let (recycled, remainder) = f.vault.preview_fee_recycle(&8_000_i128);
    assert_eq!(recycled, 0);
    assert_eq!(remainder, 8_000);
}
