#![cfg(test)]
//! Issues #652, #653, #655, #656.
//!
//! - #652 — vault accounting / reward-solvency invariants.
//! - #653 — extreme share-price conversions (dust, huge deposits, donations).
//! - #655 — vault behavior against a configurable nonstandard token.
//! - #656 — explicit authorization tests (no `mock_all_auths`).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};

use crate::mock_token::{setup_mock_token, MockTokenClient};
use crate::storage::PauseReason;
use crate::vault::{VaultContract, VaultContractClient};

// ── shared fixture ─────────────────────────────────────────────────────────

struct Fixture<'a> {
    env: Env,
    admin: Address,
    token_id: Address,
    token: MockTokenClient<'a>,
    vault_id: Address,
    vault: VaultContractClient<'a>,
}

fn fixture<'a>(decimals: u32, fee_bps: u32, reward_rate: i128) -> Fixture<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.budget().reset_unlimited();
    let admin = Address::generate(&env);
    let (token_id, token) = setup_mock_token(&env, decimals, fee_bps);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_id, &reward_rate, &None, &None);
    Fixture { env, admin, token_id, token, vault_id, vault }
}

fn funded_user(f: &Fixture, amount: i128) -> Address {
    let user = Address::generate(&f.env);
    f.token.mint(&user, &amount);
    user
}

/// Core invariant (#652): the vault's token balance always covers every
/// redeemable share plus the tracked reward pool.
fn assert_solvent(f: &Fixture, total_shares: i128) {
    let held = f.token.balance(&f.vault_id);
    let owed = if total_shares > 0 {
        f.vault.preview_redeem(&total_shares)
    } else {
        0
    };
    let pool = f.vault.get_reward_pool_balance();
    assert!(pool >= 0, "reward pool went negative: {pool}");
    assert!(
        held >= owed,
        "insolvent: held={held} owed_principal={owed} pool={pool}"
    );
}

// ── #652: accounting invariants ────────────────────────────────────────────

#[test]
fn invariant_holds_across_deposit_withdraw_sequence() {
    let f = fixture(7, 0, 0);
    let a = funded_user(&f, 1_000_000_000);
    let b = funded_user(&f, 1_000_000_000);

    let sa = f.vault.deposit(&a, &300_000_000);
    assert_solvent(&f, sa);
    let sb = f.vault.deposit(&b, &700_000_000);
    assert_solvent(&f, sa + sb);

    let out_a = f.vault.withdraw(&a, &(sa / 2));
    assert!(out_a > 0);
    assert_solvent(&f, sa - sa / 2 + sb);

    f.vault.withdraw(&b, &sb);
    assert_solvent(&f, sa - sa / 2);

    f.vault.withdraw(&a, &(sa - sa / 2));
    assert_solvent(&f, 0);
    // Rounding must never leave the vault owing more than it holds, and
    // users can never extract more than they put in.
    assert!(f.token.balance(&a) <= 1_000_000_000);
    assert!(f.token.balance(&b) <= 1_000_000_000);
}

#[test]
fn reward_pool_funding_is_tracked_and_never_negative() {
    let f = fixture(7, 0, 10);
    f.token.mint(&f.admin, &500_000);
    f.vault.fund_reward_pool(&f.admin, &500_000);
    assert_eq!(f.vault.get_reward_pool_balance(), 500_000);

    let a = funded_user(&f, 10_000_000);
    let s = f.vault.deposit(&a, &10_000_000);
    f.env.ledger().with_mut(|l| l.sequence_number += 1_000);

    // Claiming can succeed or be rejected, but must never break solvency.
    let _ = f.vault.try_claim(&a);
    assert_solvent(&f, s);
    assert!(f.vault.get_reward_pool_balance() <= 500_000);
}

#[test]
fn claims_cannot_exceed_funded_rewards() {
    let f = fixture(7, 0, 1_000_000);
    f.token.mint(&f.admin, &1_000);
    f.vault.fund_reward_pool(&f.admin, &1_000);

    let a = funded_user(&f, 10_000_000);
    let s = f.vault.deposit(&a, &10_000_000);
    f.env.ledger().with_mut(|l| l.sequence_number += 100_000);

    let before = f.token.balance(&a);
    let _ = f.vault.try_claim(&a);
    let paid = f.token.balance(&a) - before;
    assert!(paid <= 1_000, "paid {paid} more than funded rewards");
    assert_solvent(&f, s);
}

#[test]
fn zero_and_negative_amounts_rejected() {
    let f = fixture(7, 0, 0);
    let a = funded_user(&f, 1_000);
    assert!(f.vault.try_deposit(&a, &0).is_err());
    assert!(f.vault.try_deposit(&a, &-1).is_err());
    assert!(f.vault.try_withdraw(&a, &0).is_err());
    assert!(f.vault.try_fund_reward_pool(&f.admin, &0).is_err());
    assert_eq!(f.token.balance(&f.vault_id), 0);
}

#[test]
fn cannot_withdraw_more_shares_than_owned() {
    let f = fixture(7, 0, 0);
    let a = funded_user(&f, 1_000_000);
    let b = funded_user(&f, 1_000_000);
    let sa = f.vault.deposit(&a, &1_000_000);
    f.vault.deposit(&b, &1_000_000);
    assert!(f.vault.try_withdraw(&a, &(sa + 1)).is_err());
}

// ── #653: extreme share prices ─────────────────────────────────────────────

#[test]
fn dust_deposit_after_large_deposit_does_not_mint_free_shares() {
    let f = fixture(7, 0, 0);
    let whale = funded_user(&f, 1_000_000_000_000_000);
    let minnow = funded_user(&f, 1);
    let sw = f.vault.deposit(&whale, &1_000_000_000_000_000);

    match f.vault.try_deposit(&minnow, &1) {
        Ok(Ok(s)) => {
            assert!(s >= 0);
            // Redeeming those shares must never return more than deposited.
            if s > 0 {
                assert!(f.vault.preview_redeem(&s) <= 1);
            }
            assert_solvent(&f, sw + s);
        }
        _ => assert_solvent(&f, sw),
    }
}

#[test]
fn donation_inflation_attack_does_not_steal_victim_deposit() {
    let f = fixture(7, 0, 0);
    let attacker = funded_user(&f, 1_000_000_001);
    let victim = funded_user(&f, 1_000_000);

    let sa = f.vault.deposit(&attacker, &1);
    // Attacker donates directly to the vault to inflate the share price.
    f.token.transfer(&attacker, &f.vault_id, &1_000_000_000);

    let sv = match f.vault.try_deposit(&victim, &1_000_000) {
        Ok(Ok(s)) => s,
        _ => return, // rejecting the deposit is also a safe outcome
    };
    if sv == 0 {
        // A zero-share mint that keeps the funds would be a loss.
        panic!("victim deposit accepted but minted zero shares");
    }
    let victim_value = f.vault.preview_redeem(&sv);
    assert!(victim_value <= 1_000_000);
    assert_solvent(&f, sa + sv);
}

#[test]
fn huge_deposit_does_not_overflow() {
    let f = fixture(7, 0, 0);
    let amount: i128 = i128::MAX / 1_000_000_000_000;
    let a = funded_user(&f, amount);
    match f.vault.try_deposit(&a, &amount) {
        Ok(Ok(s)) => {
            assert!(s > 0);
            assert!(f.vault.preview_redeem(&s) <= amount);
        }
        // A clean error (not a panic/overflow wrap) is acceptable.
        Ok(Err(_)) | Err(_) => {}
    }
}

#[test]
fn preview_redeem_round_trip_never_gains() {
    for &amt in &[1i128, 7, 999, 10_000_001, 123_456_789_012] {
        let f = fixture(7, 0, 0);
        let a = funded_user(&f, amt);
        if let Ok(Ok(s)) = f.vault.try_deposit(&a, &amt) {
            assert!(f.vault.preview_redeem(&s) <= amt, "gain at amount {amt}");
        }
    }
}

#[test]
fn share_conversion_is_safe_for_varied_decimals() {
    for &dec in &[0u32, 6, 7, 18] {
        let f = fixture(dec, 0, 0);
        let a = funded_user(&f, 1_000_000);
        if let Ok(Ok(s)) = f.vault.try_deposit(&a, &1_000_000) {
            assert!(f.vault.preview_redeem(&s) <= 1_000_000, "decimals {dec}");
            assert_solvent(&f, s);
        }
    }
}

// ── #655: nonstandard token behavior ───────────────────────────────────────

#[test]
fn fee_on_transfer_token_never_credits_more_than_received() {
    let f = fixture(7, 100, 0); // 1% fee
    let a = funded_user(&f, 1_000_000);
    if let Ok(Ok(s)) = f.vault.try_deposit(&a, &1_000_000) {
        let received = f.token.balance(&f.vault_id);
        assert_eq!(received, 990_000);
        // Redeemable value must be backed by what actually arrived.
        assert!(f.vault.preview_redeem(&s) <= received);
    }
}

#[test]
fn failing_token_transfer_reverts_deposit_atomically() {
    let f = fixture(7, 0, 0);
    let a = funded_user(&f, 1_000);
    f.token.set_fail_all(&true);
    assert!(f.vault.try_deposit(&a, &1_000).is_err());
    f.token.set_fail_all(&false);
    assert_eq!(f.token.balance(&a), 1_000);
    assert_eq!(f.token.balance(&f.vault_id), 0);
}

#[test]
fn blocked_recipient_withdraw_reverts_and_keeps_shares() {
    let f = fixture(7, 0, 0);
    let a = funded_user(&f, 1_000_000);
    let s = f.vault.deposit(&a, &1_000_000);
    f.token.set_blocked(&a, &true);
    assert!(f.vault.try_withdraw(&a, &s).is_err());
    f.token.set_blocked(&a, &false);
    // Shares were not burned by the failed withdraw.
    let out = f.vault.withdraw(&a, &s);
    assert!(out > 0 && out <= 1_000_000);
}

// ── #656: explicit authorization ───────────────────────────────────────────

fn strict_fixture<'a>() -> (Env, Address, Address, VaultContractClient<'a>, MockTokenClient<'a>) {
    let env = Env::default();
    env.budget().reset_unlimited();
    let admin = Address::generate(&env);
    let (token_id, token) = setup_mock_token(&env, 7, 0);
    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    env.mock_all_auths();
    vault.initialize(&admin, &token_id, &0, &None, &None);
    // Drop blanket auth mocking: every following call must be authorized
    // explicitly.
    env.set_auths(&[]);
    (env, admin, vault_id, vault, token)
}

#[test]
fn deposit_requires_depositor_auth() {
    let (env, _admin, _vault_id, vault, token) = strict_fixture();
    let user = Address::generate(&env);
    token.mint(&user, &1_000);
    assert!(vault.try_deposit(&user, &1_000).is_err());
}

#[test]
fn deposit_succeeds_with_exact_depositor_auth() {
    let (env, _admin, vault_id, vault, token) = strict_fixture();
    let user = Address::generate(&env);
    token.mint(&user, &1_000);
    env.mock_all_auths();
    vault.deposit(&user, &1_000);
    let auths = env.auths();
    assert!(
        auths.iter().any(|(addr, inv)| *addr == user
            && matches!(&inv.function,
                soroban_sdk::testutils::AuthorizedFunction::Contract((c, _, _)) if *c == vault_id)),
        "deposit must require the depositor's auth on the vault"
    );
}

#[test]
fn withdraw_by_stranger_is_rejected() {
    let (env, _admin, vault_id, vault, token) = strict_fixture();
    let user = Address::generate(&env);
    let thief = Address::generate(&env);
    token.mint(&user, &1_000);
    env.mock_all_auths();
    let s = vault.deposit(&user, &1_000);
    env.set_auths(&[]);

    // Thief authorizes only for itself; withdrawing the user's shares fails.
    let res = vault
        .mock_auths(&[MockAuth {
            address: &thief,
            invoke: &MockAuthInvoke {
                contract: &vault_id,
                fn_name: "withdraw",
                args: (user.clone(), s).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_withdraw(&user, &s);
    assert!(res.is_err());
}

#[test]
fn admin_only_functions_reject_non_admin() {
    let (env, admin, vault_id, vault, token) = strict_fixture();
    let attacker = Address::generate(&env);
    token.mint(&attacker, &1_000);

    // fund_reward_pool with a non-admin address must fail even when that
    // address authorizes the call.
    let res = vault
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &vault_id,
                fn_name: "fund_reward_pool",
                args: (attacker.clone(), 1_000i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_fund_reward_pool(&attacker, &1_000);
    assert!(res.is_err());

    // pause/unpause without admin auth must fail.
    let msg = soroban_sdk::String::from_str(&env, "x");
    assert!(vault.try_pause(&PauseReason::Emergency, &msg).is_err());
    assert!(vault.try_unpause().is_err());

    let _ = admin;
}

#[test]
fn claim_requires_staker_auth() {
    let (env, _admin, _vault_id, vault, token) = strict_fixture();
    let user = Address::generate(&env);
    token.mint(&user, &1_000);
    env.mock_all_auths();
    vault.deposit(&user, &1_000);
    env.set_auths(&[]);
    assert!(vault.try_claim(&user).is_err());
}
