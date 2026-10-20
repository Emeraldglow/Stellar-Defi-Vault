#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Bytes, Env, String, Vec,
};

use crate::reward_token_audit_trail::MovementType;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

// ── Test Harness & Fixture ───────────────────────────────────────────────────

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
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
    reporter: Address,
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
        let bob = Address::generate(&env);
        let reporter = Address::generate(&env);

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
            reporter,
        }
    }

    fn set_accrued_reward(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            crate::balance::set_accrued_reward(&self.env, user, amount);
        });
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Issue #467: Reward Token Audit Trail Tests
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_log_reward_movement_monotonic_id_and_all_types() {
    let f = Fixture::new();

    let dest = Address::generate(&f.env);
    let trigger = String::from_str(&f.env, "test_func");

    let types = [
        MovementType::RewardPaid,
        MovementType::BuybackBurn,
        MovementType::TreasuryWithdraw,
        MovementType::InsurancePayout,
        MovementType::ReserveTopUp,
        MovementType::DeferredClaim,
        MovementType::MutualInsurance,
    ];

    for (idx, m_type) in types.iter().enumerate() {
        let id = f
            .vault
            .log_reward_movement(m_type, &f.vault_id, &dest, &1000, &trigger);
        assert_eq!(id, (idx as u64) + 1);
    }

    assert_eq!(f.vault.get_audit_log_count(), 7);
}

#[test]
fn test_audit_log_pagination_100_entries() {
    let f = Fixture::new();
    let dest = Address::generate(&f.env);
    let trigger = String::from_str(&f.env, "bulk");

    for _ in 0..150 {
        f.vault.log_reward_movement(
            &MovementType::RewardPaid,
            &f.vault_id,
            &dest,
            &100,
            &trigger,
        );
    }

    assert_eq!(f.vault.get_audit_log_count(), 150);

    let page0 = f.vault.get_audit_log_page(&0);
    assert_eq!(page0.len(), 100);
    assert_eq!(page0.get(0).unwrap().movement_id, 1);
    assert_eq!(page0.get(99).unwrap().movement_id, 100);

    let page1 = f.vault.get_audit_log_page(&1);
    assert_eq!(page1.len(), 50);
    assert_eq!(page1.get(0).unwrap().movement_id, 101);
    assert_eq!(page1.get(49).unwrap().movement_id, 150);
}

#[test]
fn test_audit_log_summary_sums_correctly() {
    let f = Fixture::new();
    let dest = Address::generate(&f.env);
    let trigger = String::from_str(&f.env, "manual");

    f.vault.log_reward_movement(
        &MovementType::RewardPaid,
        &f.vault_id,
        &dest,
        &1000,
        &trigger,
    );
    f.vault.log_reward_movement(
        &MovementType::DeferredClaim,
        &f.vault_id,
        &dest,
        &500,
        &trigger,
    );
    f.vault.log_reward_movement(
        &MovementType::BuybackBurn,
        &f.vault_id,
        &dest,
        &300,
        &trigger,
    );
    f.vault.log_reward_movement(
        &MovementType::TreasuryWithdraw,
        &f.vault_id,
        &dest,
        &200,
        &trigger,
    );
    f.vault.log_reward_movement(
        &MovementType::ReserveTopUp,
        &f.vault_id,
        &dest,
        &5000,
        &trigger,
    );

    let (paid, burned, withdrawn, topped_up) = f.vault.get_audit_log_summary();
    assert_eq!(paid, 1500);
    assert_eq!(burned, 300);
    assert_eq!(withdrawn, 200);
    assert_eq!(topped_up, 5000);
}

// ═════════════════════════════════════════════════════════════════════════════
// Issue #468: Stake-Funded Bug Bounty Tests
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_set_and_get_bug_bounty_contribution() {
    let f = Fixture::new();

    assert_eq!(f.vault.get_bug_bounty_contribution(&f.alice), 0);

    f.vault.set_bug_bounty_contribution_bps(&f.alice, &500); // 5%
    assert_eq!(f.vault.get_bug_bounty_contribution(&f.alice), 500);

    // Max 1000 bps (10%)
    f.vault.set_bug_bounty_contribution_bps(&f.alice, &1000);
    assert_eq!(f.vault.get_bug_bounty_contribution(&f.alice), 1000);

    // Exceeding 1000 fails
    let err = f
        .vault
        .try_set_bug_bounty_contribution_bps(&f.alice, &1001);
    assert!(err.is_err());

    // Opt-out setting 0
    f.vault.set_bug_bounty_contribution_bps(&f.alice, &0);
    assert_eq!(f.vault.get_bug_bounty_contribution(&f.alice), 0);
}

#[test]
fn test_claim_deducts_contribution_and_accumulates_fund() {
    let f = Fixture::new();

    // Alice stakes
    f.vault.deposit(&f.alice, &10_000, &0);

    // Alice opts in for 10% (1000 bps) bug bounty contribution
    f.vault.set_bug_bounty_contribution_bps(&f.alice, &1000);

    // Set accrued reward manually for test
    f.set_accrued_reward(&f.alice, 1000);

    let alice_bal_before = f.token.balance(&f.alice);
    let fund_before = f.vault.get_bug_bounty_fund_balance();
    assert_eq!(fund_before, 0);

    let claimed = f.vault.claim(&f.alice);
    // 1000 accrued - 10% (100) = 900 paid to user
    assert_eq!(claimed, 900);

    let alice_bal_after = f.token.balance(&f.alice);
    assert_eq!(alice_bal_after - alice_bal_before, 900);

    let fund_after = f.vault.get_bug_bounty_fund_balance();
    assert_eq!(fund_after, 100);
}

#[test]
fn test_pay_bug_bounty_payout_and_insufficient_revert() {
    let f = Fixture::new();

    // Build up bug bounty fund
    f.vault.deposit(&f.alice, &10_000, &0);
    f.vault.set_bug_bounty_contribution_bps(&f.alice, &1000);
    f.set_accrued_reward(&f.alice, 5000);
    f.vault.claim(&f.alice);

    assert_eq!(f.vault.get_bug_bounty_fund_balance(), 500);

    let desc_hash = Bytes::from_array(&f.env, &[7u8; 32]);
    let reporter_bal_before = f.token.balance(&f.reporter);

    // Admin pays bounty
    f.vault
        .pay_bug_bounty(&f.admin, &f.reporter, &300, &desc_hash);

    let reporter_bal_after = f.token.balance(&f.reporter);
    assert_eq!(reporter_bal_after - reporter_bal_before, 300);
    assert_eq!(f.vault.get_bug_bounty_fund_balance(), 200);

    // Paying more than remaining 200 reverts
    let err = f
        .vault
        .try_pay_bug_bounty(&f.admin, &f.reporter, &201, &desc_hash);
    assert!(err.is_err());
}

// ═════════════════════════════════════════════════════════════════════════════
// Issue #470: Cross-Pool Identity Tests
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_register_and_sync_cross_pool_identity() {
    let f = Fixture::new();

    // Create sibling vault
    let sibling_id = f.env.register_contract(None, VaultContract);
    let sibling_vault = VaultContractClient::new(&f.env, &sibling_id);
    let token_addr = f.vault.get_stake_token();
    sibling_vault.initialize(&f.admin, &token_addr, &500_u32, &None, &None);

    f.token_admin.mint(&f.alice, &50_000);
    f.vault.deposit(&f.alice, &10_000, &0);
    sibling_vault.deposit(&f.alice, &20_000, &0);

    let mut pools = Vec::new(&f.env);
    pools.push_back(f.vault_id.clone());
    pools.push_back(sibling_id.clone());

    f.vault.register_cross_pool_identity(&f.alice, &pools);

    let id = f.vault.get_cross_pool_identity(&f.alice).unwrap();
    assert_eq!(id.linked_pools.len(), 2);
    assert_eq!(id.total_staked_all_pools, 0);

    let synced_total = f.vault.sync_cross_pool_stake(&f.alice);
    assert_eq!(synced_total, 30_000);

    assert_eq!(f.vault.get_cross_pool_total_staked(&f.alice), 30_000);
}

#[test]
fn test_cross_pool_max_10_pools_enforced() {
    let f = Fixture::new();

    let mut pools = Vec::new(&f.env);
    for _ in 0..11 {
        pools.push_back(Address::generate(&f.env));
    }

    let err = f.vault.try_register_cross_pool_identity(&f.alice, &pools);
    assert!(err.is_err());
}

#[test]
fn test_cross_pool_governance_weight_toggle() {
    let f = Fixture::new();

    let sibling_id = f.env.register_contract(None, VaultContract);
    let sibling_vault = VaultContractClient::new(&f.env, &sibling_id);
    let token_addr = f.vault.get_stake_token();
    sibling_vault.initialize(&f.admin, &token_addr, &500_u32, &None, &None);

    f.vault.deposit(&f.alice, &5_000, &0);
    sibling_vault.deposit(&f.alice, &15_000, &0);

    let mut pools = Vec::new(&f.env);
    pools.push_back(f.vault_id.clone());
    pools.push_back(sibling_id);
    f.vault.register_cross_pool_identity(&f.alice, &pools);
    f.vault.sync_cross_pool_stake(&f.alice);

    // Default: disabled -> vote weight is single pool (5,000)
    assert_eq!(f.vault.get_cross_pool_governance_weight(), false);
    assert_eq!(f.vault.current_vote_weight(&f.alice), 5_000);

    // Enable cross-pool governance weight
    f.vault
        .set_cross_pool_governance_weight(&f.admin, &true);
    assert_eq!(f.vault.get_cross_pool_governance_weight(), true);
    assert_eq!(f.vault.current_vote_weight(&f.alice), 20_000);

    // Disable
    f.vault
        .set_cross_pool_governance_weight(&f.admin, &false);
    assert_eq!(f.vault.current_vote_weight(&f.alice), 5_000);
}

#[test]
fn test_sync_cross_pool_handles_unreachable_pool_gracefully() {
    let f = Fixture::new();

    f.vault.deposit(&f.alice, &7_000, &0);

    let dead_address = Address::generate(&f.env);
    let mut pools = Vec::new(&f.env);
    pools.push_back(f.vault_id.clone());
    pools.push_back(dead_address);

    f.vault.register_cross_pool_identity(&f.alice, &pools);
    let synced = f.vault.sync_cross_pool_stake(&f.alice);
    assert_eq!(synced, 7_000);
}

// ═════════════════════════════════════════════════════════════════════════════
// Issue #469: Position Value Appreciation Log Tests
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_position_value_appreciation_log_snapshots_and_growth() {
    let f = Fixture::new();

    f.vault.deposit(&f.alice, &10_000, &0);
    f.set_accrued_reward(&f.alice, 0);

    // First snapshot
    let s1 = f.vault.take_value_snapshot(&f.alice);
    assert_eq!(s1.principal, 10_000);
    assert_eq!(s1.pending_reward, 0);
    assert_eq!(s1.total_value, 10_000);
    assert_eq!(s1.appreciation_bps_since_last, 0);

    // Growth: 2,000 rewards accrued (total value 12,000, 20% growth = 2000 bps)
    f.set_accrued_reward(&f.alice, 2_000);
    let s2 = f.vault.take_value_snapshot(&f.alice);
    assert_eq!(s2.principal, 10_000);
    assert_eq!(s2.pending_reward, 2_000);
    assert_eq!(s2.total_value, 12_000);
    assert_eq!(s2.appreciation_bps_since_last, 2000);

    let log = f.vault.get_value_appreciation_log(&f.alice);
    assert_eq!(log.len(), 2);

    let total_app_bps = f.vault.get_total_appreciation_bps(&f.alice);
    assert_eq!(total_app_bps, 2000);
}

#[test]
fn test_appreciation_log_52_entry_rollover() {
    let f = Fixture::new();

    f.vault.deposit(&f.alice, &1_000, &0);

    for i in 0..60 {
        f.set_accrued_reward(&f.alice, (i + 1) * 10);
        f.vault.take_value_snapshot(&f.alice);
    }

    let log = f.vault.get_value_appreciation_log(&f.alice);
    assert_eq!(log.len(), 52);
    // Oldest entries 0..7 were dropped; entry 0 in log corresponds to i=8 (accrued 90)
    assert_eq!(log.get(0).unwrap().pending_reward, 90);
    assert_eq!(log.get(51).unwrap().pending_reward, 600);
}

#[test]
fn test_auto_snapshot_on_claim() {
    let f = Fixture::new();

    f.vault.deposit(&f.alice, &10_000, &0);
    f.set_accrued_reward(&f.alice, 1_000);

    assert_eq!(f.vault.get_value_appreciation_log(&f.alice).len(), 0);

    // First claim takes auto snapshot
    f.vault.claim(&f.alice);
    let log = f.vault.get_value_appreciation_log(&f.alice);
    assert_eq!(log.len(), 1);

    // Next claim inside 7 days doesn't snapshot
    f.set_accrued_reward(&f.alice, 500);
    f.vault.claim(&f.alice);
    assert_eq!(f.vault.get_value_appreciation_log(&f.alice).len(), 1);

    // Advance sequence by >7 days
    f.env.ledger().with_mut(|li| {
        li.sequence_number += (LEDGERS_PER_DAY * 7) + 1;
    });

    f.set_accrued_reward(&f.alice, 500);
    f.vault.claim(&f.alice);
    assert_eq!(f.vault.get_value_appreciation_log(&f.alice).len(), 2);
}
