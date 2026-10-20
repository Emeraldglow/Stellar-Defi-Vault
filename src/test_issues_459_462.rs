#![cfg(test)]

extern crate std;

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger as _},
    token, Address, Bytes, Env, String,
};

use crate::lockdrop_campaign::LockdropConfig;
use crate::position_health_auto_recovery::{RecoveryAction, RECOVERY_COOLDOWN_LEDGERS};
use crate::proof_of_humanity_hook::OracleFallbackMode;
use crate::roadmap_voting::ROADMAP_EPOCH_LEDGERS;
use crate::storage::{Loan, LoanConfig};
use crate::vault::{VaultContract, VaultContractClient};

// ── Mock proof-of-humanity oracle ────────────────────────────────────────────

#[contract]
pub struct MockHumanityOracle;

#[contractimpl]
impl MockHumanityOracle {
    pub fn set_verified(env: Env, addr: Address, verified: bool) {
        env.storage().persistent().set(&addr, &verified);
    }

    pub fn is_verified(env: Env, address: Address) -> bool {
        env.storage().persistent().get(&address).unwrap_or(false)
    }
}

// ── Fixture ──────────────────────────────────────────────────────────────────

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

        let (token_addr, token, token_admin) = create_token(&env, &admin);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        token_admin.mint(&admin, &100_000_000);
        token_admin.mint(&alice, &100_000_000);
        token_admin.mint(&bob, &100_000_000);
        token_admin.mint(&vault_id, &50_000_000);

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

    fn set_accrued_reward(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            crate::balance::set_accrued_reward(&self.env, user, amount);
        });
    }

    fn set_loan(&self, user: &Address, max_ltv_bps: u32, principal: i128, interest: i128) {
        self.env.as_contract(&self.vault_id, || {
            crate::balance::set_loan_config(
                &self.env,
                &LoanConfig {
                    max_ltv_bps,
                    interest_rate_bps: 0,
                },
            );
            crate::balance::set_loan(
                &self.env,
                user,
                &Loan {
                    principal,
                    interest_accrued: interest,
                    opened_at: 1000,
                    last_interest_at: 1000,
                },
            );
        });
    }

    fn read_loan(&self, user: &Address) -> Option<Loan> {
        self.env
            .as_contract(&self.vault_id, || crate::balance::get_loan(&self.env, user))
    }

    fn advance(&self, ledgers: u32) {
        self.env.ledger().with_mut(|li| {
            li.sequence_number += ledgers;
        });
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Issue #459: position health auto-recovery
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_health_below_threshold_triggers_recovery() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    // collateral 10_000 * 50% ltv = 5_000 borrowable; debt 4_000 -> health 12_500 bps.
    f.set_loan(&f.alice, 5_000, 4_000, 0);
    assert_eq!(f.vault.position_health_bps(&f.alice), Some(12_500));

    f.set_accrued_reward(&f.alice, 2_000);
    f.vault
        .set_recovery_config(&f.alice, &15_000, &RecoveryAction::AutoClaim, &0);

    let alice_before = f.token.balance(&f.alice);
    let value = f.vault.check_and_recover(&f.bob, &f.alice);

    assert_eq!(value, 2_000);
    assert_eq!(f.token.balance(&f.alice) - alice_before, 2_000);
    assert_eq!(
        f.env
            .as_contract(&f.vault_id, || crate::balance::get_accrued_reward(&f.env, &f.alice)),
        0
    );
}

#[test]
fn test_recovery_not_triggered_above_threshold() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    f.set_loan(&f.alice, 5_000, 4_000, 0); // health 12_500 bps
    f.set_accrued_reward(&f.alice, 2_000);
    f.vault
        .set_recovery_config(&f.alice, &10_000, &RecoveryAction::AutoClaim, &0);

    let res = f.vault.try_check_and_recover(&f.bob, &f.alice);
    assert!(res.is_err());
}

#[test]
fn test_recovery_not_triggered_without_loan() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    f.set_accrued_reward(&f.alice, 2_000);
    assert_eq!(f.vault.position_health_bps(&f.alice), None);
    f.vault
        .set_recovery_config(&f.alice, &50_000, &RecoveryAction::AutoClaim, &0);

    assert!(f.vault.try_check_and_recover(&f.bob, &f.alice).is_err());
}

#[test]
fn test_correct_action_executed_auto_repay_loan() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    f.set_loan(&f.alice, 5_000, 4_000, 0); // health 12_500 bps
    f.vault
        .set_recovery_config(&f.alice, &15_000, &RecoveryAction::AutoRepayLoan, &1_000);

    let value = f.vault.check_and_recover(&f.bob, &f.alice);
    assert_eq!(value, 1_000);

    let loan = f.read_loan(&f.alice).unwrap();
    assert_eq!(loan.principal, 3_000);
    // Alice's collateral was reduced by the repaid amount.
    assert_eq!(f.vault.staked_amount(&f.alice), 9_000);
}

#[test]
fn test_correct_action_executed_auto_unstake_partial() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    f.set_loan(&f.alice, 5_000, 4_000, 0);
    f.vault.set_recovery_config(
        &f.alice,
        &15_000,
        &RecoveryAction::AutoUnstakePartial,
        &2_000,
    );

    let value = f.vault.check_and_recover(&f.bob, &f.alice);
    assert_eq!(value, 2_000);

    // Unstaked proceeds paid down the loan (reducing LTV).
    let loan = f.read_loan(&f.alice).unwrap();
    assert_eq!(loan.principal, 2_000);
    assert_eq!(f.vault.staked_amount(&f.alice), 8_000);
}

#[test]
fn test_daily_cooldown_enforced() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    f.set_loan(&f.alice, 5_000, 4_000, 0);
    f.set_accrued_reward(&f.alice, 1_000);
    f.vault
        .set_recovery_config(&f.alice, &15_000, &RecoveryAction::AutoClaim, &0);

    f.vault.check_and_recover(&f.bob, &f.alice);

    // Second attempt inside the cooldown window reverts.
    f.set_accrued_reward(&f.alice, 1_000);
    assert!(f.vault.try_check_and_recover(&f.bob, &f.alice).is_err());

    // After the cooldown elapses it succeeds again.
    f.advance(RECOVERY_COOLDOWN_LEDGERS + 1);
    let value = f.vault.check_and_recover(&f.bob, &f.alice);
    assert_eq!(value, 1_000);
}

#[test]
fn test_keeper_earns_incentive() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    f.set_loan(&f.alice, 5_000, 4_000, 0);
    f.set_accrued_reward(&f.alice, 10_000);
    f.vault
        .set_recovery_config(&f.alice, &15_000, &RecoveryAction::AutoClaim, &0);

    let keeper_before = f.token.balance(&f.bob);
    let value = f.vault.check_and_recover(&f.bob, &f.alice);

    // 0.5% of the 10_000 recovery action value.
    assert_eq!(value, 10_000);
    assert_eq!(f.token.balance(&f.bob) - keeper_before, 50);
}

#[test]
fn test_cancel_recovery_config() {
    let f = Fixture::new();
    f.vault
        .set_recovery_config(&f.alice, &15_000, &RecoveryAction::AutoClaim, &0);
    assert!(f.vault.get_recovery_config(&f.alice).is_some());
    f.vault.cancel_recovery_config(&f.alice);
    assert!(f.vault.get_recovery_config(&f.alice).is_none());
    assert!(f.vault.try_cancel_recovery_config(&f.alice).is_err());
}

// ═════════════════════════════════════════════════════════════════════════════
// Issue #460: lockdrop campaign
// ═════════════════════════════════════════════════════════════════════════════

fn start_default_lockdrop(f: &Fixture) {
    // pool 3_000, max lock 1_000 ledgers, window 500 ledgers -> ends_at 1_500.
    f.vault.start_lockdrop(&f.admin, &3_000, &1_000, &500);
}

#[test]
fn test_longer_lock_gets_proportionally_more_reward() {
    let f = Fixture::new();
    start_default_lockdrop(&f);

    let alice_score = f.vault.commit_to_lockdrop(&f.alice, &1_000, &100);
    let bob_score = f.vault.commit_to_lockdrop(&f.bob, &1_000, &200);
    assert_eq!(alice_score, 100_000); // 1_000 * 100
    assert_eq!(bob_score, 200_000); // 1_000 * 200
    assert_eq!(f.vault.get_lockdrop_total_score(), 300_000);

    f.advance(600); // past ends_at
    f.vault.finalize_lockdrop(&f.admin);

    let alice_alloc = f.vault.get_lockdrop_allocation(&f.alice);
    let bob_alloc = f.vault.get_lockdrop_allocation(&f.bob);
    assert_eq!(alice_alloc, 1_000); // 3_000 * 100k / 300k
    assert_eq!(bob_alloc, 2_000); // 3_000 * 200k / 300k
    // Twice the lock -> twice the allocation for the same amount.
    assert_eq!(bob_alloc, alice_alloc * 2);
}

#[test]
fn test_score_calculation_correct() {
    let f = Fixture::new();
    start_default_lockdrop(&f);
    let score = f.vault.commit_to_lockdrop(&f.alice, &777, &123);
    assert_eq!(score, 777 * 123);

    let commitment = f.vault.get_lockdrop_commitment(&f.alice).unwrap();
    assert_eq!(commitment.locked_amount, 777);
    assert_eq!(commitment.lock_duration_ledgers, 123);
    assert_eq!(commitment.score, 777 * 123);
}

#[test]
fn test_exit_before_duration_reverts() {
    let f = Fixture::new();
    start_default_lockdrop(&f);
    f.vault.commit_to_lockdrop(&f.alice, &5_000, &100);

    // committed_at 1_000 + 100 = 1_100; still locked.
    assert!(f.vault.try_exit_lockdrop(&f.alice).is_err());

    f.advance(100);
    let alice_before = f.token.balance(&f.alice);
    let returned = f.vault.exit_lockdrop(&f.alice);
    assert_eq!(returned, 5_000);
    assert_eq!(f.token.balance(&f.alice) - alice_before, 5_000);
}

#[test]
fn test_reward_proportional_to_score_share_and_claim() {
    let f = Fixture::new();
    start_default_lockdrop(&f);
    // Equal amount, alice locks 3x as long as bob -> 3x the score share.
    f.vault.commit_to_lockdrop(&f.alice, &1_000, &300);
    f.vault.commit_to_lockdrop(&f.bob, &1_000, &100);

    f.advance(600);
    f.vault.finalize_lockdrop(&f.admin);

    let alice_alloc = f.vault.get_lockdrop_allocation(&f.alice);
    let bob_alloc = f.vault.get_lockdrop_allocation(&f.bob);
    assert_eq!(alice_alloc, 2_250); // 3_000 * 300k / 400k
    assert_eq!(bob_alloc, 750); // 3_000 * 100k / 400k

    let alice_before = f.token.balance(&f.alice);
    let claimed = f.vault.claim_lockdrop_reward(&f.alice);
    assert_eq!(claimed, 2_250);
    assert_eq!(f.token.balance(&f.alice) - alice_before, 2_250);

    // Double claim reverts.
    assert!(f.vault.try_claim_lockdrop_reward(&f.alice).is_err());
}

#[test]
fn test_lockdrop_one_campaign_at_a_time() {
    let f = Fixture::new();
    start_default_lockdrop(&f);
    assert!(f.vault.try_start_lockdrop(&f.admin, &1_000, &1_000, &500).is_err());
}

#[test]
fn test_finalize_before_end_reverts() {
    let f = Fixture::new();
    start_default_lockdrop(&f);
    f.vault.commit_to_lockdrop(&f.alice, &1_000, &100);
    assert!(f.vault.try_finalize_lockdrop(&f.admin).is_err());

    f.advance(600);
    f.vault.finalize_lockdrop(&f.admin);
    let cfg: LockdropConfig = f.vault.get_lockdrop_config().unwrap();
    assert!(cfg.finalized);
}

// ═════════════════════════════════════════════════════════════════════════════
// Issue #461: proof-of-humanity hook
// ═════════════════════════════════════════════════════════════════════════════

fn setup_oracle(f: &Fixture) -> Address {
    let oracle_id = f.env.register_contract(None, MockHumanityOracle);
    f.vault.set_humanity_oracle(&f.admin, &oracle_id);
    // verified min 1_000, unverified min 5_000, unverified surcharge 200 bps (2%).
    f.vault
        .set_humanity_config(&f.admin, &1_000, &5_000, &200);
    oracle_id
}

fn set_verified(env: &Env, oracle_id: &Address, addr: &Address, verified: bool) {
    let client = crate::test_issues_459_462::MockHumanityOracleClient::new(env, oracle_id);
    client.set_verified(addr, &verified);
}

#[test]
fn test_verified_human_uses_lower_minimum() {
    let f = Fixture::new();
    let oracle_id = setup_oracle(&f);
    set_verified(&f.env, &oracle_id, &f.alice, true);

    assert!(f.vault.is_verified_human(&f.alice));
    // 2_000 >= verified min 1_000, below unverified min 5_000.
    let shares = f.vault.stake_verified(&f.alice, &2_000);
    assert_eq!(shares, 2_000); // verified -> no surcharge
    assert_eq!(f.vault.staked_amount(&f.alice), 2_000);
}

#[test]
fn test_unverified_uses_higher_minimum() {
    let f = Fixture::new();
    let oracle_id = setup_oracle(&f);
    set_verified(&f.env, &oracle_id, &f.bob, false);

    assert!(!f.vault.is_verified_human(&f.bob));
    // 2_000 is below the unverified minimum of 5_000.
    assert!(f.vault.try_stake_verified(&f.bob, &2_000).is_err());
    // 6_000 clears the unverified minimum.
    let shares = f.vault.stake_verified(&f.bob, &6_000);
    assert!(shares > 0);
}

#[test]
fn test_surcharge_applied_to_unverified() {
    let f = Fixture::new();
    let oracle_id = setup_oracle(&f);
    set_verified(&f.env, &oracle_id, &f.bob, false);

    let bob_before = f.token.balance(&f.bob);
    f.vault.stake_verified(&f.bob, &10_000);

    // Full 10_000 leaves bob's wallet; 2% (200) is surcharge, 9_800 is staked.
    assert_eq!(bob_before - f.token.balance(&f.bob), 10_000);
    assert_eq!(f.vault.staked_amount(&f.bob), 9_800);
}

#[test]
fn test_oracle_failure_uses_fallback_mode() {
    let f = Fixture::new();
    // Point the oracle at an address with no contract deployed.
    let dead_oracle = Address::generate(&f.env);
    f.vault.set_humanity_oracle(&f.admin, &dead_oracle);
    f.vault.set_humanity_config(&f.admin, &1_000, &5_000, &0);

    // Restrictive (default): treated as unverified.
    f.vault
        .set_oracle_fallback_mode(&f.admin, &OracleFallbackMode::Restrictive);
    assert!(!f.vault.is_verified_human(&f.alice));

    // Permissive: treated as verified.
    f.vault
        .set_oracle_fallback_mode(&f.admin, &OracleFallbackMode::Permissive);
    assert!(f.vault.is_verified_human(&f.alice));
}

#[test]
fn test_no_oracle_treats_all_as_unverified() {
    let f = Fixture::new();
    assert!(!f.vault.is_verified_human(&f.alice));
}

// ═════════════════════════════════════════════════════════════════════════════
// Issue #462: roadmap voting
// ═════════════════════════════════════════════════════════════════════════════

fn add_item(f: &Fixture, title: &str, category: &str) -> u32 {
    f.vault.add_roadmap_item(
        &f.admin,
        &String::from_str(&f.env, title),
        &Bytes::from_array(&f.env, &[1u8; 4]),
        &String::from_str(&f.env, category),
    )
}

#[test]
fn test_100_point_budget_enforced() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    let i1 = add_item(&f, "Feature A", "core");
    let i2 = add_item(&f, "Feature B", "ux");

    f.vault.vote_roadmap_item(&f.alice, &i1, &60);
    f.vault.vote_roadmap_item(&f.alice, &i2, &40);

    let alloc = f.vault.get_roadmap_vote_allocation(&f.alice);
    assert_eq!(alloc.len(), 2);
}

#[test]
fn test_exceeding_budget_reverts() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    let i1 = add_item(&f, "Feature A", "core");
    let i2 = add_item(&f, "Feature B", "ux");

    f.vault.vote_roadmap_item(&f.alice, &i1, &60);
    // 60 + 41 = 101 > 100.
    assert!(f.vault.try_vote_roadmap_item(&f.alice, &i2, &41).is_err());
    // Re-voting the same item replaces its weight, so this is fine.
    f.vault.vote_roadmap_item(&f.alice, &i1, &100);
}

#[test]
fn test_rankings_sorted_correctly() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    let i1 = add_item(&f, "Low", "a");
    let i2 = add_item(&f, "High", "b");
    let i3 = add_item(&f, "Mid", "c");

    f.vault.vote_roadmap_item(&f.alice, &i1, &20);
    f.vault.vote_roadmap_item(&f.alice, &i2, &50);
    f.vault.vote_roadmap_item(&f.alice, &i3, &30);

    let rankings = f.vault.get_roadmap_rankings();
    assert_eq!(rankings.get(0).unwrap().id, i2);
    assert_eq!(rankings.get(1).unwrap().id, i3);
    assert_eq!(rankings.get(2).unwrap().id, i1);
}

#[test]
fn test_monthly_reset_clears_allocations() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &10_000, &0);
    let i1 = add_item(&f, "Feature A", "core");

    f.vault.vote_roadmap_item(&f.alice, &i1, &100);
    assert_eq!(f.vault.get_roadmap_vote_allocation(&f.alice).len(), 1);

    f.advance(ROADMAP_EPOCH_LEDGERS);
    // Stale epoch -> allocation reads as empty.
    assert_eq!(f.vault.get_roadmap_vote_allocation(&f.alice).len(), 0);

    // Fresh budget: previous allocation is rolled back, so item votes reset
    // to the new allocation only.
    f.vault.vote_roadmap_item(&f.alice, &i1, &80);
    let rankings = f.vault.get_roadmap_rankings();
    assert_eq!(rankings.get(0).unwrap().votes, 80);
}

#[test]
fn test_roadmap_item_caps() {
    let f = Fixture::new();
    for n in 0..20 {
        add_item(&f, "item", "cat");
        let _ = n;
    }
    assert!(f
        .vault
        .try_add_roadmap_item(
            &f.admin,
            &String::from_str(&f.env, "overflow"),
            &Bytes::from_array(&f.env, &[0u8; 4]),
            &String::from_str(&f.env, "cat"),
        )
        .is_err());
}

#[test]
fn test_title_max_length_enforced() {
    let f = Fixture::new();
    let long_title: std::string::String = core::iter::repeat('x').take(81).collect();
    assert!(f
        .vault
        .try_add_roadmap_item(
            &f.admin,
            &String::from_str(&f.env, &long_title),
            &Bytes::from_array(&f.env, &[0u8; 4]),
            &String::from_str(&f.env, "cat"),
        )
        .is_err());
}

#[test]
fn test_remove_roadmap_item() {
    let f = Fixture::new();
    let i1 = add_item(&f, "Feature A", "core");
    let i2 = add_item(&f, "Feature B", "ux");
    f.vault.remove_roadmap_item(&f.admin, &i1);

    let items = f.vault.get_roadmap_items();
    assert_eq!(items.len(), 1);
    assert_eq!(items.get(0).unwrap().id, i2);
    assert!(f.vault.try_remove_roadmap_item(&f.admin, &i1).is_err());
}
