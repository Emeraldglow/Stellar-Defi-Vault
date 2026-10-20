//! Issue #655 — Configurable test-only mock token.
//!
//! `MockToken` implements the subset of the SEP-41 token interface the vault
//! uses, with configurable behavior for exercising failure paths:
//!
//! - **decimals** — any value (e.g. 0, 6, 7, 18).
//! - **transfer fee** — basis points deducted from the amount the recipient
//!   receives (fee-on-transfer tokens).
//! - **fail transfers** — every transfer panics, simulating a frozen or
//!   malicious token.
//! - **blocklist** — transfers from/to a blocklisted address panic.
//!
//! The module is compiled only for tests so it never ships in the WASM.
#![cfg(any(test, feature = "testutils"))]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

#[contracttype]
#[derive(Clone)]
enum MockKey {
    Decimals,
    FeeBps,
    FailAll,
    Blocked(Address),
    Balance(Address),
    Supply,
}

pub const MOCK_BPS_BASE: i128 = 10_000;

#[contract]
pub struct MockToken;

#[contractimpl]
impl MockToken {
    // ── configuration ────────────────────────────────────────────────────

    pub fn configure(env: Env, decimals: u32, fee_bps: u32) {
        assert!(fee_bps as i128 <= MOCK_BPS_BASE, "fee_bps > 100%");
        env.storage().instance().set(&MockKey::Decimals, &decimals);
        env.storage().instance().set(&MockKey::FeeBps, &fee_bps);
    }

    pub fn set_fail_all(env: Env, fail: bool) {
        env.storage().instance().set(&MockKey::FailAll, &fail);
    }

    pub fn set_blocked(env: Env, who: Address, blocked: bool) {
        env.storage()
            .persistent()
            .set(&MockKey::Blocked(who), &blocked);
    }

    // ── SEP-41 subset ────────────────────────────────────────────────────

    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&MockKey::Decimals)
            .unwrap_or(7)
    }

    pub fn name(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "Mock Token")
    }

    pub fn symbol(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "MOCK")
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&MockKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage().instance().get(&MockKey::Supply).unwrap_or(0)
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        assert!(amount >= 0, "negative mint");
        let bal = Self::balance(env.clone(), to.clone());
        Self::write_balance(&env, &to, bal + amount);
        let supply = Self::total_supply(env.clone());
        env.storage()
            .instance()
            .set(&MockKey::Supply, &(supply + amount));
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        Self::move_funds(&env, &from, &to, amount);
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        Self::move_funds(&env, &from, &to, amount);
    }

    pub fn approve(_env: Env, from: Address, _spender: Address, _amount: i128, _exp: u32) {
        from.require_auth();
    }

    pub fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 {
        i128::MAX
    }
}

impl MockToken {
    fn is_blocked(env: &Env, who: &Address) -> bool {
        env.storage()
            .persistent()
            .get(&MockKey::Blocked(who.clone()))
            .unwrap_or(false)
    }

    fn write_balance(env: &Env, who: &Address, amount: i128) {
        env.storage()
            .persistent()
            .set(&MockKey::Balance(who.clone()), &amount);
    }

    fn move_funds(env: &Env, from: &Address, to: &Address, amount: i128) {
        assert!(amount >= 0, "negative transfer");
        let fail: bool = env
            .storage()
            .instance()
            .get(&MockKey::FailAll)
            .unwrap_or(false);
        assert!(!fail, "mock token: transfers disabled");
        assert!(
            !Self::is_blocked(env, from) && !Self::is_blocked(env, to),
            "mock token: address blocked"
        );

        let from_bal = Self::balance(env.clone(), from.clone());
        assert!(from_bal >= amount, "mock token: insufficient balance");

        let fee_bps: u32 = env.storage().instance().get(&MockKey::FeeBps).unwrap_or(0);
        let fee = amount * fee_bps as i128 / MOCK_BPS_BASE;
        let received = amount - fee;

        Self::write_balance(env, from, from_bal - amount);
        let to_bal = Self::balance(env.clone(), to.clone());
        Self::write_balance(env, to, to_bal + received);

        // Fee is burned.
        if fee > 0 {
            let supply = Self::total_supply(env.clone());
            env.storage()
                .instance()
                .set(&MockKey::Supply, &(supply - fee));
        }
    }
}

/// Register a configured `MockToken` and return its address and client.
pub fn setup_mock_token<'a>(env: &Env, decimals: u32, fee_bps: u32) -> (Address, MockTokenClient<'a>) {
    let id = env.register_contract(None, MockToken);
    let client = MockTokenClient::new(env, &id);
    client.configure(&decimals, &fee_bps);
    (id, client)
}
