#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger as _},
    token, Address, Env, Symbol, TryFromVal, Vec,
};

use crate::{
    balance,
    errors::VaultError,
    vault::{VaultContract, VaultContractClient},
};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
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
        let token = token::Client::new(&env, &token_addr);
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        Fixture {
            env,
            vault,
            token,
            token_admin,
            admin,
            alice,
        }
    }

    fn accrue_reward(&self, amount: i128) {
        self.token_admin.mint(&self.vault.address, &amount);
        balance::set_accrued_reward(&self.env, &self.alice, amount);
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
fn claim_fee_is_deducted_and_user_receives_remainder() {
    let f = Fixture::new();
    f.vault.set_claim_fee_bps(&f.admin, &500);
    f.accrue_reward(1_000);

    let before = f.token.balance(&f.alice);
    let claimed = f.vault.claim(&f.alice);

    assert_eq!(claimed, 950);
    assert_eq!(f.token.balance(&f.alice), before + 950);
    assert_eq!(f.vault.get_claim_fee_reserve(), 50);

    let data = event_data(&f.env, "claim_fee_collected");
    assert_eq!(
        i128::try_from_val(&f.env, &data.get(1).unwrap()).unwrap(),
        1_000
    );
    assert_eq!(
        i128::try_from_val(&f.env, &data.get(2).unwrap()).unwrap(),
        50
    );
    assert_eq!(
        i128::try_from_val(&f.env, &data.get(3).unwrap()).unwrap(),
        950
    );
}

#[test]
fn reserve_accumulates_across_claims() {
    let f = Fixture::new();
    f.vault.set_claim_fee_bps(&f.admin, &250);

    f.accrue_reward(2_000);
    assert_eq!(f.vault.claim(&f.alice), 1_950);
    f.accrue_reward(4_000);
    assert_eq!(f.vault.claim(&f.alice), 3_900);

    assert_eq!(f.vault.get_claim_fee_reserve(), 150);
}

#[test]
fn fee_zero_disables_claim_fee() {
    let f = Fixture::new();
    f.vault.set_claim_fee_bps(&f.admin, &0);
    f.accrue_reward(1_000);

    assert_eq!(f.vault.claim(&f.alice), 1_000);
    assert_eq!(f.vault.get_claim_fee_reserve(), 0);
    assert!(!has_event(&f.env, "claim_fee_collected"));
}

#[test]
fn set_claim_fee_rejects_more_than_five_percent() {
    let f = Fixture::new();
    let result = f.vault.try_set_claim_fee_bps(&f.admin, &501);
    assert_eq!(result, Err(Ok(VaultError::UnstakeFeeTooHigh)));
}

#[test]
fn withdraw_claim_fees_transfers_to_admin() {
    let f = Fixture::new();
    f.vault.set_claim_fee_bps(&f.admin, &500);
    f.accrue_reward(1_000);
    f.vault.claim(&f.alice);

    let before = f.token.balance(&f.admin);
    f.vault.withdraw_claim_fees(&f.admin, &50);

    assert_eq!(f.vault.get_claim_fee_reserve(), 0);
    assert_eq!(f.token.balance(&f.admin), before + 50);
    assert!(has_event(&f.env, "claim_fees_withdrawn"));
}
