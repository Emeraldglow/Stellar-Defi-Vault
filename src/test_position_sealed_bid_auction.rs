#![cfg(test)]
//! Tests for the sealed-bid position auction (issue #403).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Bytes, Env,
};

use crate::{
    balance,
    position_sealed_bid_auction::compute_bid_hash,
    crate::{VaultContract, VaultContractClient},
};

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
    });
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    token_client: token::Client<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
    carol: Address,
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
        let carol = Address::generate(&env);

        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);
        let token_client = token::Client::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        token_admin.mint(&bob, &10_000);
        token_admin.mint(&carol, &10_000);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            token_client,
            admin,
            alice,
            bob,
            carol,
        }
    }

    fn seed_position(&self, user: &Address, shares: i128, total_shares: i128, total_deposited: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_shares(&self.env, user, shares);
            balance::set_total_shares(&self.env, total_shares);
            balance::set_total_deposited(&self.env, total_deposited);
        });
    }

    fn bid_hash(&self, amount: i128, salt: &Bytes) -> Bytes {
        self.env
            .as_contract(&self.vault_id, || compute_bid_hash(&self.env, amount, salt))
    }
}

#[test]
fn highest_revealed_bid_wins_and_settlement_transfers_position() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);

    let auction_id = f.vault.list_position_for_auction(&f.alice, &100, &100, &100);

    let bob_salt = Bytes::from_array(&f.env, &[1u8; 8]);
    let carol_salt = Bytes::from_array(&f.env, &[2u8; 8]);
    let bob_hash = f.bid_hash(150, &bob_salt);
    let carol_hash = f.bid_hash(200, &carol_salt);

    f.vault.commit_bid(&f.bob, &auction_id, &bob_hash);
    f.vault.commit_bid(&f.carol, &auction_id, &carol_hash);

    set_ledger(&f.env, 1_101); // past commit_deadline (1_000 + 100)
    f.vault.reveal_bid(&f.bob, &auction_id, &150, &bob_salt);
    f.vault.reveal_bid(&f.carol, &auction_id, &200, &carol_salt);

    let listing = f.vault.get_auction(&auction_id).unwrap();
    assert_eq!(listing.highest_bid, 200);
    assert_eq!(listing.winner, Some(f.carol.clone()));

    set_ledger(&f.env, 1_201); // past reveal_deadline (1_100 + 100)
    f.vault.settle_auction(&auction_id);

    // Position transferred to the winner.
    assert_eq!(f.vault.shares_of(&f.carol), 1_000);
    assert_eq!(f.vault.shares_of(&f.alice), 0);
    // Winning bid paid to the seller.
    assert_eq!(f.token_client.balance(&f.alice), 200);

    let settled = f.vault.get_auction(&auction_id).unwrap();
    assert!(settled.settled);
}

#[test]
fn unrevealed_bid_forfeits_locked_funds() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    let auction_id = f.vault.list_position_for_auction(&f.alice, &100, &100, &100);

    let bob_salt = Bytes::from_array(&f.env, &[3u8; 8]);
    let bob_hash = f.bid_hash(150, &bob_salt);
    f.vault.commit_bid(&f.bob, &auction_id, &bob_hash);
    // Bob never reveals.

    let carol_salt = Bytes::from_array(&f.env, &[4u8; 8]);
    let carol_hash = f.bid_hash(200, &carol_salt);
    f.vault.commit_bid(&f.carol, &auction_id, &carol_hash);

    set_ledger(&f.env, 1_101);
    f.vault.reveal_bid(&f.carol, &auction_id, &200, &carol_salt);

    set_ledger(&f.env, 1_201);
    f.vault.settle_auction(&auction_id);

    let bob_balance_before_refund = f.token_client.balance(&f.bob);
    f.vault.refund_losing_bids(&auction_id);

    // Bob's locked min_bid (100) is forfeited to the slash treasury
    // (defaults to admin), not refunded to him.
    assert_eq!(f.token_client.balance(&f.bob), bob_balance_before_refund);
    assert_eq!(f.token_client.balance(&f.admin), 100);
}

#[test]
fn below_min_bid_reveal_is_ignored() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    let auction_id = f.vault.list_position_for_auction(&f.alice, &100, &100, &100);

    let bob_salt = Bytes::from_array(&f.env, &[5u8; 8]);
    // Bids below min_bid (100).
    let bob_hash = f.bid_hash(50, &bob_salt);
    f.vault.commit_bid(&f.bob, &auction_id, &bob_hash);

    set_ledger(&f.env, 1_101);
    f.vault.reveal_bid(&f.bob, &auction_id, &50, &bob_salt);

    let listing = f.vault.get_auction(&auction_id).unwrap();
    assert_eq!(listing.highest_bid, 0);
    assert_eq!(listing.winner, None);

    let bid = f.vault.get_auction_bid(&auction_id, &f.bob).unwrap();
    assert!(!bid.revealed);
}

#[test]
fn no_winner_returns_position_to_seller() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    let auction_id = f.vault.list_position_for_auction(&f.alice, &100, &100, &100);

    // No bids at all.
    set_ledger(&f.env, 1_201);
    f.vault.settle_auction(&auction_id);

    assert_eq!(f.vault.shares_of(&f.alice), 1_000);
}

#[test]
fn only_one_active_auction_per_seller() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 1_000, 1_000, 1_000);
    f.vault.list_position_for_auction(&f.alice, &100, &100, &100);

    let result = f
        .vault
        .try_list_position_for_auction(&f.alice, &100, &100, &100);
    assert!(result.is_err());
}

