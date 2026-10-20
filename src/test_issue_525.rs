#![cfg(test)]
//! Tests for issue #525: graceful pool sunset via `initiate_sunset()`.
//!
//! Covers the four behaviours the issue asks for: new deposits are blocked the
//! moment a sunset starts, withdrawals stay fully open and fee-free for the
//! whole window, `get_sunset_status()` reports the deadline only while a sunset
//! is in effect, and `sunset_initiated` is emitted.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    token, vec, Address, Env, IntoVal, Symbol, Val,
};

use crate::balance;
use crate::errors::VaultError;
use crate::vault::{VaultContract, VaultContractClient};

/// Moves the ledger to `sequence`, mirroring the helper the other suites use.
fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    admin: Address,
    alice: Address,
    bob: Address,
    stake_addr: Address,
    stake: token::Client<'a>,
    stake_admin: token::StellarAssetClient<'a>,
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
            li.sequence_number = 1_000;
        });

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let stake_addr = env.register_stellar_asset_contract(admin.clone());
        let stake = token::Client::new(&env, &stake_addr);
        let stake_admin = token::StellarAssetClient::new(&env, &stake_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &stake_addr, &0_u32, &None, &None);

        stake_admin.mint(&alice, &1_000_000);
        stake_admin.mint(&bob, &1_000_000);

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            alice,
            bob,
            stake_addr,
            stake,
            stake_admin,
        }
    }

    fn protocol_fees_collected(&self) -> i128 {
        self.env.as_contract(&self.vault_id, || {
            balance::get_protocol_fee_collected(&self.env)
        })
    }

    /// Charges the maximum unstake fee, so a waived fee is unmistakable.
    fn charge_max_unstake_fee(&self) {
        self.vault.set_unstake_fee_bps(&self.admin, &500);
    }

    fn stake_as(&self, user: &Address, amount: i128) -> i128 {
        self.vault.stake(user, &amount, &0)
    }
}

// ── Deposits are blocked once the sunset starts ──────────────────────────────

#[test]
fn deposits_are_rejected_from_the_moment_the_sunset_starts() {
    let f = Fixture::new();
    f.stake_as(&f.alice, 1_000);

    set_ledger(&f.env, 2_000);
    f.vault.initiate_sunset(&f.admin, &5_000);

    assert_eq!(
        f.vault.try_stake(&f.bob, &500),
        Err(Ok(VaultError::PoolShuttingDown)),
        "a new deposit must be rejected while the pool is sunsetting"
    );
    assert_eq!(
        f.vault.try_deposit(&f.bob, &500, &None),
        Err(Ok(VaultError::PoolShuttingDown))
    );
    assert_eq!(
        f.vault.try_stake_and_claim(&f.bob, &500),
        Err(Ok(VaultError::PoolShuttingDown))
    );
    assert_eq!(
        f.vault
            .try_deposit_for_many(&f.bob, &vec![&f.env, f.bob.clone()], &vec![&f.env, 500i128]),
        Err(Ok(VaultError::PoolShuttingDown))
    );
}

#[test]
fn a_rejected_deposit_leaves_no_state_behind() {
    let f = Fixture::new();
    f.stake_as(&f.alice, 1_000);
    f.vault.initiate_sunset(&f.admin, &5_000);

    let bob_before = f.stake.balance(&f.bob);
    let vault_before = f.stake.balance(&f.vault.address);

    assert!(f.vault.try_stake(&f.bob, &500).is_err());

    assert_eq!(f.vault.shares_of(&f.bob), 0, "no position was opened");
    assert_eq!(
        f.stake.balance(&f.bob),
        bob_before,
        "no tokens left the user"
    );
    assert_eq!(
        f.stake.balance(&f.vault.address),
        vault_before,
        "no tokens arrived in the vault"
    );
}

#[test]
fn existing_stakers_can_still_exit_during_a_sunset() {
    let f = Fixture::new();
    f.stake_as(&f.alice, 1_000);
    f.vault.initiate_sunset(&f.admin, &5_000);

    set_ledger(&f.env, 3_000);
    let withdrawn = f.vault.unstake(&f.alice, &1_000);

    assert_eq!(withdrawn, 1_000);
    assert_eq!(f.vault.shares_of(&f.alice), 0);
    assert_eq!(
        f.stake.balance(&f.alice),
        1_000_000,
        "back to the full balance"
    );
}

#[test]
fn unstake_all_and_claims_stay_open_during_a_sunset() {
    let f = Fixture::new();
    f.stake_as(&f.alice, 1_000);
    f.vault.initiate_sunset(&f.admin, &5_000);

    // Nothing has accrued, so this claims 0 — the point is that it does not
    // revert on a sunsetting pool.
    assert_eq!(f.vault.claim(&f.alice), 0);

    let withdrawn = f.vault.unstake_all(&f.alice);
    assert_eq!(withdrawn, 1_000);
    assert_eq!(f.stake.balance(&f.alice), 1_000_000);
}

// ── Withdrawals are fee-free during the sunset ───────────────────────────────

#[test]
fn withdrawals_are_fee_free_during_a_sunset() {
    let f = Fixture::new();
    f.charge_max_unstake_fee();
    f.stake_as(&f.alice, 1_000);
    f.vault.initiate_sunset(&f.admin, &5_000);

    let alice_before = f.stake.balance(&f.alice);
    let withdrawn = f.vault.unstake(&f.alice, &1_000);

    assert_eq!(withdrawn, 1_000, "the whole position comes back, fee-free");
    assert_eq!(
        f.stake.balance(&f.alice) - alice_before,
        1_000,
        "no tokens were skimmed as a fee"
    );
    assert_eq!(
        f.protocol_fees_collected(),
        0,
        "a waived fee must not be booked as protocol revenue"
    );
}

#[test]
fn withdraw_to_is_fee_free_during_a_sunset() {
    let f = Fixture::new();
    f.charge_max_unstake_fee();
    f.stake_as(&f.alice, 1_000);
    f.vault.initiate_sunset(&f.admin, &5_000);

    let recipient = Address::generate(&f.env);
    f.vault.withdraw_to(&f.alice, &1_000, &recipient);

    assert_eq!(
        f.stake.balance(&recipient),
        1_000,
        "the recipient receives the full amount"
    );
    assert_eq!(f.protocol_fees_collected(), 0);
}

#[test]
fn the_unstake_fee_still_applies_before_any_sunset() {
    let f = Fixture::new();
    f.charge_max_unstake_fee();
    f.stake_as(&f.alice, 1_000);

    let alice_before = f.stake.balance(&f.alice);
    f.vault.unstake(&f.alice, &1_000);

    // Control case: without a sunset the 5% fee is still charged, so the
    // fee-free assertions above are measuring the sunset and not a missing fee.
    // `unstake` reports the gross position size, so measure the fee in the
    // tokens alice actually received and in the protocol-fee tally.
    assert_eq!(
        f.stake.balance(&f.alice) - alice_before,
        950,
        "50 of the 1000 position is kept as the unstake fee"
    );
    assert_eq!(f.protocol_fees_collected(), 50);
}

// ── Status query ─────────────────────────────────────────────────────────────

#[test]
fn status_is_none_until_a_sunset_is_initiated() {
    let f = Fixture::new();

    assert_eq!(
        f.vault.get_sunset_status(),
        None,
        "a pool that was never sunset has no deadline"
    );
}

#[test]
fn status_reports_the_deadline_and_stays_reported() {
    let f = Fixture::new();

    f.vault.initiate_sunset(&f.admin, &5_000);
    assert_eq!(f.vault.get_sunset_status(), Some(5_000));

    // Well past the deadline: the window has elapsed, but the sunset is one-way
    // so deposits stay blocked and the status must not flip back to None.
    set_ledger(&f.env, 99_000);
    assert_eq!(f.vault.get_sunset_status(), Some(5_000));
    assert_eq!(
        f.vault.try_stake(&f.bob, &500),
        Err(Ok(VaultError::PoolShuttingDown)),
        "an elapsed deadline must not silently reopen deposits"
    );
}

#[test]
fn only_the_admin_can_initiate_a_sunset() {
    let f = Fixture::new();
    let intruder = Address::generate(&f.env);

    assert_eq!(
        f.vault.try_initiate_sunset(&intruder, &5_000),
        Err(Ok(VaultError::Unauthorized))
    );
    assert_eq!(f.vault.get_sunset_status(), None);

    // An unauthorized attempt must not have blocked deposits either.
    f.stake_as(&f.bob, 100);
    assert_eq!(f.vault.shares_of(&f.bob), 100);
}

// ── Event ────────────────────────────────────────────────────────────────────

#[test]
fn initiating_a_sunset_emits_sunset_initiated() {
    let f = Fixture::new();

    f.vault.initiate_sunset(&f.admin, &5_000);

    let all = f.env.events().all();
    let found = all.iter().any(|(_, topics, _)| {
        let first: Symbol = topics.get(0).unwrap().into_val(&f.env);
        first == Symbol::new(&f.env, "snst_ini")
    });
    assert!(
        found,
        "the sunset_initiated event (topic `snst_ini`) must be published"
    );

    // A second sunset re-announces the window rather than being a no-op.
    let before = all.len();
    f.vault.initiate_sunset(&f.admin, &9_000);
    assert!(f.env.events().all().len() > before);
    assert_eq!(f.vault.get_sunset_status(), Some(9_000));
}
