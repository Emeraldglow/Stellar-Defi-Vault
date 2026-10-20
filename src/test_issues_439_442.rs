#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env, String, Vec,
};

use crate::emission_schedule_history::EmissionDataPoint;
use crate::operator_reputation_score::OperatorReputationData;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY, STELLAR_LEDGERS_PER_YEAR};

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

        let (token_addr, token, token_admin) = create_token(&env, &admin);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        token_admin.mint(&alice, &100_000_000);
        token_admin.mint(&bob, &100_000_000);
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
        }
    }
}

// ── Issue #439: stake_gated_ipfs_storage ────────────────────────────────────

#[test]
fn test_set_ipfs_storage_config_requires_admin() {
    let f = Fixture::new();
    let result = f
        .vault
        .try_set_ipfs_storage_config(&f.alice, &500_i128, &5_u32);
    assert!(result.is_err());
}

#[test]
fn test_qualifying_staker_pins_hash() {
    let f = Fixture::new();
    f.vault.set_ipfs_storage_config(&f.admin, &500_i128, &5_u32);
    f.vault.stake(&f.alice, &1000, &0);
    let hash = String::from_str(&f.env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG");
    let desc = String::from_str(&f.env, "vault diagram");
    f.vault
        .pin_ipfs_hash(&f.alice, &hash, &desc)
        .expect("qualifying staker should pin");
    let records = f.vault.get_ipfs_hashes(&f.alice);
    assert_eq!(records.len(), 1);
    assert_eq!(records.get(0).unwrap().hash, hash);
    assert_eq!(records.get(0).unwrap().description, desc);
    assert_eq!(records.get(0).unwrap().pinned_at, 1000);
}

#[test]
fn test_below_threshold_rejected() {
    let f = Fixture::new();
    f.vault.set_ipfs_storage_config(&f.admin, &500_i128, &5_u32);
    f.vault.stake(&f.bob, &100, &0);
    let hash = String::from_str(&f.env, "hash-below-threshold");
    let desc = String::from_str(&f.env, "too small");
    let result = f.vault.try_pin_ipfs_hash(&f.bob, &hash, &desc);
    assert!(result.is_err());
}

#[test]
fn test_hash_length_limits_enforced() {
    let f = Fixture::new();
    f.vault.set_ipfs_storage_config(&f.admin, &500_i128, &5_u32);
    f.vault.stake(&f.alice, &1000, &0);

    let short_desc = String::from_str(&f.env, "");

    // 65-char hash (over IPFS CID v1 max of 64)
    let long_hash = String::from_str(&f.env, "x".repeat(65).as_str());
    let result = f.vault.try_pin_ipfs_hash(&f.alice, &long_hash, &short_desc);
    assert!(result.is_err());

    // 101-char description (over max of 100)
    let ok_hash = String::from_str(&f.env, "short-hash");
    let long_desc = String::from_str(&f.env, "x".repeat(101).as_str());
    let result = f.vault.try_pin_ipfs_hash(&f.alice, &ok_hash, &long_desc);
    assert!(result.is_err());
}

#[test]
fn test_max_hashes_per_user_enforced() {
    let f = Fixture::new();
    f.vault.set_ipfs_storage_config(&f.admin, &500_i128, &2_u32);
    f.vault.stake(&f.alice, &1000, &0);
    let d = String::from_str(&f.env, "d");
    f.vault
        .pin_ipfs_hash(&f.alice, &String::from_str(&f.env, "h1"), &d)
        .unwrap();
    f.vault
        .pin_ipfs_hash(&f.alice, &String::from_str(&f.env, "h2"), &d)
        .unwrap();
    let result = f
        .vault
        .try_pin_ipfs_hash(&f.alice, &String::from_str(&f.env, "h3"), &d);
    assert!(result.is_err());
    assert_eq!(f.vault.get_ipfs_hashes(&f.alice).len(), 2);
}

#[test]
fn test_unpin_removes_correctly() {
    let f = Fixture::new();
    f.vault.set_ipfs_storage_config(&f.admin, &500_i128, &5_u32);
    f.vault.stake(&f.alice, &1000, &0);
    let d = String::from_str(&f.env, "d");
    f.vault
        .pin_ipfs_hash(&f.alice, &String::from_str(&f.env, "h1"), &d)
        .unwrap();
    f.vault
        .pin_ipfs_hash(&f.alice, &String::from_str(&f.env, "h2"), &d)
        .unwrap();
    f.vault
        .unpin_ipfs_hash(&f.alice, &String::from_str(&f.env, "h1"))
        .expect("unpin should succeed");
    let records = f.vault.get_ipfs_hashes(&f.alice);
    assert_eq!(records.len(), 1);
    assert_eq!(records.get(0).unwrap().hash, String::from_str(&f.env, "h2"));
}

#[test]
fn test_unpin_missing_hash_fails() {
    let f = Fixture::new();
    f.vault.set_ipfs_storage_config(&f.admin, &500_i128, &5_u32);
    f.vault.stake(&f.alice, &1000, &0);
    let result = f
        .vault
        .try_unpin_ipfs_hash(&f.alice, &String::from_str(&f.env, "nope"));
    assert!(result.is_err());
}

#[test]
fn test_admin_can_update_config() {
    let f = Fixture::new();
    f.vault.set_ipfs_storage_config(&f.admin, &500_i128, &2_u32);
    f.vault
        .set_ipfs_storage_config(&f.admin, &1000_i128, &3_u32);
    let config = f.vault.get_ipfs_storage_config().unwrap();
    assert_eq!(config.min_stake, 1000);
    assert_eq!(config.max_hashes_per_user, 3);
}

// ── Issue #440: emission_schedule_history ────────────────────────────────────

#[test]
fn test_emission_sample_requires_admin() {
    let f = Fixture::new();
    let result = f.vault.try_take_emission_sample(&f.alice);
    assert!(result.is_err());
}

#[test]
fn test_sample_stored_correctly() {
    let f = Fixture::new();
    f.vault.take_emission_sample(&f.admin).unwrap();
    let history = f.vault.get_emission_history();
    assert_eq!(history.len(), 1);
    let sample = history.get(0).unwrap();
    assert_eq!(sample.ledger, 1000);
    assert_eq!(sample.base_rate_bps, 500);
    assert_eq!(sample.effective_rate_bps, 500);
    assert_eq!(sample.total_staked_at_sample, 0);
    assert_eq!(sample.daily_emission, 0);
}

#[test]
fn test_daily_emission_formula() {
    let f = Fixture::new();
    // Park a large balance: annual rate at 100% + one year's worth staked →
    // daily emission = one day's worth (LEDGERS_PER_DAY).
    f.vault.set_reward_rate_bps(&10_000_u32);
    f.token_admin
        .mint(&f.alice, &(STELLAR_LEDGERS_PER_YEAR as i128));
    f.vault.stake(&f.alice, &(STELLAR_LEDGERS_PER_YEAR as i128), &0);
    f.vault.take_emission_sample(&f.admin).unwrap();

    let history = f.vault.get_emission_history();
    let sample = history.get(0).unwrap();
    assert_eq!(sample.base_rate_bps, 10_000);
    assert_eq!(sample.effective_rate_bps, 10_000);
    assert_eq!(
        sample.total_staked_at_sample,
        STELLAR_LEDGERS_PER_YEAR as i128
    );
    // 1 year staked at 100% APR → 100% paid over the year → per day = 1 day.
    assert_eq!(sample.daily_emission, LEDGERS_PER_DAY as i128);
    assert_eq!(
        sample.daily_emission as i128,
        sample.total_staked_at_sample * 10_000 * (LEDGERS_PER_DAY as i128)
            / (10_000 * (STELLAR_LEDGERS_PER_YEAR as i128))
    );
}

#[test]
fn test_history_rolls_at_100() {
    let f = Fixture::new();
    let mut ledger = 1000u32;
    for _ in 0..101 {
        f.vault.take_emission_sample(&f.admin).unwrap();
        ledger += 10;
        set_ledger(&f.env, ledger);
    }
    let history = f.vault.get_emission_history();
    assert_eq!(history.len(), 100);
    // Oldest sample (ledger 1000) was dropped; newest (ledger 2000) kept.
    assert_eq!(history.get(0).unwrap().ledger, 1010);
    assert_eq!(history.get(99).unwrap().ledger, 2000);
}

#[test]
fn test_filter_by_ledger_works() {
    let f = Fixture::new();
    f.vault.take_emission_sample(&f.admin).unwrap(); // ledger 1000
    set_ledger(&f.env, 2000);
    f.vault.take_emission_sample(&f.admin).unwrap();
    set_ledger(&f.env, 3000);
    f.vault.take_emission_sample(&f.admin).unwrap();

    let all = f.vault.get_emission_history_since(2000);
    assert_eq!(all.len(), 2);
    assert_eq!(all.get(0).unwrap().ledger, 2000);
    assert_eq!(all.get(1).unwrap().ledger, 3000);

    let none_after = f.vault.get_emission_history_since(9999);
    assert_eq!(none_after.len(), 0);
}

// ── Issue #441: minimum_unstake_amount ───────────────────────────────────────

#[test]
fn test_set_min_unstake_requires_admin() {
    let f = Fixture::new();
    let result = f.vault.try_set_min_unstake_amount(&f.alice, &1000_i128);
    assert!(result.is_err());
}

#[test]
fn test_set_and_get_min_unstake() {
    let f = Fixture::new();
    assert_eq!(f.vault.get_min_unstake_amount(), 0);
    f.vault
        .set_min_unstake_amount(&f.admin, &1000_i128)
        .unwrap();
    assert_eq!(f.vault.get_min_unstake_amount(), 1000);
    // 0 disables the check
    f.vault.set_min_unstake_amount(&f.admin, &0_i128).unwrap();
    assert_eq!(f.vault.get_min_unstake_amount(), 0);
}

#[test]
fn test_unstake_above_minimum_succeeds() {
    let f = Fixture::new();
    f.vault
        .set_min_unstake_amount(&f.admin, &1000_i128)
        .unwrap();
    f.vault.stake(&f.alice, &5000, &0);
    assert!(f.vault.unstake(&f.alice, &4000).is_ok());
}

#[test]
fn test_unstake_below_minimum_fails() {
    let f = Fixture::new();
    f.vault
        .set_min_unstake_amount(&f.admin, &1000_i128)
        .unwrap();
    f.vault.stake(&f.alice, &5000, &0);
    let result = f.vault.try_unstake(&f.alice, &500);
    assert!(result.is_err());
}

#[test]
fn test_full_position_exit_always_allowed() {
    let f = Fixture::new();
    f.vault
        .set_min_unstake_amount(&f.admin, &1000_i128)
        .unwrap();
    // Direction: min 1000, so unstaking a fully-open 500 position is a full exit.
    f.vault.stake(&f.alice, &500, &0);
    assert!(f.vault.unstake(&f.alice, &500).is_ok());
}

#[test]
fn test_min_zero_disables_check() {
    let f = Fixture::new();
    f.vault
        .set_min_unstake_amount(&f.admin, &1000_i128)
        .unwrap();
    f.vault.set_min_unstake_amount(&f.admin, &0_i128).unwrap();
    f.vault.stake(&f.alice, &5000, &0);
    assert!(f.vault.unstake(&f.alice, &500).is_ok());
}

#[test]
fn test_nearest_valid_unstake_helper_rounds_up() {
    let f = Fixture::new();
    f.vault
        .set_min_unstake_amount(&f.admin, &1000_i128)
        .unwrap();
    assert_eq!(f.vault.get_nearest_valid_unstake(&500), 1000);
    assert_eq!(f.vault.get_nearest_valid_unstake(&1000), 1000);
    assert_eq!(f.vault.get_nearest_valid_unstake(&2000), 2000);
}

// ── Issue #442: operator_reputation_score ────────────────────────────────────

#[test]
fn test_perfect_pool_scores_100() {
    let f = Fixture::new();
    let data = OperatorReputationData {
        pool_uptime_ledgers: STELLAR_LEDGERS_PER_YEAR,
        solvency_ledgers: STELLAR_LEDGERS_PER_YEAR,
        governance_participation_bps: 10_000,
        slash_disputes_lost: 0,
        pool_average_rating_bps: 10_000,
        pool_age_days: 365,
    };
    f.vault
        .record_operator_reputation(&f.admin, &f.alice, &data)
        .unwrap();
    let score = f.vault.compute_operator_score(&f.alice);
    assert_eq!(score.uptime_score, 25);
    assert_eq!(score.solvency_score, 25);
    assert_eq!(score.governance_score, 25);
    assert_eq!(score.dispute_score, 25);
    assert_eq!(score.community_score, 25);
    assert_eq!(score.total_score, 100);
}

#[test]
fn test_lost_disputes_reduce_score() {
    let f = Fixture::new();
    // Baseline components below max so the total isn't pinned at the 100 cap.
    let base = |lost: u32| OperatorReputationData {
        pool_uptime_ledgers: STELLAR_LEDGERS_PER_YEAR / 2, // 12
        solvency_ledgers: LEDGERS_PER_DAY * 100,           // 12
        governance_participation_bps: 4000,                // 10
        slash_disputes_lost: lost,
        pool_average_rating_bps: 2000, // 5
        pool_age_days: 200,
    };
    f.vault
        .record_operator_reputation(&f.admin, &f.alice, &base(0))
        .unwrap();
    let clean = f.vault.compute_operator_score(&f.alice);
    assert_eq!(clean.total_score, 64); // 12+12+10+25+5

    f.vault
        .record_operator_reputation(&f.admin, &f.alice, &base(3))
        .unwrap();
    let penalized = f.vault.compute_operator_score(&f.alice);
    assert_eq!(penalized.dispute_score, 10); // 25 - 3*5
    assert_eq!(penalized.total_score, 49); // 12+12+10+10+5
}

#[test]
fn test_dispute_score_floors_at_zero() {
    let f = Fixture::new();
    let data = OperatorReputationData {
        pool_uptime_ledgers: STELLAR_LEDGERS_PER_YEAR,
        solvency_ledgers: STELLAR_LEDGERS_PER_YEAR,
        governance_participation_bps: 10_000,
        slash_disputes_lost: 6,
        pool_average_rating_bps: 10_000,
        pool_age_days: 365,
    };
    f.vault
        .record_operator_reputation(&f.admin, &f.alice, &data)
        .unwrap();
    let score = f.vault.compute_operator_score(&f.alice);
    assert_eq!(score.dispute_score, 0); // 25 - 30, floored
}

#[test]
fn test_low_uptime_reduces_score() {
    let f = Fixture::new();
    let data = OperatorReputationData {
        pool_uptime_ledgers: 0,
        solvency_ledgers: 0,
        governance_participation_bps: 10_000,
        slash_disputes_lost: 0,
        pool_average_rating_bps: 10_000,
        pool_age_days: 365,
    };
    f.vault
        .record_operator_reputation(&f.admin, &f.alice, &data)
        .unwrap();
    let score = f.vault.compute_operator_score(&f.alice);
    assert_eq!(score.uptime_score, 0);
    assert_eq!(score.solvency_score, 0);
    assert_eq!(score.total_score, 75);
}

#[test]
fn test_component_scores_calculate_correctly() {
    let f = Fixture::new();
    let data = OperatorReputationData {
        pool_uptime_ledgers: STELLAR_LEDGERS_PER_YEAR / 2, // half year → ~12
        solvency_ledgers: LEDGERS_PER_DAY * 100,           // 100 days solvent
        governance_participation_bps: 4000,                // 4000/400 = 10
        slash_disputes_lost: 1,                            // 25 - 5 = 20
        pool_average_rating_bps: 2000,                     // 2000/400 = 5
        pool_age_days: 200,
    };
    f.vault
        .record_operator_reputation(&f.admin, &f.alice, &data)
        .unwrap();
    let score = f.vault.compute_operator_score(&f.alice);
    assert_eq!(score.uptime_score, 12); // (6_307_200/2)/(6_307_200/25) = 12.5 → 12
    assert_eq!(score.solvency_score, 12); // 100*25/200 = 12.5 → 12
    assert_eq!(score.governance_score, 10);
    assert_eq!(score.dispute_score, 20);
    assert_eq!(score.community_score, 5);
    assert_eq!(score.total_score, 59);
}

#[test]
fn test_unrecorded_operator_scores_zero() {
    let f = Fixture::new();
    let score = f.vault.compute_operator_score(&f.bob);
    assert_eq!(score.total_score, 0);
    assert_eq!(score.pools_operated, 1);
}
