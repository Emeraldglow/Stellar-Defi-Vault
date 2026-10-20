#![cfg(test)]
//! Tests for issues #697-#700: share withdrawal allowances, keeper
//! authorization, extra reward tokens, and reward epochs.
//!
//! Positions and reward balances are seeded straight into storage so each test
//! exercises only the feature under test.

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    token, Address, Env, Symbol, TryFromVal,
};

use crate::balance;
use crate::errors::VaultFeature6Error;
use crate::vault::{VaultContract, VaultContractClient};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    token: token::Client<'a>,
    admin: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
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
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
        token::StellarAssetClient::new(&env, &token_addr).mint(&vault_id, &10_000_000);

        Fixture {
            env,
            vault,
            vault_id,
            token,
            admin,
        }
    }

    fn advance(&self, ledgers: u32) {
        self.env
            .ledger()
            .with_mut(|li| li.sequence_number += ledgers);
    }

    fn now(&self) -> u32 {
        self.env.ledger().sequence()
    }

    /// Give `user` `shares` at a 1:1 share price.
    fn seed_position(&self, user: &Address, shares: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_shares(&self.env, user, shares);
            balance::set_total_shares(&self.env, balance::get_total_shares(&self.env) + shares);
            balance::set_total_deposited(
                &self.env,
                balance::get_total_deposited(&self.env) + shares,
            );
        });
    }

    fn shares_of(&self, user: &Address) -> i128 {
        self.env
            .as_contract(&self.vault_id, || balance::get_shares(&self.env, user))
    }

    fn new_reward_token(&self) -> (Address, token::StellarAssetClient<'a>) {
        let addr = self
            .env
            .register_stellar_asset_contract(Address::generate(&self.env));
        let client = token::StellarAssetClient::new(&self.env, &addr);
        (addr, client)
    }
}

// ── Issue #697: share withdrawal allowances ─────────────────────────────────

#[test]
fn withdraw_from_pays_recipient_and_reduces_allowance() {
    let f = Fixture::new();
    let (owner, spender, recipient) = (
        Address::generate(&f.env),
        Address::generate(&f.env),
        Address::generate(&f.env),
    );
    f.seed_position(&owner, 1000);
    f.vault
        .approve_withdrawal(&owner, &spender, &600, &(f.now() + 100));
    assert_eq!(f.vault.get_withdrawal_allowance(&owner, &spender), 600);

    let paid = f.vault.withdraw_from(&spender, &owner, &400, &recipient);

    assert_eq!(paid, 400);
    assert_eq!(f.token.balance(&recipient), 400);
    assert_eq!(f.token.balance(&spender), 0);
    assert_eq!(f.shares_of(&owner), 600);
    assert_eq!(f.vault.get_withdrawal_allowance(&owner, &spender), 200);
}

#[test]
fn withdraw_from_over_allowance_reverts() {
    let f = Fixture::new();
    let (owner, spender) = (Address::generate(&f.env), Address::generate(&f.env));
    f.seed_position(&owner, 1000);
    f.vault
        .approve_withdrawal(&owner, &spender, &100, &(f.now() + 100));

    let res = f
        .vault
        .try_withdraw_from(&spender, &owner, &101, &spender);

    assert_eq!(res, Err(Ok(VaultFeature6Error::InsufficientAllowance)));
    assert_eq!(f.shares_of(&owner), 1000);
    assert_eq!(f.vault.get_withdrawal_allowance(&owner, &spender), 100);
}

#[test]
fn withdraw_from_without_allowance_reverts() {
    let f = Fixture::new();
    let (owner, spender) = (Address::generate(&f.env), Address::generate(&f.env));
    f.seed_position(&owner, 1000);

    let res = f.vault.try_withdraw_from(&spender, &owner, &1, &spender);

    assert_eq!(res, Err(Ok(VaultFeature6Error::InsufficientAllowance)));
}

#[test]
fn withdraw_from_expired_allowance_reverts() {
    let f = Fixture::new();
    let (owner, spender) = (Address::generate(&f.env), Address::generate(&f.env));
    f.seed_position(&owner, 1000);
    f.vault
        .approve_withdrawal(&owner, &spender, &500, &(f.now() + 10));

    f.advance(10);

    assert_eq!(f.vault.get_withdrawal_allowance(&owner, &spender), 0);
    let res = f.vault.try_withdraw_from(&spender, &owner, &1, &spender);
    assert_eq!(res, Err(Ok(VaultFeature6Error::AllowanceExpired)));
}

#[test]
fn owner_can_revoke_allowance() {
    let f = Fixture::new();
    let (owner, spender) = (Address::generate(&f.env), Address::generate(&f.env));
    f.seed_position(&owner, 1000);
    f.vault
        .approve_withdrawal(&owner, &spender, &500, &(f.now() + 100));

    f.vault
        .approve_withdrawal(&owner, &spender, &0, &0);

    assert_eq!(f.vault.get_withdrawal_allowance(&owner, &spender), 0);
    let res = f.vault.try_withdraw_from(&spender, &owner, &1, &spender);
    assert_eq!(res, Err(Ok(VaultFeature6Error::InsufficientAllowance)));
}

#[test]
fn approve_withdrawal_rejects_bad_inputs() {
    let f = Fixture::new();
    let (owner, spender) = (Address::generate(&f.env), Address::generate(&f.env));

    let negative = f
        .vault
        .try_approve_withdrawal(&owner, &spender, &-1, &(f.now() + 100));
    assert_eq!(negative, Err(Ok(VaultFeature6Error::InvalidAmount)));

    let past = f
        .vault
        .try_approve_withdrawal(&owner, &spender, &10, &f.now());
    assert_eq!(past, Err(Ok(VaultFeature6Error::InvalidExpiry)));
}

// ── Issue #698: keeper authorization ────────────────────────────────────────

#[test]
fn authorize_and_revoke_keeper() {
    let f = Fixture::new();
    let (user, keeper) = (Address::generate(&f.env), Address::generate(&f.env));
    assert!(!f.vault.is_authorized_keeper(&user, &keeper));

    f.vault.authorize_keeper(&user, &keeper);
    assert!(f.vault.is_authorized_keeper(&user, &keeper));

    f.vault.revoke_keeper(&user, &keeper);
    assert!(!f.vault.is_authorized_keeper(&user, &keeper));
}

#[test]
fn at_most_three_keepers_per_user() {
    let f = Fixture::new();
    let user = Address::generate(&f.env);
    for _ in 0..3 {
        f.vault.authorize_keeper(&user, &Address::generate(&f.env));
    }

    let fourth = Address::generate(&f.env);
    let res = f.vault.try_authorize_keeper(&user, &fourth);
    assert_eq!(res, Err(Ok(VaultFeature6Error::KeeperLimitReached)));

    // Re-authorizing an existing keeper is not a new slot.
    let existing = Address::generate(&f.env);
    let other = Address::generate(&f.env);
    f.vault.authorize_keeper(&other, &existing);
    f.vault.authorize_keeper(&other, &existing);
    assert!(f.vault.is_authorized_keeper(&other, &existing));
}

#[test]
fn unauthorized_keeper_cannot_claim_or_compound() {
    let f = Fixture::new();
    let (user, keeper) = (Address::generate(&f.env), Address::generate(&f.env));

    let claim = f.vault.try_claim_for(&keeper, &user);
    assert_eq!(claim, Err(Ok(VaultFeature6Error::KeeperNotAuthorized)));

    let compound = f.vault.try_compound_for(&keeper, &user);
    assert_eq!(compound, Err(Ok(VaultFeature6Error::KeeperNotAuthorized)));
}

#[test]
fn claim_for_pays_user_never_keeper() {
    let f = Fixture::new();
    let (user, keeper) = (Address::generate(&f.env), Address::generate(&f.env));
    f.seed_position(&user, 1000);
    f.env.as_contract(&f.vault_id, || {
        balance::set_accrued_reward(&f.env, &user, 500);
        balance::set_reward_pool_balance(&f.env, 5_000);
    });
    f.vault.authorize_keeper(&user, &keeper);

    let paid = f.vault.claim_for(&keeper, &user);

    assert!(paid > 0);
    assert_eq!(f.token.balance(&user), paid);
    assert_eq!(f.token.balance(&keeper), 0);
}

#[test]
fn revoked_keeper_is_rejected_immediately() {
    let f = Fixture::new();
    let (user, keeper) = (Address::generate(&f.env), Address::generate(&f.env));
    f.vault.authorize_keeper(&user, &keeper);
    f.vault.revoke_keeper(&user, &keeper);

    let res = f.vault.try_claim_for(&keeper, &user);

    assert_eq!(res, Err(Ok(VaultFeature6Error::KeeperNotAuthorized)));
}

// ── Issue #699: multiple reward tokens ──────────────────────────────────────

/// Shares chosen so that 100 ledgers at 1_000 bps earn exactly 100_000.
const SHARES: i128 = 63_072_000_000;

fn assert_close(actual: i128, expected: i128) {
    assert!(
        (actual - expected).abs() <= 2,
        "expected about {expected}, got {actual}"
    );
}

fn pending_of(f: &Fixture, user: &Address, token: &Address) -> i128 {
    f.vault
        .get_pending_rewards(user)
        .iter()
        .find(|(t, _)| t == token)
        .map(|(_, amount)| amount)
        .unwrap()
}

#[test]
fn add_reward_token_enforces_limit_and_admin() {
    let f = Fixture::new();
    for _ in 0..3 {
        let (addr, _) = f.new_reward_token();
        f.vault.add_reward_token(&f.admin, &addr, &1_000);
    }

    let (fourth, _) = f.new_reward_token();
    let res = f.vault.try_add_reward_token(&f.admin, &fourth, &1_000);
    assert_eq!(res, Err(Ok(VaultFeature6Error::RewardTokenLimitReached)));
}

#[test]
fn add_reward_token_rejects_duplicates_bad_rate_and_non_admin() {
    let f = Fixture::new();
    let (addr, _) = f.new_reward_token();
    f.vault.add_reward_token(&f.admin, &addr, &1_000);

    let dup = f.vault.try_add_reward_token(&f.admin, &addr, &1_000);
    assert_eq!(dup, Err(Ok(VaultFeature6Error::RewardTokenAlreadyAdded)));

    let (other, _) = f.new_reward_token();
    let zero = f.vault.try_add_reward_token(&f.admin, &other, &0);
    assert_eq!(zero, Err(Ok(VaultFeature6Error::InvalidRate)));

    let stranger = Address::generate(&f.env);
    let res = f.vault.try_add_reward_token(&stranger, &other, &1_000);
    assert_eq!(res, Err(Ok(VaultFeature6Error::Unauthorized)));
}

#[test]
fn extra_tokens_accrue_independently() {
    let f = Fixture::new();
    let user = Address::generate(&f.env);
    let (t1, _) = f.new_reward_token();
    let (t2, _) = f.new_reward_token();
    f.vault.add_reward_token(&f.admin, &t1, &1_000);
    f.vault.add_reward_token(&f.admin, &t2, &2_000);
    f.seed_position(&user, SHARES);

    f.advance(100);

    assert_close(pending_of(&f, &user, &t1), 100_000);
    assert_close(pending_of(&f, &user, &t2), 200_000);
}

#[test]
fn share_changes_settle_at_the_old_share_count() {
    let f = Fixture::new();
    let user = Address::generate(&f.env);
    let (t1, _) = f.new_reward_token();
    f.vault.add_reward_token(&f.admin, &t1, &1_000);
    f.seed_position(&user, SHARES);

    f.advance(100);
    f.env.as_contract(&f.vault_id, || {
        balance::set_shares(&f.env, &user, SHARES / 2);
    });
    f.advance(100);

    // 100 ledgers at full size, then 100 at half size.
    assert_close(pending_of(&f, &user, &t1), 150_000);
}

#[test]
fn claim_token_pays_one_token_from_its_pool() {
    let f = Fixture::new();
    let user = Address::generate(&f.env);
    let funder = Address::generate(&f.env);
    let (t1, t1_admin) = f.new_reward_token();
    let (t2, _) = f.new_reward_token();
    f.vault.add_reward_token(&f.admin, &t1, &1_000);
    f.vault.add_reward_token(&f.admin, &t2, &2_000);
    t1_admin.mint(&funder, &1_000_000);
    f.vault.fund_reward_token(&funder, &t1, &1_000_000);
    f.seed_position(&user, SHARES);
    f.advance(100);

    let paid = f.vault.claim_token(&user, &t1);

    assert_close(paid, 100_000);
    assert_eq!(token::Client::new(&f.env, &t1).balance(&user), paid);
    assert_close(pending_of(&f, &user, &t1), 0);
    assert_close(pending_of(&f, &user, &t2), 200_000);
}

#[test]
fn dry_token_does_not_block_others() {
    let f = Fixture::new();
    let user = Address::generate(&f.env);
    let funder = Address::generate(&f.env);
    let (funded, funded_admin) = f.new_reward_token();
    let (dry, _) = f.new_reward_token();
    f.vault.add_reward_token(&f.admin, &dry, &2_000);
    f.vault.add_reward_token(&f.admin, &funded, &1_000);
    funded_admin.mint(&funder, &1_000_000);
    f.vault.fund_reward_token(&funder, &funded, &1_000_000);
    f.seed_position(&user, SHARES);
    f.advance(100);

    let results = f.vault.claim_all(&user);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap(), (dry.clone(), 0));
    let (paid_token, paid) = results.get(1).unwrap();
    assert_eq!(paid_token, funded);
    assert_close(paid, 100_000);
    // The dry token's earnings stay pending until it is funded.
    assert_close(pending_of(&f, &user, &dry), 200_000);
}

#[test]
fn claim_token_rejects_unknown_token() {
    let f = Fixture::new();
    let user = Address::generate(&f.env);
    let (unknown, _) = f.new_reward_token();

    let res = f.vault.try_claim_token(&user, &unknown);

    assert_eq!(res, Err(Ok(VaultFeature6Error::RewardTokenNotFound)));
}

#[test]
fn fund_reward_token_rejects_unknown_token_and_zero() {
    let f = Fixture::new();
    let funder = Address::generate(&f.env);
    let (unknown, _) = f.new_reward_token();
    let (known, _) = f.new_reward_token();
    f.vault.add_reward_token(&f.admin, &known, &1_000);

    let res = f.vault.try_fund_reward_token(&funder, &unknown, &10);
    assert_eq!(res, Err(Ok(VaultFeature6Error::RewardTokenNotFound)));

    let zero = f.vault.try_fund_reward_token(&funder, &known, &0);
    assert_eq!(zero, Err(Ok(VaultFeature6Error::ZeroAmount)));
}

// ── Issue #700: reward epochs ───────────────────────────────────────────────

fn apply(f: &Fixture, amount: i128) -> i128 {
    f.env.as_contract(&f.vault_id, || {
        crate::vault::reward_epochs::apply_budget(&f.env, amount)
    })
}

#[test]
fn no_epoch_config_leaves_accrual_unchanged() {
    let f = Fixture::new();

    assert_eq!(f.vault.get_current_epoch(), (0, 0, 0, 0));
    assert_eq!(apply(&f, 12_345), 12_345);
}

#[test]
fn set_epoch_config_rejects_invalid_values_and_non_admin() {
    let f = Fixture::new();

    let zero_len = f.vault.try_set_epoch_config(&f.admin, &0, &100);
    assert_eq!(zero_len, Err(Ok(VaultFeature6Error::InvalidEpochConfig)));

    let negative = f.vault.try_set_epoch_config(&f.admin, &10, &-1);
    assert_eq!(negative, Err(Ok(VaultFeature6Error::InvalidEpochConfig)));

    let stranger = Address::generate(&f.env);
    let res = f.vault.try_set_epoch_config(&stranger, &10, &100);
    assert_eq!(res, Err(Ok(VaultFeature6Error::Unauthorized)));
}

#[test]
fn epoch_budget_cap_is_enforced() {
    let f = Fixture::new();
    let start = f.now();
    f.vault.set_epoch_config(&f.admin, &100, &1_000);

    assert_eq!(apply(&f, 600), 600);
    assert_eq!(apply(&f, 600), 400);
    assert_eq!(apply(&f, 10), 0);
    assert_eq!(f.vault.get_current_epoch(), (0, start + 100, 1_000, 1_000));
}

#[test]
fn epoch_rolls_over_and_discards_unused_budget() {
    let f = Fixture::new();
    let start = f.now();
    f.vault.set_epoch_config(&f.admin, &100, &1_000);
    assert_eq!(apply(&f, 400), 400);

    f.advance(100);

    // The view already reports the new epoch, with a fresh budget.
    assert_eq!(f.vault.get_current_epoch(), (1, start + 200, 0, 1_000));
    // The 600 left over in epoch 0 is gone: epoch 1 can emit exactly 1_000.
    assert_eq!(apply(&f, 1_500), 1_000);
    assert_eq!(apply(&f, 1), 0);
    assert_eq!(f.vault.get_current_epoch(), (1, start + 200, 1_000, 1_000));
}

#[test]
fn epoch_rolled_event_is_emitted_on_boundary_crossing() {
    let f = Fixture::new();
    f.vault.set_epoch_config(&f.admin, &100, &1_000);
    apply(&f, 100);

    f.advance(100);
    apply(&f, 100);

    let events = f.env.events().all();
    let (_, topics, _) = events.last().unwrap();
    let name = Symbol::try_from_val(&f.env, &topics.get(0).unwrap()).unwrap();
    assert_eq!(name, Symbol::new(&f.env, "epoch_rolled"));
}

#[test]
fn skipped_epochs_are_counted() {
    let f = Fixture::new();
    let start = f.now();
    f.vault.set_epoch_config(&f.admin, &100, &1_000);

    f.advance(350);

    assert_eq!(f.vault.get_current_epoch(), (3, start + 400, 0, 1_000));
    assert_eq!(apply(&f, 5), 5);
    assert_eq!(f.vault.get_current_epoch(), (3, start + 400, 5, 1_000));
}
