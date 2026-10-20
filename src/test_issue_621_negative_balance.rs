#![cfg(test)]

//! Issue #621: negative-balance impossibility.
//!
//! `balance::set_shares` / `set_total_shares` / `set_total_deposited` /
//! `set_reward_pool_balance` / `set_accrued_reward` / `set_total_rewards_paid`
//! / `set_reward_remainder` / `set_total_rewards_added` /
//! `set_total_ever_staked` / `set_insurance_fund_balance` /
//! `set_revenue_share_pool` / `set_escrow_balance` / `set_unstake_fee_reserve`
//! / `set_yield_deployed` now reject a negative value via
//! `balance::assert_non_negative`, panicking with
//! `VaultInvariantError::NegativeBalance` instead of ever committing it to
//! storage. These are unit tests of that backstop itself, called directly
//! through `env.as_contract` — the public API (`stake`/`unstake`/`claim`/
//! etc.) already validates amounts against the caller's balance before
//! reaching these setters, so this invariant should never actually be
//! reachable through normal contract usage; it exists in case that upstream
//! validation is ever missing or wrong.

extern crate std;

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::{balance, vault::VaultContract};

fn contract_env() -> (Env, Address) {
    let env = Env::default();
    let vault_id = env.register_contract(None, VaultContract);
    (env, vault_id)
}

#[test]
fn set_shares_accepts_zero_and_positive_values() {
    let (env, vault_id) = contract_env();
    let user = Address::generate(&env);
    env.as_contract(&vault_id, || {
        balance::set_shares(&env, &user, 0);
        assert_eq!(balance::get_shares(&env, &user), 0);
        balance::set_shares(&env, &user, 1_000);
        assert_eq!(balance::get_shares(&env, &user), 1_000);
    });
}

#[test]
#[should_panic]
fn set_shares_rejects_negative_value() {
    let (env, vault_id) = contract_env();
    let user = Address::generate(&env);
    env.as_contract(&vault_id, || {
        balance::set_shares(&env, &user, -1);
    });
}

#[test]
#[should_panic]
fn set_total_shares_rejects_negative_value() {
    let (env, vault_id) = contract_env();
    env.as_contract(&vault_id, || {
        balance::set_total_shares(&env, -1);
    });
}

#[test]
#[should_panic]
fn set_total_deposited_rejects_negative_value() {
    let (env, vault_id) = contract_env();
    env.as_contract(&vault_id, || {
        balance::set_total_deposited(&env, -1);
    });
}

#[test]
#[should_panic]
fn set_reward_pool_balance_rejects_negative_value() {
    let (env, vault_id) = contract_env();
    env.as_contract(&vault_id, || {
        balance::set_reward_pool_balance(&env, -1);
    });
}

#[test]
#[should_panic]
fn set_accrued_reward_rejects_negative_value() {
    let (env, vault_id) = contract_env();
    let user = Address::generate(&env);
    env.as_contract(&vault_id, || {
        balance::set_accrued_reward(&env, &user, -1);
    });
}

#[test]
#[should_panic]
fn set_total_rewards_paid_rejects_negative_value() {
    let (env, vault_id) = contract_env();
    env.as_contract(&vault_id, || {
        balance::set_total_rewards_paid(&env, -1);
    });
}

#[test]
#[should_panic]
fn set_reward_remainder_rejects_negative_value() {
    let (env, vault_id) = contract_env();
    let user = Address::generate(&env);
    env.as_contract(&vault_id, || {
        balance::set_reward_remainder(&env, &user, -1);
    });
}

#[test]
#[should_panic]
fn set_insurance_fund_balance_rejects_negative_value() {
    let (env, vault_id) = contract_env();
    env.as_contract(&vault_id, || {
        balance::set_insurance_fund_balance(&env, -1);
    });
}

#[test]
#[should_panic]
fn set_escrow_balance_rejects_negative_value() {
    let (env, vault_id) = contract_env();
    let user = Address::generate(&env);
    env.as_contract(&vault_id, || {
        balance::set_escrow_balance(&env, &user, -1);
    });
}

/// A negative write must never reach storage: after the panic unwinds (and
/// in production would abort the whole transaction), a fresh read still
/// returns the last valid value, not the rejected negative one.
#[test]
fn rejected_write_never_reaches_storage() {
    let (env, vault_id) = contract_env();
    let user = Address::generate(&env);
    env.as_contract(&vault_id, || {
        balance::set_shares(&env, &user, 500);
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        env.as_contract(&vault_id, || {
            balance::set_shares(&env, &user, -500);
        });
    }));
    assert!(result.is_err());

    env.as_contract(&vault_id, || {
        assert_eq!(balance::get_shares(&env, &user), 500);
    });
}
