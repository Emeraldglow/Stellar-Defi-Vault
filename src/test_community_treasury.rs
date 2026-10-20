#![cfg(test)]

use soroban_sdk::{
    testutils::{Events, Ledger as _},
    token, Address, Env, String, Symbol, TryFromVal,
};

use crate::vault::{VaultContract, VaultContractClient};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    token: token::Client<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
    recipient: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        set_ledger(&env, 1);

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_addr);
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);
        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);

        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
        token_admin.mint(&alice, &2_000_000);
        token_admin.mint(&bob, &2_000_000);

        Self {
            env,
            vault,
            token,
            admin,
            alice,
            bob,
            recipient,
        }
    }

    fn fund_treasury_from_fee(&self, user: &Address, stake: i128, withdraw: i128, bps: u32) {
        self.vault.set_treasury_contribution_bps(&self.admin, &bps);
        self.vault.set_unstake_fee_bps(&self.admin, &500);
        self.vault.deposit(user, &stake, &0);
        self.vault.withdraw(user, &withdraw);
    }
}

fn set_ledger(env: &Env, sequence: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = sequence;
        li.min_persistent_entry_ttl = 10_000_000;
        li.max_entry_ttl = 10_000_000;
    });
}

fn has_event(env: &Env, event_name: &str) -> bool {
    env.events().all().iter().any(|(_, topics, _)| {
        topics.iter().any(|topic| {
            Symbol::try_from_val(env, &topic)
                .map(|symbol| symbol == Symbol::new(env, event_name))
                .unwrap_or(false)
        })
    })
}

#[test]
fn treasury_funded_from_protocol_fees() {
    let f = Fixture::new();

    f.fund_treasury_from_fee(&f.alice, 600_000, 300_000, 5_000);

    assert_eq!(f.vault.get_community_treasury_balance(), 7_500);
    assert_eq!(f.vault.get_reward_pool_balance(), 7_500);
}

#[test]
fn proposal_passes_and_pays_recipient() {
    let f = Fixture::new();
    f.fund_treasury_from_fee(&f.alice, 700_000, 300_000, 10_000);
    f.vault.deposit(&f.bob, &100_000, &0);

    let proposal_id = f.vault.propose_spending(
        &f.alice,
        &f.recipient,
        &10_000,
        &String::from_str(&f.env, "audit"),
        &10,
    );
    f.vault.vote_spending(&f.alice, &proposal_id, &true);
    f.vault.vote_spending(&f.bob, &proposal_id, &false);

    let before = f.token.balance(&f.recipient);
    set_ledger(&f.env, 12);
    f.vault.execute_spending(&proposal_id);

    assert_eq!(f.token.balance(&f.recipient), before + 10_000);
    assert_eq!(f.vault.get_community_treasury_balance(), 5_000);
    assert!(
        f.vault
            .get_spending_proposal(&proposal_id)
            .unwrap()
            .executed
    );
    assert!(has_event(&f.env, "spending_executed"));
}

#[test]
fn failed_proposal_is_blocked() {
    let f = Fixture::new();
    f.fund_treasury_from_fee(&f.alice, 700_000, 300_000, 10_000);
    f.vault.deposit(&f.bob, &800_000, &0);

    let proposal_id = f.vault.propose_spending(
        &f.alice,
        &f.recipient,
        &10_000,
        &String::from_str(&f.env, "marketing"),
        &10,
    );
    f.vault.vote_spending(&f.alice, &proposal_id, &true);
    f.vault.vote_spending(&f.bob, &proposal_id, &false);

    set_ledger(&f.env, 12);
    assert!(f.vault.try_execute_spending(&proposal_id).is_err());
    assert_eq!(f.token.balance(&f.recipient), 0);
    assert_eq!(f.vault.get_community_treasury_balance(), 15_000);
}

#[test]
fn insufficient_treasury_balance_reverts_execution() {
    let f = Fixture::new();
    f.vault.deposit(&f.alice, &100_000, &0);

    let proposal_id = f.vault.propose_spending(
        &f.alice,
        &f.recipient,
        &1_000,
        &String::from_str(&f.env, "charity"),
        &5,
    );
    f.vault.vote_spending(&f.alice, &proposal_id, &true);

    set_ledger(&f.env, 7);
    assert!(f.vault.try_execute_spending(&proposal_id).is_err());
    assert_eq!(f.token.balance(&f.recipient), 0);
}
