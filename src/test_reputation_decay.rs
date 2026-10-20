#![cfg(test)]
//! Tests for the reputation score time-decay mechanism.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::{
    reputation_decay::DecayConfig,
    vault::{VaultContract, VaultContractClient, BOOST_BPS_BASE},
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
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
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);
        token_admin.mint(&alice, &10_000_000);
        token_admin.mint(&bob, &10_000_000);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            admin,
            alice,
            bob,
        }
    }
}

// ── set_reputation_decay_rate ────────────────────────────────────────────────

#[test]
fn admin_can_set_decay_rate() {
    let f = Fixture::new();
    f.vault.set_reputation_decay_rate(&100_u32, &100_000_u32);

    let cfg = f.vault.get_reputation_decay_config().unwrap();
    assert_eq!(cfg.decay_bps_per_epoch, 100);
    assert_eq!(cfg.epoch_ledgers, 100_000);
}

#[test]
fn decay_config_defaults_to_none() {
    let f = Fixture::new();
    assert!(f.vault.get_reputation_decay_config().is_none());
}

#[test]
fn decay_rate_zero_bps_over_10000_rejected() {
    let f = Fixture::new();
    let result = f.vault.try_set_reputation_decay_rate(&10_001_u32, &100_000_u32);
    assert_eq!(result, Err(Ok(crate::errors::VaultError::InvalidRate)));
}

#[test]
fn decay_rate_zero_epoch_rejected() {
    let f = Fixture::new();
    let result = f.vault.try_set_reputation_decay_rate(&100_u32, &0_u32);
    assert_eq!(result, Err(Ok(crate::errors::VaultError::InvalidRate)));
}

// ── apply_reputation_decay ───────────────────────────────────────────────────

#[test]
fn inactive_user_score_decays() {
    let f = Fixture::new();
    // Alice stakes at ledger 1_000.
    f.vault.stake(&f.alice, &5_000, &0);

    // Configure decay: 100 bps per 10_000 ledgers.
    f.vault.set_reputation_decay_rate(&100_u32, &10_000_u32);

    let score_before = f.vault.get_reputation_score(&f.alice);
    assert!(score_before.total_score > 0);

    // Advance 10_000 ledgers (1 epoch) without any activity.
    set_ledger(&f.env, 11_000);

    let score_after = f.vault.apply_reputation_decay(&f.alice);
    // Should have decayed by 100 bps.
    assert_eq!(
        score_after.total_score,
        score_before.total_score.saturating_sub(100)
    );
}

#[test]
fn apply_reputation_decay_requires_affected_user_auth() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000);
    f.vault.set_reputation_decay_rate(&100_u32, &10_000_u32);
    set_ledger(&f.env, 11_000);
    f.env.set_auths(&[]);

    assert!(f.vault.try_apply_reputation_decay(&f.alice).is_err());
}

#[test]
fn active_user_score_unchanged() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);
    f.vault.set_reputation_decay_rate(&100_u32, &10_000_u32);

    let score_before = f.vault.get_reputation_score(&f.alice);

    // Alice claims at ledger 5_000 — updates activity.
    set_ledger(&f.env, 5_000);
    f.vault.claim(&f.alice);

    // Advance to 11_000 but alice was active at 5_000, so only 0 epochs.
    set_ledger(&f.env, 11_000);

    let score_after = f.vault.apply_reputation_decay(&f.alice);
    // No decay — less than 1 epoch since last claim (6_000 ledgers < 10_000).
    assert_eq!(score_after.total_score, score_before.total_score);
}

#[test]
fn score_floors_at_zero() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &1_000, &0);
    // 10_000 bps (100%) per epoch = full wipeout per epoch.
    f.vault.set_reputation_decay_rate(&10_000_u32, &10_000_u32);

    set_ledger(&f.env, 1_000);
    let _score_before = f.vault.get_reputation_score(&f.alice);

    // Advance 5 epochs.
    set_ledger(&f.env, 51_000);

    let score_after = f.vault.apply_reputation_decay(&f.alice);
    assert_eq!(score_after.total_score, 0);
}

#[test]
fn decay_rate_zero_disables_feature() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);
    // 0 bps per epoch = no decay.
    f.vault.set_reputation_decay_rate(&0_u32, &10_000_u32);

    let score_before = f.vault.get_reputation_score(&f.alice);

    set_ledger(&f.env, 100_000);

    let score_after = f.vault.apply_reputation_decay(&f.alice);
    assert_eq!(score_after.total_score, score_before.total_score);
}

#[test]
fn no_decay_config_means_no_decay() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    let score_before = f.vault.get_reputation_score(&f.alice);

    // No decay config set — advance far.
    set_ledger(&f.env, 1_000_000);

    let score_after = f.vault.apply_reputation_decay(&f.alice);
    assert_eq!(score_after.total_score, score_before.total_score);
}

#[test]
fn non_staker_returns_zeros() {
    let f = Fixture::new();
    f.vault.set_reputation_decay_rate(&100_u32, &10_000_u32);

    let score = f.vault.apply_reputation_decay(&f.alice);
    assert_eq!(score.total_score, 0);
    assert_eq!(score.duration_score, 0);
    assert_eq!(score.consistency_score, 0);
    assert_eq!(score.size_score, 0);
    assert_eq!(score.streak_score, 0);
}

#[test]
fn get_reputation_score_applies_decay_lazily() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);
    f.vault.set_reputation_decay_rate(&100_u32, &10_000_u32);

    set_ledger(&f.env, 1_000);
    let score_before = f.vault.get_reputation_score(&f.alice);

    // Advance 1 epoch without activity.
    set_ledger(&f.env, 11_000);

    // get_reputation_score should apply decay automatically.
    let score_after = f.vault.get_reputation_score(&f.alice);
    assert_eq!(
        score_after.total_score,
        score_before.total_score.saturating_sub(100)
    );
}

#[test]
fn multiple_epochs_decay_correctly() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);
    // 200 bps per epoch.
    f.vault.set_reputation_decay_rate(&200_u32, &10_000_u32);

    set_ledger(&f.env, 1_000);
    let score_before = f.vault.get_reputation_score(&f.alice);

    // Advance 3 epochs.
    set_ledger(&f.env, 31_000);

    let score_after = f.vault.apply_reputation_decay(&f.alice);
    // 3 epochs * 200 bps = 600 bps decay.
    assert_eq!(
        score_after.total_score,
        score_before.total_score.saturating_sub(600)
    );
}

#[test]
fn sub_components_preserved_after_decay() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);
    f.vault.set_reputation_decay_rate(&100_u32, &10_000_u32);

    set_ledger(&f.env, 1_000);
    let score_before = f.vault.get_reputation_score(&f.alice);

    set_ledger(&f.env, 11_000);
    let score_after = f.vault.apply_reputation_decay(&f.alice);

    // Sub-components are NOT decayed — only total_score.
    assert_eq!(score_after.duration_score, score_before.duration_score);
    assert_eq!(score_after.consistency_score, score_before.consistency_score);
    assert_eq!(score_after.size_score, score_before.size_score);
    assert_eq!(score_after.streak_score, score_before.streak_score);
}
