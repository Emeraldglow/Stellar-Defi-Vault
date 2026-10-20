#![cfg(test)]
//! Tests for the capacity utilization forecast (issue #402).

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

        set_ledger(&env, 1_000_000);

        Fixture {
            env,
            vault,
            vault_id,
            alice,
        }
    }

    fn seed_tvl(&self, total_deposited: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_total_deposited(&self.env, total_deposited);
        });
    }

    fn seed_pool_cap(&self, cap: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_pool_cap(&self.env, cap);
        });
    }
}

#[test]
fn correct_days_calculation_for_known_inflow() {
    let f = Fixture::new();
    f.seed_tvl(30_000);
    f.seed_pool_cap(100_000);

    // 7 stakes of 1_000 each, all within the 7-day (120_960-ledger) window
    // as of the check below => 7_000 total, daily rate 1_000.
    for _ in 0..7u32 {
        f.vault.record_stake_inflow(&f.alice, &1_000);
    }

    assert_eq!(f.vault.get_7day_stake_inflow(), 7_000);
    assert_eq!(f.vault.get_daily_stake_rate(), 1_000);

    let forecast = f.vault.get_capacity_forecast();
    assert_eq!(forecast.current_tvl, 30_000);
    assert_eq!(forecast.pool_cap, 100_000);
    assert_eq!(forecast.remaining_capacity, 70_000);
    assert_eq!(forecast.days_until_full, Some(70));
}

#[test]
fn none_when_no_cap_set() {
    let f = Fixture::new();
    f.seed_tvl(30_000);
    f.vault.record_stake_inflow(&f.alice, &1_000);

    let forecast = f.vault.get_capacity_forecast();
    assert_eq!(forecast.pool_cap, 0);
    assert_eq!(forecast.days_until_full, None);
}

#[test]
fn warning_fires_under_seven_days() {
    let f = Fixture::new();
    f.seed_tvl(90_000);
    f.seed_pool_cap(100_000);
    // 7-day inflow 35_000 => daily rate 5_000 => remaining 10_000 / 5_000 =
    // 2 days, under the 7-day warning threshold.
    f.vault.record_stake_inflow(&f.alice, &35_000);

    let events_before = f.env.events().all().len();
    let forecast = f.vault.get_capacity_forecast();
    assert_eq!(forecast.days_until_full, Some(2));
    let events_after = f.env.events().all().len();
    assert!(events_after > events_before);
}

#[test]
fn zero_inflow_returns_none() {
    let f = Fixture::new();
    f.seed_tvl(30_000);
    f.seed_pool_cap(100_000);
    // No stakes recorded at all.
    assert_eq!(f.vault.get_7day_stake_inflow(), 0);

    let forecast = f.vault.get_capacity_forecast();
    assert_eq!(forecast.days_until_full, None);
}
