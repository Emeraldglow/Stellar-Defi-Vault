#![cfg(test)]
//! Tests for issue #593: standardized custom error enum replacing ad hoc
//! panic messages.
//!
//! Every revert condition that used to be a raw `panic!("SomeString")` in the
//! issue #498–#505 extension modules is now a typed `VaultFeature5Error`
//! (or a propagated `VaultError` from the admin gate), so callers get stable,
//! matchable numeric codes instead of string parsing. Each test below pins one
//! error path to its exact variant — a regression to a generic panic would
//! surface as `Err(Err(..))` (host failure) and fail the assertion.

use soroban_sdk::{testutils::{Address as _, Ledger as _}, Address, Env, Vec};

use crate::errors::{VaultError, VaultFeature5Error};
use crate::storage::AdminAction;
use crate::vault::{VaultContract, VaultContractClient};

/// Deploys the vault initialized with a throwaway stake token, returning the
/// client and admin. No token flows are exercised, so the asset contract is
/// only there to satisfy `initialize`.
fn setup(env: &Env) -> (VaultContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(env, &contract_id);
    let token_addr = env.register_stellar_asset_contract(admin.clone());
    client.initialize(&admin, &token_addr, &0_u32, &None, &None);
    (client, admin)
}

// ── Numeric code stability ──────────────────────────────────────────────────
// The codes are a public API: integrators match on them. Renumbering is a
// breaking change, so pin the exact discriminants.

#[test]
fn vault_error_codes_are_stable() {
    assert_eq!(VaultError::NotInitialized as u32, 1);
    assert_eq!(VaultError::AlreadyInitialized as u32, 2);
    assert_eq!(VaultError::Unauthorized as u32, 3);
    assert_eq!(VaultError::ZeroAmount as u32, 4);
    assert_eq!(VaultError::InsufficientShares as u32, 5);
    assert_eq!(VaultError::VaultPaused as u32, 6);
    assert_eq!(VaultError::ArithmeticError as u32, 8);
    assert_eq!(VaultError::PositionNotFound as u32, 18);
}

#[test]
fn vault_feature5_error_codes_are_stable() {
    assert_eq!(VaultFeature5Error::Unauthorized as u32, 1);
    assert_eq!(VaultFeature5Error::NotInitialized as u32, 2);
    assert_eq!(VaultFeature5Error::InvalidSplitRecipients as u32, 3);
    assert_eq!(VaultFeature5Error::InvalidSplitBpsSum as u32, 4);
    assert_eq!(VaultFeature5Error::UnregisteredToken as u32, 5);
    assert_eq!(VaultFeature5Error::TimelockNotExpired as u32, 6);
    assert_eq!(VaultFeature5Error::ActionNotFound as u32, 7);
    assert_eq!(VaultFeature5Error::AutoCompoundNotEnabled as u32, 8);
    assert_eq!(VaultFeature5Error::NoSharesToTokenize as u32, 9);
    assert_eq!(VaultFeature5Error::NotNftOwner as u32, 10);
}

// ── Issue #499: treasury split ──────────────────────────────────────────────
// Replaces panic!("InvalidSplitRecipients") / panic!("InvalidSplitBpsSum").

#[test]
fn treasury_split_rejects_too_many_recipients() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let mut recipients = Vec::new(&env);
    let mut bps = Vec::new(&env);
    for _ in 0..4 {
        recipients.push_back(Address::generate(&env));
        bps.push_back(2500);
    }
    let res = client.try_set_treasury_split(&admin, &recipients, &bps);
    assert_eq!(res, Err(Ok(VaultFeature5Error::InvalidSplitRecipients)));
}

#[test]
fn treasury_split_rejects_length_mismatch() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let mut recipients = Vec::new(&env);
    recipients.push_back(Address::generate(&env));
    recipients.push_back(Address::generate(&env));
    let mut bps = Vec::new(&env);
    bps.push_back(10000);
    let res = client.try_set_treasury_split(&admin, &recipients, &bps);
    assert_eq!(res, Err(Ok(VaultFeature5Error::InvalidSplitRecipients)));
}

#[test]
fn treasury_split_rejects_bad_bps_sum() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let mut recipients = Vec::new(&env);
    recipients.push_back(Address::generate(&env));
    let mut bps = Vec::new(&env);
    bps.push_back(9999);
    let res = client.try_set_treasury_split(&admin, &recipients, &bps);
    assert_eq!(res, Err(Ok(VaultFeature5Error::InvalidSplitBpsSum)));
}

#[test]
fn treasury_split_accepts_valid_config() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let mut recipients = Vec::new(&env);
    recipients.push_back(Address::generate(&env));
    recipients.push_back(Address::generate(&env));
    let mut bps = Vec::new(&env);
    bps.push_back(5000);
    bps.push_back(5000);
    let res = client.try_set_treasury_split(&admin, &recipients, &bps);
    assert_eq!(res, Ok(Ok(())));
}

// ── Issue #501: reward token swap ───────────────────────────────────────────
// Replaces unwrap_or_else(|| panic!("UnregisteredToken")).

#[test]
fn swap_secondary_reward_rejects_unregistered_token() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let unregistered = Address::generate(&env);
    let res = client.try_swap_secondary_reward(&admin, &unregistered, &100, &1);
    assert_eq!(res, Err(Ok(VaultFeature5Error::UnregisteredToken)));
}

// ── Issue #503: admin timelock ──────────────────────────────────────────────
// Replaces panic!("TimelockNotExpired") / panic!("ActionNotFound").

#[test]
fn execute_admin_action_rejects_unexpired_timelock() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    client.set_timelock_delay(&100);
    let action_id = client.queue_admin_action(&admin, &AdminAction::Pause);

    // executable_at = current ledger + 100, so executing right away reverts.
    let res = client.try_execute_admin_action(&admin, &action_id);
    assert_eq!(res, Err(Ok(VaultFeature5Error::TimelockNotExpired)));

    // After the delay the same call succeeds — the typed error did not change
    // *when* the contract reverts, only how the reason is communicated.
    env.ledger().with_mut(|li| li.sequence_number += 100);
    let res = client.try_execute_admin_action(&admin, &action_id);
    assert_eq!(res, Ok(Ok(())));
}

#[test]
fn execute_admin_action_rejects_unknown_id() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let res = client.try_execute_admin_action(&admin, &999);
    assert_eq!(res, Err(Ok(VaultFeature5Error::ActionNotFound)));
}

#[test]
fn cancel_admin_action_rejects_unknown_id() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let res = client.try_cancel_admin_action(&admin, &999);
    assert_eq!(res, Err(Ok(VaultFeature5Error::ActionNotFound)));
}

// ── Issue #504: auto compound ───────────────────────────────────────────────
// Replaces panic!("AutoCompoundNotEnabled").

#[test]
fn compound_rejects_user_without_opt_in() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let user = Address::generate(&env);
    let res = client.try_compound(&user);
    assert_eq!(res, Err(Ok(VaultFeature5Error::AutoCompoundNotEnabled)));
}

#[test]
fn compound_succeeds_after_opt_in() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let user = Address::generate(&env);
    client.set_auto_compound(&user, &true);
    // No accrued rewards, so this is a no-op — but it must not revert.
    let res = client.try_compound(&user);
    assert_eq!(res, Ok(Ok(())));
}

// ── Issue #505: tokenize position ───────────────────────────────────────────
// Replaces panic!("NoSharesToTokenize") / panic!("NotNFTOwner").

#[test]
fn tokenize_position_rejects_zero_shares() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let user = Address::generate(&env);
    let res = client.try_tokenize_position(&user);
    assert_eq!(res, Err(Ok(VaultFeature5Error::NoSharesToTokenize)));
}

#[test]
fn redeem_position_nft_rejects_non_owner() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    // Token id 1 was never minted, so no one owns it — any caller reverts.
    let stranger = Address::generate(&env);
    let res = client.try_redeem_position_nft(&stranger, &1);
    assert_eq!(res, Err(Ok(VaultFeature5Error::NotNftOwner)));
}

// ── Admin gates propagate typed errors, never untyped panics ────────────────
// The extension entrypoints used to `.unwrap()` the admin check; now the
// `VaultError` propagates. Before initialization the vault has no admin, so
// every admin-gated extension call must surface `NotInitialized` as a typed
// error (`Err(Ok(..))`), not a host panic (`Err(Err(..))`).

#[test]
fn admin_gates_return_typed_errors_instead_of_panicking() {
    let env = Env::default();
    env.mock_all_auths();
    // Deliberately NOT initialized.
    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let mut recipients = Vec::new(&env);
    recipients.push_back(Address::generate(&env));
    let mut bps = Vec::new(&env);
    bps.push_back(10000);

    assert_eq!(
        client.try_set_treasury_split(&admin, &recipients, &bps),
        Err(Ok(VaultFeature5Error::NotInitialized))
    );
    assert_eq!(
        client.try_set_reward_token_swap_path(
            &admin,
            &Address::generate(&env),
            &Address::generate(&env)
        ),
        Err(Ok(VaultError::NotInitialized))
    );
    assert_eq!(
        client.try_enable_withdrawal_queue(&admin, &true),
        Err(Ok(VaultError::NotInitialized))
    );
    assert_eq!(
        client.try_queue_admin_action(&admin, &AdminAction::Pause),
        Err(Ok(VaultError::NotInitialized))
    );
    assert_eq!(
        client.try_execute_admin_action(&admin, &1),
        Err(Ok(VaultFeature5Error::NotInitialized))
    );
    assert_eq!(
        client.try_cancel_admin_action(&admin, &1),
        Err(Ok(VaultFeature5Error::NotInitialized))
    );
}
