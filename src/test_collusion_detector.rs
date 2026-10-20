#![cfg(test)]
//! Tests for the coordinated stake/unstake pattern detector (issue #406).

extern crate std;

use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

use crate::{
    balance,
    collusion_detector::CollusionPattern,
    crate::{VaultContract, VaultContractClient},
};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    alice: Address,
    bob: Address,
    carol: Address,
    dave: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 10_000;
        });

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let carol = Address::generate(&env);
        let dave = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        Fixture {
            env,
            vault,
            vault_id,
            alice,
            bob,
            carol,
            dave,
        }
    }

    fn seed_activity(&self, user: &Address, history: &[(u32, i128)]) {
        self.env.as_contract(&self.vault_id, || {
            let mut all = balance::get_all_stakers(&self.env);
            all.push_back(user.clone());
            balance::set_all_stakers(&self.env, &all);

            let mut hist: Vec<(u32, i128)> = Vec::new(&self.env);
            for (ledger, amount) in history {
                hist.push_back((*ledger, *amount));
            }
            balance::set_stake_history(&self.env, user, &hist);
        });
    }
}

#[test]
fn coordinated_stakes_trigger_alert() {
    let f = Fixture::new();
    f.seed_activity(&f.alice, &[(5_000, 1_000)]);
    f.seed_activity(&f.bob, &[(5_300, 1_020)]);
    f.seed_activity(&f.carol, &[(5_800, 980)]);

    let alerts = f.vault.check_collusion();
    assert_eq!(alerts.len(), 1);
    let alert = alerts.get(0).unwrap();
    assert_eq!(alert.pattern_type, CollusionPattern::CoordinatedStake);
    assert_eq!(alert.addresses.len(), 3);

    assert_eq!(f.vault.get_collusion_alerts().len(), 1);
}

#[test]
fn wash_pattern_detected() {
    let f = Fixture::new();
    f.seed_activity(&f.dave, &[(1_000, 500), (1_500, 510), (2_000, 520)]);

    let alerts = f.vault.check_collusion();
    assert_eq!(alerts.len(), 1);
    let alert = alerts.get(0).unwrap();
    assert_eq!(alert.pattern_type, CollusionPattern::WashPattern);
    assert_eq!(alert.addresses.len(), 1);
    assert_eq!(alert.addresses.get(0).unwrap(), f.dave);
    assert_eq!(alert.total_coordinated_amount, 1_530);
}

#[test]
fn alert_dismissed_correctly() {
    let f = Fixture::new();
    f.seed_activity(&f.alice, &[(5_000, 1_000)]);
    f.seed_activity(&f.bob, &[(5_300, 1_020)]);
    f.seed_activity(&f.carol, &[(5_800, 980)]);

    f.vault.check_collusion();
    assert_eq!(f.vault.get_collusion_alerts().len(), 1);

    f.vault.dismiss_alert(&0);
    assert_eq!(f.vault.get_collusion_alerts().len(), 0);
}

#[test]
fn false_positive_threshold_not_triggered_for_legitimate_similar_amounts() {
    let f = Fixture::new();
    // Only two addresses with similar amounts â€” below the 3-address threshold.
    f.seed_activity(&f.alice, &[(5_000, 1_000)]);
    f.seed_activity(&f.bob, &[(5_300, 1_020)]);

    let alerts = f.vault.check_collusion();
    assert_eq!(alerts.len(), 0);
    assert_eq!(f.vault.get_collusion_alerts().len(), 0);
}

