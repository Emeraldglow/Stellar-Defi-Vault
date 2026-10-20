#![cfg(test)]
//! Tests for community-voted slash disputes (issue #336).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env, String, Symbol, TryFromVal,
};

use crate::{
    balance,
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
    admin: Address,
    treasury: Address,
    slashed_user: Address,
    voter: Address,
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
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
        let treasury = Address::generate(&env);
        let slashed_user = Address::generate(&env);
        let voter = Address::generate(&env);

        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_addr);
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);
        token_admin.mint(&vault_id, &1_000_000);

        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            treasury,
            slashed_user,
            voter,
            token,
            token_admin,
        }
    }

    fn seed_voter_position(&self, user: &Address, shares: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_shares(&self.env, user, shares);
            balance::set_total_shares(&self.env, shares.max(balance::get_total_shares(&self.env)));
            balance::set_total_deposited(&self.env, shares.max(balance::get_total_deposited(&self.env)));
        });
    }
}

#[test]
fn valid_dispute_can_be_filed() {
    let f = Fixture::new();
    f.vault.record_slash(&f.slashed_user, &1, &500);

    let dispute_id = f.vault.dispute_slash(
        &f.slashed_user,
        &1,
        &String::from_str(&f.env, "evidence was fabricated"),
    );

    let dispute = f.vault.get_dispute(&dispute_id).unwrap();
    assert_eq!(dispute.slash_id, 1);
    assert_eq!(dispute.disputer, f.slashed_user);
    assert!(!dispute.resolved);
}

#[test]
fn only_the_slashed_user_can_dispute() {
    let f = Fixture::new();
    f.vault.record_slash(&f.slashed_user, &1, &500);

    let result = f.vault.try_dispute_slash(
        &f.voter,
        &1,
        &String::from_str(&f.env, "not me"),
    );
    assert!(result.is_err());
}

#[test]
fn dispute_window_is_enforced() {
    let f = Fixture::new();
    f.vault.set_slash_dispute_window(&100);
    f.vault.record_slash(&f.slashed_user, &1, &500);

    set_ledger(&f.env, 1_200); // past the 100-ledger window
    let result = f.vault.try_dispute_slash(
        &f.slashed_user,
        &1,
        &String::from_str(&f.env, "too late"),
    );
    assert!(result.is_err());
}

#[test]
fn vote_tallied_correctly_and_overturn_resolves_without_moving_funds() {
    let f = Fixture::new();
    f.vault.set_slash_dispute_window(&1_000);
    f.vault.record_slash(&f.slashed_user, &1, &500);
    let dispute_id = f.vault.dispute_slash(
        &f.slashed_user,
        &1,
        &String::from_str(&f.env, "reason"),
    );

    f.seed_voter_position(&f.voter, 10_000);
    f.vault.vote_on_dispute(&f.voter, &dispute_id, &true); // overturn

    let dispute = f.vault.get_dispute(&dispute_id).unwrap();
    assert_eq!(dispute.votes_overturn, 10_000);
    assert_eq!(dispute.votes_uphold, 0);

    set_ledger(&f.env, 2_500); // past the deadline
    f.vault.resolve_dispute(&dispute_id);

    let resolved = f.vault.get_dispute(&dispute_id).unwrap();
    assert!(resolved.resolved);
    // Overturned: no transfer to treasury.
    assert_eq!(f.token.balance(&f.treasury), 0);
}

#[test]
fn uphold_outcome_sends_disputed_amount_to_treasury() {
    let f = Fixture::new();
    f.vault.set_slash_dispute_window(&1_000);
    f.env.as_contract(&f.vault_id, || {
        balance::set_slash_treasury(&f.env, &f.treasury);
    });
    f.vault.record_slash(&f.slashed_user, &1, &500);
    let dispute_id = f.vault.dispute_slash(
        &f.slashed_user,
        &1,
        &String::from_str(&f.env, "reason"),
    );

    f.seed_voter_position(&f.voter, 10_000);
    f.vault.vote_on_dispute(&f.voter, &dispute_id, &false); // uphold

    set_ledger(&f.env, 2_500);
    f.vault.resolve_dispute(&dispute_id);

    assert_eq!(f.token.balance(&f.treasury), 500);
}

#[test]
fn max_five_open_disputes_pool_wide() {
    let f = Fixture::new();
    f.vault.set_slash_dispute_window(&10_000);
    let users: std::vec::Vec<Address> = (0..6).map(|_| Address::generate(&f.env)).collect();

    for (i, user) in users.iter().enumerate().take(5) {
        f.vault.record_slash(user, &(i as u32), &100);
        f.vault.dispute_slash(user, &(i as u32), &String::from_str(&f.env, "r"));
    }

    // 6th dispute should be rejected — 5 already open.
    let sixth = &users[5];
    f.vault.record_slash(sixth, &5, &100);
    let result = f.vault.try_dispute_slash(sixth, &5, &String::from_str(&f.env, "r"));
    assert!(result.is_err());
}

#[test]
fn dispute_filed_event_emitted() {
    let f = Fixture::new();
    f.vault.record_slash(&f.slashed_user, &1, &500);
    f.vault.dispute_slash(&f.slashed_user, &1, &String::from_str(&f.env, "reason"));

    let events = f.env.events().all();
    let found = events.iter().any(|(topics, _data)| match topics.get(0) {
        Some(val) => Symbol::try_from_val(&f.env, &val)
            .map(|t| t == Symbol::new(&f.env, "sd_filed"))
            .unwrap_or(false),
        None => false,
    });
    assert!(found, "expected dispute_filed event");
    let _ = f.admin;
    let _ = f.token_admin;
}
