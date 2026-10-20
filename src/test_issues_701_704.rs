#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    token, Address, BytesN, Env, Symbol, TryFromVal,
};

use crate::{
    errors::VaultError,
    vault::{VaultContract, VaultContractClient},
};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    token: token::Client<'a>,
    token_admin: token::StellarAssetClient<'a>,
    admin: Address,
    alice: Address,
}

impl<'a> Fixture<'a> {
    fn new(reward_rate_bps: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 1000;
        });

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);

        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_addr);
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &reward_rate_bps, &None, &None);

        token_admin.mint(&alice, &10_000_000);

        Fixture {
            env,
            vault,
            vault_id,
            token,
            token_admin,
            admin,
            alice,
        }
    }

    fn advance(&self, ledgers: u32) {
        self.env.ledger().with_mut(|li| {
            li.sequence_number += ledgers;
        });
    }

    fn has_event(&self, name: &str) -> bool {
        self.env.events().all().iter().any(|(_, topics, _)| {
            topics
                .get(0)
                .and_then(|val| Symbol::try_from_val(&self.env, &val).ok())
                .map(|topic| topic == Symbol::new(&self.env, name))
                .unwrap_or(false)
        })
    }
}

// ── Issue #701: emission reduction schedule ──────────────────────────────────

#[test]
fn test_reduction_steps_down_at_interval_boundaries() {
    let f = Fixture::new(1000);
    // 25% cut every 100 ledgers, schedule starts at ledger 1000.
    f.vault.set_halving_schedule(&f.admin, &100, &2_500, &0);
    assert_eq!(f.vault.get_halving_reduction_bps(), 2_500);

    assert_eq!(f.vault.get_next_reduction(), Some((1100, 750)));
    f.advance(100);
    assert_eq!(f.vault.get_next_reduction(), Some((1200, 562)));
    f.advance(100);
    assert_eq!(f.vault.get_next_reduction(), Some((1300, 421)));
}

#[test]
fn test_reduction_never_drops_below_floor() {
    let f = Fixture::new(1000);
    f.vault.set_halving_schedule(&f.admin, &100, &5_000, &400);

    assert_eq!(f.vault.get_next_reduction(), Some((1100, 500)));
    f.advance(100);
    assert_eq!(f.vault.get_next_reduction(), Some((1200, 400)));
    f.advance(1_000);
    assert_eq!(f.vault.get_next_reduction(), Some((2300, 400)));
}

#[test]
fn test_disabling_schedule_restores_static_rate() {
    let f = Fixture::new(1000);
    assert_eq!(f.vault.get_next_reduction(), None);

    f.vault.set_halving_schedule(&f.admin, &100, &5_000, &0);
    f.advance(250);
    assert_eq!(f.vault.get_current_halving_count(), 2);

    f.vault.disable_halving_schedule(&f.admin);
    assert_eq!(f.vault.get_halving_config(), None);
    assert_eq!(f.vault.get_next_reduction(), None);
    assert_eq!(f.vault.get_current_halving_count(), 0);
    assert_eq!(f.vault.get_reward_rate_bps(), 1000);

    let again = f.vault.try_disable_halving_schedule(&f.admin);
    assert_eq!(again, Err(Ok(VaultError::NoCampaignActive)));
}

#[test]
fn test_reduction_bps_must_be_in_range() {
    let f = Fixture::new(1000);
    let zero = f.vault.try_set_halving_schedule(&f.admin, &100, &0, &0);
    assert_eq!(zero, Err(Ok(VaultError::InvalidRate)));
    let too_high = f.vault.try_set_halving_schedule(&f.admin, &100, &10_001, &0);
    assert_eq!(too_high, Err(Ok(VaultError::InvalidRate)));
}

// ── Issue #702: per-user epoch reward cap ────────────────────────────────────

/// Epoch mode with a 100-ledger epoch paying 1_000 per epoch, alice staked
/// 1_000_000, and a funded reward pool.
fn epoch_fixture<'a>() -> Fixture<'a> {
    let f = Fixture::new(0);
    f.vault.set_epoch_mode(&f.admin, &100, &1_000);
    f.token_admin.mint(&f.admin, &5_000);
    f.vault.fund_reward_pool(&f.admin, &5_000);
    f.vault.stake(&f.alice, &1_000_000);
    f
}

#[test]
fn test_user_epoch_cap_is_enforced() {
    let f = epoch_fixture();
    f.advance(100);
    f.vault.finalize_epoch(&f.admin);
    assert_eq!(f.vault.epoch_reward(&f.alice, &1), 1_000);

    f.vault.set_user_epoch_reward_cap(&f.admin, &400);
    assert_eq!(f.vault.get_user_epoch_reward_cap(), 400);
    assert_eq!(f.vault.epoch_reward(&f.alice, &1), 400);
    assert_eq!(f.vault.get_user_epoch_rewards(&f.alice), 400);

    // `0` disables the cap again.
    f.vault.set_user_epoch_reward_cap(&f.admin, &0);
    assert_eq!(f.vault.epoch_reward(&f.alice, &1), 1_000);
    assert_eq!(f.vault.get_user_epoch_rewards(&f.alice), 1_000);
}

#[test]
fn test_user_epoch_cap_resets_each_epoch_and_excess_stays_in_pool() {
    let f = epoch_fixture();
    f.vault.set_user_epoch_reward_cap(&f.admin, &400);

    f.advance(100);
    f.vault.finalize_epoch(&f.admin);
    f.advance(100);
    f.vault.finalize_epoch(&f.admin);

    // Each epoch is capped on its own, so two epochs pay 2 * 400.
    let paid = f.vault.claim_epoch_rewards(&f.alice);
    assert_eq!(paid, 800);
    // Only the capped amount left the pool; the 1_200 of excess stayed in it.
    assert_eq!(f.vault.get_reward_pool_balance(), 5_000 - 800);
}

#[test]
fn test_negative_user_epoch_cap_rejected() {
    let f = Fixture::new(0);
    let result = f.vault.try_set_user_epoch_reward_cap(&f.admin, &-1);
    assert_eq!(result, Err(Ok(VaultError::InvalidRewardAmount)));
}

// ── Issue #703: fee holiday ──────────────────────────────────────────────────

#[test]
fn test_fee_waived_in_window_and_restored_after_expiry() {
    let f = Fixture::new(0);
    f.vault.set_unstake_fee_bps(&f.admin, &100); // 1%
    f.vault.stake(&f.alice, &1_000_000);

    f.vault.start_fee_holiday(&f.admin, &1050);
    assert_eq!(f.vault.get_fee_holiday(), Some(1050));
    assert!(f.has_event("fee_holiday_started"));

    let before = f.token.balance(&f.alice);
    f.vault.unstake(&f.alice, &100_000);
    assert_eq!(f.token.balance(&f.alice) - before, 100_000);

    // Past the end ledger the holiday lapses without any admin call.
    f.advance(100);
    assert_eq!(f.vault.get_fee_holiday(), None);
    let before = f.token.balance(&f.alice);
    f.vault.unstake(&f.alice, &100_000);
    assert_eq!(f.token.balance(&f.alice) - before, 99_000);
    assert!(f.has_event("fee_holiday_ended"));
}

#[test]
fn test_end_fee_holiday_restores_fee_early() {
    let f = Fixture::new(0);
    f.vault.set_unstake_fee_bps(&f.admin, &100);
    f.vault.stake(&f.alice, &1_000_000);

    f.vault.start_fee_holiday(&f.admin, &5000);
    f.vault.end_fee_holiday(&f.admin);
    assert!(f.has_event("fee_holiday_ended"));
    assert_eq!(f.vault.get_fee_holiday(), None);

    let before = f.token.balance(&f.alice);
    f.vault.unstake(&f.alice, &100_000);
    assert_eq!(f.token.balance(&f.alice) - before, 99_000);

    let again = f.vault.try_end_fee_holiday(&f.admin);
    assert_eq!(again, Err(Ok(VaultError::NoCampaignActive)));
}

#[test]
fn test_fee_holiday_must_end_in_the_future() {
    let f = Fixture::new(0);
    let result = f.vault.try_start_fee_holiday(&f.admin, &1000);
    assert_eq!(result, Err(Ok(VaultError::InvalidRate)));
}

// ── Issue #704: insurance reserve ────────────────────────────────────────────

#[test]
fn test_insurance_share_is_routed_from_fees() {
    let f = Fixture::new(0);
    f.vault.set_unstake_fee_bps(&f.admin, &100); // 1%
    f.vault.set_insurance_fee_share_bps(&f.admin, &2_000); // 20% of fees
    assert_eq!(f.vault.get_insurance_fee_share_bps(), 2_000);
    f.vault.stake(&f.alice, &1_000_000);

    f.vault.unstake(&f.alice, &500_000);

    // Fee = 5_000: 1_000 to the reserve, 4_000 to the reward pool.
    assert_eq!(f.vault.get_insurance_reserve(), 1_000);
    assert_eq!(f.vault.get_reward_pool_balance(), 4_000);
    // The reserve never touches principal: what the vault holds is exactly the
    // remaining stake plus the reward pool plus the reserve.
    assert_eq!(f.token.balance(&f.vault_id), 500_000 + 4_000 + 1_000);
}

#[test]
fn test_use_insurance_reserve_is_capped_per_call() {
    let f = Fixture::new(0);
    f.vault.set_unstake_fee_bps(&f.admin, &100);
    f.vault.set_insurance_fee_share_bps(&f.admin, &2_000);
    f.vault.stake(&f.alice, &1_000_000);
    f.vault.unstake(&f.alice, &500_000);

    let reason = BytesN::from_array(&f.env, &[7u8; 32]);

    // At most half of the 1_000 reserve per call.
    let too_much = f.vault.try_use_insurance_reserve(&f.admin, &501, &reason);
    assert_eq!(too_much, Err(Ok(VaultError::WithdrawalLimitExceeded)));
    let zero = f.vault.try_use_insurance_reserve(&f.admin, &0, &reason);
    assert_eq!(zero, Err(Ok(VaultError::ZeroAmount)));

    f.vault.use_insurance_reserve(&f.admin, &500, &reason);
    assert!(f.has_event("insurance_reserve_used"));
    assert_eq!(f.vault.get_insurance_reserve(), 500);
    assert_eq!(f.vault.get_reward_pool_balance(), 4_500);
}

#[test]
fn test_insurance_share_above_100_percent_rejected() {
    let f = Fixture::new(0);
    let result = f.vault.try_set_insurance_fee_share_bps(&f.admin, &10_001);
    assert_eq!(result, Err(Ok(VaultError::InvalidRate)));
}
