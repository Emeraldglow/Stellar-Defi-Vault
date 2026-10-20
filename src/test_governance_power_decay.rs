#![cfg(test)]
//! Tests for governance vote weight decay (issue #404).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

use crate::{
    balance,
    storage::DataKey,
    crate::{VaultContract, VaultContractClient},
};

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
        let token_addr = env.register_stellar_asset_contract(admin.clone());

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            alice,
        }
    }

    /// Seed a staker's position directly (no `stake()` entrypoint currently
    /// works on `main` â€” see `epoch_reward_cap.rs` and sibling test files'
    /// notes on the same pre-existing gap).
    fn seed_position(&self, user: &Address, shares: i128, total_shares: i128, total_deposited: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_shares(&self.env, user, shares);
            balance::set_total_shares(&self.env, total_shares);
            balance::set_total_deposited(&self.env, total_deposited);
        });
    }

    fn set_staked_at(&self, user: &Address, ledger: u32) {
        self.env.as_contract(&self.vault_id, || {
            self.env
                .storage()
                .persistent()
                .set(&DataKey::StakedAtLedger(user.clone()), &ledger);
        });
    }
}

#[test]
fn active_voter_gets_full_weight() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    f.set_staked_at(&f.alice, 1_000);
    f.vault.set_governance_decay_config(&10, &1_000);

    f.vault.record_governance_vote(&f.alice);
    assert_eq!(f.vault.get_effective_vote_weight(&f.alice), 1_000);
}

#[test]
fn inactive_voter_gets_decayed_weight() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    f.set_staked_at(&f.alice, 1_000);
    // 10 inactivity epochs grace, then 500 bps decay per epoch beyond that.
    f.vault.set_governance_decay_config(&10, &500);
    f.vault.record_governance_vote(&f.alice);

    // 15 epochs (days) elapsed => 5 epochs beyond the grace window.
    set_ledger(&f.env, 1_000 + 15 * 17_280);
    let effective = f.vault.get_effective_vote_weight(&f.alice);
    // 5 epochs * 500 bps = 2500 bps decay => 75% of raw weight.
    assert_eq!(effective, 750);
}

#[test]
fn decay_floors_at_twenty_percent() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    f.set_staked_at(&f.alice, 1_000);
    f.vault.set_governance_decay_config(&0, &5_000);
    f.vault.record_governance_vote(&f.alice);

    // Far beyond enough epochs to fully saturate decay.
    set_ledger(&f.env, 1_000 + 1_000 * 17_280);
    let effective = f.vault.get_effective_vote_weight(&f.alice);
    assert_eq!(effective, 200); // 20% floor of 1_000
}

#[test]
fn voting_resets_decay_clock() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    f.set_staked_at(&f.alice, 1_000);
    f.vault.set_governance_decay_config(&10, &500);
    f.vault.record_governance_vote(&f.alice);

    set_ledger(&f.env, 1_000 + 20 * 17_280);
    assert!(f.vault.get_effective_vote_weight(&f.alice) < 1_000);

    // Voting again resets the clock â€” weight is back to full immediately.
    f.vault.record_governance_vote(&f.alice);
    assert_eq!(f.vault.get_effective_vote_weight(&f.alice), 1_000);
}

#[test]
fn no_config_means_no_decay() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    f.set_staked_at(&f.alice, 1_000);

    set_ledger(&f.env, 1_000 + 100 * 17_280);
    assert_eq!(f.vault.get_effective_vote_weight(&f.alice), 1_000);
}

