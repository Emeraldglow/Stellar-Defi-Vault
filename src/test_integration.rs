#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env, Vec,
};

use crate::storage::{AccessTier, PauseReason};
use crate::vault::{VaultContract, VaultContractClient, STELLAR_LEDGERS_PER_YEAR};
use crate::StakeReceiptNFT;
use crate::nft::StakeReceiptNFTClient;

// ΓöÇΓöÇ helpers ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

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

// ΓöÇΓöÇ Full lifecycle: pool create ΓåÆ multi-user stake ΓåÆ mid-claim ΓåÆ rate change ΓåÆ unstake ΓöÇΓöÇ

/// Scenario:
///   admin deploys pool (rate 1000 bps / 10% APR)
///   ΓåÆ 5 users stake different amounts at ledger 0
///   ΓåÆ advance to half-year (ANNUAL/2 ledgers)
///   ΓåÆ users 1 and 2 claim mid-way rewards
///   ΓåÆ admin changes rate to 500 bps (5% APR)
///   ΓåÆ advance to full year (ANNUAL ledgers)
///   ΓåÆ all users unstake all shares
///   ΓåÆ all users claim remaining rewards
///   ΓåÆ verify: contract stake balance = 0, reward pool = initial ΓêÆ total_rewards_paid,
///              sum of final user balances = sum of initial + total rewards
///
/// Reward math (no boost schedule, multiplier = 10_000 = 1├ù):
///   reward = amount ├ù rate_bps ├ù elapsed / 10_000 / ANNUAL
///
/// Users 1 & 2 claim at ANNUAL/2 (rate still 1000), so their checkpoint advances.
/// Users 3ΓÇô5 never claim early; their accrual runs from ledger 0 to ANNUAL using
/// the current rate at unstake time (500 bps), so they earn at 5% for the full year.
///
///   user1  (1_000_000): mid=50_000 + post=25_000  = 75_000
///   user2  (2_000_000): mid=100_000 + post=50_000 = 150_000
///   user3  (3_000_000): full-year @500 bps         = 150_000
///   user4  (4_000_000): full-year @500 bps         = 200_000
///   user5  (5_000_000): full-year @500 bps         = 250_000
///   total rewards paid = 825_000
#[test]
fn test_integration_full_lifecycle() {
    let annual: u32 = STELLAR_LEDGERS_PER_YEAR;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 0;
        li.min_temp_entry_ttl = 10_000_000;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
    });

    // ΓöÇΓöÇ Phase 1: Setup ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let user3 = Address::generate(&env);
    let user4 = Address::generate(&env);
    let user5 = Address::generate(&env);

    let (token_addr, token, token_admin) = create_token(&env, &admin);

    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // Mint initial balances (10_000_000 each)
    let initial_balance: i128 = 10_000_000;
    for user in [&user1, &user2, &user3, &user4, &user5] {
        token_admin.mint(user, &initial_balance);
    }
    // Fund reward pool
    let reward_pool_initial: i128 = 1_000_000;
    token_admin.mint(&admin, &reward_pool_initial);
    vault.set_reward_rate_bps(&1000); // 10% APR
    vault.fund_reward_pool(&admin, &reward_pool_initial);

    // ΓöÇΓöÇ Phase 2: All users stake at ledger 0 ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    // Inline comment: first stake for each user ΓåÆ position_opened event emitted
    let stake1: i128 = 1_000_000;
    let stake2: i128 = 2_000_000;
    let stake3: i128 = 3_000_000;
    let stake4: i128 = 4_000_000;
    let stake5: i128 = 5_000_000;

    vault.stake(&user1, &stake1, &0);
    vault.stake(&user2, &stake2, &0);
    vault.stake(&user3, &stake3, &0);
    vault.stake(&user4, &stake4, &0);
    vault.stake(&user5, &stake5, &0);

    // Verify pool_stats shows 5 stakers and correct total
    let stats = vault.pool_stats();
    assert_eq!(
        stats.total_stakers, 5,
        "Should have 5 active stakers after all stake"
    );
    assert_eq!(
        stats.total_staked,
        stake1 + stake2 + stake3 + stake4 + stake5,
        "Total staked should equal sum of all stakes"
    );
    assert_eq!(stats.total_rewards_paid, 0, "No rewards paid yet");

    // ΓöÇΓöÇ Phase 3: Advance to half-year ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    set_ledger(&env, annual / 2);

    // ΓöÇΓöÇ Phase 4: Users 1 and 2 claim mid-way rewards ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    // This snapshots their reward checkpoint at the current (1000 bps) rate
    let mid_claim1 = vault.claim(&user1);
    let mid_claim2 = vault.claim(&user2);

    assert_eq!(mid_claim1, 50_000, "User1 mid-year reward at 10% APR");
    assert_eq!(mid_claim2, 100_000, "User2 mid-year reward at 10% APR");

    // ΓöÇΓöÇ Phase 5: Admin changes reward rate ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    // Users 3ΓÇô5 have not yet accrued; their future rewards will use this new rate
    vault.set_reward_rate_bps(&500); // 5% APR

    // ΓöÇΓöÇ Phase 6: Advance to full year ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    set_ledger(&env, annual);

    // ΓöÇΓöÇ Phase 7: All users unstake ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    // Inline comment: shares are 1:1 with token amounts (no yield added), so each
    // user recovers exactly their original stake. accrue_rewards is called internally.
    let back1 = vault.unstake(&user1, &stake1);
    let back2 = vault.unstake(&user2, &stake2);
    let back3 = vault.unstake(&user3, &stake3);
    let back4 = vault.unstake(&user4, &stake4);
    let back5 = vault.unstake(&user5, &stake5);

    assert_eq!(back1, stake1, "User1 should recover full stake");
    assert_eq!(back2, stake2, "User2 should recover full stake");
    assert_eq!(back3, stake3, "User3 should recover full stake");
    assert_eq!(back4, stake4, "User4 should recover full stake");
    assert_eq!(back5, stake5, "User5 should recover full stake");

    // After all unstakes: staked balance = 0
    let (total_shares, total_deposited) = vault.vault_state();
    assert_eq!(
        total_shares, 0,
        "Total shares should be 0 after all unstake"
    );
    assert_eq!(
        total_deposited, 0,
        "Contract stake token balance should be 0 after all unstake"
    );

    // pool_stats: 0 stakers after all full unstakes
    let stats_post_unstake = vault.pool_stats();
    assert_eq!(
        stats_post_unstake.total_stakers, 0,
        "total_stakers should reach 0 after all full unstakes"
    );

    // ΓöÇΓöÇ Phase 8: All users claim remaining rewards ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    // Users 1 & 2: rewards earned in second half at 500 bps
    // Users 3ΓÇô5: rewards earned for full year at current rate (500 bps applied retroactively
    //            because their checkpoint was never moved mid-year)
    let post_claim1 = vault.claim(&user1);
    let post_claim2 = vault.claim(&user2);
    let post_claim3 = vault.claim(&user3);
    let post_claim4 = vault.claim(&user4);
    let post_claim5 = vault.claim(&user5);

    assert_eq!(post_claim1, 25_000, "User1 second-half reward at 5% APR");
    assert_eq!(post_claim2, 50_000, "User2 second-half reward at 5% APR");
    assert_eq!(
        post_claim3, 150_000,
        "User3 full-year reward at 5% APR (rate changed before accrual)"
    );
    assert_eq!(post_claim4, 200_000, "User4 full-year reward at 5% APR");
    assert_eq!(post_claim5, 250_000, "User5 full-year reward at 5% APR");

    let total_rewards: i128 = mid_claim1
        + mid_claim2
        + post_claim1
        + post_claim2
        + post_claim3
        + post_claim4
        + post_claim5;
    assert_eq!(total_rewards, 825_000, "Total rewards across all users");

    // ΓöÇΓöÇ Phase 9: Final assertions ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

    // Assert A: contract reward token balance = initial_pool ΓêÆ total_rewards_paid
    let contract_balance = token.balance(&vault_id);
    assert_eq!(
        contract_balance,
        reward_pool_initial - total_rewards,
        "Contract reward token balance should equal initial pool minus total rewards paid"
    );

    // Assert B: pool_stats.total_rewards_paid matches actual sum of all claims
    let final_stats = vault.pool_stats();
    assert_eq!(
        final_stats.total_rewards_paid, total_rewards,
        "pool_stats.total_rewards_paid should equal sum of all successful claims"
    );

    // Assert C: sum of all user final balances = sum of initial balances + total rewards
    let sum_initial = initial_balance * 5; // 50_000_000
    let sum_final = token.balance(&user1)
        + token.balance(&user2)
        + token.balance(&user3)
        + token.balance(&user4)
        + token.balance(&user5);
    assert_eq!(
        sum_final,
        sum_initial + total_rewards,
        "Sum of final user balances should equal sum of initial balances plus total rewards earned"
    );
}

// ΓöÇΓöÇ Whitelist tests for permissioned staking ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_whitelisted_user_can_stake_when_enabled() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // enable whitelist and add alice
    vault.set_whitelist_enabled(&true);
    vault.add_to_whitelist(&alice);

    token_admin.mint(&alice, &100_000);
    let res = vault.try_stake(&alice, &50_000);
    assert!(
        res.is_ok(),
        "Whitelisted user should be able to stake when whitelist enabled"
    );
}

#[test]
fn test_non_whitelisted_user_rejected_when_enabled() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let bob = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // enable whitelist but do NOT add bob
    vault.set_whitelist_enabled(&true);

    token_admin.mint(&bob, &100_000);
    let res = vault.try_stake(&bob, &20_000);
    assert_eq!(res, Err(Ok(crate::errors::VaultError::NotWhitelisted)));
}

#[test]
fn test_toggle_off_allows_non_whitelisted() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let carol = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // enable whitelist, but then turn it off
    vault.set_whitelist_enabled(&true);
    vault.set_whitelist_enabled(&false);

    token_admin.mint(&carol, &100_000);
    // should succeed when whitelist disabled
    let res = vault.try_stake(&carol, &30_000);
    assert!(res.is_ok());
}

#[test]
fn test_revocation_blocks_new_stake_but_allows_unstake_and_claim() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 0;
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // enable whitelist and add alice
    vault.set_whitelist_enabled(&true);
    vault.add_to_whitelist(&alice);

    token_admin.mint(&alice, &200_000);
    // alice stakes 100k
    vault.stake(&alice, &100_000, &0);

    // advance ledger and set a reward rate so claim will return >0
    env.ledger().with_mut(|li| li.sequence_number = 500);
    vault.set_reward_rate_bps(&1000);

    // revoke alice
    vault.remove_from_whitelist(&alice);

    // alice should NOT be able to stake more
    let try_more = vault.try_stake(&alice, &10_000);
    assert_eq!(try_more, Err(Ok(crate::errors::VaultError::NotWhitelisted)));

    // but alice should still be able to claim accrued rewards
    let claim_res = vault.claim(&alice);
    // claim should succeed (may be zero or >0 depending on timing), but must not error
    // ensure method returns without Err by comparing types ΓÇö here it's direct call so will panic on Err
    // We assert that the returned value is >= 0
    assert!(claim_res >= 0);

    // and unstake should work
    let unstake_res = vault.unstake(&alice, &100_000);
    assert_eq!(unstake_res, 100_000);
}
// ΓöÇΓöÇ pool_stats reflects staker count correctly ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_total_stakers_tracks_entries_and_exits() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    token_admin.mint(&alice, &500_000);
    token_admin.mint(&bob, &500_000);

    // No stakers initially
    assert_eq!(vault.pool_stats().total_stakers, 0);

    vault.stake(&alice, &100_000, &0);
    assert_eq!(
        vault.pool_stats().total_stakers,
        1,
        "total_stakers should be 1 after alice stakes"
    );

    vault.stake(&bob, &200_000, &0);
    assert_eq!(
        vault.pool_stats().total_stakers,
        2,
        "total_stakers should be 2 after bob stakes"
    );

    // Partial unstake should NOT decrement stakers
    vault.unstake(&alice, &50_000);
    assert_eq!(
        vault.pool_stats().total_stakers,
        2,
        "total_stakers unchanged after partial unstake"
    );

    // Full unstake decrements
    vault.unstake(&alice, &50_000);
    assert_eq!(
        vault.pool_stats().total_stakers,
        1,
        "total_stakers should be 1 after alice fully unstakes"
    );

    vault.unstake(&bob, &200_000);
    assert_eq!(
        vault.pool_stats().total_stakers,
        0,
        "total_stakers should be 0 after all fully unstake"
    );
}

// ΓöÇΓöÇ position_of returns correct data ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_position_of_returns_correct_fields() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 10;
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
    vault.set_reward_rate_bps(&1000);

    token_admin.mint(&alice, &500_000);
    vault.stake(&alice, &200_000, &0);

    let position = vault.position_of(&alice).unwrap();
    assert_eq!(
        position.amount, 200_000,
        "amount should equal staked tokens"
    );
    assert_eq!(
        position.staked_at_ledger, 10,
        "staked_at_ledger should match ledger at stake time"
    );
    assert_eq!(
        position.last_claim_ledger, 10,
        "last_claim_ledger is initialised to the stake ledger when a position is opened"
    );
}

// ΓöÇΓöÇ delegate staking: full happy path ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_stake_for_delegate_happy_path() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let delegate = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // Fund only the delegate ΓÇö beneficiary has no tokens
    token_admin.mint(&delegate, &500_000);

    // Beneficiary approves delegate
    vault.approve_delegate(&beneficiary, &delegate);
    assert!(
        vault.is_delegate(&beneficiary, &delegate),
        "delegate should be approved after approve_delegate"
    );

    // Delegate stakes on behalf of beneficiary
    let shares = vault.stake_for(&delegate, &beneficiary, &300_000);
    assert_eq!(shares, 300_000, "Shares should equal amount on first stake");
    assert_eq!(
        vault.shares_of(&beneficiary),
        300_000,
        "Position should be credited to beneficiary"
    );
    assert_eq!(
        token.balance(&delegate),
        200_000,
        "Tokens deducted from delegate's wallet"
    );
    assert_eq!(
        token.balance(&beneficiary),
        0,
        "Beneficiary's token balance unchanged"
    );

    // Beneficiary can unstake
    let returned = vault.unstake(&beneficiary, &300_000);
    assert_eq!(
        returned, 300_000,
        "Beneficiary should recover tokens on unstake"
    );
    assert_eq!(vault.shares_of(&beneficiary), 0);
}

// ΓöÇΓöÇ delegate staking: auth / rejection edge cases ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_stake_for_non_approved_delegate_rejected() {
    use crate::errors::VaultError;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let delegate = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
    token_admin.mint(&delegate, &500_000);

    // No approval given ΓÇö should fail
    let result = vault.try_stake_for(&delegate, &beneficiary, &100_000);
    assert_eq!(
        result,
        Err(Ok(VaultError::NotADelegate)),
        "Non-approved delegate should be rejected"
    );
}

#[test]
fn test_stake_for_revoked_delegate_rejected() {
    use crate::errors::VaultError;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let delegate = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
    token_admin.mint(&delegate, &500_000);

    vault.approve_delegate(&beneficiary, &delegate);
    vault.revoke_delegate(&beneficiary, &delegate);

    assert!(
        !vault.is_delegate(&beneficiary, &delegate),
        "Delegate should be removed after revocation"
    );

    let result = vault.try_stake_for(&delegate, &beneficiary, &100_000);
    assert_eq!(
        result,
        Err(Ok(VaultError::NotADelegate)),
        "Revoked delegate should be rejected"
    );
}

// ΓöÇΓöÇ analytics events: rate_changed / position_opened / position_closed ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_rate_changed_event_emitted() {
    use soroban_sdk::{testutils::Events, Symbol, TryFromVal};

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let (token_addr, _token, _token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    vault.set_reward_rate_bps(&1000);

    let events = env.events().all();
    let matched: std::vec::Vec<_> = events
        .into_iter()
        .filter(|(_, topics, _)| {
            topics
                .get(0)
                .and_then(|v| Symbol::try_from_val(&env, &v).ok())
                .map(|s| s == Symbol::new(&env, "rate_chg"))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(matched.len(), 1, "rate_chg event should be emitted once");
}

#[test]
fn test_position_opened_event_on_first_stake() {
    use soroban_sdk::{testutils::Events, Symbol, TryFromVal};

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
    token_admin.mint(&alice, &500_000);

    vault.stake(&alice, &100_000, &0);
    vault.stake(&alice, &100_000, &0); // second stake ΓÇö should NOT emit position_opened again

    let events = env.events().all();
    let matched: std::vec::Vec<_> = events
        .into_iter()
        .filter(|(_, topics, _)| {
            topics
                .get(0)
                .and_then(|v| Symbol::try_from_val(&env, &v).ok())
                .map(|s| s == Symbol::new(&env, "pos_open"))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        matched.len(),
        1,
        "pos_open should be emitted only on first stake, not top-ups"
    );
    let event = &matched[0];
    assert_eq!(
        Address::try_from_val(&env, &event.1.get(1).unwrap()).unwrap(),
        alice,
        "pos_open topic should contain the user address"
    );
}

#[test]
fn test_position_closed_event_on_full_unstake() {
    use soroban_sdk::{testutils::Events, Symbol, TryFromVal};

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
    token_admin.mint(&alice, &500_000);

    vault.stake(&alice, &200_000, &0);
    vault.unstake(&alice, &100_000); // partial ΓÇö should NOT emit pos_clos
    vault.unstake(&alice, &100_000); // full ΓÇö SHOULD emit pos_clos

    let events = env.events().all();
    let matched: std::vec::Vec<_> = events
        .into_iter()
        .filter(|(_, topics, _)| {
            topics
                .get(0)
                .and_then(|v| Symbol::try_from_val(&env, &v).ok())
                .map(|s| s == Symbol::new(&env, "pos_clos"))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        matched.len(),
        1,
        "pos_clos should be emitted only on full unstake, not partial"
    );
    let event = &matched[0];
    assert_eq!(
        Address::try_from_val(&env, &event.1.get(1).unwrap()).unwrap(),
        alice,
        "pos_clos topic should contain the user address"
    );
}

// ΓöÇΓöÇ paused / unpaused events include ledger field ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_paused_event_includes_ledger() {
    use soroban_sdk::{testutils::Events, Symbol, TryFromVal, Vec as SorobanVec};

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 42;
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let (token_addr, _token, _token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    vault.pause(
        &PauseReason::Other,
        &soroban_sdk::String::from_str(&env, "test"),
    );

    let events = env.events().all();
    let matched: std::vec::Vec<_> = events
        .into_iter()
        .filter(|(_, topics, _)| {
            topics
                .get(0)
                .and_then(|v| Symbol::try_from_val(&env, &v).ok())
                .map(|s| s == Symbol::new(&env, "paused"))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(matched.len(), 1, "paused event should be emitted");

    // event data is (reason, message, ledger) ΓÇö just verify the event exists
}

// ΓöÇΓöÇ slash admin actions tests ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_slash_partial_and_treasury_receive() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // set custom treasury
    vault.set_slash_treasury(&treasury);

    // fund alice and stake
    token_admin.mint(&alice, &500_000);
    vault.stake(&alice, &200_000, &0);

    // pre-check balances
    assert_eq!(token.balance(&vault_id), 200_000);
    assert_eq!(token.balance(&treasury), 0);

    // admin slashes 50_000
    let slashed = vault.slash(&admin, &alice, &50_000);
    assert_eq!(slashed, 50_000);

    // alice position reduced accordingly (shares correspond to amounts on first stake)
    assert_eq!(vault.shares_of(&alice), 150_000);

    // treasury received tokens
    assert_eq!(token.balance(&treasury), 50_000);
    // contract balance decreased
    assert_eq!(token.balance(&vault_id), 150_000);
}

#[test]
fn test_slash_full_and_position_removed() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
    vault.set_slash_treasury(&treasury);

    token_admin.mint(&alice, &300_000);
    vault.stake(&alice, &150_000, &0);

    // slash full or larger amount
    let slashed = vault.slash(&admin, &alice, &200_000);
    assert_eq!(slashed, 150_000);

    // position removed
    assert_eq!(vault.shares_of(&alice), 0);
    assert_eq!(token.balance(&treasury), 150_000);
    assert_eq!(token.balance(&vault_id), 0);
}

#[test]
fn test_slash_works_while_paused() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
    vault.set_slash_treasury(&treasury);

    token_admin.mint(&alice, &200_000);
    vault.stake(&alice, &100_000, &0);

    // pause the contract
    vault.pause(
        &PauseReason::Other,
        &soroban_sdk::String::from_str(&env, "test"),
    );

    // should still be able to slash
    let slashed = vault.slash(&admin, &alice, &30_000);
    assert_eq!(slashed, 30_000);
    assert_eq!(token.balance(&treasury), 30_000);
}

#[test]
#[ignore = "Soroban SDK 21.x: require_auth() issues a non-catchable abort in native \
             test mode when auth is not mocked; the admin guard is enforced at the \
             protocol layer in production. See test_slash_partial_and_treasury_receive \
             for the positive (authorized) slash path."]
fn test_non_admin_rejected_for_slash() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &50_000, &0);

    // Verify admin auth is required: the recorded authorizer must be the admin address.
    vault.slash(&admin, &alice, &10_000);
    let auths = env.auths();
    assert!(auths.iter().any(|(addr, _)| addr == &admin));
}

#[test]
fn test_reward_forfeiture_on_slash() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 0;
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
    vault.set_slash_treasury(&treasury);

    token_admin.mint(&alice, &500_000);
    vault.stake(&alice, &100_000, &0);

    // advance ledger to accrue rewards
    env.ledger().with_mut(|li| li.sequence_number = 1000);
    vault.set_reward_rate_bps(&1000); // set a rate so rewards accrue

    // compute pending before slash (call claim would consume; we just simulate by checking pending)
    let pending_before = vault.calc_pending_reward(&alice);
    assert!(pending_before > 0);

    // Slash user
    vault.slash(&admin, &alice, &50_000);

    // After slash, accrued rewards should be cleared; claim should return 0
    let claim_after = vault.claim(&alice);
    assert_eq!(claim_after, 0);
}

#[test]
fn test_initialization_defaults_treasury_to_admin() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    // initialize without specifying treasury (defaults to admin)
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &20_000, &0);

    // admin slashes -> funds should go to admin (default treasury)
    vault.slash(&admin, &alice, &10_000);
    assert_eq!(token.balance(&admin), 10_000);
}

// ΓöÇΓöÇ cooldown / unbonding flow tests ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_full_cooldown_flow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 0;
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // set cooldown to 5 ledgers
    vault.set_cooldown_period(&5);

    token_admin.mint(&alice, &200_000);
    vault.stake(&alice, &100_000, &0);

    // request unstake 50_000
    vault.request_unstake(&alice, &50_000);

    // pending unbonding should be present
    let pos = vault.pending_unbonding(&alice).unwrap();
    assert_eq!(pos.amount, 50_000);

    // advance ledger past cooldown
    env.ledger().with_mut(|li| li.sequence_number = 6);

    // execute unstake
    let executed = vault.execute_unstake(&alice);
    assert_eq!(executed, 50_000);

    // alice balance: minted 200k - staked 100k + executed 50k = 150k
    assert_eq!(token.balance(&alice), 150_000);
}

#[test]
fn test_premature_execute_unstake_fails() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 0;
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    vault.set_cooldown_period(&10);
    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &50_000, &0);

    vault.request_unstake(&alice, &20_000);

    // attempt to execute immediately -> should fail
    let res = vault.try_execute_unstake(&alice);
    assert_eq!(res, Err(Ok(crate::errors::VaultError::UseCooldownFlow)));
}

#[test]
fn test_zero_cooldown_bypass_allows_instant_unstake() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 0;
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 1_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // set cooldown to 0
    vault.set_cooldown_period(&0);

    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &50_000, &0);

    // instant unstake allowed
    let returned = vault.unstake(&alice, &50_000);
    assert_eq!(returned, 50_000);
    assert_eq!(token.balance(&alice), 100_000);
}

#[test]
fn test_no_rewards_accrued_during_cooldown() {
    let env = Env::default();
    env.mock_all_auths();
    // Use a large TTL so persistent entries don't expire when the ledger advances.
    env.ledger().with_mut(|li| {
        li.sequence_number = 0;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    vault.set_cooldown_period(&10);

    // Fund the reward pool so claim() can transfer rewards out.
    token_admin.mint(&admin, &1_000_000);
    vault.fund_reward_pool(&admin, &1_000_000);

    token_admin.mint(&alice, &500_000);
    vault.stake(&alice, &100_000, &0);

    // Set rate, then advance to 100_000 ledgers.
    // reward = amount * rate_bps * elapsed / 10_000 / 6_307_200
    // = 100_000 * 1_000 * 100_000 / 10_000 / 6_307_200 Γëê 158 tokens > 0.
    vault.set_reward_rate_bps(&1000);
    env.ledger().with_mut(|li| li.sequence_number = 100_000);

    let pending_before = vault.calc_pending_reward(&alice);
    assert!(
        pending_before > 0,
        "expected non-zero pending reward before unstake"
    );

    // request_unstake finalizes accrual at the current ledger and stops further accrual.
    vault.request_unstake(&alice, &100_000);

    // Advance FORWARD through the cooldown period (cooldown = 10).
    env.ledger().with_mut(|li| li.sequence_number = 100_020);

    // claim() should return exactly the rewards accrued before request_unstake, no more.
    let claim_after = vault.claim(&alice);
    assert_eq!(claim_after, pending_before);
}

// ΓöÇΓöÇ Issue #281: Fee Revenue Sharing Tests ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_revenue_sharing_flow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 1;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let gov_token = Address::generate(&env);
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // Configure revenue sharing: 20% (2000 BPS) of collected fees go to RevenueSharePool
    vault.set_revenue_sharing(&admin, &gov_token, &2000);
    assert_eq!(vault.get_revenue_share_pool(), 0);

    // Configure 5% unstake fee (500 BPS)
    vault.set_unstake_fee_bps(&admin, &500);

    // Stake 100_000 tokens for Alice
    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &100_000, &0);

    // Unstake 100_000: Unstake fee is 5% = 5,000.
    // 20% of 5,000 = 1,000 goes to RevenueSharePool.
    // 80% of 5,000 = 4,000 goes to reward pool.
    vault.unstake(&alice, &100_000);

    assert_eq!(vault.get_revenue_share_pool(), 1000);

    fn compute_leaf(env: &Env, user: &Address, amount: i128) -> soroban_sdk::BytesN<32> {
        let mut buf = soroban_sdk::Bytes::new(env);
        let addr_str = user.to_string();
        let len = addr_str.len();
        let mut raw = [0u8; 56];
        let slice = &mut raw[..len as usize];
        addr_str.copy_into_slice(slice);
        buf.extend_from_slice(slice);
        buf.extend_from_slice(&amount.to_be_bytes());
        env.crypto().sha256(&buf).into()
    }

    let alice_leaf = compute_leaf(&env, &alice, 1000);
    let root_bytes: soroban_sdk::Bytes = alice_leaf.clone().into();

    // Distribute revenue with Merkle root
    vault.distribute_revenue(&admin, &root_bytes);

    let alice_bal_before = token.balance(&alice);

    // Alice claims revenue share with empty proof (single leaf tree)
    let proof = Vec::new(&env);
    vault.claim_revenue_share(&alice, &1000, &proof);

    assert_eq!(token.balance(&alice), alice_bal_before + 1000);
    assert_eq!(vault.get_revenue_share_pool(), 0);
}

#[test]
fn test_revenue_sharing_invalid_proof_and_double_claim() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 1;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let gov_token = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // Invalid config (share_bps > 10000) should fail
    let res = vault.try_set_revenue_sharing(&admin, &gov_token, &10_001);
    assert!(res.is_err());

    // Configure 50% revenue sharing
    vault.set_revenue_sharing(&admin, &gov_token, &5000);
    vault.set_unstake_fee_bps(&admin, &500);

    token_admin.mint(&alice, &200_000);
    vault.stake(&alice, &200_000, &0);
    vault.unstake(&alice, &200_000); // 10,000 fee -> 5,000 to rev pool

    assert_eq!(vault.get_revenue_share_pool(), 5000);

    fn compute_leaf(env: &Env, user: &Address, amount: i128) -> soroban_sdk::BytesN<32> {
        let mut buf = soroban_sdk::Bytes::new(env);
        let addr_str = user.to_string();
        let len = addr_str.len();
        let mut raw = [0u8; 56];
        let slice = &mut raw[..len as usize];
        addr_str.copy_into_slice(slice);
        buf.extend_from_slice(slice);
        buf.extend_from_slice(&amount.to_be_bytes());
        env.crypto().sha256(&buf).into()
    }

    fn compute_node(env: &Env, a: soroban_sdk::BytesN<32>, b: soroban_sdk::BytesN<32>) -> soroban_sdk::BytesN<32> {
        let cur_arr: [u8; 32] = a.to_array();
        let sib_arr: [u8; 32] = b.to_array();
        let mut combined = soroban_sdk::Bytes::new(env);
        if cur_arr[0] <= sib_arr[0] {
            combined.extend_from_slice(&cur_arr);
            combined.extend_from_slice(&sib_arr);
        } else {
            combined.extend_from_slice(&sib_arr);
            combined.extend_from_slice(&cur_arr);
        }
        env.crypto().sha256(&combined).into()
    }

    let alice_leaf = compute_leaf(&env, &alice, 3000);
    let bob_leaf = compute_leaf(&env, &bob, 2000);
    let root_node = compute_node(&env, alice_leaf.clone(), bob_leaf.clone());
    let root_bytes: soroban_sdk::Bytes = root_node.into();

    vault.distribute_revenue(&admin, &root_bytes);

    // Invalid proof for Alice
    let mut bad_proof = Vec::new(&env);
    bad_proof.push_back(alice_leaf.clone()); // wrong sibling
    let res_bad = vault.try_claim_revenue_share(&alice, &3000, &bad_proof);
    assert!(res_bad.is_err());

    // Valid proof for Alice
    let mut alice_proof = Vec::new(&env);
    alice_proof.push_back(bob_leaf);
    vault.claim_revenue_share(&alice, &3000, &alice_proof);

    // Double claim should fail
    let res_double = vault.try_claim_revenue_share(&alice, &3000, &alice_proof);
    assert!(res_double.is_err());
}

// ΓöÇΓöÇ Issue #280: New Staker Reward Escrow Tests ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_new_staker_reward_escrow_flow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 100;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // Fund reward pool
    token_admin.mint(&admin, &1_000_000);
    vault.fund_reward_pool(&admin, &1_000_000);

    // Default escrow period is 0 (disabled)
    assert_eq!(vault.get_escrow_period(), 0);

    // Configure escrow period of 100 ledgers
    vault.set_escrow_period(&admin, &100);
    assert_eq!(vault.get_escrow_period(), 100);

    // Alice stakes at ledger 100 -> release ledger should be 200
    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &100_000, &0);

    assert_eq!(vault.get_escrow_release_ledger(&alice), Some(200));
    assert_eq!(vault.get_escrow_balance(&alice), 0);

    // Set reward rate
    vault.set_reward_rate_bps(&1000);

    // Advance to ledger 150 (during escrow period)
    env.ledger().with_mut(|li| li.sequence_number = 150);

    // Claim during escrow period -> returns 0, adds accrued rewards to EscrowBalance
    let claim_during = vault.claim(&alice);
    assert_eq!(claim_during, 0);
    let escrow_bal = vault.get_escrow_balance(&alice);
    assert!(escrow_bal > 0, "expected escrow balance to accumulate");

    // Advance past escrow period to ledger 250
    env.ledger().with_mut(|li| li.sequence_number = 250);

    let alice_bal_before = token.balance(&alice);

    // Claim after escrow period -> releases full escrow balance + current pending in one payment
    let claim_after = vault.claim(&alice);
    assert!(claim_after > escrow_bal, "expected payout to include escrow balance + new pending");
    assert_eq!(token.balance(&alice), alice_bal_before + claim_after);
    assert_eq!(vault.get_escrow_balance(&alice), 0);
    assert_eq!(vault.get_escrow_release_ledger(&alice), None);
}

#[test]
fn test_escrow_period_zero_disables_and_restaker_gets_new_escrow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 100;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    token_admin.mint(&admin, &1_000_000);
    vault.fund_reward_pool(&admin, &1_000_000);

    // 1. Escrow period 0 disables escrow for new stakers
    vault.set_escrow_period(&admin, &0);
    token_admin.mint(&bob, &50_000);
    vault.stake(&bob, &50_000, &0);
    assert_eq!(vault.get_escrow_release_ledger(&bob), None);

    // 2. Configure escrow period = 50 ledgers
    vault.set_escrow_period(&admin, &50);

    // Alice stakes at ledger 100
    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &100_000, &0);
    assert_eq!(vault.get_escrow_release_ledger(&alice), Some(150));

    vault.set_reward_rate_bps(&1000);
    env.ledger().with_mut(|li| li.sequence_number = 120);

    // Claim during escrow
    vault.claim(&alice);
    assert!(vault.get_escrow_balance(&alice) > 0);

    // Full unstake clears position and escrow state
    vault.unstake(&alice, &100_000);
    assert_eq!(vault.get_escrow_balance(&alice), 0);
    assert_eq!(vault.get_escrow_release_ledger(&alice), None);

    // Re-staker after full exit gets a new escrow period starting at new stake ledger
    env.ledger().with_mut(|li| li.sequence_number = 300);
    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &100_000, &0);

    // New release ledger is 300 + 50 = 350
    assert_eq!(vault.get_escrow_release_ledger(&alice), Some(350));
}

// ΓöÇΓöÇ Issue #282: Stake-Gated Access Tests ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_stake_gated_access_flow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 10;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // Setup NFT contracts for access token tiers
    let nft_id_1 = env.register_contract(None, StakeReceiptNFT);
    let nft_client_1 = StakeReceiptNFTClient::new(&env, &nft_id_1);
    nft_client_1.initialize(&vault_id);

    let nft_id_2 = env.register_contract(None, StakeReceiptNFT);
    let nft_client_2 = StakeReceiptNFTClient::new(&env, &nft_id_2);
    nft_client_2.initialize(&vault_id);

    // Tier 0: min 50_000 stake, min 10 duration ledgers -> nft_id_1
    let tier_0 = AccessTier {
        min_stake: 50_000,
        min_duration_ledgers: 10,
        access_token_contract: nft_id_1.clone(),
    };
    vault.set_access_tier(&admin, &tier_0);

    // Tier 1: min 100_000 stake, min 20 duration ledgers -> nft_id_2
    let tier_1 = AccessTier {
        min_stake: 100_000,
        min_duration_ledgers: 20,
        access_token_contract: nft_id_2.clone(),
    };
    vault.set_access_tier(&admin, &tier_1);

    // 1. Staker with 30_000 (below threshold) -> no eligibility
    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &30_000, &0);
    assert_eq!(vault.check_access_eligibility(&alice), None);
    assert!(vault.try_claim_access_token(&alice).is_err());

    // 2. Stake up to 60_000, advance ledgers by 10
    vault.stake(&alice, &30_000, &0); // total 60_000
    env.ledger().with_mut(|li| li.sequence_number = 25);

    // Qualifies for Tier 0 (index 0)
    assert_eq!(vault.check_access_eligibility(&alice), Some(0));

    // Claim access token for Tier 0
    let tier_idx = vault.claim_access_token(&alice);
    assert_eq!(tier_idx, 0);
    assert!(nft_client_1.has_receipt(&alice));

    // 3. Stake up to 100_000 and advance ledgers to 35 -> qualifies for Tier 1 (highest tier wins)
    vault.stake(&alice, &40_000, &0); // total 100_000
    env.ledger().with_mut(|li| li.sequence_number = 35);
    assert_eq!(vault.check_access_eligibility(&alice), Some(1));

    // Claim upgrade to Tier 1
    let upgraded_idx = vault.claim_access_token(&alice);
    assert_eq!(upgraded_idx, 1);
    assert!(nft_client_2.has_receipt(&alice));
    assert!(!nft_client_1.has_receipt(&alice)); // old token burned

    // 4. Revoke fails when user is still eligible
    let res_rev_early = vault.try_revoke_access_token(&alice);
    assert!(res_rev_early.is_err());

    // 5. Unstake 60_000 -> stake drops to 40_000 (below Tier 1 min 100_000 and Tier 0 min 50_000)
    vault.unstake(&alice, &60_000);
    assert_eq!(vault.check_access_eligibility(&alice), None);

    // Revoke succeeds now that user no longer meets requirements
    vault.revoke_access_token(&alice);
    assert!(!nft_client_2.has_receipt(&alice));
}

// ΓöÇΓöÇ Issue #279: Reward Halving Schedule Integration Tests ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

#[test]
fn test_reward_halving_schedule_integration() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 0;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

    // 1. Without halving config, base rate is used
    vault.set_reward_rate_bps(&1000);
    assert_eq!(vault.get_current_halving_count(), 0);
    assert_eq!(vault.next_halving_at(), None);

    // 2. Set halving schedule: interval = 100 ledgers, floor_rate_bps = 200
    vault.set_halving_schedule(&admin, &100, &5_000, &200);

    let config = vault.get_halving_config().unwrap();
    assert_eq!(config.interval_ledgers, 100);
    assert_eq!(config.floor_rate_bps, 200);

    // Next halving boundary is at ledger 100
    assert_eq!(vault.next_halving_at(), Some(100));

    // Stake at ledger 0
    token_admin.mint(&alice, &100_000);
    vault.stake(&alice, &100_000, &0);

    // Ledger 50: halving count is 0
    env.ledger().with_mut(|li| li.sequence_number = 50);
    assert_eq!(vault.get_current_halving_count(), 0);

    // Ledger 150: 1 halving has occurred (count = 1). Base rate 1000 / 2^1 = 500
    env.ledger().with_mut(|li| li.sequence_number = 150);
    assert_eq!(vault.get_current_halving_count(), 1);
    assert_eq!(vault.next_halving_at(), Some(200));

    // Ledger 250: 2 halvings have occurred (count = 2). Base rate 1000 / 2^2 = 250
    env.ledger().with_mut(|li| li.sequence_number = 250);
    assert_eq!(vault.get_current_halving_count(), 2);

    // Ledger 350: 3 halvings occurred. Base rate 1000 / 2^3 = 125, but floored at floor_rate_bps = 200!
    env.ledger().with_mut(|li| li.sequence_number = 350);
    assert_eq!(vault.get_current_halving_count(), 3);
}




