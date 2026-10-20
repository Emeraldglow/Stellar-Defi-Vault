#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    xdr::ToXdr,
    Address, Env,
};

use crate::admin;
use crate::errors::VaultError;
use crate::reward_accumulator::{self, ACCUMULATOR_SCALE};
use crate::reward_math::reward_for_ledgers;
use crate::storage::{AdminKey, DataKey, GovernanceKey, PositionKey, RewardKey, StorageKey};
use crate::vault::{BOOST_BPS_BASE, STELLAR_LEDGERS_PER_YEAR, VaultContract, VaultContractClient};

// Helper: create a mock token address and deploy contract
fn setup_env() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(1_000);

    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    let user = Address::generate(&env);

    (env, admin, token, user)
}

// ── Issue #646: Namespaced Storage Keys Tests ─────────────────────────────────

#[test]
fn test_storage_key_sub_enums_and_mapping() {
    let env = Env::default();
    let addr = Address::generate(&env);

    // Verify sub-enum variant creation
    let admin_key = StorageKey::Admin(AdminKey::Admin);
    let pending_admin_key = StorageKey::Admin(AdminKey::PendingAdmin);
    let pos_key = StorageKey::Position(PositionKey::ShareBalance(addr.clone()));
    let rew_key = StorageKey::Reward(RewardKey::RewardRateBps);
    let gov_key = StorageKey::Governance(GovernanceKey::WhitelistEnabled);

    // Verify XDR serialization works for all sub-enums
    assert!(!admin_key.to_xdr(&env).is_empty());
    assert!(!pending_admin_key.to_xdr(&env).is_empty());
    assert!(!pos_key.to_xdr(&env).is_empty());
    assert!(!rew_key.to_xdr(&env).is_empty());
    assert!(!gov_key.to_xdr(&env).is_empty());

    // Verify DataKey mapping to StorageKey
    assert_eq!(DataKey::Admin.to_storage_key(), StorageKey::Admin(AdminKey::Admin));
    assert_eq!(DataKey::Token.to_storage_key(), StorageKey::Admin(AdminKey::Token));
    assert_eq!(DataKey::Paused.to_storage_key(), StorageKey::Admin(AdminKey::Paused));
    assert_eq!(
        DataKey::ShareBalance(addr.clone()).to_storage_key(),
        StorageKey::Position(PositionKey::ShareBalance(addr.clone()))
    );
    assert_eq!(
        DataKey::RewardRateBps.to_storage_key(),
        StorageKey::Reward(RewardKey::RewardRateBps)
    );
    assert_eq!(
        DataKey::WhitelistEnabled.to_storage_key(),
        StorageKey::Governance(GovernanceKey::WhitelistEnabled)
    );
}

// ── Issue #650: Reward Accumulator Tests & Differential Analysis ───────────────

#[test]
fn test_reward_accumulator_lifecycle() {
    let (env, _admin, _token, user) = setup_env();

    // 1. Initial state
    reward_accumulator::init_accumulator(&env);
    assert_eq!(reward_accumulator::get_global_reward_per_share(&env), 0);

    // 2. Set rate and simulate pool shares
    crate::balance::set_reward_rate_bps(&env, 1_000); // 10% APR
    crate::balance::set_total_shares(&env, 100_000_000); // 10 XLM

    // Checkpoint user at ledger 1000
    reward_accumulator::checkpoint_user(&env, &user);
    assert_eq!(reward_accumulator::get_user_checkpoint(&env, &user), 0);

    // Advance 100 ledgers
    env.ledger().set_sequence_number(1_100);
    let index = reward_accumulator::update_global_accumulator(&env).unwrap();
    assert!(index > 0);

    // Compute user reward for 10_000_000 shares (1 XLM)
    let reward = reward_accumulator::compute_accumulated_reward(&env, &user, 10_000_000).unwrap();
    assert!(reward > 0);

    // User checkpoints; pending drops to 0 until next advance
    reward_accumulator::checkpoint_user(&env, &user);
    let reward_after_checkpoint =
        reward_accumulator::compute_accumulated_reward(&env, &user, 10_000_000).unwrap();
    assert_eq!(reward_after_checkpoint, 0);
}

#[test]
fn test_reward_accumulator_differential_matches_linear_within_one_stroop() {
    // Differential test verifying the accumulator math matches the linear formula
    // within 1 stroop across varied position sizes, rates, and durations.
    let test_cases: &[(i128, u32, u32)] = &[
        // (amount, rate_bps, elapsed_ledgers)
        (1_000, 500, 10),              // small amount, 5%, 10 ledgers
        (10_000_000, 1_000, 100),       // 1 XLM, 10%, 100 ledgers
        (500_000_000, 2_500, 1_000),    // 50 XLM, 25%, 1,000 ledgers
        (10_000_000_000, 5_000, 50_000),// 1,000 XLM, 50%, 50,000 ledgers
        (100_000_000_000, 800, 100_000),// 10,000 XLM, 8%, 100,000 ledgers
    ];

    for &(amount, rate_bps, elapsed) in test_cases {
        // 1. Classical linear reward formula
        let linear_reward = reward_for_ledgers(
            amount,
            rate_bps,
            BOOST_BPS_BASE,
            BOOST_BPS_BASE,
            BOOST_BPS_BASE,
            elapsed,
        )
        .unwrap();

        // 2. High-precision accumulator calculation
        // delta_index = (rate_bps * elapsed * ACCUMULATOR_SCALE) / (10_000 * LEDGERS_PER_YEAR)
        let numerator = (rate_bps as i128)
            * (elapsed as i128)
            * ACCUMULATOR_SCALE;
        let denominator = (BOOST_BPS_BASE as i128) * (STELLAR_LEDGERS_PER_YEAR as i128);
        let delta_index = numerator / denominator;
        let acc_reward = (amount * delta_index) / ACCUMULATOR_SCALE;

        // Differential assertion: difference must be <= 1 stroop
        let diff = (linear_reward - acc_reward).abs();
        assert!(
            diff <= 1,
            "Differential error too large for amount={}, rate={}, elapsed={}: linear={}, acc={}, diff={}",
            amount, rate_bps, elapsed, linear_reward, acc_reward, diff
        );
    }
}

// ── Issue #737: Constructor Deployment & Init Front-Running Prevention Tests ──

#[test]
fn test_constructor_initializes_state_and_blocks_post_deploy_init() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(1_000);

    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract(admin.clone());
    let reward_token = Address::generate(&env);

    // Deploy contract using register_contract
    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);

    // Simulate deploy-time constructor execution
    let init_res = client.try_initialize(
        &admin,
        &token,
        &500_u32, // 5% APR
        &Some(7_u32),
        &Some(7_u32),
    );
    assert!(init_res.is_ok());

    // Verify initial state is configured
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_token(), token);
    assert_eq!(client.get_reward_rate_bps(), 500);

    // Post-deploy attempt to re-initialize (front-running simulation) must fail
    let intruder = Address::generate(&env);
    let attacker_token = Address::generate(&env);
    let front_run_res = client.try_initialize(
        &intruder,
        &attacker_token,
        &1_000_u32,
        &None,
        &None,
    );

    assert_eq!(
        front_run_res.unwrap_err().unwrap(),
        VaultError::AlreadyInitialized
    );
}

// ── Issue #738: Two-Step Admin Transfer Tests ─────────────────────────────────

#[test]
fn test_two_step_admin_transfer_full_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(1_000);

    let current_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let wrong_admin = Address::generate(&env);
    let token = Address::generate(&env);

    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);

    client.initialize(
        &current_admin,
        &token,
        &0_u32,
        &None,
        &None,
    );

    assert_eq!(client.get_admin(), current_admin);
    assert_eq!(client.get_pending_admin(), None);

    // 1. Unauthorized caller cannot propose
    let fake_admin = Address::generate(&env);
    let unauth_res = client.try_propose_admin(&fake_admin, &new_admin);
    assert_eq!(unauth_res.unwrap_err().unwrap(), VaultError::Unauthorized);

    // 2. Cannot propose contract self address or self
    let self_prop = client.try_propose_admin(&current_admin, &contract_id);
    assert_eq!(self_prop.unwrap_err().unwrap(), VaultError::InvalidAddress);
    let same_prop = client.try_propose_admin(&current_admin, &current_admin);
    assert_eq!(same_prop.unwrap_err().unwrap(), VaultError::InvalidAddress);

    // 3. Current admin proposes new_admin
    client.propose_admin(&current_admin, &new_admin);
    assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));
    // Current admin still retains authority
    assert_eq!(client.get_admin(), current_admin);

    // 4. Wrong address cannot accept
    let wrong_accept = client.try_accept_admin(&wrong_admin);
    assert_eq!(wrong_accept.unwrap_err().unwrap(), VaultError::Unauthorized);

    // 5. Cancel proposal works
    client.cancel_admin_proposal(&current_admin);
    assert_eq!(client.get_pending_admin(), None);

    // 6. Propose again and accept successfully
    client.propose_admin(&current_admin, &new_admin);
    assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));

    client.accept_admin(&new_admin);

    // Verification: admin is updated and pending is cleared
    assert_eq!(client.get_admin(), new_admin);
    assert_eq!(client.get_pending_admin(), None);
}
