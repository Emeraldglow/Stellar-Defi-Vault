#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::vault::{VaultContract, VaultContractClient, LEDGERS_PER_DAY};

fn create_token<'a>(
    env: &Env,
    admin: &Address,
) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let address = env.register_stellar_asset_contract(admin.clone());
    let client = token::Client::new(env, &address);
    let admin_client = token::StellarAssetClient::new(env, &address);
    (address, client, admin_client)
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    admin: Address,
    alice: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 1000;
        });

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);

        let (token_addr, _token, token_admin) = create_token(&env, &admin);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        token_admin.mint(&alice, &1_000_000);

        Fixture {
            env,
            vault,
            admin,
            alice,
        }
    }

    fn advance(&self, ledgers: u32) {
        self.env.ledger().with_mut(|li| {
            li.sequence_number += ledgers;
        });
    }
}

// ── Issue #554: per-user rolling 24h withdrawal limit ───────────────────────

#[test]
fn withdrawal_under_cap_succeeds_and_is_tracked() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &1_000, &0);
    f.vault.set_daily_withdrawal_limit(&600);

    let returned = f.vault.withdraw(&f.alice, &500);
    assert_eq!(returned, 500);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 100);
}

#[test]
fn cumulative_withdrawals_exceeding_cap_revert() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &1_000, &0);
    f.vault.set_daily_withdrawal_limit(&600);

    f.vault.withdraw(&f.alice, &500);
    // 500 + 200 = 700 > 600 cap, even though each individual call is small
    // and neither alone would trip a single-transaction circuit breaker.
    let result = f.vault.try_withdraw(&f.alice, &200);
    assert!(result.is_err());
    // Nothing was recorded/transferred from the failed attempt.
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 100);
}

#[test]
fn window_rolls_forward_and_old_withdrawals_age_out() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &1_000, &0);
    f.vault.set_daily_withdrawal_limit(&600);

    f.vault.withdraw(&f.alice, &600);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 0);
    assert!(f.vault.try_withdraw(&f.alice, &1).is_err());

    // Still within the same rolling window just before it elapses.
    f.advance(LEDGERS_PER_DAY - 1);
    assert!(f.vault.try_withdraw(&f.alice, &1).is_err());

    // Once a full LEDGERS_PER_DAY has elapsed since the window opened, the
    // tracker resets and the user gets fresh headroom.
    f.advance(1);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 600);
    let returned = f.vault.withdraw(&f.alice, &400);
    assert_eq!(returned, 400);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 200);
}

#[test]
fn disabled_limit_does_not_restrict_withdrawals() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &1_000, &0);
    // Limit left at its default (0 = disabled).
    let returned = f.vault.withdraw(&f.alice, &1_000);
    assert_eq!(returned, 1_000);
    assert_eq!(f.vault.get_remaining_daily_limit(&f.alice), 0);
}

#[test]
fn unstake_all_is_subject_to_the_same_cap() {
    let f = Fixture::new();
    f.vault.stake(&f.alice, &1_000, &0);
    f.vault.set_daily_withdrawal_limit(&500);

    // A full-position exit isn't a backdoor around the per-user cap.
    let result = f.vault.try_unstake_all(&f.alice);
    assert!(result.is_err());
    assert_eq!(f.vault.shares_of(&f.alice), 1_000);
}
