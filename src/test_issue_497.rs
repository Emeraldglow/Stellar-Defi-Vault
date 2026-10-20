#![cfg(test)]
//! Tests for issue #497: break-glass principal withdrawal while paused.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _}, token, Address, Env, IntoVal, String, Symbol, Val,
};

use crate::{balance, errors::VaultError, storage::PauseReason, vault::{VaultContract, VaultContractClient}};

struct Fixture<'a> {
    env: Env,
    client: VaultContractClient<'a>,
    vault_id: Address,
    admin: Address,
    alice: Address,
    token: token::Client<'a>,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();
        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_id);
        let token_admin = token::StellarAssetClient::new(&env, &token_id);
        let vault_id = env.register_contract(None, VaultContract);
        let client = VaultContractClient::new(&env, &vault_id);
        client.initialize(&admin, &token_id, &0_u32, &None, &None);
        token_admin.mint(&alice, &1_000);
        Self { env, client, vault_id, admin, alice, token }
    }

    fn pause(&self) {
        self.client.pause(
            &PauseReason::Other,
            &String::from_str(&self.env, "Emergency test pause"),
        );
    }
}

#[test]
fn paused_emergency_withdraw_returns_only_principal_and_forfeits_rewards() {
    let f = Fixture::new();
    f.client.stake(&f.alice, &100);
    f.env.as_contract(&f.vault_id, || balance::set_accrued_reward(&f.env, &f.alice, 27));
    f.pause();

    assert_eq!(f.client.emergency_withdraw(&f.alice), 100);
    assert_eq!(f.token.balance(&f.alice), 1_000);
    assert_eq!(f.env.as_contract(&f.vault_id, || balance::get_shares(&f.env, &f.alice)), 0);
    assert_eq!(f.env.as_contract(&f.vault_id, || balance::get_total_shares(&f.env)), 0);
    assert_eq!(f.env.as_contract(&f.vault_id, || balance::get_total_deposited(&f.env)), 0);
    assert_eq!(f.env.as_contract(&f.vault_id, || balance::get_accrued_reward(&f.env, &f.alice)), 0);
    let found = f.env.events().all().iter().any(|(_, topics, data)| {
        let topic: Val = topics.get(0).unwrap().into_val(&f.env);
        let payload: Val = (100_i128, 27_i128, f.env.ledger().sequence()).into_val(&f.env);
        topic == Val::from(Symbol::new(&f.env, "emg_wdraw")) && data == payload
    });
    assert!(found, "emergency withdrawal should report principal, forfeited rewards, and ledger");
}

#[test]
fn emergency_withdraw_reverts_while_unpaused() {
    let f = Fixture::new();
    f.client.stake(&f.alice, &100);
    assert_eq!(f.client.try_emergency_withdraw(&f.alice), Err(Ok(VaultError::VaultPaused)));
}

#[test]
fn emergency_withdraw_reverts_for_zero_position() {
    let f = Fixture::new();
    f.pause();
    assert_eq!(f.client.try_emergency_withdraw(&f.alice), Err(Ok(VaultError::PositionNotFound)));
}
