#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, Env, String, Vec,
};

use crate::{
    errors::{VaultBoostGaugeError, VaultError},
    storage::ProposableParam,
    vault::{VaultContract, VaultContractClient},
};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
}

impl<'a> Fixture<'a> {
    fn new(reward_rate_bps: u32) -> Self {
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

        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_addr);
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &reward_rate_bps, &None, &None);

        token_admin.mint(&alice, &10_000_000);
        token_admin.mint(&bob, &10_000_000);
        token_admin.mint(&admin, &10_000_000);

        Fixture {
            env,
            vault,
            vault_id,
            token,
            token_admin,
            admin,
            alice,
            bob,
        }
    }

    fn advance(&self, ledgers: u32) {
        self.env.ledger().with_mut(|li| {
            li.sequence_number += ledgers;
        });
    }
}

// ── Issue #757: Referral stats & claimable earnings ──────────────────────────

#[test]
fn test_referral_stats_and_claim_earnings() {
    let f = Fixture::new(1000);
    f.vault.stake(&f.alice, &100_000);
    f.vault.register_referrer(&f.alice);
    f.vault.set_referral_bonus_bps(&f.admin, &500, &250);
    f.vault.fund_reward_pool(&f.admin, &100_000);

    let alice_before = f.token.balance(&f.alice);
    let bob_before = f.token.balance(&f.bob);

    // Bob deposits 20_000 with Alice as referrer
    f.vault.deposit(&f.bob, &20_000, &Some(f.alice.clone()));

    // Earnings accrue to claimable balance rather than being paid immediately
    assert_eq!(f.token.balance(&f.alice), alice_before);
    assert_eq!(f.token.balance(&f.bob), bob_before - 20_000 + 500);

    // Verify stats
    let stats = f.vault.get_referral_stats(&f.alice);
    assert_eq!(stats.referee_count, 1);
    assert_eq!(stats.total_referred_deposits, 20_000);
    assert_eq!(stats.earned, 1_000);
    assert_eq!(stats.claimed, 0);

    // Claim referral earnings
    let claimed = f.vault.claim_referral_earnings(&f.alice);
    assert_eq!(claimed, 1_000);
    assert_eq!(f.token.balance(&f.alice), alice_before + 1_000);

    // Stats show claimed balance updated
    let stats_after = f.vault.get_referral_stats(&f.alice);
    assert_eq!(stats_after.earned, 1_000);
    assert_eq!(stats_after.claimed, 1_000);

    // Subsequent claim returns 0
    let second_claim = f.vault.claim_referral_earnings(&f.alice);
    assert_eq!(second_claim, 0);
}

// ── Issue #758: Vote-escrow style reward boost ────────────────────────────────

#[test]
fn test_ve_boost_linear_decay_and_max_cap() {
    let f = Fixture::new(1000);
    // Config: max duration = 100_000 ledgers, max boost = 5000 bps (50%), scale = 10_000
    f.vault.set_ve_boost_config(&f.admin, &100_000, &5_000, &10_000);

    f.vault.lock_for_boost(&f.bob, &10_000, &100_000);

    // At full duration (100_000 remaining), boost is at maximum cap (5000 bps)
    assert_eq!(f.vault.get_ve_boost_bps(&f.bob), 5_000);

    // Advance 50_000 ledgers (50% remaining)
    f.advance(50_000);
    assert_eq!(f.vault.get_ve_boost_bps(&f.bob), 2_500);

    // Advance another 25_000 ledgers (25% remaining)
    f.advance(25_000);
    assert_eq!(f.vault.get_ve_boost_bps(&f.bob), 1_250);

    // Advance past expiry
    f.advance(25_000);
    assert_eq!(f.vault.get_ve_boost_bps(&f.bob), 0);
}

#[test]
fn test_ve_boost_early_unlock_reverts_and_successful_unlock() {
    let f = Fixture::new(1000);
    let bob_before = f.token.balance(&f.bob);

    f.vault.lock_for_boost(&f.bob, &5_000, &10_000);
    assert_eq!(f.token.balance(&f.bob), bob_before - 5_000);

    // Early unlock attempt fails
    let err = f.vault.try_unlock_boost(&f.bob);
    assert_eq!(err, Err(Ok(VaultBoostGaugeError::LockNotExpired)));

    // Advance to maturity
    f.advance(10_000);

    // Unlock succeeds and returns locked tokens
    let unlocked = f.vault.unlock_boost(&f.bob);
    assert_eq!(unlocked, 5_000);
    assert_eq!(f.token.balance(&f.bob), bob_before);

    // Boost is zero
    assert_eq!(f.vault.get_ve_boost_bps(&f.bob), 0);
}

// ── Issue #759: Gauge voting for emissions ────────────────────────────────────

#[test]
fn test_gauge_voting_directs_weights_and_revoting() {
    let f = Fixture::new(1000);
    f.vault.stake(&f.alice, &100_000);

    // Alice votes 60% sub-pool 1, 40% sub-pool 2
    let alloc = vec![&f.env, (1u32, 6_000u32), (2u32, 4_000u32)];
    f.vault.vote_gauge(&f.alice, &alloc);

    let weights = f.vault.get_gauge_weights();
    assert_eq!(weights.len(), 2);
    assert_eq!(weights.get(0).unwrap(), (1, 6_000));
    assert_eq!(weights.get(1).unwrap(), (2, 4_000));

    // Re-voting replaces prior allocation
    let new_alloc = vec![&f.env, (1u32, 2_000u32), (2u32, 8_000u32)];
    f.vault.vote_gauge(&f.alice, &new_alloc);

    let new_weights = f.vault.get_gauge_weights();
    assert_eq!(new_weights.get(0).unwrap(), (1, 2_000));
    assert_eq!(new_weights.get(1).unwrap(), (2, 8_000));
}

#[test]
fn test_gauge_voting_bounds_and_admin_override() {
    let f = Fixture::new(1000);
    f.vault.stake(&f.alice, &100_000);

    let alloc = vec![&f.env, (1u32, 2_000u32), (2u32, 8_000u32)];
    f.vault.vote_gauge(&f.alice, &alloc);

    // Admin sets bounds on pool 1: min 3000 bps, max 7000 bps
    f.vault.set_gauge_bounds(&f.admin, &1, &3_000, &7_000);

    let weights = f.vault.get_gauge_weights();
    assert_eq!(weights.get(0).unwrap(), (1, 3_000)); // clamped up to min_bps

    // Admin override enforces exact weights
    let override_alloc = vec![&f.env, (1u32, 5_000u32), (2u32, 5_000u32)];
    f.vault.set_gauge_override(&f.admin, &Some(override_alloc));

    let overridden = f.vault.get_gauge_weights();
    assert_eq!(overridden.get(0).unwrap(), (1, 5_000));
    assert_eq!(overridden.get(1).unwrap(), (2, 5_000));
}

#[test]
fn test_gauge_vote_invalid_allocation_fails() {
    let f = Fixture::new(1000);
    f.vault.stake(&f.alice, &100_000);

    // Sum does not equal 10000
    let invalid = vec![&f.env, (1u32, 4_000u32), (2u32, 4_000u32)];
    let res = f.vault.try_vote_gauge(&f.alice, &invalid);
    assert_eq!(res, Err(Ok(VaultBoostGaugeError::InvalidAllocation)));
}

// ── Issue #760: Vote receipt query get_vote() ─────────────────────────────────

#[test]
fn test_vote_receipt_and_change_vote() {
    let f = Fixture::new(1000);
    f.vault.stake(&f.alice, &100_000);

    let prop_id = f.vault.create_proposal(
        &f.alice,
        &String::from_str(&f.env, "Test Proposal"),
        &ProposableParam::RewardRateBps(1500),
        &500,
    );

    // Non-voters report has_voted = false and get_vote = None
    assert_eq!(f.vault.has_voted(&prop_id, &f.alice), false);
    assert_eq!(f.vault.get_vote(&prop_id, &f.alice), None);
    assert_eq!(f.vault.has_voted(&prop_id, &f.bob), false);
    assert_eq!(f.vault.get_vote(&prop_id, &f.bob), None);

    // Cast vote in support
    f.vault.vote(&f.alice, &prop_id, &true);

    assert_eq!(f.vault.has_voted(&prop_id, &f.alice), true);
    let receipt = f.vault.get_vote(&prop_id, &f.alice).unwrap();
    assert_eq!(receipt.support, true);
    assert!(receipt.weight > 0);
    assert_eq!(receipt.ledger, 1000);

    // Change vote to against
    f.vault.change_vote(&f.alice, &prop_id, &false);

    let updated_receipt = f.vault.get_vote(&prop_id, &f.alice).unwrap();
    assert_eq!(updated_receipt.support, false);
    assert_eq!(updated_receipt.weight, receipt.weight);

    // Bob still has not voted
    assert_eq!(f.vault.has_voted(&prop_id, &f.bob), false);
    assert_eq!(f.vault.get_vote(&prop_id, &f.bob), None);
}
