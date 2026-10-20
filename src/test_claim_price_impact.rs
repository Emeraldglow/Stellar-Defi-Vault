#![cfg(test)]
//! Tests for the reward-claim price impact estimator (issue #355).

extern crate std;

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger as _},
    symbol_short, Address, Env,
};

use crate::{
    balance,
    vault::{VaultContract, VaultContractClient},
};

// ---------------------------------------------------------------------------
// Mock DEX liquidity pool implementing `get_reserves() -> (i128, i128)`
// ---------------------------------------------------------------------------

#[contract]
pub struct MockDexPool;

#[cfg_attr(not(test), contractimpl)]
impl MockDexPool {
    pub fn set_reserves(env: Env, reward_reserve: i128, quote_reserve: i128) {
        env.storage()
            .instance()
            .set(&symbol_short!("res_a"), &reward_reserve);
        env.storage()
            .instance()
            .set(&symbol_short!("res_b"), &quote_reserve);
    }

    pub fn get_reserves(env: Env) -> (i128, i128) {
        let reward_reserve: i128 = env
            .storage()
            .instance()
            .get(&symbol_short!("res_a"))
            .unwrap_or(0);
        let quote_reserve: i128 = env
            .storage()
            .instance()
            .get(&symbol_short!("res_b"))
            .unwrap_or(0);
        (reward_reserve, quote_reserve)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    alice: Address,
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
        let token = env.register_stellar_asset_contract(admin.clone());

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture { env, vault, vault_id, alice }
    }

    /// Register a mock pool seeded with the given reserves, returning its id.
    fn seed_pool(&self, reward_reserve: i128, quote_reserve: i128) -> Address {
        let pool_id = self.env.register_contract(None, MockDexPool);
        let pool = MockDexPoolClient::new(&self.env, &pool_id);
        pool.set_reserves(&reward_reserve, &quote_reserve);
        pool_id
    }

    fn set_pending_reward(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_accrued_reward(&self.env, user, amount);
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// AC: small claim (relative to liquidity) returns a low impact estimate.
#[test]
fn small_claim_returns_low_impact() {
    let f = Fixture::new();
    let pool = f.seed_pool(1_000_000, 1_000_000);
    f.set_pending_reward(&f.alice, 10_000); // 1% of liquidity

    let est = f.vault.estimate_claim_price_impact(&f.alice, &pool);

    assert_eq!(est.claim_amount, 10_000);
    assert_eq!(est.pool_liquidity, 1_000_000);
    // 10_000 * 10_000 / (1_000_000 + 10_000) = 99 bps
    assert_eq!(est.estimated_impact_bps, 99);
    assert!(est.estimated_impact_bps < 100);
}

/// AC: large claim returns a high impact estimate.
#[test]
fn large_claim_returns_high_impact() {
    let f = Fixture::new();
    let pool = f.seed_pool(1_000_000, 1_000_000);
    f.set_pending_reward(&f.alice, 500_000); // 50% of liquidity

    let est = f.vault.estimate_claim_price_impact(&f.alice, &pool);

    // 500_000 * 10_000 / (1_000_000 + 500_000) = 3333 bps
    assert_eq!(est.estimated_impact_bps, 3_333);
    assert!(est.estimated_impact_bps > 1_000);
}

/// AC: recommended max claim correct for known liquidity (impact < 100 bps).
#[test]
fn recommended_max_claim_correct() {
    let f = Fixture::new();
    let pool = f.seed_pool(990_000, 990_000);
    f.set_pending_reward(&f.alice, 10_000);

    let est = f.vault.estimate_claim_price_impact(&f.alice, &pool);

    // 990_000 * 100 / 9_900 = 10_000
    assert_eq!(est.recommended_max_claim, 10_000);
}

/// AC: negligible pending reward (<0.01% of liquidity) returns zero impact.
#[test]
fn negligible_claim_returns_zero_impact() {
    let f = Fixture::new();
    let pool = f.seed_pool(1_000_000, 1_000_000);
    f.set_pending_reward(&f.alice, 50); // 0.005% of liquidity

    let est = f.vault.estimate_claim_price_impact(&f.alice, &pool);

    assert_eq!(est.claim_amount, 50);
    assert_eq!(est.pool_liquidity, 1_000_000);
    assert_eq!(est.estimated_impact_bps, 0);
}

/// AC: unreachable DEX pool is handled gracefully (zero-impact estimate).
#[test]
fn unreachable_pool_handled_gracefully() {
    let f = Fixture::new();
    f.set_pending_reward(&f.alice, 1_000);

    // A freshly generated address is not a registered contract, so the
    // cross-contract `get_reserves` call must fail.
    let bogus_pool = Address::generate(&f.env);

    let est = f.vault.estimate_claim_price_impact(&f.alice, &bogus_pool);

    assert_eq!(est.claim_amount, 1_000);
    assert_eq!(est.estimated_impact_bps, 0);
    assert_eq!(est.recommended_max_claim, 0);
    assert_eq!(est.pool_liquidity, 0);
}

/// Read-only guarantee: no state change from the estimate itself.
#[test]
fn no_position_returns_zero_claim_amount() {
    let f = Fixture::new();
    let pool = f.seed_pool(1_000_000, 1_000_000);

    // Alice has no accrued reward yet.
    let est = f.vault.estimate_claim_price_impact(&f.alice, &pool);

    assert_eq!(est.claim_amount, 0);
    assert_eq!(est.estimated_impact_bps, 0);
    assert_eq!(est.pool_liquidity, 1_000_000);
}