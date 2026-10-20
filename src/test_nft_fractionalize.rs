#![cfg(test)]
//! Tests for NFT receipt fractionalization.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::{
    errors::VaultError,
    nft_fractionalize::{MAX_FRACTIONS, MIN_FRACTIONS},
    vault::{VaultContract, VaultContractClient},
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
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
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
        let token = token::Client::new(&env, &token_addr);
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
            token,
            token_admin,
            admin,
            alice,
            bob,
        }
    }
}

// ── fractionalize_nft ────────────────────────────────────────────────────────

#[test]
fn fractionalize_locks_position() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);

    assert!(f.vault.is_position_fractionalized(&f.alice));
}

#[test]
fn fractionalize_rejects_non_staker() {
    let f = Fixture::new();

    let result = f.vault.try_fractionalize_nft(&f.alice, &10);
    assert_eq!(result, Err(Ok(VaultError::PositionNotFound)));
}

#[test]
fn fractionalize_below_min_fractions_rejected() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    let result = f.vault.try_fractionalize_nft(&f.alice, &(MIN_FRACTIONS - 1));
    assert_eq!(result, Err(Ok(VaultError::InvalidRate)));
}

#[test]
fn fractionalize_above_max_fractions_rejected() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    let result = f.vault.try_fractionalize_nft(&f.alice, &(MAX_FRACTIONS + 1));
    assert_eq!(result, Err(Ok(VaultError::InvalidRate)));
}

#[test]
fn fractionalize_min_and_max_valid() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &MIN_FRACTIONS);
    assert!(f.vault.is_position_fractionalized(&f.alice));

    // Reconstruct to try max.
    f.vault.reconstruct_nft(&f.alice);

    f.vault.fractionalize_nft(&f.alice, &MAX_FRACTIONS);
    assert!(f.vault.is_position_fractionalized(&f.alice));
}

#[test]
fn double_fractionalize_rejected() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);

    let result = f.vault.try_fractionalize_nft(&f.alice, &10);
    assert_eq!(result, Err(Ok(VaultError::AlreadyInitialized)));
}

#[test]
fn non_owner_cannot_fractionalize() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    // bob tries to fractionalize alice's NFT — bob has no position, so
    // PositionNotFound is returned.
    let result = f.vault.try_fractionalize_nft(&f.bob, &10);
    assert_eq!(result, Err(Ok(VaultError::PositionNotFound)));
}

#[test]
fn fractionalize_gives_owner_all_fractions() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &100);

    assert_eq!(f.vault.get_fraction_balance(&f.alice, &f.alice), 100);
}

// ── position lock prevents unstake ───────────────────────────────────────────

#[test]
fn fractionalized_position_cannot_unstake() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);

    // Unstake should be blocked.
    let result = f.vault.try_unstake(&f.alice, &1_000);
    assert_eq!(result, Err(Ok(VaultError::ContractStopped)));
}

#[test]
fn reconstructed_position_can_unstake() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);
    f.vault.reconstruct_nft(&f.alice);

    // Unstake should work now.
    f.vault.unstake(&f.alice, &1_000);
}

// ── reconstruct_nft ──────────────────────────────────────────────────────────

#[test]
fn reconstruct_restores_position() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);
    assert!(f.vault.is_position_fractionalized(&f.alice));

    f.vault.reconstruct_nft(&f.alice);
    assert!(!f.vault.is_position_fractionalized(&f.alice));
}

#[test]
fn reconstruct_with_partial_fractions_fails() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);

    // Transfer some fractions to bob.
    f.vault.transfer_fractions(&f.alice, &f.bob, &3);

    // Reconstruction should fail — not all fractions returned.
    let result = f.vault.try_reconstruct_nft(&f.alice);
    assert_eq!(result, Err(Ok(VaultError::InsufficientShares)));
}

#[test]
fn reconstruct_after_returning_all_fractions() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);
    f.vault.transfer_fractions(&f.alice, &f.bob, &5);

    // Bob returns fractions.
    f.vault.transfer_fractions(&f.alice, &f.alice, &5);

    f.vault.reconstruct_nft(&f.alice);
    assert!(!f.vault.is_position_fractionalized(&f.alice));
}

#[test]
fn reconstruct_when_not_fractionalized_fails() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    let result = f.vault.try_reconstruct_nft(&f.alice);
    assert_eq!(result, Err(Ok(VaultError::PositionNotFound)));
}

// ── transfer_fractions ───────────────────────────────────────────────────────

#[test]
fn transfer_fractions_updates_balances() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &100);
    f.vault.transfer_fractions(&f.alice, &f.bob, &30);

    assert_eq!(f.vault.get_fraction_balance(&f.alice, &f.alice), 70);
    assert_eq!(f.vault.get_fraction_balance(&f.alice, &f.bob), 30);
}

#[test]
fn transfer_more_than_balance_fails() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);

    let result = f.vault.try_transfer_fractions(&f.alice, &f.bob, &11);
    assert_eq!(result, Err(Ok(VaultError::InsufficientShares)));
}

#[test]
fn zero_transfer_rejected() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);

    let result = f.vault.try_transfer_fractions(&f.alice, &f.bob, &0);
    assert_eq!(result, Err(Ok(VaultError::ZeroAmount)));
}

// ── read-only queries ────────────────────────────────────────────────────────

#[test]
fn is_position_fractionalized_false_initially() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    assert!(!f.vault.is_position_fractionalized(&f.alice));
}

#[test]
fn get_fraction_balance_zero_for_non_holder() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);

    assert_eq!(f.vault.get_fraction_balance(&f.alice, &f.bob), 0);
}

#[test]
fn get_fraction_holders_returns_owner() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &10);

    let holders = f.vault.get_fraction_holders(&f.alice);
    assert_eq!(holders.len(), 1);
    assert_eq!(holders.get(0).unwrap(), f.alice);
}

#[test]
fn get_fraction_holders_after_transfer() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &5_000, &0);

    f.vault.fractionalize_nft(&f.alice, &100);
    f.vault.transfer_fractions(&f.alice, &f.bob, &25);

    let holders = f.vault.get_fraction_holders(&f.alice);
    assert_eq!(holders.len(), 2);
}
