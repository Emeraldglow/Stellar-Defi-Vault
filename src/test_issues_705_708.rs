#![cfg(test)]
//! Tests for withdraw-only mode (#705), position split/merge (#706),
//! beneficiary inheritance (#707) and dust cleanup (#708).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::{
    balance,
    errors::{VaultError, VaultLockError, VaultSafetyError},
    storage::{DataKey, StakePosition},
    vault::{withdraw_only_mode::PoolMode, VaultContract, VaultContractClient},
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
    token: token::Client<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
    keeper: Address,
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
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let keeper = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token = token::Client::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        // The vault holds the staked tokens that exits and dust sweeps pay out.
        token::StellarAssetClient::new(&env, &token_addr).mint(&vault_id, &1_000_000);
        set_ledger(&env, 1_000);

        Fixture {
            env,
            vault,
            vault_id,
            token,
            admin,
            alice,
            bob,
            keeper,
        }
    }

    /// Seed a 1:1 position directly, backed by the tokens minted to the vault.
    fn seed(&self, user: &Address, shares: i128) {
        self.env.as_contract(&self.vault_id, || {
            balance::set_shares(&self.env, user, shares);
            balance::set_total_shares(&self.env, balance::get_total_shares(&self.env) + shares);
            balance::set_total_deposited(
                &self.env,
                balance::get_total_deposited(&self.env) + shares,
            );
            balance::register_staker(&self.env, user);
            balance::set_reward_checkpoint_ledger(&self.env, user, self.env.ledger().sequence());
            self.env.storage().persistent().set(
                &DataKey::StakedAtLedger(user.clone()),
                &self.env.ledger().sequence(),
            );
        });
    }

    fn set_splits(&self, user: &Address, splits: &[StakePosition]) {
        let mut list = soroban_sdk::Vec::new(&self.env);
        for position in splits {
            list.push_back(position.clone());
        }
        self.env.as_contract(&self.vault_id, || {
            balance::set_split_positions(&self.env, user, &list);
        });
    }

    fn has_share_entry(&self, user: &Address) -> bool {
        self.env.as_contract(&self.vault_id, || {
            self.env
                .storage()
                .persistent()
                .has(&DataKey::ShareBalance(user.clone()))
        })
    }
}

// ── Issue #705: withdraw-only mode ───────────────────────────────────────────

#[test]
fn pool_mode_follows_enter_and_exit() {
    let f = Fixture::new();
    assert_eq!(f.vault.get_pool_mode(), PoolMode::Normal);

    f.vault.enter_withdraw_only_mode(&f.admin);
    assert_eq!(f.vault.get_pool_mode(), PoolMode::WithdrawOnly);

    f.vault.exit_withdraw_only_mode(&f.admin);
    assert_eq!(f.vault.get_pool_mode(), PoolMode::Normal);
}

#[test]
fn only_admin_can_change_pool_mode() {
    let f = Fixture::new();
    assert_eq!(
        f.vault.try_enter_withdraw_only_mode(&f.alice),
        Err(Ok(VaultSafetyError::Unauthorized))
    );
    f.vault.enter_withdraw_only_mode(&f.admin);
    assert_eq!(
        f.vault.try_exit_withdraw_only_mode(&f.alice),
        Err(Ok(VaultSafetyError::Unauthorized))
    );
    assert_eq!(f.vault.get_pool_mode(), PoolMode::WithdrawOnly);
}

#[test]
fn withdraw_only_mode_blocks_inflows() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);
    f.vault.enter_withdraw_only_mode(&f.admin);

    assert_eq!(
        f.vault.try_stake(&f.alice, &1_000),
        Err(Ok(VaultError::PoolShuttingDown))
    );
    assert_eq!(
        f.vault.try_deposit(&f.alice, &1_000, &None),
        Err(Ok(VaultError::PoolShuttingDown))
    );
    assert_eq!(
        f.vault.try_lock_position(&f.alice, &100),
        Err(Ok(VaultLockError::WithdrawOnlyMode))
    );
    assert_eq!(f.vault.shares_of(&f.alice), 10_000);
}

#[test]
fn withdraw_only_mode_keeps_exits_open_and_fee_free() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);
    f.vault.set_unstake_fee_bps(&f.admin, &500);
    f.vault.enter_withdraw_only_mode(&f.admin);

    let returned = f.vault.withdraw(&f.alice, &10_000);

    // No 5% unstake fee is taken while the pool is withdraw-only.
    assert_eq!(returned, 10_000);
    assert_eq!(f.token.balance(&f.alice), 10_000);
    assert_eq!(f.vault.shares_of(&f.alice), 0);
}

// ── Issue #706: split and merge ──────────────────────────────────────────────

#[test]
fn split_position_conserves_shares() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);

    let new_id = f.vault.split_position(&f.alice, &0, &3_000);

    assert_eq!(new_id, 1);
    assert_eq!(f.vault.shares_of(&f.alice), 7_000);
    let splits = f.vault.get_split_positions(&f.alice);
    assert_eq!(splits.len(), 1);
    assert_eq!(splits.get(0).unwrap().amount, 3_000);

    // Splitting a split position works the same way by id.
    let second_id = f.vault.split_position(&f.alice, &1, &1_000);
    assert_eq!(second_id, 2);
    let splits = f.vault.get_split_positions(&f.alice);
    assert_eq!(splits.get(0).unwrap().amount, 2_000);
    assert_eq!(splits.get(1).unwrap().amount, 1_000);
    assert_eq!(
        f.vault.shares_of(&f.alice)
            + splits.get(0).unwrap().amount
            + splits.get(1).unwrap().amount,
        10_000
    );
}

#[test]
fn split_position_rejects_invalid_requests() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);

    for shares in [0_i128, -1, 10_000, 20_000] {
        assert_eq!(
            f.vault.try_split_position(&f.alice, &0, &shares),
            Err(Ok(VaultSafetyError::InvalidSplitAmount))
        );
    }
    assert_eq!(
        f.vault.try_split_position(&f.alice, &5, &100),
        Err(Ok(VaultSafetyError::PositionNotFound))
    );
}

#[test]
fn merge_positions_restores_the_original_position() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);
    f.vault.split_position(&f.alice, &0, &3_000);

    let kept = f.vault.merge_positions(&f.alice, &1, &0);

    assert_eq!(kept, 0);
    assert_eq!(f.vault.shares_of(&f.alice), 10_000);
    assert_eq!(f.vault.get_split_positions(&f.alice).len(), 0);
}

#[test]
fn merge_positions_combines_two_split_positions() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);
    f.vault.split_position(&f.alice, &0, &3_000);
    f.vault.split_position(&f.alice, &0, &2_000);

    let kept = f.vault.merge_positions(&f.alice, &2, &1);

    assert_eq!(kept, 1);
    let splits = f.vault.get_split_positions(&f.alice);
    assert_eq!(splits.len(), 1);
    assert_eq!(splits.get(0).unwrap().amount, 5_000);
    assert_eq!(f.vault.shares_of(&f.alice), 5_000);
}

#[test]
fn merge_positions_rejects_incompatible_positions() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);
    f.set_splits(
        &f.alice,
        &[StakePosition {
            amount: 2_000,
            staked_at_ledger: 1,
            last_claim_ledger: 1,
        }],
    );

    assert_eq!(
        f.vault.try_merge_positions(&f.alice, &0, &1),
        Err(Ok(VaultSafetyError::IncompatiblePositions))
    );
    assert_eq!(
        f.vault.try_merge_positions(&f.alice, &0, &0),
        Err(Ok(VaultSafetyError::IncompatiblePositions))
    );
    assert_eq!(f.vault.shares_of(&f.alice), 10_000);
    assert_eq!(f.vault.get_split_positions(&f.alice).len(), 1);
}

#[test]
fn split_and_merge_leave_reward_totals_unchanged() {
    let f = Fixture::new();
    f.vault.set_reward_rate_bps(&1_000);
    f.seed(&f.alice, 1_000_000);
    set_ledger(&f.env, 100_000);

    let before = f.vault.calc_pending_reward(&f.alice);
    assert!(before > 0);

    f.vault.split_position(&f.alice, &0, &400_000);
    assert_eq!(f.vault.calc_pending_reward(&f.alice), before);

    f.vault.merge_positions(&f.alice, &0, &1);
    assert_eq!(f.vault.calc_pending_reward(&f.alice), before);
}

// ── Issue #707: beneficiary inheritance ──────────────────────────────────────

#[test]
fn premature_inheritance_claim_reverts() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);
    f.vault.set_beneficiary(&f.alice, &f.bob, &500);

    set_ledger(&f.env, 1_000 + 499);
    assert_eq!(
        f.vault.try_claim_inheritance(&f.bob, &f.alice),
        Err(Ok(VaultSafetyError::InactivityPeriodNotElapsed))
    );
    assert_eq!(f.vault.shares_of(&f.alice), 10_000);
}

#[test]
fn inheritance_moves_the_full_position() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);
    f.vault.split_position(&f.alice, &0, &3_000);
    f.vault.set_beneficiary(&f.alice, &f.bob, &500);

    set_ledger(&f.env, 1_000 + 500);
    f.vault.claim_inheritance(&f.bob, &f.alice);

    assert_eq!(f.vault.shares_of(&f.alice), 0);
    assert_eq!(f.vault.shares_of(&f.bob), 7_000);
    assert_eq!(f.vault.get_split_positions(&f.alice).len(), 0);
    let moved = f.vault.get_split_positions(&f.bob);
    assert_eq!(moved.len(), 1);
    assert_eq!(moved.get(0).unwrap().amount, 3_000);
    assert!(f.vault.get_beneficiary(&f.alice).is_none());
}

#[test]
fn owner_activity_resets_the_inactivity_timer() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);
    f.vault.set_beneficiary(&f.alice, &f.bob, &500);

    // Owner interacts 400 ledgers in, so the clock restarts from there.
    set_ledger(&f.env, 1_000 + 400);
    f.vault.split_position(&f.alice, &0, &1_000);

    set_ledger(&f.env, 1_000 + 500);
    assert_eq!(
        f.vault.try_claim_inheritance(&f.bob, &f.alice),
        Err(Ok(VaultSafetyError::InactivityPeriodNotElapsed))
    );

    set_ledger(&f.env, 1_000 + 400 + 500);
    f.vault.claim_inheritance(&f.bob, &f.alice);
    assert_eq!(f.vault.shares_of(&f.bob), 9_000);
}

#[test]
fn beneficiary_designation_is_validated_and_clearable() {
    let f = Fixture::new();
    f.seed(&f.alice, 10_000);

    assert_eq!(
        f.vault.try_set_beneficiary(&f.alice, &f.alice, &500),
        Err(Ok(VaultSafetyError::InvalidBeneficiary))
    );
    assert_eq!(
        f.vault.try_set_beneficiary(&f.alice, &f.bob, &0),
        Err(Ok(VaultSafetyError::ZeroAmount))
    );
    assert_eq!(
        f.vault.try_claim_inheritance(&f.bob, &f.alice),
        Err(Ok(VaultSafetyError::NoBeneficiary))
    );

    f.vault.set_beneficiary(&f.alice, &f.bob, &500);
    let stranger = Address::generate(&f.env);
    set_ledger(&f.env, 1_000 + 500);
    assert_eq!(
        f.vault.try_claim_inheritance(&stranger, &f.alice),
        Err(Ok(VaultSafetyError::Unauthorized))
    );

    f.vault.clear_beneficiary(&f.alice);
    assert_eq!(
        f.vault.try_claim_inheritance(&f.bob, &f.alice),
        Err(Ok(VaultSafetyError::NoBeneficiary))
    );
    assert_eq!(
        f.vault.try_clear_beneficiary(&f.alice),
        Err(Ok(VaultSafetyError::NoBeneficiary))
    );
}

// ── Issue #708: dust cleanup ─────────────────────────────────────────────────

#[test]
fn only_sub_threshold_positions_are_closable() {
    let f = Fixture::new();
    f.seed(&f.alice, 500);
    f.seed(&f.bob, 5_000);

    assert_eq!(
        f.vault.try_close_dust_position(&f.keeper, &f.alice),
        Err(Ok(VaultSafetyError::DustThresholdNotSet))
    );

    f.vault.set_dust_threshold(&f.admin, &1_000);
    assert_eq!(f.vault.get_dust_threshold(), 1_000);
    assert_eq!(
        f.vault.try_close_dust_position(&f.keeper, &f.bob),
        Err(Ok(VaultSafetyError::NotDustPosition))
    );
    assert_eq!(f.vault.shares_of(&f.bob), 5_000);

    assert_eq!(f.vault.close_dust_position(&f.keeper, &f.alice), 500);
}

#[test]
fn closing_dust_returns_funds_and_removes_the_entry() {
    let f = Fixture::new();
    f.seed(&f.alice, 500);
    f.seed(&f.bob, 5_000);
    f.vault.set_dust_threshold(&f.admin, &1_000);
    assert!(f.has_share_entry(&f.alice));

    let paid = f.vault.close_dust_position(&f.keeper, &f.alice);

    assert_eq!(paid, 500);
    assert_eq!(f.token.balance(&f.alice), 500);
    assert_eq!(f.token.balance(&f.keeper), 0);
    assert!(!f.has_share_entry(&f.alice));
    assert_eq!(f.vault.shares_of(&f.alice), 0);
    // Pool totals only lose the closed position.
    assert_eq!(f.vault.total_staked(), 5_000);
    assert_eq!(f.token.balance(&f.vault_id), 1_000_000 - 500);
}

#[test]
fn keeper_tip_and_treasury_are_applied() {
    let f = Fixture::new();
    f.seed(&f.alice, 500);
    f.seed(&f.bob, 400);
    f.vault.set_dust_threshold(&f.admin, &1_000);
    f.vault.set_dust_keeper_tip_bps(&f.admin, &1_000);

    assert_eq!(
        f.vault.try_set_dust_keeper_tip_bps(&f.admin, &1_001),
        Err(Ok(VaultSafetyError::InvalidTipBps))
    );

    assert_eq!(f.vault.close_dust_position(&f.keeper, &f.alice), 450);
    assert_eq!(f.token.balance(&f.alice), 450);
    assert_eq!(f.token.balance(&f.keeper), 50);

    let treasury = Address::generate(&f.env);
    f.vault.set_dust_treasury(&f.admin, &Some(treasury.clone()));
    assert_eq!(f.vault.close_dust_position(&f.keeper, &f.bob), 360);
    assert_eq!(f.token.balance(&treasury), 360);
    assert_eq!(f.token.balance(&f.bob), 0);
    assert_eq!(f.token.balance(&f.keeper), 50 + 40);
}
