#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env, String, Symbol, TryFromVal,
};

use crate::vault::{VaultContract, VaultContractClient, CONTRACT_VERSION};

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
    bob: Address,
    token_addr: Address,
    token_admin: token::StellarAssetClient<'a>,
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
        let bob = Address::generate(&env);

        let (token_addr, _token, token_admin) = create_token(&env, &admin);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &1000_u32, &None, &None);

        token_admin.mint(&alice, &10_000_000);
        token_admin.mint(&bob, &10_000_000);

        Fixture {
            env,
            vault,
            admin,
            alice,
            bob,
            token_addr,
            token_admin,
        }
    }

    fn advance(&self, ledgers: u32) {
        self.env.ledger().with_mut(|li| {
            li.sequence_number += ledgers;
        });
    }
}

// ── Issue #541: Deposit receipt memo support ─────────────────────────────────

#[test]
fn test_deposit_with_memo_emits_event_with_memo() {
    let f = Fixture::new();
    let memo_str = String::from_str(&f.env, "tx-ref-12345");
    let shares = f.vault.deposit_with_memo(&f.alice, &1_000, &memo_str);
    assert_eq!(shares, 1_000);

    // Verify memo appears in the emitted deposit_completed event
    let (_, topics, data) = f.env.events().all().last().unwrap();
    let topic = Symbol::try_from_val(&f.env, &topics.get(0).unwrap()).unwrap();
    assert_eq!(topic, Symbol::new(&f.env, "deposit_completed"));

    let depositor = Address::try_from_val(&f.env, &topics.get(1).unwrap()).unwrap();
    assert_eq!(depositor, f.alice);

    let (amount, shares_minted, emitted_memo, ledger) =
        <(i128, i128, String, u32)>::try_from_val(&f.env, &data).unwrap();
    assert_eq!(amount, 1_000);
    assert_eq!(shares_minted, 1_000);
    assert_eq!(emitted_memo, memo_str);
    assert_eq!(ledger, 1000);

    // Verify user shares were properly updated
    assert_eq!(f.vault.shares_of(&f.alice), 1_000);
}

#[test]
fn test_deposit_with_memo_oversized_reverts() {
    let f = Fixture::new();
    // 65 bytes string (> 64 bytes cap)
    let oversized = String::from_str(
        &f.env,
        "12345678901234567890123456789012345678901234567890123456789012345",
    );
    assert_eq!(oversized.len(), 65);

    let result = f.vault.try_deposit_with_memo(&f.alice, &1_000, &oversized);
    assert!(result.is_err());
    assert_eq!(f.vault.shares_of(&f.alice), 0);
}

#[test]
fn test_deposit_without_memo_still_works() {
    let f = Fixture::new();
    // Standard deposit without memo still works via existing function
    let shares = f.vault.deposit(&f.alice, &1_000, &0);
    assert_eq!(shares, 1_000);
    assert_eq!(f.vault.shares_of(&f.alice), 1_000);

    // Stake also works
    let shares_bob = f.vault.stake(&f.bob, &2_000, &0);
    assert_eq!(shares_bob, 2_000);
    assert_eq!(f.vault.shares_of(&f.bob), 2_000);
}

// ── Issue #540: Scheduled reward-rate ramp ───────────────────────────────────

#[test]
fn test_rate_ramp_linear_interpolation_mid_ramp() {
    let f = Fixture::new();
    // Base rate is 1000 bps
    assert_eq!(f.vault.get_current_rate(), 1000);

    // Start ramp from 1000 to 3000 over 200 ledgers
    f.vault.set_rate_ramp(&f.admin, &3000, &200);

    // Mid-ramp after 100 ledgers (50% progress): 1000 + (3000 - 1000) * 100 / 200 = 2000 bps
    f.advance(100);
    assert_eq!(f.vault.get_current_rate(), 2000);
}

#[test]
fn test_rate_ramp_reaches_exact_target_at_ramp_end() {
    let f = Fixture::new();
    f.vault.set_rate_ramp(&f.admin, &4000, &100);

    // Before ramp end (ledger 75): 1000 + 3000 * 75 / 100 = 3250 bps
    f.advance(75);
    assert_eq!(f.vault.get_current_rate(), 3250);

    // At exact ramp end (100 ledgers): reaches 4000 bps
    f.advance(25);
    assert_eq!(f.vault.get_current_rate(), 4000);

    // Beyond ramp end: stays at target rate
    f.advance(50);
    assert_eq!(f.vault.get_current_rate(), 4000);

    // Complete ramp settles into base rate and emits rate_ramp_completed
    let final_rate = f.vault.complete_rate_ramp();
    assert_eq!(final_rate, 4000);
    assert_eq!(f.vault.get_reward_rate_bps(), 4000);
}

#[test]
fn test_rate_ramp_cancellation_freezes_correctly() {
    let f = Fixture::new();
    // Ramp from 1000 to 5000 over 200 ledgers
    f.vault.set_rate_ramp(&f.admin, &5000, &200);

    // Advance 50 ledgers (25% progress): 1000 + 4000 * 50 / 200 = 2000 bps
    f.advance(50);
    assert_eq!(f.vault.get_current_rate(), 2000);

    // Admin cancels ramp -> freezes at 2000 bps
    f.vault.cancel_rate_ramp(&f.admin);
    assert_eq!(f.vault.get_current_rate(), 2000);
    assert_eq!(f.vault.get_reward_rate_bps(), 2000);

    // Further ledger progression keeps rate frozen
    f.advance(100);
    assert_eq!(f.vault.get_current_rate(), 2000);
    assert_eq!(f.vault.get_reward_rate_bps(), 2000);
}

#[test]
fn test_rate_ramp_downwards_interpolation() {
    let f = Fixture::new();
    // Ramp down from 1000 to 200 over 100 ledgers
    f.vault.set_rate_ramp(&f.admin, &200, &100);

    // Mid-ramp after 50 ledgers: 1000 - 800 * 50 / 100 = 600 bps
    f.advance(50);
    assert_eq!(f.vault.get_current_rate(), 600);

    f.advance(50);
    assert_eq!(f.vault.get_current_rate(), 200);
}

// ── Issue #539: Per-token fee override ───────────────────────────────────────

#[test]
fn test_token_fee_override_overridden_token_uses_own_fee() {
    let f = Fixture::new();
    let (other_token, _, _) = create_token(&f.env, &f.admin);

    // Default global unstake fee is 0
    assert_eq!(f.vault.get_effective_unstake_fee_bps(&other_token), 0);
    assert_eq!(f.vault.get_effective_deposit_fee_bps(&other_token), 0);

    // Set token-specific fee override: 50 bps deposit, 250 bps unstake
    f.vault.set_token_fee_override(&f.admin, &other_token, &Some(50), &Some(250));

    let (dep_fee, unstake_fee) = f.vault.get_token_fee_override(&other_token);
    assert_eq!(dep_fee, Some(50));
    assert_eq!(unstake_fee, Some(250));

    assert_eq!(f.vault.get_effective_deposit_fee_bps(&other_token), 50);
    assert_eq!(f.vault.get_effective_unstake_fee_bps(&other_token), 250);
}

#[test]
fn test_token_fee_override_non_overridden_token_falls_back_to_global_default() {
    let f = Fixture::new();
    let (token_b, _, _) = create_token(&f.env, &f.admin);

    // Set global default unstake fee to 100 bps
    f.vault.set_unstake_fee_bps(&f.admin, &100);
    assert_eq!(f.vault.get_unstake_fee_bps(), 100);

    // token_b has no override
    let (dep_fee, unstake_fee) = f.vault.get_token_fee_override(&token_b);
    assert_eq!(dep_fee, None);
    assert_eq!(unstake_fee, None);

    // Falls back to global default (100 bps unstake, 0 bps deposit)
    assert_eq!(f.vault.get_effective_unstake_fee_bps(&token_b), 100);
    assert_eq!(f.vault.get_effective_deposit_fee_bps(&token_b), 0);
}

#[test]
fn test_token_fee_override_clearing_restores_default_behavior() {
    let f = Fixture::new();
    let (token_c, _, _) = create_token(&f.env, &f.admin);

    f.vault.set_unstake_fee_bps(&f.admin, &150);

    // Set override
    f.vault.set_token_fee_override(&f.admin, &token_c, &Some(20), &Some(300));
    assert_eq!(f.vault.get_effective_unstake_fee_bps(&token_c), 300);

    // Clear override by setting both to None
    f.vault.set_token_fee_override(&f.admin, &token_c, &None, &None);

    let (dep_fee, unstake_fee) = f.vault.get_token_fee_override(&token_c);
    assert_eq!(dep_fee, None);
    assert_eq!(unstake_fee, None);

    // Restores default fee (150 bps)
    assert_eq!(f.vault.get_effective_unstake_fee_bps(&token_c), 150);
    assert_eq!(f.vault.get_effective_deposit_fee_bps(&token_c), 0);
}

#[test]
fn test_token_fee_override_applied_in_unstake() {
    let f = Fixture::new();
    // Stake 10_000
    f.vault.stake(&f.alice, &10_000, &0);

    // Set global unstake fee to 100 bps (1%)
    f.vault.set_unstake_fee_bps(&f.admin, &100);

    // Override vault token unstake fee to 300 bps (3%)
    f.vault.set_token_fee_override(&f.admin, &f.token_addr, &None, &Some(300));

    // Unstake 10_000 shares: with 300 bps (3%) fee, fee is 300, payout is 9_700
    let returned = f.vault.unstake(&f.alice, &10_000);
    assert_eq!(returned, 10_000); // unstake returns gross amount, transfers net
}

// ── Issue #538: Read-only contract version query ─────────────────────────

#[test]
fn test_get_contract_version_returns_expected_constant() {
    let f = Fixture::new();
    let version = f.vault.get_contract_version();
    assert_eq!(version, String::from_str(&f.env, CONTRACT_VERSION));
    assert_eq!(version, String::from_str(&f.env, "0.1.0"));
}
