#![cfg(test)]
//! Tests for burn-and-redeem NFT-triggered position exit (issue #410).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::{
    balance,
    errors::VaultError,
    nft::{StakeReceiptNFT, StakeReceiptNFTClient},
    storage::DataKey,
    vault::{VaultContract, VaultContractClient},
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
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
    nft: StakeReceiptNFTClient<'a>,
    alice: Address,
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

        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_addr);
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        // Fund the contract so redemptions can be paid out.
        token_admin.mint(&vault_id, &10_000_000);

        let nft_id = env.register_contract(None, StakeReceiptNFT);
        let nft = StakeReceiptNFTClient::new(&env, &nft_id);
        nft.initialize(&vault_id);
        vault.set_nft_contract(&nft_id);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            token,
            token_admin,
            nft,
            alice,
        }
    }

    /// Seed a staker's position and mint the matching receipt directly,
    /// bypassing `stake()` (which doesn't mint a receipt on this branch).
    fn seed_position(&self, user: &Address, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            let total_shares = balance::get_total_shares(&self.env);
            let total_deposited = balance::get_total_deposited(&self.env);
            balance::set_shares(&self.env, user, total_shares + amount);
            balance::set_total_shares(&self.env, total_shares + amount);
            balance::set_total_deposited(&self.env, total_deposited + amount);
            self.env
                .storage()
                .persistent()
                .set(&DataKey::StakedAtLedger(user.clone()), &self.env.ledger().sequence());
        });
        self.nft.mint(user, &self.vault_id, &amount, &self.env.ledger().sequence());
    }
}

#[test]
fn burn_and_redeem_returns_principal_and_burns_receipt() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 100_000);
    assert!(f.nft.has_receipt(&f.alice));

    let balance_before = f.token.balance(&f.alice);
    let returned = f.vault.burn_and_redeem(&f.alice);

    assert_eq!(returned, 100_000);
    assert_eq!(f.token.balance(&f.alice), balance_before + 100_000);
    assert!(!f.nft.has_receipt(&f.alice));
    assert_eq!(f.vault.shares_of(&f.alice), 0);
}

#[test]
fn burn_and_redeem_settles_pending_reward_first() {
    let f = Fixture::new();
    f.seed_position(&f.alice, 100_000);
    f.env.as_contract(&f.vault_id, || {
        balance::set_accrued_reward(&f.env, &f.alice, 5_000);
    });

    let balance_before = f.token.balance(&f.alice);
    let returned = f.vault.burn_and_redeem(&f.alice);

    assert_eq!(returned, 100_000);
    assert_eq!(f.token.balance(&f.alice), balance_before + 105_000);
    f.env.as_contract(&f.vault_id, || {
        assert_eq!(balance::get_accrued_reward(&f.env, &f.alice), 0);
    });
}

#[test]
fn burn_and_redeem_rejects_caller_with_no_receipt() {
    let f = Fixture::new();
    let result = f.vault.try_burn_and_redeem(&f.alice);
    assert_eq!(result, Err(Ok(VaultError::PositionNotFound)));
}

#[test]
fn burn_and_redeem_blocked_while_lock_up_active() {
    let f = Fixture::new();
    f.env.as_contract(&f.vault_id, || {
        f.env
            .storage()
            .instance()
            .set(&DataKey::LockPeriod, &1_000_u32);
    });
    f.seed_position(&f.alice, 100_000);

    let result = f.vault.try_burn_and_redeem(&f.alice);
    assert_eq!(result, Err(Ok(VaultError::InvalidRate)));
    assert!(f.nft.has_receipt(&f.alice));

    set_ledger(&f.env, 1_000 + 1_000 + 1);
    let returned = f.vault.burn_and_redeem(&f.alice);
    assert_eq!(returned, 100_000);
}

