#![cfg(test)]

extern crate std;

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Events, Ledger as _},
    token, Address, Env, String, Symbol, TryFromVal, Vec,
};

use crate::{
    balance,
    peg_stabilization::PegConfig,
    vault::{VaultContract, VaultContractClient},
};

#[contract]
struct MockPegOracle;

#[contractimpl]
impl MockPegOracle {
    pub fn set_price(env: Env, asset_id: String, price: i128) {
        env.storage()
            .instance()
            .set(&(Symbol::new(&env, "price"), asset_id), &price);
    }

    pub fn get_price(env: Env, asset_id: String) -> i128 {
        env.storage()
            .instance()
            .get(&(Symbol::new(&env, "price"), asset_id))
            .unwrap_or(0)
    }
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    oracle: MockPegOracleClient<'a>,
    admin: Address,
    alice: Address,
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
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
        token_admin.mint(&vault_id, &100_000);

        let oracle_id = env.register_contract(None, MockPegOracle);
        let oracle = MockPegOracleClient::new(&env, &oracle_id);

        Fixture {
            env,
            vault,
            oracle,
            admin,
            alice,
        }
    }

    fn configure(&self, lower_bps: u32, upper_bps: u32, max_buyback: i128) {
        let config = PegConfig {
            target_price: 100,
            lower_band_bps: lower_bps,
            upper_band_bps: upper_bps,
            oracle: self.oracle.address.clone(),
            asset_id: String::from_str(&self.env, "RWD"),
        };
        self.vault.set_peg_config(&self.admin, &config);
        self.vault
            .set_max_buyback_per_check(&self.admin, &max_buyback);
    }

    fn set_price(&self, price: i128) {
        self.oracle
            .set_price(&String::from_str(&self.env, "RWD"), &price);
    }
}

fn has_event(env: &Env, name: &str) -> bool {
    env.events()
        .all()
        .iter()
        .any(|(_, topics, _)| match topics.get(0) {
            Some(val) => Symbol::try_from_val(env, &val)
                .map(|topic| topic == Symbol::new(env, name))
                .unwrap_or(false),
            None => false,
        })
}

fn event_data(env: &Env, name: &str) -> Vec<soroban_sdk::Val> {
    let events = env.events().all();
    for (_, topics, data) in events.iter() {
        let matches = match topics.get(0) {
            Some(val) => Symbol::try_from_val(env, &val)
                .map(|topic| topic == Symbol::new(env, name))
                .unwrap_or(false),
            None => false,
        };
        if matches {
            return Vec::<soroban_sdk::Val>::try_from_val(env, &data).unwrap();
        }
    }
    Vec::new(env)
}

#[test]
fn below_band_triggers_buyback() {
    let f = Fixture::new();
    f.configure(500, 500, 2_000);
    balance::set_reward_pool_balance(&f.env, 5_000);
    f.set_price(94);

    f.vault.check_peg();

    assert_eq!(balance::get_reward_pool_balance(&f.env), 3_000);
    assert!(has_event(&f.env, "peg_buyback_triggered"));
}

#[test]
fn above_band_halts_emissions() {
    let f = Fixture::new();
    f.configure(500, 500, 2_000);
    f.set_price(106);

    f.vault.check_peg();

    assert_eq!(f.vault.emissions_halted_by_peg(), true);
    assert!(has_event(&f.env, "emissions_halted_by_peg"));
}

#[test]
fn halted_emissions_pause_reward_distribution() {
    let f = Fixture::new();
    f.configure(500, 500, 2_000);
    f.set_price(106);
    f.vault.check_peg();
    balance::set_accrued_reward(&f.env, &f.alice, 1_000);

    let claimed = f.vault.claim(&f.alice);

    assert_eq!(claimed, 0);
    assert_eq!(balance::get_accrued_reward(&f.env, &f.alice), 1_000);
}

#[test]
fn within_band_restores_emissions() {
    let f = Fixture::new();
    f.configure(500, 500, 2_000);
    f.set_price(106);
    f.vault.check_peg();

    f.set_price(100);
    f.vault.check_peg();

    assert_eq!(f.vault.emissions_halted_by_peg(), false);
    assert!(has_event(&f.env, "emissions_restored_by_peg"));
}

#[test]
fn max_buyback_limit_respected() {
    let f = Fixture::new();
    f.configure(500, 500, 700);
    balance::set_reward_pool_balance(&f.env, 5_000);
    f.set_price(90);

    f.vault.check_peg();

    assert_eq!(balance::get_reward_pool_balance(&f.env), 4_300);
    let data = event_data(&f.env, "peg_buyback_triggered");
    let amount_spent = i128::try_from_val(&f.env, &data.get(2).unwrap()).unwrap();
    assert_eq!(amount_spent, 700);
}
