#![cfg(test)]
//! Tests for the time-weighted average reward rate (issue #400).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

use crate::{
    balance,
    vault::{VaultContract, VaultContractClient},
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

    fn seed_position(
        &self,
        user: &Address,
        shares: i128,
        total_shares: i128,
        total_deposited: i128,
    ) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_shares(&self.env, user, shares);
            balance::set_total_shares(&self.env, total_shares);
            balance::set_total_deposited(&self.env, total_deposited);
        });
    }

    fn set_staked_at(&self, user: &Address, ledger: u32) {
        self.env.as_contract(&self.vault_id, || {
            self.env.storage().persistent().set(
                &crate::storage::DataKey::StakedAtLedger(user.clone()),
                &ledger,
            );
        });
    }
}

#[test]
fn constant_rate_matches_spot() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    f.set_staked_at(&f.alice, 1_000);

    // No checkpoints recorded => TWA falls back to the current spot rate.
    set_ledger(&f.env, 1_000 + 100_000);
    let twa = f.vault.get_pending_reward_twa(&f.alice);

    let spot_rate = 500i128;
    let elapsed = 100_000i128;
    let expected = 1_000i128 * spot_rate * elapsed / (10_000 * 6_307_200);
    assert_eq!(twa, expected);
}

#[test]
fn two_rate_period_returns_weighted_average() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    f.set_staked_at(&f.alice, 0);

    set_ledger(&f.env, 0);
    f.vault.record_rate_checkpoint(&500);
    set_ledger(&f.env, 100);
    f.vault.record_rate_checkpoint(&1_000);
    set_ledger(&f.env, 200);

    // Rate 500 bps for ledgers [0,100), rate 1000 bps for [100,200) =>
    // weighted average (500*100 + 1000*100) / 200 = 750 bps.
    let twa_rate_expected = 750i128;
    let elapsed = 200i128;
    let expected = 1_000i128 * twa_rate_expected * elapsed / (10_000 * 6_307_200);
    assert_eq!(f.vault.get_pending_reward_twa(&f.alice), expected);
}

#[test]
fn checkpoint_history_caps_at_fifty() {
    let f = Fixture::new();
    for i in 0..60u32 {
        set_ledger(&f.env, i);
        f.vault.record_rate_checkpoint(&(100 + i as i128));
    }

    let checkpoints = f.vault.get_rate_checkpoints();
    assert_eq!(checkpoints.len(), 50);
    // Oldest 10 were dropped; the earliest remaining checkpoint is from
    // ledger 10, rate 110.
    assert_eq!(checkpoints.get(0).unwrap().valid_from, 10);
}

#[test]
fn rate_accuracy_delta_matches_manual_calc() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    f.set_staked_at(&f.alice, 1_000);

    set_ledger(&f.env, 1_000 + 50_000);
    let twa = f.vault.get_pending_reward_twa(&f.alice);
    let spot = 0i128; // no AccruedReward seeded
    let delta = f.vault.get_rate_accuracy_delta(&f.alice);
    assert_eq!(delta, spot - twa);
}
