#![cfg(test)]
//! Tests for the position value-at-risk estimate (issue #408).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

use crate::{
    balance,
    storage::DataKey,
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

    fn seed_position(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_shares(&self.env, user, amount);
            balance::set_total_shares(&self.env, amount);
            balance::set_total_deposited(&self.env, amount);
            self.env
                .storage()
                .persistent()
                .set(&DataKey::StakedAtLedger(user.clone()), &self.env.ledger().sequence());
        });
    }

    fn set_lock_config(&self, lock_period: u32, penalty_bps: u32) {
        self.env.as_contract(&self.vault_id, || {
            self.env
                .storage()
                .instance()
                .set(&DataKey::LockPeriod, &lock_period);
            self.env
                .storage()
                .instance()
                .set(&DataKey::EarlyExitPenaltyBps, &penalty_bps);
        });
    }
}

#[test]
fn no_position_returns_zero_report() {
    let f = Fixture::new();
    let report = f.vault.get_position_var(&f.alice, &1_000);
    assert_eq!(report.position_amount, 0);
    assert_eq!(report.early_exit_loss, 0);
    assert_eq!(report.max_slash_exposure, 0);
    assert_eq!(report.lock_opportunity_cost, 0);
    assert_eq!(report.reward_price_drop_impact, 0);
    assert_eq!(report.total_var_bps, 0);
}

#[test]
fn no_lock_returns_zero_early_exit_loss() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 100_000);
    // No lock config set — lock_period defaults to 0.

    let report = f.vault.get_position_var(&f.alice, &0);
    assert_eq!(report.early_exit_loss, 0);
    assert_eq!(report.lock_opportunity_cost, 0);
    assert_eq!(report.max_slash_exposure, 100_000);
    assert_eq!(report.position_amount, 100_000);
}

#[test]
fn full_penalty_position_calculated_correctly() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 100_000);
    f.set_lock_config(1_000, 500); // 5% early exit penalty, locked for 1000 ledgers

    let report = f.vault.get_position_var(&f.alice, &0);
    assert_eq!(report.early_exit_loss, 5_000); // 5% of 100_000
}

#[test]
fn price_drop_impact_scales_linearly() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 100_000);
    f.env.as_contract(&f.vault_id, || {
        balance::set_accrued_reward(&f.env, &f.alice, 10_000);
    });

    let report_10pct = f.vault.get_position_var(&f.alice, &1_000);
    assert_eq!(report_10pct.reward_price_drop_impact, 1_000);

    let report_20pct = f.vault.get_position_var(&f.alice, &2_000);
    assert_eq!(report_20pct.reward_price_drop_impact, 2_000);
}
