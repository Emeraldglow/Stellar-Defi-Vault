#![cfg(test)]

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env};

use crate::vault::{VaultContract, VaultContractClient};

#[contract]
struct MockSixDecimalToken;

#[contractimpl]
impl MockSixDecimalToken {
    pub fn decimals(_env: Env) -> u32 {
        6
    }

    pub fn balance(env: Env, owner: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&(symbol_short!("bal"), owner))
            .unwrap_or(0)
    }

    pub fn mint(env: Env, recipient: Address, amount: i128) {
        let balance = Self::balance(env.clone(), recipient.clone());
        env.storage()
            .persistent()
            .set(&(symbol_short!("bal"), recipient), &(balance + amount));
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        let from_balance = Self::balance(env.clone(), from.clone());
        assert!(from_balance >= amount);
        let to_balance = Self::balance(env.clone(), to.clone());
        env.storage()
            .persistent()
            .set(&(symbol_short!("bal"), from), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&(symbol_short!("bal"), to), &(to_balance + amount));
    }
}

#[test]
fn share_price_uses_queried_non_default_token_decimals() {
    // A six-decimal token has 1_000_000 base units per token. Shares retain
    // the vault's fixed seven-decimal internal precision, so one token mints
    // 10_000_000 shares and preview_redeem returns the original base units.
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let token_id = env.register_contract(None, MockSixDecimalToken);
    let token = token::Client::new(&env, &token_id);
    let token_admin = MockSixDecimalTokenClient::new(&env, &token_id);
    token_admin.mint(&alice, &1_000_000);

    let vault_id = env.register_contract(None, VaultContract);
    let vault = VaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token_id, &0, &None, &None);
    assert_eq!(vault.stake_decimals(), 6);

    let shares = vault.deposit(&alice, &1_000_000);
    assert_eq!(shares, 10_000_000);
    assert_eq!(vault.preview_redeem(&shares), 1_000_000);
    assert_eq!(token.balance(&vault_id), 1_000_000);
}
