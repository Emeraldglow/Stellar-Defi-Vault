#![cfg(test)]
//! Tests for the regulatory compliance report generator (issue #409).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

use crate::{
    balance,
    compliance_report::MAX_COMPLIANCE_REPORTS,
    errors::VaultError,
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
    admin: Address,
    alice: Address,
    bob: Address,
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
        let bob = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            alice,
            bob,
        }
    }

    fn seed_position(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            let total_shares = balance::get_total_shares(&self.env);
            let total_deposited = balance::get_total_deposited(&self.env);
            balance::set_shares(&self.env, user, amount);
            balance::set_total_shares(&self.env, total_shares + amount);
            balance::set_total_deposited(&self.env, total_deposited + amount);
            let mut stakers = balance::get_all_stakers(&self.env);
            stakers.push_back(user.clone());
            balance::set_all_stakers(&self.env, &stakers);
        });
    }

    fn set_kyc_approved(&self, user: &Address, approved: bool) {
        self.env.as_contract(&self.vault_id, || {
            self.env
                .storage()
                .persistent()
                .set(&DataKey::KycApproved(user.clone()), &approved);
        });
    }
}

#[test]
fn report_fields_populated_correctly() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 100_000);
    f.seed_position(&f.bob, 50_000);
    f.set_kyc_approved(&f.alice, true);

    f.env.as_contract(&f.vault_id, || {
        balance::set_total_rewards_paid(&f.env, 7_000);
        balance::add_protocol_fee_collected(&f.env, 1_200);
    });

    let report = f.vault.generate_compliance_report(&1_000, &2_000);

    assert_eq!(report.ledger_from, 1_000);
    assert_eq!(report.ledger_to, 2_000);
    assert_eq!(report.unique_stakers, 2);
    assert_eq!(report.peak_tvl, 150_000);
    assert_eq!(report.total_rewards_paid, 7_000);
    assert_eq!(report.total_fees_collected, 1_200);
    assert_eq!(report.kyc_approved_stakers, 1);
    assert_eq!(report.generated_at, 1_000);
}

#[test]
fn history_capped_at_max_reports() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 100_000);

    for i in 0..(MAX_COMPLIANCE_REPORTS + 3) {
        f.vault.generate_compliance_report(&(i * 10), &(i * 10 + 5));
    }

    let all = f.vault.get_all_compliance_reports();
    assert_eq!(all.len(), MAX_COMPLIANCE_REPORTS);
    // Oldest three were evicted; the retained history starts at report id 3.
    assert_eq!(all.get(0).unwrap().report_id, 3);
}

#[test]
fn ledger_range_validated() {
    let f = Fixture::new();
    let result = f.vault.try_generate_compliance_report(&2_000, &1_000);
    assert_eq!(result, Err(Ok(VaultError::InvalidRate)));

    let result = f.vault.try_generate_compliance_report(&1_000, &1_000);
    assert_eq!(result, Err(Ok(VaultError::InvalidRate)));
}

