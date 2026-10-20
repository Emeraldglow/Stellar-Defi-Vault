#![cfg(test)]
//! Tests for balance checkpoints (#709), TWAB (#710), stake hooks (#711) and
//! governance quorum/threshold params (#712).

extern crate std;

use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, Events as _, Ledger as _},
    token, Address, Env, Symbol, TryFromVal, Vec,
};

use crate::{
    balance,
    balance_checkpoints::{Checkpoint, MAX_CHECKPOINTS},
    errors::VaultError,
    storage::ProposableParam,
    vault::{VaultContract, VaultContractClient},
};

#[contract]
pub struct RecordingHook;

#[contractimpl]
impl RecordingHook {
    pub fn on_action(env: Env, user: Address, action: Symbol, amount: i128) {
        let calls: u32 = env
            .storage()
            .instance()
            .get(&symbol_short!("calls"))
            .unwrap_or(0);
        env.storage().instance().set(&symbol_short!("calls"), &(calls + 1));
        env.storage()
            .instance()
            .set(&symbol_short!("last"), &(user, action, amount));
    }

    pub fn calls(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("calls"))
            .unwrap_or(0)
    }

    pub fn last(env: Env) -> (Address, Symbol, i128) {
        env.storage().instance().get(&symbol_short!("last")).unwrap()
    }
}

#[contract]
pub struct FailingHook;

#[contractimpl]
impl FailingHook {
    pub fn on_action(_env: Env, _user: Address, _action: Symbol, _amount: i128) {
        panic!("hook failure");
    }
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    admin: Address,
    token_admin: token::StellarAssetClient<'a>,
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
        let token_address = env.register_stellar_asset_contract(admin.clone());
        let token_admin = token::StellarAssetClient::new(&env, &token_address);
        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_address, &0, &None, &None);
        Fixture {
            env,
            vault,
            vault_id,
            admin,
            token_admin,
        }
    }

    fn set_ledger(&self, sequence: u32) {
        self.env
            .ledger()
            .with_mut(|li| li.sequence_number = sequence);
    }

    /// Deposit `amount` for a fresh or existing user at `ledger`; returns the
    /// shares minted.
    fn deposit_at(&self, user: &Address, amount: i128, ledger: u32) -> i128 {
        self.set_ledger(ledger);
        self.token_admin.mint(user, &amount);
        self.vault.deposit(user, &amount, &None)
    }

    fn hook_calls(&self, hook: &Address) -> u32 {
        RecordingHookClient::new(&self.env, hook).calls()
    }
}

// ── Issue #709: historical balance checkpoints ───────────────────────────────

#[test]
fn balance_at_before_between_and_after_checkpoints() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);

    let first = f.deposit_at(&alice, 1_000, 100);
    let second = f.deposit_at(&alice, 500, 200);
    f.set_ledger(300);
    f.vault.withdraw(&alice, &first);
    let after_withdraw = second;
    f.set_ledger(400);

    assert_eq!(f.vault.balance_at(&alice, &99), 0);
    assert_eq!(f.vault.balance_at(&alice, &100), first);
    assert_eq!(f.vault.balance_at(&alice, &199), first);
    assert_eq!(f.vault.balance_at(&alice, &200), first + second);
    assert_eq!(f.vault.balance_at(&alice, &299), first + second);
    assert_eq!(f.vault.balance_at(&alice, &300), after_withdraw);
    assert_eq!(f.vault.balance_at(&alice, &400), after_withdraw);
}

#[test]
fn total_supply_at_tracks_pool_wide_shares() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);

    let a = f.deposit_at(&alice, 1_000, 100);
    let b = f.deposit_at(&bob, 400, 150);
    f.set_ledger(200);

    assert_eq!(f.vault.total_supply_at(&99), 0);
    assert_eq!(f.vault.total_supply_at(&100), a);
    assert_eq!(f.vault.total_supply_at(&149), a);
    assert_eq!(f.vault.total_supply_at(&150), a + b);
    assert_eq!(f.vault.total_supply_at(&200), a + b);
}

#[test]
fn future_ledger_lookup_is_rejected() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);
    f.deposit_at(&alice, 1_000, 100);

    assert_eq!(
        f.vault.try_balance_at(&alice, &101),
        Err(Ok(VaultError::EpochNotFinalized))
    );
    assert_eq!(
        f.vault.try_total_supply_at(&101),
        Err(Ok(VaultError::EpochNotFinalized))
    );
}

#[test]
fn same_ledger_changes_collapse_into_one_checkpoint() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);

    f.deposit_at(&alice, 1_000, 100);
    f.deposit_at(&alice, 500, 100);
    let total = balance_of(&f, &alice);

    assert_eq!(f.vault.balance_at(&alice, &100), total);
    let history: Vec<Checkpoint> = f.env.as_contract(&f.vault_id, || {
        f.env
            .storage()
            .persistent()
            .get(&(symbol_short!("ck_usr"), alice.clone()))
            .unwrap()
    });
    assert_eq!(history.len(), 1);
}

#[test]
fn history_is_bounded_and_recent_answers_stay_exact() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);

    let count = MAX_CHECKPOINTS + 5;
    let mut running = 0;
    let mut balances = std::vec::Vec::new();
    for i in 1..=count {
        running += f.deposit_at(&alice, 10, i);
        balances.push(running);
    }
    f.set_ledger(count);

    let history: Vec<Checkpoint> = f.env.as_contract(&f.vault_id, || {
        f.env
            .storage()
            .persistent()
            .get(&(symbol_short!("ck_usr"), alice.clone()))
            .unwrap()
    });
    assert_eq!(history.len(), MAX_CHECKPOINTS);
    assert_eq!(f.vault.balance_at(&alice, &count), running);
    // Ledger 60 is inside the retained range (6..=105).
    assert_eq!(f.vault.balance_at(&alice, &60), balances[59]);
}

fn balance_of(f: &Fixture, user: &Address) -> i128 {
    f.env
        .as_contract(&f.vault_id, || balance::get_shares(&f.env, user))
}

// ── Issue #710: time-weighted average balance ────────────────────────────────

#[test]
fn twab_of_constant_balance_equals_balance() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);
    let shares = f.deposit_at(&alice, 1_000, 100);
    f.set_ledger(500);

    assert_eq!(f.vault.get_twab(&alice, &100, &500), shares);
    assert_eq!(f.vault.get_twab(&alice, &200, &300), shares);
}

#[test]
fn twab_reflects_a_mid_window_balance_change() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);
    let first = f.deposit_at(&alice, 1_000, 100);
    let second = f.deposit_at(&alice, 1_000, 150);
    f.set_ledger(200);

    let expected = (first * 50 + (first + second) * 50) / 100;
    assert_eq!(f.vault.get_twab(&alice, &100, &200), expected);
}

#[test]
fn flash_deposit_barely_moves_twab() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    let a = f.deposit_at(&alice, 1_000, 100);
    let b = f.deposit_at(&bob, 1_000_000, 199);
    f.set_ledger(200);

    // Bob held his shares for one ledger of a 100-ledger window.
    assert_eq!(f.vault.get_twab(&bob, &100, &200), b / 100);
    assert_eq!(f.vault.get_twab(&alice, &100, &200), a);
    assert_eq!(f.vault.get_total_twab(&100, &200), (a * 100 + b) / 100);
}

#[test]
fn twab_rejects_empty_and_future_windows() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);
    f.deposit_at(&alice, 1_000, 100);

    assert_eq!(
        f.vault.try_get_twab(&alice, &100, &100),
        Err(Ok(VaultError::ZeroAmount))
    );
    assert_eq!(
        f.vault.try_get_twab(&alice, &100, &101),
        Err(Ok(VaultError::EpochNotFinalized))
    );
    assert_eq!(
        f.vault.try_get_total_twab(&100, &101),
        Err(Ok(VaultError::EpochNotFinalized))
    );
}

#[test]
fn twab_rejects_windows_starting_before_pruned_history() {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);

    let count = MAX_CHECKPOINTS + 5;
    for i in 1..=count {
        f.deposit_at(&alice, 10, i);
    }
    f.set_ledger(count);

    // Ledger 1 predates the oldest retained checkpoint (6).
    assert_eq!(
        f.vault.try_get_twab(&alice, &1, &count),
        Err(Ok(VaultError::EpochNotFinalized))
    );
    assert!(f.vault.try_get_twab(&alice, &60, &count).is_ok());
}

// ── Issue #711: stake and unstake hooks ──────────────────────────────────────

#[test]
fn hook_is_called_on_stake_and_unstake() {
    let f = Fixture::new();
    let hook = f.env.register_contract(None, RecordingHook);
    f.vault.register_hook(&f.admin, &hook);
    let alice = Address::generate(&f.env);

    let shares = f.deposit_at(&alice, 1_000, 10);
    let client = RecordingHookClient::new(&f.env, &hook);
    assert_eq!(client.calls(), 1);
    assert_eq!(client.last(), (alice.clone(), symbol_short!("stake"), 1_000));

    let returned = f.vault.withdraw(&alice, &shares);
    assert_eq!(client.calls(), 2);
    assert_eq!(client.last(), (alice, symbol_short!("unstake"), returned));
}

#[test]
fn failing_hook_does_not_block_the_action_and_emits_hook_failed() {
    let f = Fixture::new();
    let failing = f.env.register_contract(None, FailingHook);
    let recording = f.env.register_contract(None, RecordingHook);
    f.vault.register_hook(&f.admin, &failing);
    f.vault.register_hook(&f.admin, &recording);
    let alice = Address::generate(&f.env);

    let shares = f.deposit_at(&alice, 1_000, 10);

    assert_eq!(balance_of(&f, &alice), shares);
    // The hook after the failing one is still notified.
    assert_eq!(f.hook_calls(&recording), 1);
    let failed = f.env.events().all().iter().any(|(_, topics, _)| {
        topics
            .get(0)
            .and_then(|t| Symbol::try_from_val(&f.env, &t).ok())
            == Some(symbol_short!("hook_fail"))
    });
    assert!(failed);
}

#[test]
fn removed_hook_is_no_longer_called() {
    let f = Fixture::new();
    let hook = f.env.register_contract(None, RecordingHook);
    f.vault.register_hook(&f.admin, &hook);
    let alice = Address::generate(&f.env);

    f.deposit_at(&alice, 1_000, 10);
    assert_eq!(f.hook_calls(&hook), 1);

    f.vault.remove_hook(&f.admin, &hook);
    assert_eq!(f.vault.get_hooks().len(), 0);
    f.deposit_at(&alice, 1_000, 11);
    assert_eq!(f.hook_calls(&hook), 1);

    assert_eq!(
        f.vault.try_remove_hook(&f.admin, &hook),
        Err(Ok(VaultError::PositionNotFound))
    );
}

#[test]
fn hook_registration_is_capped_and_deduplicated() {
    let f = Fixture::new();
    let mut hooks = std::vec::Vec::new();
    for _ in 0..3 {
        let hook = f.env.register_contract(None, RecordingHook);
        f.vault.register_hook(&f.admin, &hook);
        hooks.push(hook);
    }

    let extra = f.env.register_contract(None, RecordingHook);
    assert_eq!(
        f.vault.try_register_hook(&f.admin, &extra),
        Err(Ok(VaultError::MaxPositionsReached))
    );
    assert_eq!(
        f.vault.try_register_hook(&f.admin, &hooks[0]),
        Err(Ok(VaultError::AlreadyInitialized))
    );
    assert_eq!(
        f.vault.try_register_hook(&f.admin, &f.vault_id),
        Err(Ok(VaultError::InvalidAddress))
    );
    assert_eq!(f.vault.get_hooks().len(), 3);
}

// ── Issue #712: governance quorum and pass threshold ─────────────────────────

/// Alice stakes 600 and bob 400, so alice holds 60% of the vote weight.
fn governance_fixture<'a>() -> (Fixture<'a>, Address, Address) {
    let f = Fixture::new();
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    f.deposit_at(&alice, 600, 1);
    f.deposit_at(&bob, 400, 1);
    (f, alice, bob)
}

fn finalize(f: &Fixture, proposal_id: u32) {
    f.set_ledger(1_000);
    f.vault.enact_proposal(&proposal_id);
}

#[test]
fn governance_params_default_to_unset_and_validate_input() {
    let f = Fixture::new();
    let unset = f.vault.get_governance_params();
    assert_eq!((unset.quorum_bps, unset.pass_threshold_bps), (0, 0));

    f.vault.set_governance_params(&f.admin, &4_000, &6_000);
    let params = f.vault.get_governance_params();
    assert_eq!((params.quorum_bps, params.pass_threshold_bps), (4_000, 6_000));

    assert_eq!(
        f.vault.try_set_governance_params(&f.admin, &10_001, &5_000),
        Err(Ok(VaultError::InvalidRate))
    );
    assert_eq!(
        f.vault.try_set_governance_params(&f.admin, &4_000, &0),
        Err(Ok(VaultError::InvalidRate))
    );
    assert_eq!(
        f.vault.try_set_governance_params(&f.admin, &4_000, &10_001),
        Err(Ok(VaultError::InvalidRate))
    );
    let stranger = Address::generate(&f.env);
    assert_eq!(
        f.vault.try_set_governance_params(&stranger, &4_000, &6_000),
        Err(Ok(VaultError::Unauthorized))
    );
}

#[test]
fn proposal_below_quorum_fails() {
    let (f, alice, bob) = governance_fixture();
    f.vault.set_governance_params(&f.admin, &7_000, &5_000);
    let id = f
        .vault
        .create_proposal(&alice, &ProposableParam::RewardRate, &500, &100);

    // Only bob votes: 40% participation against a 70% quorum, all in favor.
    f.vault.vote(&bob, &id, &true);
    finalize(&f, id);

    assert_eq!(f.vault.get_reward_rate_bps(), 0);
}

#[test]
fn proposal_meeting_quorum_and_threshold_passes() {
    let (f, alice, bob) = governance_fixture();
    f.vault.set_governance_params(&f.admin, &7_000, &5_000);
    let id = f
        .vault
        .create_proposal(&alice, &ProposableParam::RewardRate, &500, &100);

    f.vault.vote(&alice, &id, &true);
    f.vault.vote(&bob, &id, &false);
    finalize(&f, id);

    assert_eq!(f.vault.get_reward_rate_bps(), 500);
}

#[test]
fn exact_threshold_passes_and_one_basis_point_above_fails() {
    let (f, alice, bob) = governance_fixture();

    // Support is exactly 60%.
    f.vault.set_governance_params(&f.admin, &0, &6_000);
    let passing = f
        .vault
        .create_proposal(&alice, &ProposableParam::RewardRate, &500, &100);
    f.vault.vote(&alice, &passing, &true);
    f.vault.vote(&bob, &passing, &false);

    f.vault.set_governance_params(&f.admin, &0, &6_001);
    let failing = f
        .vault
        .create_proposal(&alice, &ProposableParam::RewardRate, &700, &100);
    f.vault.vote(&alice, &failing, &true);
    f.vault.vote(&bob, &failing, &false);

    finalize(&f, passing);
    assert_eq!(f.vault.get_reward_rate_bps(), 500);
    f.vault.enact_proposal(&failing);
    assert_eq!(f.vault.get_reward_rate_bps(), 500);
}

#[test]
fn snapshotted_params_are_honored_after_a_mid_vote_change() {
    let (f, alice, bob) = governance_fixture();
    f.vault.set_governance_params(&f.admin, &0, &5_000);
    let id = f
        .vault
        .create_proposal(&alice, &ProposableParam::RewardRate, &500, &100);

    // Tightening the rules mid-vote must not affect the open proposal.
    f.vault.set_governance_params(&f.admin, &10_000, &10_000);
    f.vault.vote(&alice, &id, &true);
    f.vault.vote(&bob, &id, &false);
    finalize(&f, id);

    assert_eq!(f.vault.get_reward_rate_bps(), 500);
}

#[test]
fn unconfigured_pool_keeps_the_simple_majority_rule() {
    let (f, alice, bob) = governance_fixture();
    let id = f
        .vault
        .create_proposal(&alice, &ProposableParam::RewardRate, &500, &100);

    f.vault.vote(&alice, &id, &true);
    f.vault.vote(&bob, &id, &false);
    finalize(&f, id);

    assert_eq!(f.vault.get_reward_rate_bps(), 500);
}
