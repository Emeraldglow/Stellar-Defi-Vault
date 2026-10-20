#![cfg(test)]
//! Tests for the pool comfort score (issue #399).

extern crate std;

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::balance;
use crate::vault::{VaultContract, VaultContractClient};

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
        // 500 bps reward rate.
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        Fixture {
            env,
            vault,
            vault_id,
            alice,
        }
    }

    fn set_unstake_fee_bps(&self, bps: u32) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_unstake_fee_bps(&self.env, bps);
        });
    }
}

#[test]
fn no_profile_returns_max_score() {
    let f = Fixture::new();
    let score = f.vault.get_comfort_score(&f.alice);
    assert_eq!(score.score, 100);
    assert!(!score.lock_flag && !score.apy_flag && !score.fee_flag);
    assert!(!score.slash_flag && !score.audit_flag);
}

#[test]
fn no_conflicts_returns_hundred() {
    let f = Fixture::new();
    f.vault.set_risk_profile(
        &f.alice,
        &crate::comfort_score::UserRiskProfile {
            max_lock_days: 0,
            min_apy_bps: 100,
            max_fee_bps: 1_000,
            requires_no_slash: false,
            requires_audited: false,
        },
    );

    let score = f.vault.get_comfort_score(&f.alice);
    assert_eq!(score.score, 100);
}

#[test]
fn each_flag_deducts_twenty() {
    let f = Fixture::new();
    f.vault.set_pool_audited(&false);
    f.vault.set_risk_profile(
        &f.alice,
        &crate::comfort_score::UserRiskProfile {
            max_lock_days: 0,
            min_apy_bps: 100,
            max_fee_bps: 1_000,
            requires_no_slash: false,
            requires_audited: true, // pool is not audited => one flag
        },
    );

    let score = f.vault.get_comfort_score(&f.alice);
    assert!(score.audit_flag);
    assert_eq!(score.score, 80);
}

#[test]
fn all_flags_returns_zero() {
    let f = Fixture::new();
    f.vault.set_pool_lock_days(&30);
    f.vault.set_pool_slash_risk(&true);
    f.vault.set_pool_audited(&false);
    f.set_unstake_fee_bps(200);

    f.vault.set_risk_profile(
        &f.alice,
        &crate::comfort_score::UserRiskProfile {
            max_lock_days: 10,  // pool lock (30) exceeds this
            min_apy_bps: 1_000, // pool rate (500) is below this
            max_fee_bps: 100,   // pool fee (200) exceeds this
            requires_no_slash: true,
            requires_audited: true,
        },
    );

    let score = f.vault.get_comfort_score(&f.alice);
    assert!(score.lock_flag);
    assert!(score.apy_flag);
    assert!(score.fee_flag);
    assert!(score.slash_flag);
    assert!(score.audit_flag);
    assert_eq!(score.score, 0);
}
