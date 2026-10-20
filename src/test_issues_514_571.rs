#![cfg(test)]
//! Tests for issues #514–#517:
//! - #514 deposit allowlist for permissioned early-access period
//! - #515 per-user withdrawal rate limiting (anti-MEV)
//! - #516 partial reward claim via claim_partial
//! - #517 minimum interval between claim calls

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::allowlist_rate_limits::{self as al, AllowlistRateLimitError};
use crate::errors::VaultError;
use crate::vault::{VaultContract, VaultContractClient};

fn create_token<'a>(
    env: &Env,
    admin: &Address,
) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let address = env.register_stellar_asset_contract(admin.clone());
    let client = token::Client::new(env, &address);
    let admin_client = token::StellarAssetClient::new(env, &address);
    (address, client, admin_client)
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    admin: Address,
    #[allow(dead_code)]
    token_addr: Address,
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
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
        let (token_addr, token, token_admin) = create_token(&env, &admin);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            token_addr,
            token,
            token_admin,
        }
    }

    fn funded_user(&self, amount: i128) -> Address {
        let user = Address::generate(&self.env);
        self.token_admin.mint(&user, &amount);
        user
    }

    fn back_vault_with_rewards(&self, amount: i128) {
        self.token_admin.mint(&self.vault.address, &amount);
    }

    fn seed_accrued_reward(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            crate::balance::set_accrued_reward(&self.env, user, amount);
        });
    }
}

// ── Issue #514: deposit allowlist ────────────────────────────────────────────

#[test]
fn allowlist_disabled_preserves_open_access() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    // By default allowlist is disabled — anyone can stake
    let result = f.vault.stake(&user, &1_000_000_00, &0);
    assert!(result.is_ok());
}

#[test]
fn allowlist_blocks_non_members_when_enabled() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    // Enable allowlist
    al::set_allowlist_enabled(&f.env, &f.admin, true);
    assert!(al::is_allowlist_enabled(&f.env));

    // User is NOT on the allowlist
    assert!(!al::is_allowlisted(&f.env, &user));

    // check_allowlist should reject
    let result = al::check_allowlist(&f.env, &user);
    assert_eq!(result, Err(AllowlistRateLimitError::NotAllowlisted));
}

#[test]
fn allowlist_adds_and_removes_members() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);
    let user2 = f.funded_user(1_000_000_00);

    al::set_allowlist_enabled(&f.env, &f.admin, true);

    // Add users
    let users = soroban_sdk::Vec::from_array(&f.env, [user.clone(), user2.clone()]);
    al::add_to_allowlist(&f.env, &f.admin, &users);

    assert!(al::is_allowlisted(&f.env, &user));
    assert!(al::is_allowlisted(&f.env, &user2));

    // Remove user2
    let remove = soroban_sdk::Vec::from_array(&f.env, [user2.clone()]);
    al::remove_from_allowlist(&f.env, &f.admin, &remove);

    assert!(al::is_allowlisted(&f.env, &user));
    assert!(!al::is_allowlisted(&f.env, &user2));
}

#[test]
fn allowlist_takes_effect_immediately() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    // User can stake before allowlist
    assert!(al::check_allowlist(&f.env, &user).is_ok());

    // Enable allowlist — user is now blocked
    al::set_allowlist_enabled(&f.env, &f.admin, true);
    assert_eq!(al::check_allowlist(&f.env, &user), Err(AllowlistRateLimitError::NotAllowlisted));

    // Add user — immediately unblocked
    let users = soroban_sdk::Vec::from_array(&f.env, [user.clone()]);
    al::add_to_allowlist(&f.env, &f.admin, &users);
    assert!(al::check_allowlist(&f.env, &user).is_ok());

    // Remove user — immediately blocked again
    al::remove_from_allowlist(&f.env, &f.admin, &users);
    assert_eq!(al::check_allowlist(&f.env, &user), Err(AllowlistRateLimitError::NotAllowlisted));
}

// ── Issue #515: withdrawal rate limiting ─────────────────────────────────────

#[test]
fn withdrawal_interval_zero_disables_check() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    // Default interval is 0
    assert_eq!(al::get_user_withdrawal_interval(&f.env), 0);

    // Should always pass
    assert!(al::check_withdrawal_interval(&f.env, &user).is_ok());
}

#[test]
fn withdrawal_within_interval_reverts() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    // Set interval to 10 ledgers
    al::set_user_withdrawal_interval(&f.env, &f.admin, &10);
    assert_eq!(al::get_user_withdrawal_interval(&f.env), 10);

    // Record a withdrawal at ledger 1000
    f.env.ledger().with_mut(|li| li.sequence_number = 1000);
    al::record_withdrawal(&f.env, &user);

    // Check at ledger 1005 (within interval) — should fail
    f.env.ledger().with_mut(|li| li.sequence_number = 1005);
    assert_eq!(
        al::check_withdrawal_interval(&f.env, &user),
        Err(AllowlistRateLimitError::WithdrawalTooSoon)
    );
}

#[test]
fn withdrawal_after_interval_succeeds() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    al::set_user_withdrawal_interval(&f.env, &f.admin, &10);

    // Record at ledger 1000
    f.env.ledger().with_mut(|li| li.sequence_number = 1000);
    al::record_withdrawal(&f.env, &user);

    // Check at ledger 1010 (exactly at boundary) — should pass
    f.env.ledger().with_mut(|li| li.sequence_number = 1010);
    assert!(al::check_withdrawal_interval(&f.env, &user).is_ok());
}

#[test]
fn withdrawal_interval_0_disables_check() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    al::set_user_withdrawal_interval(&f.env, &f.admin, &0);
    f.env.ledger().with_mut(|li| li.sequence_number = 1000);
    al::record_withdrawal(&f.env, &user);

    // Immediate next ledger — should pass because interval is 0
    f.env.ledger().with_mut(|li| li.sequence_number = 1001);
    assert!(al::check_withdrawal_interval(&f.env, &user).is_ok());
}

// ── Issue #516: partial reward claim ─────────────────────────────────────────

#[test]
fn partial_claim_reduces_pending_by_exact_amount() {
    let pending: i128 = 10_000;
    let claim_amount: i128 = 3_000;

    let result = al::check_partial_claim_amount(pending, claim_amount);
    assert!(result.is_ok());

    // The remaining should be 7_000
    let remaining = pending - claim_amount;
    assert_eq!(remaining, 7_000);
}

#[test]
fn partial_claim_over_claim_reverts() {
    let pending: i128 = 5_000;
    let claim_amount: i128 = 10_000;

    assert_eq!(
        al::check_partial_claim_amount(pending, claim_amount),
        Err(AllowlistRateLimitError::InsufficientPendingReward)
    );
}

#[test]
fn partial_claim_zero_amount_reverts() {
    let pending: i128 = 5_000;

    assert_eq!(
        al::check_partial_claim_amount(pending, 0),
        Err(AllowlistRateLimitError::InsufficientPendingReward)
    );
}

#[test]
fn partial_claim_negative_amount_reverts() {
    let pending: i128 = 5_000;

    assert_eq!(
        al::check_partial_claim_amount(pending, -1_000),
        Err(AllowlistRateLimitError::InsufficientPendingReward)
    );
}

#[test]
fn partial_claim_exact_pending_succeeds() {
    let pending: i128 = 5_000;

    assert!(al::check_partial_claim_amount(pending, pending).is_ok());
}

// ── Issue #517: claim cooldown ───────────────────────────────────────────────

#[test]
fn claim_cooldown_zero_disables_check() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    // Default cooldown is 0
    assert_eq!(al::get_claim_cooldown(&f.env), 0);
    assert!(al::check_claim_cooldown(&f.env, &user).is_ok());
}

#[test]
fn claim_within_cooldown_reverts() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    // Set cooldown to 20 ledgers
    al::set_claim_cooldown(&f.env, &f.admin, &20);

    // Record a claim at ledger 1000
    f.env.ledger().with_mut(|li| li.sequence_number = 1000);
    al::record_claim(&f.env, &user);

    // Check at ledger 1010 (within cooldown) — should fail
    f.env.ledger().with_mut(|li| li.sequence_number = 1010);
    assert_eq!(
        al::check_claim_cooldown(&f.env, &user),
        Err(AllowlistRateLimitError::ClaimTooSoon)
    );
}

#[test]
fn claim_after_cooldown_succeeds() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    al::set_claim_cooldown(&f.env, &f.admin, &20);

    // Record at ledger 1000
    f.env.ledger().with_mut(|li| li.sequence_number = 1000);
    al::record_claim(&f.env, &user);

    // Check at ledger 1020 (exactly at boundary) — should pass
    f.env.ledger().with_mut(|li| li.sequence_number = 1020);
    assert!(al::check_claim_cooldown(&f.env, &user).is_ok());
}

#[test]
fn claim_cooldown_zero_disables_check() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    al::set_claim_cooldown(&f.env, &f.admin, &0);
    f.env.ledger().with_mut(|li| li.sequence_number = 1000);
    al::record_claim(&f.env, &user);

    // Immediate next ledger — should pass
    f.env.ledger().with_mut(|li| li.sequence_number = 1001);
    assert!(al::check_claim_cooldown(&f.env, &user).is_ok());
}

#[test]
fn last_claim_ledger_none_before_first_claim() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    assert_eq!(al::get_last_claim_ledger(&f.env, &user), None);
}

#[test]
fn last_claim_ledger_set_after_record() {
    let f = Fixture::new();
    let user = f.funded_user(1_000_000_00);

    f.env.ledger().with_mut(|li| li.sequence_number = 500);
    al::record_claim(&f.env, &user);

    assert_eq!(al::get_last_claim_ledger(&f.env, &user), Some(500));
}
