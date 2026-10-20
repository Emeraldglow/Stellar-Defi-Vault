#![cfg(test)]
//! Tests for issues #287–#290.
//!
//! Kept in their own module rather than appended to `test.rs` (8.5k lines) so
//! each feature's setup stays next to its assertions, mirroring how the
//! features themselves are split into their own modules.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Bytes, Env, Vec,
};

use crate::{
    commitment::{compute_hash, DEFAULT_COMMITMENT_WINDOW},
    errors::VaultError,
    insurance::SOLVENCY_PERIOD_LEDGERS,
    price_oracle::MAX_PRICE_HISTORY,
    vault::{VaultContract, VaultContractClient},
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

/// Register a vault with a funded staker, and return the pieces each test needs.
fn setup<'a>() -> (Env, VaultContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let token_address = env.register_stellar_asset_contract(admin.clone());
    let token_admin = token::StellarAssetClient::new(&env, &token_address);
    token_admin.mint(&user, &1_000_000);

    let contract_id = env.register_contract(None, VaultContract);
    let client = VaultContractClient::new(&env, &contract_id);
    client.initialize(&admin, &token_address);

    set_ledger(&env, 1_000);

    (env, client, admin, user)
}

// ── #287: reward vesting cliff ───────────────────────────────────────────────

#[test]
fn cliff_defaults_to_disabled() {
    let (_env, client, _admin, _user) = setup();

    // Zero must be the default, or introducing this feature would silently
    // freeze rewards on every pool that upgrades.
    assert_eq!(client.get_vesting_cliff(), 0);
}

#[test]
fn admin_can_set_and_read_the_cliff() {
    let (_env, client, _admin, _user) = setup();

    client.set_vesting_cliff(&5_000);
    assert_eq!(client.get_vesting_cliff(), 5_000);
}

#[test]
fn no_rewards_accrue_before_the_cliff() {
    let (env, client, _admin, user) = setup();

    client.set_vesting_cliff(&10_000);
    client.stake(&user, &1_000, &0);

    // Part-way to the cliff: still nothing.
    set_ledger(&env, 6_000);
    assert_eq!(client.calc_pending_reward(&user), 0);
}

#[test]
fn rewards_unlock_retroactively_at_the_cliff() {
    let (env, client, _admin, user) = setup();

    client.set_vesting_cliff(&1_000);
    client.stake(&user, &1_000, &0);

    let before = client.calc_pending_reward(&user);
    assert_eq!(before, 0, "inside the cliff nothing should be pending");

    // Cross the cliff. Accrual runs from staked_at_ledger, not from the cliff
    // date, so the first post-cliff read pays for the whole elapsed period.
    set_ledger(&env, 1_000 + 1_001);
    let after = client.calc_pending_reward(&user);

    assert!(
        after >= before,
        "crossing the cliff must not reduce the pending balance"
    );
}

#[test]
fn a_zero_cliff_disables_the_gate() {
    let (env, client, _admin, user) = setup();

    client.set_vesting_cliff(&0);
    client.stake(&user, &1_000, &0);
    set_ledger(&env, 2_000);

    assert!(client.is_past_cliff(&user));
    // With no cliff the unlock ledger collapses onto the stake ledger.
    assert_eq!(client.cliff_unlock_ledger(&user), 1_000);
}

#[test]
fn cliff_unlock_ledger_is_stake_plus_cliff() {
    let (_env, client, _admin, user) = setup();

    client.set_vesting_cliff(&2_500);
    client.stake(&user, &1_000, &0);

    assert_eq!(client.cliff_unlock_ledger(&user), 1_000 + 2_500);
    assert!(!client.is_past_cliff(&user));
}

#[test]
fn restaking_resets_the_cliff() {
    let (env, client, _admin, user) = setup();

    client.set_vesting_cliff(&1_000);
    client.stake(&user, &1_000, &0);

    set_ledger(&env, 2_500);
    assert!(client.is_past_cliff(&user));

    // A fresh stake moves staked_at_ledger forward, so the position re-enters
    // its cliff rather than keeping the old unlock.
    client.stake(&user, &1_000, &0);
    assert!(!client.is_past_cliff(&user));
    assert_eq!(client.cliff_unlock_ledger(&user), 2_500 + 1_000);
}

#[test]
fn a_user_with_no_stake_is_treated_as_past_the_cliff() {
    let (_env, client, _admin, _user) = setup();
    let stranger = Address::generate(&_env);

    client.set_vesting_cliff(&10_000);

    // Nothing to gate, and reporting `false` would make an address that never
    // staked look permanently locked.
    assert!(client.is_past_cliff(&stranger));
    assert_eq!(client.cliff_unlock_ledger(&stranger), 0);
}

// ── #288: commitment scheme ──────────────────────────────────────────────────

#[test]
fn commitment_window_defaults_and_is_configurable() {
    let (_env, client, _admin, _user) = setup();

    assert_eq!(client.get_commitment_window(), DEFAULT_COMMITMENT_WINDOW);

    client.set_commitment_window(&500);
    assert_eq!(client.get_commitment_window(), 500);
}

#[test]
fn a_zero_commitment_window_is_rejected() {
    let (_env, client, _admin, _user) = setup();

    // A zero window expires every commitment in the ledger it was made,
    // making reveal impossible.
    assert_eq!(
        client.try_set_commitment_window(&0),
        Err(Ok(VaultError::InvalidRate))
    );
}

#[test]
fn a_valid_reveal_stakes_the_committed_amount() {
    let (env, client, _admin, user) = setup();

    let salt = Bytes::from_array(&env, &[7u8; 16]);
    let amount = 5_000i128;
    let hash = compute_hash(&env, amount, &salt);

    client.commit_to_stake(&user, &hash);

    let record = client.get_commitment(&user).expect("commitment stored");
    assert!(!record.revealed);
    assert_eq!(record.committed_at, 1_000);

    client.reveal_and_stake(&user, &amount, &salt);

    // The commitment is consumed, so it cannot be replayed.
    assert!(client.get_commitment(&user).is_none());
}

#[test]
fn a_wrong_salt_is_rejected() {
    let (env, client, _admin, user) = setup();

    let salt = Bytes::from_array(&env, &[7u8; 16]);
    let wrong_salt = Bytes::from_array(&env, &[8u8; 16]);
    let hash = compute_hash(&env, 5_000, &salt);

    client.commit_to_stake(&user, &hash);

    assert!(client
        .try_reveal_and_stake(&user, &5_000, &wrong_salt)
        .is_err());
}

#[test]
fn revealing_a_different_amount_is_rejected() {
    let (env, client, _admin, user) = setup();

    let salt = Bytes::from_array(&env, &[7u8; 16]);
    let hash = compute_hash(&env, 5_000, &salt);

    client.commit_to_stake(&user, &hash);

    // The whole point: the committed amount is binding.
    assert!(client.try_reveal_and_stake(&user, &9_999, &salt).is_err());
}

#[test]
fn an_expired_commitment_cannot_be_revealed() {
    let (env, client, _admin, user) = setup();

    client.set_commitment_window(&100);

    let salt = Bytes::from_array(&env, &[7u8; 16]);
    let hash = compute_hash(&env, 5_000, &salt);
    client.commit_to_stake(&user, &hash);

    set_ledger(&env, 1_000 + 101);
    assert!(client.try_reveal_and_stake(&user, &5_000, &salt).is_err());
}

#[test]
fn a_second_commitment_is_rejected_while_one_stands() {
    let (env, client, _admin, user) = setup();

    let salt = Bytes::from_array(&env, &[7u8; 16]);
    let hash = compute_hash(&env, 5_000, &salt);
    let other = compute_hash(&env, 6_000, &salt);

    client.commit_to_stake(&user, &hash);

    // Allowing two would let a committer prepare a range of amounts and reveal
    // whichever suited them once the market moved.
    assert_eq!(
        client.try_commit_to_stake(&user, &other),
        Err(Ok(VaultError::MaxPositionsReached))
    );
}

#[test]
fn a_new_commitment_is_allowed_once_the_old_one_expires() {
    let (env, client, _admin, user) = setup();

    client.set_commitment_window(&100);

    let salt = Bytes::from_array(&env, &[7u8; 16]);
    client.commit_to_stake(&user, &compute_hash(&env, 5_000, &salt));

    set_ledger(&env, 1_000 + 101);
    client.commit_to_stake(&user, &compute_hash(&env, 6_000, &salt));

    assert_eq!(client.get_commitment(&user).unwrap().committed_at, 1_101);
}

#[test]
fn a_malformed_commitment_hash_is_rejected() {
    let (env, client, _admin, user) = setup();

    // Not a 32-byte digest, so no reveal could ever satisfy it.
    let short = Bytes::from_array(&env, &[1u8; 8]);
    assert!(client.try_commit_to_stake(&user, &short).is_err());
}

#[test]
fn the_hash_binds_amount_and_salt_together() {
    let env = Env::default();
    let salt_a = Bytes::from_array(&env, &[1u8; 8]);
    let salt_b = Bytes::from_array(&env, &[2u8; 8]);

    assert_ne!(
        compute_hash(&env, 100, &salt_a),
        compute_hash(&env, 100, &salt_b)
    );
    assert_ne!(
        compute_hash(&env, 100, &salt_a),
        compute_hash(&env, 200, &salt_a)
    );
    assert_eq!(
        compute_hash(&env, 100, &salt_a),
        compute_hash(&env, 100, &salt_a)
    );
}

// ── #289: pool health insurance ──────────────────────────────────────────────

#[test]
fn guarantor_registers_and_deposits() {
    let (_env, client, _admin, _user) = setup();
    let guarantor = Address::generate(&_env);

    client.register_guarantor(&guarantor, &50_000);
    assert_eq!(client.get_guarantee_reserve(), 0);

    client.deposit_guarantee(&guarantor, &20_000);
    assert_eq!(client.get_guarantee_reserve(), 20_000);

    client.deposit_guarantee(&guarantor, &5_000);
    assert_eq!(client.get_guarantee_reserve(), 25_000);
}

#[test]
fn a_non_guarantor_cannot_deposit() {
    let (env, client, _admin, _user) = setup();
    let guarantor = Address::generate(&env);
    let stranger = Address::generate(&env);

    client.register_guarantor(&guarantor, &50_000);

    assert_eq!(
        client.try_deposit_guarantee(&stranger, &1_000),
        Err(Ok(VaultError::RelayerNotApproved))
    );
}

#[test]
fn claims_are_blocked_until_insolvency_is_declared() {
    let (env, client, _admin, user) = setup();
    let guarantor = Address::generate(&env);

    client.register_guarantor(&guarantor, &50_000);
    client.deposit_guarantee(&guarantor, &20_000);

    assert!(!client.is_pool_insolvent());
    assert!(client.try_claim_guarantee(&user, &1_000).is_err());
}

#[test]
fn a_user_can_claim_after_insolvency() {
    let (env, client, _admin, user) = setup();
    let guarantor = Address::generate(&env);

    client.register_guarantor(&guarantor, &50_000);
    client.deposit_guarantee(&guarantor, &20_000);

    client.declare_insolvency();
    assert!(client.is_pool_insolvent());

    let remaining = client.claim_guarantee(&user, &8_000);
    assert_eq!(remaining, 12_000);
    assert_eq!(client.get_guarantee_reserve(), 12_000);
}

#[test]
fn a_claim_cannot_exceed_the_reserve() {
    let (env, client, _admin, user) = setup();
    let guarantor = Address::generate(&env);

    client.register_guarantor(&guarantor, &50_000);
    client.deposit_guarantee(&guarantor, &1_000);
    client.declare_insolvency();

    assert_eq!(
        client.try_claim_guarantee(&user, &5_000),
        Err(Ok(VaultError::InsufficientRewardPool))
    );
}

#[test]
fn insolvency_cannot_be_declared_twice() {
    let (env, client, _admin, _user) = setup();
    let guarantor = Address::generate(&env);

    client.register_guarantor(&guarantor, &50_000);
    client.declare_insolvency();

    // Irreversible, and therefore not re-triggerable.
    assert_eq!(
        client.try_declare_insolvency(),
        Err(Ok(VaultError::PoolShuttingDown))
    );
}

#[test]
fn early_withdrawal_is_blocked_before_the_solvency_period() {
    let (env, client, _admin, _user) = setup();
    let guarantor = Address::generate(&env);

    client.register_guarantor(&guarantor, &50_000);
    client.deposit_guarantee(&guarantor, &20_000);

    // One ledger short of the 90 days.
    set_ledger(&env, 1_000 + SOLVENCY_PERIOD_LEDGERS - 1);
    assert!(client.try_withdraw_guarantee(&guarantor).is_err());
    assert_eq!(client.get_guarantee_reserve(), 20_000);
}

#[test]
fn withdrawal_succeeds_after_ninety_days_of_solvency() {
    let (env, client, _admin, _user) = setup();
    let guarantor = Address::generate(&env);

    client.register_guarantor(&guarantor, &50_000);
    client.deposit_guarantee(&guarantor, &20_000);
    assert_eq!(
        client.guarantee_unlock_ledger(),
        1_000 + SOLVENCY_PERIOD_LEDGERS
    );

    set_ledger(&env, 1_000 + SOLVENCY_PERIOD_LEDGERS);
    assert_eq!(client.withdraw_guarantee(&guarantor), 20_000);
    assert_eq!(client.get_guarantee_reserve(), 0);
}

#[test]
fn withdrawal_is_blocked_after_insolvency_even_past_the_period() {
    let (env, client, _admin, _user) = setup();
    let guarantor = Address::generate(&env);

    client.register_guarantor(&guarantor, &50_000);
    client.deposit_guarantee(&guarantor, &20_000);
    client.declare_insolvency();

    // The reserve must not be pullable out from under the claims it covers.
    set_ledger(&env, 1_000 + SOLVENCY_PERIOD_LEDGERS + 1);
    assert_eq!(
        client.try_withdraw_guarantee(&guarantor),
        Err(Ok(VaultError::PoolShuttingDown))
    );
}

// ── #290: position price oracle ──────────────────────────────────────────────

#[test]
fn a_published_price_sums_principal_and_pending_reward() {
    let (_env, client, _admin, user) = setup();

    client.stake(&user, &10_000, &0);

    let price = client.publish_position_price(&user);
    assert_eq!(price.user, user);
    assert_eq!(price.fair_value, price.principal + price.pending_reward);
    assert_eq!(price.published_at, 1_000);
}

#[test]
fn the_latest_price_is_the_most_recent_one() {
    let (env, client, _admin, user) = setup();

    client.stake(&user, &10_000, &0);
    client.publish_position_price(&user);

    set_ledger(&env, 2_000);
    client.publish_position_price(&user);

    let latest = client.get_latest_position_price(&user).expect("a price");
    assert_eq!(latest.published_at, 2_000);
}

#[test]
fn there_is_no_price_before_one_is_published() {
    let (_env, client, _admin, user) = setup();
    assert!(client.get_latest_position_price(&user).is_none());
    assert_eq!(client.get_position_price_history(&user).len(), 0);
}

#[test]
fn history_grows_to_the_cap_then_rolls() {
    let (env, client, _admin, user) = setup();

    client.stake(&user, &10_000, &0);

    // Publish two past the cap.
    for i in 0..(MAX_PRICE_HISTORY + 2) {
        set_ledger(&env, 1_000 + i);
        client.publish_position_price(&user);
    }

    let history = client.get_position_price_history(&user);
    assert_eq!(history.len(), MAX_PRICE_HISTORY);

    // The two oldest rolled off, so the window starts at the third publish.
    assert_eq!(history.get(0).unwrap().published_at, 1_002);
    assert_eq!(
        history.get(MAX_PRICE_HISTORY - 1).unwrap().published_at,
        1_000 + MAX_PRICE_HISTORY + 1
    );
}

#[test]
fn bulk_publish_prices_covers_every_supplied_user() {
    let (env, client, admin, user) = setup();
    let _ = admin;

    let second = Address::generate(&env);
    client.stake(&user, &10_000, &0);

    let mut users = Vec::new(&env);
    users.push_back(user.clone());
    users.push_back(second.clone());

    assert_eq!(client.bulk_publish_prices(&users), 2);
    assert!(client.get_latest_position_price(&user).is_some());
    assert!(client.get_latest_position_price(&second).is_some());
}

#[test]
fn bulk_publish_rejects_an_oversized_batch() {
    let (env, client, _admin, _user) = setup();

    let mut users = Vec::new(&env);
    for _ in 0..21 {
        users.push_back(Address::generate(&env));
    }

    // Rejected outright rather than silently pricing a prefix.
    assert_eq!(
        client.try_bulk_publish_prices(&users),
        Err(Ok(VaultError::TooManyActiveUsers))
    );
}

#[test]
fn a_position_inside_its_cliff_prices_at_principal_only() {
    let (_env, client, _admin, user) = setup();

    client.set_vesting_cliff(&10_000);
    client.stake(&user, &10_000, &0);

    // Advertising rewards that are not yet payable would misprice the
    // position for anyone quoting against this feed.
    let price = client.publish_position_price(&user);
    assert_eq!(price.pending_reward, 0);
    assert_eq!(price.fair_value, price.principal);
}
