#![cfg(test)]
//! Tests for the per-user rolling 24-hour withdrawal limit (issue #554).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env, InvokeError,
};

use crate::errors::VaultCampaignError;
use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
}

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| li.sequence_number = sequence);
}

fn setup<'a>() -> Fixture<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.budget().reset_unlimited();
    env.ledger().with_mut(|li| {
        li.min_temp_entry_ttl = 10_000_000;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
        li.sequence_number = 1000;
    });

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    let token_addr = env.register_stellar_asset_contract(admin.clone());
    let token_admin = token::StellarAssetClient::new(&env, &token_addr);

    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

    token_admin.mint(&alice, &100_000);
    token_admin.mint(&bob, &100_000);
    token_admin.mint(&vault_id, &100_000);
    vault.stake(&alice, &10_000, &0);
    vault.stake(&bob, &10_000, &0);

    Fixture { env, vault, admin, alice, bob }
}

/// `withdraw`/`unstake` return `VaultError`, so a `DailyLimitExceeded` panic
/// surfaces as a raw contract error code that doesn't map to any `VaultError`.
fn is_daily_limit_error<T, E>(
    result: Result<T, Result<E, InvokeError>>,
) -> bool {
    matches!(
        result,
        Err(Err(InvokeError::Contract(code)))
            if code == VaultCampaignError::DailyLimitExceeded as u32
    )
}

#[test]
fn test_limit_disabled_by_default() {
    let f = setup();
    assert_eq!(f.vault.get_daily_withdrawal_limit(), 0);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), i128::MAX);
    f.vault.withdraw(&f.alice, &10_000);
}

#[test]
fn test_set_daily_withdrawal_limit_admin_only_and_rejects_negative() {
    let f = setup();
    assert_eq!(
        f.vault.try_set_daily_withdrawal_limit(&f.alice, &1_000),
        Err(Ok(VaultCampaignError::Unauthorized))
    );
    assert_eq!(
        f.vault.try_set_daily_withdrawal_limit(&f.admin, &-1),
        Err(Ok(VaultCampaignError::InvalidDailyLimit))
    );
    f.vault.set_daily_withdrawal_limit(&f.admin, &1_000);
    assert_eq!(f.vault.get_daily_withdrawal_limit(), 1_000);
}

#[test]
fn test_withdrawals_under_cap_succeed_and_reduce_remaining() {
    let f = setup();
    f.vault.set_daily_withdrawal_limit(&f.admin, &1_000);

    f.vault.withdraw(&f.alice, &400);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 600);
    f.vault.unstake(&f.alice, &600);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 0);

    // Per-user: bob's allowance is untouched by alice's withdrawals.
    assert_eq!(f.vault.get_remaining_daily_limit(&f.bob), 1_000);
}

#[test]
fn test_exceeding_cap_reverts_with_daily_limit_exceeded() {
    let f = setup();
    f.vault.set_daily_withdrawal_limit(&f.admin, &1_000);

    // A single oversized withdrawal is rejected.
    assert!(is_daily_limit_error(f.vault.try_withdraw(&f.alice, &1_001)));

    // So is a cumulative one — via withdraw and unstake alike.
    f.vault.withdraw(&f.alice, &700);
    assert!(is_daily_limit_error(f.vault.try_withdraw(&f.alice, &301)));
    assert!(is_daily_limit_error(f.vault.try_unstake(&f.alice, &301)));

    // The rejected attempts weren't recorded.
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 300);
}

#[test]
fn test_window_rolls_forward_and_old_withdrawals_age_out() {
    let f = setup();
    f.vault.set_daily_withdrawal_limit(&f.admin, &1_000);

    f.vault.withdraw(&f.alice, &600); // ledger 1000
    set_ledger(&f.env, 1000 + LEDGERS_PER_DAY / 2);
    f.vault.withdraw(&f.alice, &400); // ledger 1000 + half a day
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 0);

    // One ledger before the first withdrawal ages out: still at the cap.
    set_ledger(&f.env, 1000 + LEDGERS_PER_DAY - 1);
    assert!(is_daily_limit_error(f.vault.try_withdraw(&f.alice, &1)));

    // Exactly a day after the first withdrawal it drops out of the window,
    // freeing its 600 — but the later 400 still counts (rolling, not a reset).
    set_ledger(&f.env, 1000 + LEDGERS_PER_DAY);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 600);
    assert!(is_daily_limit_error(f.vault.try_withdraw(&f.alice, &601)));
    f.vault.withdraw(&f.alice, &600);

    // A day after the second withdrawal, only the latest 600 remains.
    set_ledger(&f.env, 1000 + LEDGERS_PER_DAY / 2 + LEDGERS_PER_DAY);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 400);
}

#[test]
fn test_setting_zero_disables_the_limit() {
    let f = setup();
    f.vault.set_daily_withdrawal_limit(&f.admin, &100);
    assert!(is_daily_limit_error(f.vault.try_withdraw(&f.alice, &101)));

    f.vault.set_daily_withdrawal_limit(&f.admin, &0);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), i128::MAX);
    f.vault.withdraw(&f.alice, &5_000);
}
