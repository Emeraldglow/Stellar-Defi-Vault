#![cfg(test)]
//! Tests for proposal categories (#761), expedited proposals (#762),
//! vote delegation expiry (#763), and strategy adapter interface (#764).

extern crate std;

use soroban_sdk::{
    contract, contractimpl, testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::{
    balance,
    errors::GovernanceStrategyError,
    storage::ProposableParam,
    vault::{VaultContract, VaultContractClient},
    vault_extensions_761_764::{
        CategoryParams, ProposalCategory, StrategyConfig, MAX_STRATEGIES,
        MIN_EXPEDITED_BOND,
    },
};

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

// ── Mock Strategy Adapter ───────────────────────────────────────────────────

#[contract]
pub struct MockStrategyAdapter;

#[contractimpl]
impl MockStrategyAdapter {
    pub fn deposit(env: Env, amount: i128) {
        let current: i128 = env.storage().instance().get(&1u32).unwrap_or(0);
        env.storage().instance().set(&1u32, &(current + amount));
    }

    pub fn withdraw(env: Env, amount: i128) -> i128 {
        let current: i128 = env.storage().instance().get(&1u32).unwrap_or(0);
        let actual = if amount > current { current } else { amount };
        env.storage().instance().set(&1u32, &(current - actual));
        actual
    }

    pub fn total_assets(env: Env) -> i128 {
        env.storage().instance().get(&1u32).unwrap_or(0)
    }
}

// ── Test Fixture ────────────────────────────────────────────────────────────

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    token: token::Client<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
    charlie: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.sequence_number = 100;
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
        });

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let charlie = Address::generate(&env);

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract(token_admin);
        let token = token::Client::new(&env, &token_id);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);

        vault.initialize(
            &admin,
            &token_id,
            &token_id,
            &100, // 1% reward
            &0,   // no lock
            &0,   // no penalty
            &0,   // no fee
            &100, // min stake
        );

        // Fund accounts
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id);
        token_admin_client.mint(&alice, &100_000_000_000);
        token_admin_client.mint(&bob, &100_000_000_000);
        token_admin_client.mint(&charlie, &100_000_000_000);
        token_admin_client.mint(&admin, &100_000_000_000);

        Fixture {
            env,
            vault,
            vault_id,
            token,
            admin,
            alice,
            bob,
            charlie,
        }
    }
}

// ── Tests for Issue #761: Proposal Categories ───────────────────────────────

#[test]
fn test_category_params_set_and_query() {
    let f = Fixture::new();

    // Query default params
    let def_param = f.vault.get_category_params(&ProposalCategory::Parameter);
    assert_eq!(def_param.quorum_bps, 1_000);
    assert_eq!(def_param.threshold_bps, 5_000);

    let def_treasury = f.vault.get_category_params(&ProposalCategory::Treasury);
    assert_eq!(def_treasury.quorum_bps, 2_000);
    assert_eq!(def_treasury.threshold_bps, 6_000);

    // Admin updates Upgrade category
    f.vault.set_category_params(
        &f.admin,
        &ProposalCategory::Upgrade,
        &4_000,
        &7_000,
        &6_000,
    );

    let updated = f.vault.get_category_params(&ProposalCategory::Upgrade);
    assert_eq!(updated.quorum_bps, 4_000);
    assert_eq!(updated.threshold_bps, 7_000);
    assert_eq!(updated.period_ledgers, 6_000);

    // Validation: invalid threshold (< 5000 bps) reverts
    let err = f.vault.try_set_category_params(
        &f.admin,
        &ProposalCategory::Upgrade,
        &1_000,
        &4_999, // too low
        &1_000,
    );
    assert!(err.is_err());
}

#[test]
fn test_categorized_proposal_snapshots_rules() {
    let f = Fixture::new();

    // Alice stakes to be able to propose
    f.vault.stake(&f.alice, &1_000_000);

    // Initial category params
    f.vault.set_category_params(
        &f.admin,
        &ProposalCategory::Parameter,
        &1_500,
        &5_500,
        &500,
    );

    // Create categorized proposal
    let prop_id = f.vault.create_categorized_proposal(
        &f.alice,
        &ProposalCategory::Parameter,
        &ProposableParam::UnstakeFee,
        &200,
    );

    let prop = f.vault.get_categorized_proposal(&prop_id).unwrap();
    assert_eq!(prop.snapshotted_quorum_bps, 1_500);
    assert_eq!(prop.snapshotted_threshold_bps, 5_500);
    assert_eq!(prop.snapshotted_period_ledgers, 500);

    // Admin changes category params later
    f.vault.set_category_params(
        &f.admin,
        &ProposalCategory::Parameter,
        &3_000,
        &8_000,
        &2_000,
    );

    // Proposal snapshot remains unchanged!
    let prop_after = f.vault.get_categorized_proposal(&prop_id).unwrap();
    assert_eq!(prop_after.snapshotted_quorum_bps, 1_500);
    assert_eq!(prop_after.snapshotted_threshold_bps, 5_500);
}

// ── Tests for Issue #762: Expedited Proposals ───────────────────────────────

#[test]
fn test_expedited_proposal_lifecycle_and_bond_refund() {
    let f = Fixture::new();

    f.vault.stake(&f.alice, &10_000_000_000);
    f.vault.stake(&f.bob, &10_000_000_000);

    let alice_bal_before = f.token.balance(&f.alice);

    // Create expedited proposal
    let prop_id = f.vault.create_expedited_proposal(
        &f.alice,
        &ProposableParam::PoolCap,
        &500_000_000_000,
        &MIN_EXPEDITED_BOND,
    );

    // Proposer bond transferred
    assert_eq!(f.token.balance(&f.alice), alice_bal_before - MIN_EXPEDITED_BOND);

    let prop = f.vault.get_categorized_proposal(&prop_id).unwrap();
    assert!(prop.is_expedited);
    assert_eq!(prop.snapshotted_quorum_bps, 4_000); // 40%
    assert_eq!(prop.snapshotted_threshold_bps, 7_500); // 75%

    // Vote on proposal
    f.vault.vote_categorized_proposal(&f.alice, &prop_id, &true);
    f.vault.vote_categorized_proposal(&f.bob, &prop_id, &true);

    // Fast-forward past voting period + delay
    set_ledger(&f.env, prop.ends_at + prop.execution_delay_ledgers + 10);

    // Enact proposal: passes higher bar, bond refunded
    f.vault.enact_expedited_proposal(&prop_id);

    assert_eq!(f.token.balance(&f.alice), alice_bal_before);
    let prop_final = f.vault.get_categorized_proposal(&prop_id).unwrap();
    assert!(prop_final.enacted);
}

#[test]
fn test_expedited_proposal_slashes_bond_on_quorum_failure() {
    let f = Fixture::new();

    // Large total stake
    f.vault.stake(&f.bob, &100_000_000_000);
    f.vault.stake(&f.alice, &1_000_000); // tiny stake

    let alice_bal_before = f.token.balance(&f.alice);

    let prop_id = f.vault.create_expedited_proposal(
        &f.alice,
        &ProposableParam::MinStake,
        &500,
        &MIN_EXPEDITED_BOND,
    );

    // Only alice votes with tiny stake, bob does not vote -> Quorum fails!
    f.vault.vote_categorized_proposal(&f.alice, &prop_id, &true);

    let prop = f.vault.get_categorized_proposal(&prop_id).unwrap();
    set_ledger(&f.env, prop.ends_at + prop.execution_delay_ledgers + 10);

    // Enacting fails quorum
    let err = f.vault.try_enact_expedited_proposal(&prop_id);
    assert!(err.is_err());

    // Alice's bond was NOT refunded (slashed!)
    assert_eq!(f.token.balance(&f.alice), alice_bal_before - MIN_EXPEDITED_BOND);
}

#[test]
fn test_expedited_proposal_guardian_veto() {
    let f = Fixture::new();

    f.vault.stake(&f.alice, &10_000_000_000);

    let prop_id = f.vault.create_expedited_proposal(
        &f.alice,
        &ProposableParam::UnstakeFee,
        &50,
        &MIN_EXPEDITED_BOND,
    );

    let guardian = Address::generate(&f.env);
    f.vault.veto_expedited_proposal(&guardian, &prop_id);

    let prop = f.vault.get_categorized_proposal(&prop_id).unwrap();
    assert!(prop.vetoed);

    set_ledger(&f.env, prop.ends_at + prop.execution_delay_ledgers + 10);
    let err = f.vault.try_enact_expedited_proposal(&prop_id);
    assert!(err.is_err());
}

// ── Tests for Issue #763: Expiry to Vote Delegation ─────────────────────────

#[test]
fn test_vote_delegation_expiry_lifecycle() {
    let f = Fixture::new();

    set_ledger(&f.env, 200);

    f.vault.stake(&f.alice, &1_000_000);
    f.vault.stake(&f.bob, &2_000_000);

    // Alice delegates to Bob with expiry at ledger 300
    f.vault.delegate_vote_weight_with_expiry(&f.alice, &f.bob, &Some(300));

    // Active at ledger 250
    let active_del = f.vault.get_delegate(&f.alice);
    assert_eq!(active_del, Some((f.bob.clone(), Some(300))));

    let weight_bob_250 = f.vault.effective_vote_weight_at_ledger(&f.bob, &250);
    assert_eq!(weight_bob_250, 3_000_000); // 2M own + 1M alice

    let weight_alice_250 = f.vault.effective_vote_weight_at_ledger(&f.alice, &250);
    assert_eq!(weight_alice_250, 0); // delegated away

    // Fast-forward past expiry (ledger 350)
    set_ledger(&f.env, 350);

    // Delegation automatically expired
    assert_eq!(f.vault.get_delegate(&f.alice), None);

    // Weight returns automatically to Alice!
    let weight_bob_350 = f.vault.effective_vote_weight_at_ledger(&f.bob, &350);
    assert_eq!(weight_bob_350, 2_000_000); // only bob's own

    let weight_alice_350 = f.vault.effective_vote_weight_at_ledger(&f.alice, &350);
    assert_eq!(weight_alice_350, 1_000_000); // returned to alice!

    // Renewal works: extend expiry to 500
    f.vault.delegate_vote_weight_with_expiry(&f.alice, &f.bob, &Some(500));
    assert_eq!(f.vault.get_delegate(&f.alice), Some((f.bob.clone(), Some(500))));
}

#[test]
fn test_vote_delegation_permanent_without_expiry() {
    let f = Fixture::new();

    f.vault.stake(&f.alice, &5_000_000);
    f.vault.stake(&f.bob, &5_000_000);

    // None expiry means permanent delegation
    f.vault.delegate_vote_weight_with_expiry(&f.alice, &f.bob, &None);

    set_ledger(&f.env, 10_000);
    assert_eq!(f.vault.get_delegate(&f.alice), Some((f.bob.clone(), None)));

    let bob_weight = f.vault.effective_vote_weight_at_ledger(&f.bob, &10_000);
    assert_eq!(bob_weight, 10_000_000);
}

// ── Tests for Issue #764: StrategyAdapter Interface ─────────────────────────

#[test]
fn test_strategy_adapter_lifecycle_and_share_price() {
    let f = Fixture::new();

    // Register mock strategy adapter
    let adapter_id = f.env.register_contract(None, MockStrategyAdapter);

    // Admin adds strategy with 30% cap (3000 bps)
    f.vault.add_strategy(&f.admin, &adapter_id, &3_000);

    let strategies = f.vault.get_strategies();
    assert_eq!(strategies.len(), 1);
    assert_eq!(strategies.get(0).unwrap().cap_bps, 3_000);

    // Stake funds in vault
    f.vault.stake(&f.alice, &10_000_000);

    // Allocate funds to strategy (up to 30% = 3,000,000)
    let keeper = Address::generate(&f.env);
    f.vault.allocate_to_strategy(&keeper, &adapter_id, &2_000_000);

    // Strategy total assets reflects deposit
    let strat_assets = f.vault.total_strategy_assets();
    assert_eq!(strat_assets, 2_000_000);

    // Exceeding strategy cap reverts
    let err = f.vault.try_allocate_to_strategy(&keeper, &adapter_id, &2_000_000);
    assert!(err.is_err());

    // Share price includes strategy assets
    let (total_assets, total_shares) = f.vault.get_vault_share_price();
    assert_eq!(total_assets, 10_000_000); // 8M idle + 2M strategy
    assert_eq!(total_shares, 10_000_000);

    // Max 3 strategies enforcement
    let s2 = Address::generate(&f.env);
    let s3 = Address::generate(&f.env);
    let s4 = Address::generate(&f.env);

    f.vault.add_strategy(&f.admin, &s2, &2_000);
    f.vault.add_strategy(&f.admin, &s3, &2_000);

    // 4th strategy rejected
    let err_max = f.vault.try_add_strategy(&f.admin, &s4, &1_000);
    assert!(err_max.is_err());
}
