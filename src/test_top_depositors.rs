#![cfg(test)]
//! Tests for issue #523: `get_top_depositors` leaderboard query.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env, Vec,
};

use crate::balance;
use crate::vault::{VaultContract, VaultContractClient, MAX_TOP_DEPOSITORS};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
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
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        Fixture {
            env,
            vault,
            vault_id,
            token_admin,
        }
    }

    /// Mints `amount` to a fresh address and stakes all of it, so the pool
    /// records a position of that size.
    fn depositor(&self, amount: i128) -> Address {
        let user = Address::generate(&self.env);
        self.token_admin.mint(&user, &amount);
        self.vault.deposit(&user, &amount, &0);
        user
    }

    /// Overwrites the pool's share/deposit counters to move the share price
    /// off 1:1 (tests only), so position sizes stop matching share balances.
    fn seed_share_price(&self, total_shares: i128, total_deposited: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_total_shares(&self.env, total_shares);
            balance::set_total_deposited(&self.env, total_deposited);
        });
    }
}

/// Unwraps a leaderboard response into a plain `std::vec::Vec` so tests can
/// compare it against a literal.
fn entries(rows: &Vec<(Address, i128)>) -> std::vec::Vec<(Address, i128)> {
    let mut out = std::vec::Vec::new();
    for i in 0..rows.len() {
        out.push(rows.get(i).unwrap());
    }
    out
}

#[test]
fn returns_top_depositors_sorted_descending() {
    let f = Fixture::new();
    let alice = f.depositor(100);
    let bob = f.depositor(300);
    let carol = f.depositor(200);

    let top = entries(&f.vault.get_top_depositors(&2_u32));
    assert_eq!(
        top,
        std::vec![(bob.clone(), 300), (carol.clone(), 200)],
        "top 2 must be the two largest positions, highest first"
    );

    // Widening the limit appends the next largest, still descending.
    let all = entries(&f.vault.get_top_depositors(&3_u32));
    assert_eq!(all, std::vec![(bob, 300), (carol, 200), (alice, 100)]);
}

#[test]
fn equal_sizes_keep_registration_order() {
    let f = Fixture::new();
    let alice = f.depositor(100);
    let bob = f.depositor(100);

    let top = entries(&f.vault.get_top_depositors(&2_u32));
    assert_eq!(top, std::vec![(alice, 100), (bob, 100)]);
}

#[test]
fn limit_above_depositor_count_returns_every_depositor() {
    let f = Fixture::new();
    let alice = f.depositor(100);
    let bob = f.depositor(300);
    let carol = f.depositor(200);

    let top = entries(&f.vault.get_top_depositors(&50_u32));
    assert_eq!(top, std::vec![(bob, 300), (carol, 200), (alice, 100)]);
}

#[test]
fn zero_limit_returns_empty() {
    let f = Fixture::new();
    f.depositor(100);
    f.depositor(300);

    assert!(f.vault.get_top_depositors(&0_u32).is_empty());
}

#[test]
fn empty_pool_returns_empty() {
    let f = Fixture::new();
    assert!(f.vault.get_top_depositors(&10_u32).is_empty());
}

#[test]
fn limit_is_capped_at_fifty_rows() {
    let f = Fixture::new();
    // 55 depositors sized 10..=550, so the true top 50 runs 550 down to 60.
    for i in 1..=55 {
        f.depositor(i * 10);
    }

    let rows = f.vault.get_top_depositors(&u32::MAX);
    assert_eq!(rows.len(), MAX_TOP_DEPOSITORS);
    assert_eq!(rows.get(0).unwrap().1, 550);
    // The five smallest depositors fell off the bottom of the cap.
    assert_eq!(rows.get(MAX_TOP_DEPOSITORS - 1).unwrap().1, 60);

    let top = entries(&rows);
    for pair in top.windows(2) {
        assert!(pair[0].1 >= pair[1].1, "rows must stay sorted descending");
    }
}

#[test]
fn ranks_by_position_size_not_raw_shares() {
    let f = Fixture::new();
    let alice = f.depositor(100);
    let bob = f.depositor(300);
    // 100 shares outstanding against 1,000 deposited: each share is worth 10
    // stake-token units, so share balances alone would understate the sizes.
    f.seed_share_price(100, 1_000);

    assert_eq!(f.vault.shares_of(&alice), 100);
    assert_eq!(f.vault.shares_of(&bob), 300);

    let top = entries(&f.vault.get_top_depositors(&2_u32));
    assert_eq!(top, std::vec![(bob, 3_000), (alice, 1_000)]);
}

#[test]
fn exited_positions_are_skipped() {
    let f = Fixture::new();
    let alice = f.depositor(100);
    let bob = f.depositor(300);
    let carol = f.depositor(200);

    f.vault.unstake_all(&bob);
    assert_eq!(f.vault.shares_of(&bob), 0);

    let top = entries(&f.vault.get_top_depositors(&10_u32));
    assert_eq!(
        top,
        std::vec![(carol, 200), (alice, 100)],
        "a fully unstaked depositor drops off the board"
    );
}

#[test]
fn query_needs_no_auth_and_changes_no_state() {
    let f = Fixture::new();
    f.depositor(100);
    let bob = f.depositor(300);

    let before = f.vault.get_pool_summary();
    let rows = f.vault.get_top_depositors(&5_u32);
    let after = f.vault.get_pool_summary();

    assert_eq!(rows.len(), 2);
    assert_eq!(f.env.auths().len(), 0, "leaderboard must not require auth");
    assert_eq!(f.vault.shares_of(&bob), 300);
    assert_eq!(
        (before.total_deposited, before.depositor_count),
        (after.total_deposited, after.depositor_count),
        "read-only query must not touch pool accounting"
    );
}
