#![cfg(test)]
//! Tests for the staker diversity / stake-concentration score (issue #407).

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
        let token_addr = env.register_stellar_asset_contract(admin.clone());

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
        }
    }

    fn seed_stakers(&self, amounts: &[i128]) {
        self.env.as_contract(&self.vault_id, || {
            let mut stakers = balance::get_all_stakers(&self.env);
            for amount in amounts {
                let staker = Address::generate(&self.env);
                balance::set_shares(&self.env, &staker, *amount);
                stakers.push_back(staker);
            }
            balance::set_all_stakers(&self.env, &stakers);
        });
    }
}

#[test]
fn single_staker_is_minimum_diversity() {
    let f = Fixture::new();
    f.seed_stakers(&[100_000]);

    let report = f.vault.get_staker_diversity_report();
    assert_eq!(report.staker_count, 1);
    assert_eq!(report.herfindahl_index, 10_000);
    assert_eq!(report.diversity_score_bps, 0);
    assert_eq!(report.top_1_pct_share_bps, 10_000);
}

#[test]
fn equal_stakers_are_highly_diverse() {
    let f = Fixture::new();
    let amounts = [1_000i128; 100];
    f.seed_stakers(&amounts);

    let report = f.vault.get_staker_diversity_report();
    assert_eq!(report.staker_count, 100);
    // HHI for 100 equal stakers = 10_000 / 100 = 100 bps.
    assert_eq!(report.herfindahl_index, 100);
    assert_eq!(report.diversity_score_bps, 9_900);
    // Top 1% (1 staker) and top 10% (10 stakers) hold an equal share each.
    assert_eq!(report.top_1_pct_share_bps, 100);
    assert_eq!(report.top_10_pct_share_bps, 1_000);
}

#[test]
fn known_concentration_verified_manually() {
    let f = Fixture::new();
    // One whale with 90%, nine minnows sharing the remaining 10% equally.
    let mut amounts = std::vec![900_000i128];
    amounts.extend(std::vec![11_111i128; 9]);
    f.seed_stakers(&amounts);

    let report = f.vault.get_staker_diversity_report();
    assert_eq!(report.staker_count, 10);
    // Whale alone accounts for ~90% of stake.
    assert!(report.top_1_pct_share_bps >= 8_900);
    // A single dominant staker pushes diversity well below the equal-share case.
    assert!(report.diversity_score_bps < 9_000);
}

#[test]
fn no_stakers_returns_zero_report() {
    let f = Fixture::new();
    let report = f.vault.get_staker_diversity_report();
    assert_eq!(report.staker_count, 0);
    assert_eq!(report.diversity_score_bps, 0);
}
