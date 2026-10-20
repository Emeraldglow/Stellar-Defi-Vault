#![cfg(test)]

use soroban_sdk::{
    testutils::{Events, Ledger as _},
    token, Address, Env, Symbol, TryFromVal,
};

use crate::vault::{VaultContract, VaultContractClient};

struct Fixture<'a> {
    env: Env,
    vault_id: Address,
    vault: VaultContractClient<'a>,
    token: token::Client<'a>,
    admin: Address,
    alice: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        set_ledger(&env, 100);

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_addr);
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);
        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);

        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);
        token_admin.mint(&vault_id, &1_000_000);

        Self {
            env,
            vault_id,
            vault,
            token,
            admin,
            alice,
        }
    }

    fn set_accrued_reward(&self, amount: i128) {
        self.env.as_contract(&self.vault_id, || {
            crate::balance::set_accrued_reward(&self.env, &self.alice, amount);
        });
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
fn large_claim_is_queued_with_delay_in_range() {
    let f = Fixture::new();
    f.vault.set_mev_protection_threshold(&f.admin, &10_000);
    f.set_accrued_reward(25_000);

    let before = f.token.balance(&f.alice);
    let claimed = f.vault.claim(&f.alice);
    let pending = f.vault.get_pending_claim(&f.alice).unwrap();

    assert_eq!(claimed, 0);
    assert_eq!(f.token.balance(&f.alice), before);
    assert_eq!(pending.amount, 25_000);
    assert!(pending.executable_at >= 101);
    assert!(pending.executable_at <= 110);
    assert!(has_event(&f.env, "claim_queued"));
}

#[test]
fn small_claim_executes_immediately() {
    let f = Fixture::new();
    f.vault.set_mev_protection_threshold(&f.admin, &10_000);
    f.set_accrued_reward(9_999);

    let before = f.token.balance(&f.alice);
    let claimed = f.vault.claim(&f.alice);

    assert_eq!(claimed, 9_999);
    assert_eq!(f.token.balance(&f.alice), before + 9_999);
    assert!(f.vault.get_pending_claim(&f.alice).is_none());
}

#[test]
fn execute_before_delay_reverts() {
    let f = Fixture::new();
    f.vault.set_mev_protection_threshold(&f.admin, &10_000);
    f.set_accrued_reward(20_000);
    f.vault.claim(&f.alice);

    assert!(f.vault.try_execute_pending_claim(&f.alice).is_err());
    assert_eq!(f.token.balance(&f.alice), 0);
}

#[test]
fn execute_after_delay_transfers_pending_claim() {
    let f = Fixture::new();
    f.vault.set_mev_protection_threshold(&f.admin, &10_000);
    f.set_accrued_reward(20_000);
    f.vault.claim(&f.alice);
    let pending = f.vault.get_pending_claim(&f.alice).unwrap();

    set_ledger(&f.env, pending.executable_at);
    let paid = f.vault.execute_pending_claim(&f.alice);

    assert_eq!(paid, 20_000);
    assert_eq!(f.token.balance(&f.alice), 20_000);
    assert!(f.vault.get_pending_claim(&f.alice).is_none());
    assert!(has_event(&f.env, "claim_executed"));
}

#[test]
fn cancel_pending_claim_restores_accrued_reward() {
    let f = Fixture::new();
    f.vault.set_mev_protection_threshold(&f.admin, &10_000);
    f.set_accrued_reward(20_000);
    f.vault.claim(&f.alice);

    f.vault.cancel_pending_claim(&f.alice);

    assert!(f.vault.get_pending_claim(&f.alice).is_none());
    assert_eq!(f.token.balance(&f.alice), 0);
    f.vault.set_mev_protection_threshold(&f.admin, &0);
    assert_eq!(f.vault.claim(&f.alice), 20_000);
}
