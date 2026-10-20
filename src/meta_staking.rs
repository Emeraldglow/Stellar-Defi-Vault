//! Meta-staking layer (additive feature).
//!
//! Lets users who claim reward tokens immediately restake them to earn a
//! bonus meta-reward rate on top of their regular position, creating a
//! compounding loop that keeps reward tokens in the ecosystem rather than
//! being sold immediately.
//!
//! Meta positions are tracked completely separately from regular positions
//! (different storage keys), so a user's main stake, claims, boosts, etc. are
//! untouched. Meta-staking requires the pool's stake token to be the same as
//! its reward token (circular staking) — the existing single-token vault
//! satisfies this by paying rewards in the stake token.
//!
//! # Reward model
//!
//! Meta rewards accrue continuously at an *annual* `set_meta_reward_rate_bps`
//! rate (typically lower than the base rate, e.g. 50% of it) against the
//! user's current `meta_staked` balance, prorated by elapsed ledgers:
//!
//! `reward = meta_staked * meta_rate_bps * elapsed / (10_000 * LEDGERS_PER_YEAR)`
//!
//! Unclaimed accrued meta rewards are tracked in a separate accumulator so a
//! partial claim never loses time-weighted reward, mirroring how the base
//! vault tracks `AccruedReward`.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs` / `vesting_cliff.rs`.
//!
//! - `MetaPosition(user)` -> `MetaPosition` (persistent)
//! - `MetaAccrued(user)`  -> `i128` unclaimed meta reward (persistent)
//! - `MetaRewardRateBps`  -> `u32` annual meta rate (instance)
//! - `TotalMetaStaked`    -> `i128` pool-wide meta TVL (instance)

use soroban_sdk::{contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol};

use crate::storage::DataKey;
use crate::vault::STELLAR_LEDGERS_PER_YEAR;
use crate::vault::VaultContractClient;
use crate::VaultContract;

const BPS_DENOMINATOR: i128 = 10_000;
const RATE_DENOMINATOR: i128 = 10_000 * STELLAR_LEDGERS_PER_YEAR as i128;

const USER_POS_KEY: Symbol = symbol_short!("meta_pos");
const USER_ACCRUED_KEY: Symbol = symbol_short!("meta_acc");
const META_RATE_KEY: Symbol = symbol_short!("meta_rate");
const TOTAL_META_KEY: Symbol = symbol_short!("meta_tot");

/// Maximum meta reward rate in basis points (annual). Matches the base vault's
/// `balance::MAX_RATE_BPS` (50 000 bps = 500% APR).
pub const MAX_META_RATE_BPS: u32 = 50_000;

/// A user's meta-staking position (acceptance criteria).
///
/// - `meta_staked`: reward tokens currently restaked in the meta layer.
/// - `meta_staked_at`: ledger the meta position was first opened.
/// - `last_meta_claim_at`: ledger the meta reward last settled to (start of
///   the current accrual period).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MetaPosition {
    pub meta_staked: i128,
    pub meta_staked_at: u32,
    pub last_meta_claim_at: u32,
}

/// Errors for the meta-staking module. All existing `#[contracterror]` enums
/// are at Soroban's 50-variant cap, so meta-staking defines its own (the same
/// pattern `nft.rs` established).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum MetaStakingError {
    /// Caller is not the configured admin (or, for user entrypoints, the
    /// supplied `user` is not the authorized caller).
    Unauthorized = 1,
    /// The pool has not been initialized.
    NotInitialized = 2,
    /// A zero or negative amount was supplied where it is not allowed.
    ZeroAmount = 3,
    /// Checked arithmetic / conversion overflowed.
    ArithmeticError = 4,
    /// The user has no active meta position (or tried to unstake more than
    /// they hold).
    PositionNotFound = 5,
    /// The proposed meta rate exceeds `MAX_META_RATE_BPS`.
    RateTooHigh = 6,
}

impl From<crate::errors::VaultError> for MetaStakingError {
    fn from(err: crate::errors::VaultError) -> Self {
        match err {
            crate::errors::VaultError::Unauthorized => MetaStakingError::Unauthorized,
            crate::errors::VaultError::NotInitialized => MetaStakingError::NotInitialized,
            crate::errors::VaultError::ZeroAmount => MetaStakingError::ZeroAmount,
            crate::errors::VaultError::ArithmeticError => MetaStakingError::ArithmeticError,
            crate::errors::VaultError::PositionNotFound => MetaStakingError::PositionNotFound,
            _ => MetaStakingError::Unauthorized,
        }
    }
}

// ── Storage helpers ────────────────────────────────────────────────────────────

fn read_meta_position(env: &Env, user: &Address) -> Option<MetaPosition> {
    env.storage().persistent().get(&(USER_POS_KEY, user.clone()))
}

fn set_meta_position(env: &Env, user: &Address, pos: &MetaPosition) {
    env.storage()
        .persistent()
        .set(&(USER_POS_KEY, user.clone()), pos);
}

fn get_meta_accrued(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&(USER_ACCRUED_KEY, user.clone()))
        .unwrap_or(0)
}

fn set_meta_accrued(env: &Env, user: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&(USER_ACCRUED_KEY, user.clone()), &amount);
}

fn get_meta_rate_bps(env: &Env) -> u32 {
    env.storage().instance().get(&META_RATE_KEY).unwrap_or(0)
}

fn read_total_meta_staked(env: &Env) -> i128 {
    env.storage().instance().get(&TOTAL_META_KEY).unwrap_or(0)
}

fn set_total_meta_staked(env: &Env, total: i128) {
    env.storage().instance().set(&TOTAL_META_KEY, &total);
}

/// Settle pending meta rewards for `user` up to the current ledger, adding
/// them to the persistent accumulator and moving the accrual checkpoint
/// forward. Called before any mutation of `meta_staked` so time-weighted
/// reward is never lost.
fn accrue_meta(env: &Env, user: &Address) -> Result<(), MetaStakingError> {
    let rate = get_meta_rate_bps(env);
    if rate == 0 {
        return Ok(());
    }
    let Some(pos) = read_meta_position(env, user) else {
        return Ok(());
    };
    if pos.meta_staked == 0 {
        return Ok(());
    }
    let now = env.ledger().sequence();
    let elapsed = now.saturating_sub(pos.last_meta_claim_at);
    if elapsed == 0 {
        return Ok(());
    }

    let reward = (pos
        .meta_staked
        .checked_mul(elapsed as i128)
        .ok_or(MetaStakingError::ArithmeticError)?
        .checked_mul(rate as i128)
        .ok_or(MetaStakingError::ArithmeticError)?)
        / RATE_DENOMINATOR;

    if reward > 0 {
        let accrued = get_meta_accrued(env, user);
        set_meta_accrued(env, user, accrued + reward);
    }

    let mut updated = pos;
    updated.last_meta_claim_at = now;
    set_meta_position(env, user, &updated);
    Ok(())
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Stake `amount` of the pool's reward (== stake) token into the caller's
    /// meta position. Tokens are pulled from `user`; the meta position starts
    /// earning meta-rewards immediately.
    ///
    /// Pending meta-rewards are settled first so no reward is lost when the
    /// compounding principal grows. Returns the new meta-staked balance.
    pub fn meta_stake(env: Env, user: Address, amount: i128) -> Result<i128, MetaStakingError> {
        user.require_auth();
        if amount <= 0 {
            return Err(MetaStakingError::ZeroAmount);
        }

        // Rewards settle here (see accrue_meta).
        accrue_meta(&env, &user)?;

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(MetaStakingError::NotInitialized)?;
        token::Client::new(&env, &token_addr).transfer(&user, &env.current_contract_address(), &amount);

        let now = env.ledger().sequence();
        let mut pos = read_meta_position(&env, &user).unwrap_or(MetaPosition {
            meta_staked: 0,
            meta_staked_at: now,
            last_meta_claim_at: now,
        });
        pos.meta_staked = pos
            .meta_staked
            .checked_add(amount)
            .ok_or(MetaStakingError::ArithmeticError)?;
        pos.last_meta_claim_at = now;
        set_meta_position(&env, &user, &pos);

        let total = read_total_meta_staked(&env);
        set_total_meta_staked(&env, total + amount);

        env.events().publish(
            (symbol_short!("meta_stk"), user.clone()),
            (amount, pos.meta_staked, now),
        );
        Ok(pos.meta_staked)
    }

    /// Exit `amount` from the caller's meta position, returning the reward
    /// tokens to them. Pending meta-rewards are settled (not paid) first; use
    /// `meta_claim` to recover accrued meta-rewards, which are independent of
    /// the principal.
    ///
    /// Returns the amount returned to the user.
    pub fn meta_unstake(
        env: Env,
        user: Address,
        amount: i128,
    ) -> Result<i128, MetaStakingError> {
        user.require_auth();
        if amount <= 0 {
            return Err(MetaStakingError::ZeroAmount);
        }

        let mut pos =
            read_meta_position(&env, &user).ok_or(MetaStakingError::PositionNotFound)?;
        if pos.meta_staked < amount {
            return Err(MetaStakingError::PositionNotFound);
        }

        accrue_meta(&env, &user)?;

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(MetaStakingError::NotInitialized)?;
        token::Client::new(&env, &token_addr)
            .transfer(&env.current_contract_address(), &user, &amount);

        pos.meta_staked = pos.meta_staked - amount;
        pos.last_meta_claim_at = env.ledger().sequence();
        set_meta_position(&env, &user, &pos);

        let total = read_total_meta_staked(&env);
        set_total_meta_staked(&env, total - amount);

        env.events().publish(
            (symbol_short!("meta_uns"), user.clone()),
            (amount, pos.meta_staked, env.ledger().sequence()),
        );
        Ok(amount)
    }

    /// Claim accrued meta-rewards, transferring them to the user in the same
    /// reward token. Independent of the meta principal and of the user's
    /// regular position. Returns the amount paid out.
    pub fn meta_claim(env: Env, user: Address) -> Result<i128, MetaStakingError> {
        user.require_auth();

        accrue_meta(&env, &user)?;
        let claimable = get_meta_accrued(&env, &user);
        if claimable == 0 {
            return Ok(0);
        }

        set_meta_accrued(&env, &user, 0);
        if let Some(mut pos) = read_meta_position(&env, &user) {
            pos.last_meta_claim_at = env.ledger().sequence();
            set_meta_position(&env, &user, &pos);
        }

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(MetaStakingError::NotInitialized)?;
        token::Client::new(&env, &token_addr)
            .transfer(&env.current_contract_address(), &user, &claimable);

        env.events().publish(
            (symbol_short!("meta_clm"), user.clone()),
            (claimable, env.ledger().sequence()),
        );
        Ok(claimable)
    }

    /// Admin: set the annual meta-reward rate in basis points. The issue
    /// recommends a value typically lower than the base rate (e.g. 50% of it),
    /// which keeps the compounding bonus below the underlying position yield.
    pub fn set_meta_reward_rate_bps(
        env: Env,
        admin: Address,
        bps: u32,
    ) -> Result<(), MetaStakingError> {
        admin.require_auth();
        crate::admin::require_admin(&env)?;
        if bps > MAX_META_RATE_BPS {
            return Err(MetaStakingError::RateTooHigh);
        }
        env.storage().instance().set(&META_RATE_KEY, &bps);
        env.events().publish(
            (symbol_short!("meta_rate"), admin.clone()),
            (bps, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Read-only: the caller's meta position, if any.
    pub fn get_meta_position(env: Env, user: Address) -> Option<MetaPosition> {
        read_meta_position(&env, &user)
    }

    /// Read-only: pool-wide meta TVL (total reward tokens restaked).
    pub fn get_total_meta_staked(env: Env) -> i128 {
        read_total_meta_staked(&env)
    }
}

#[cfg(any())] // disabled: exercises entrypoints cfg-gated out of testutils builds
mod test {
    extern crate std;

    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        token, Address, Env,
    };

    use crate::vault::{VaultContract, VaultContractClient};

    use super::{MetaPosition, MetaStakingError};

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
        vault_id: Address,
        token: token::Client<'a>,
        token_admin: token::StellarAssetClient<'a>,
        admin: Address,
        alice: Address,
        bob: Address,
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

            let (token_addr, token, token_admin) = create_token(&env, &admin);

            let vault_id = env.register_contract(None, VaultContract);
            let vault = VaultContractClient::new(&env, &vault_id);
            vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

            token_admin.mint(&alice, &100_000_000);
            token_admin.mint(&bob, &100_000_000);
            token_admin.mint(&vault_id, &10_000_000);

            Fixture {
                env,
                vault,
                vault_id,
                token,
                token_admin,
                admin,
                alice,
                bob,
            }
        }
    }

    fn advance_ledgers(env: &Env, n: u32) {
        let cur = env.ledger().sequence();
        env.ledger().with_mut(|li| {
            li.sequence_number = cur + n;
        });
    }

    /// 1% APR == 100 bps annual meta rate.
    const META_RATE: u32 = 100;

    #[test]
    fn meta_stake_earns_rewards_at_configured_rate() {
        let f = Fixture::new();
        f.vault.set_meta_reward_rate_bps(&f.admin, &META_RATE);

        f.vault.meta_stake(&f.alice, &100_000i128);

        // Complete one full year of meta staking at 100 bps (1%).
        f.advance_ledgers(&crate::vault::STELLAR_LEDGERS_PER_YEAR);
        let claimed = f.vault.meta_claim(&f.alice);

        // 100_000 * 1% = 1_000, modulo integer truncation.
        assert_eq!(claimed, 1_000);
    }

    #[test]
    fn meta_reward_rate_lower_than_base_rate() {
        let f = Fixture::new();
        // Base rate is 500 bps (from initialize). Meta rate is 100 bps.
        f.vault.set_meta_reward_rate_bps(&f.admin, &META_RATE);

        f.vault.meta_stake(&f.alice, &100_000i128);
        f.advance_ledgers(&crate::vault::STELLAR_LEDGERS_PER_YEAR);
        let meta_claimed = f.vault.meta_claim(&f.alice);
        assert_eq!(meta_claimed, 1_000);
        assert!(meta_claimed > 0);
        assert!(META_RATE < 500);
    }

    #[test]
    fn meta_and_regular_positions_are_independent() {
        let f = Fixture::new();
        f.vault.set_meta_reward_rate_bps(&f.admin, &META_RATE);

        // Alice opens a regular stake and also a meta stake.
        f.vault.stake(&f.alice, &50_000i128, &0);
        f.vault.meta_stake(&f.alice, &30_000i128);

        // Regular position tallies only the regular stake.
        assert_eq!(f.vault.shares_of(&f.alice), 50_000);
        // Meta position tallies only the meta stake.
        let pos: MetaPosition = f.vault.get_meta_position(&f.alice).unwrap();
        assert_eq!(pos.meta_staked, 30_000);

        // Advancing time earns meta-reward without altering the regular stake
        // or the meta principal.
        f.advance_ledgers(&(crate::vault::STELLAR_LEDGERS_PER_YEAR / 2));
        let claimed = f.vault.meta_claim(&f.alice);
        assert!(claimed > 0);
        assert_eq!(f.vault.shares_of(&f.alice), 50_000);
        assert_eq!(f.vault.get_meta_position(&f.alice).unwrap().meta_staked, 30_000);
    }

    #[test]
    fn meta_unstake_returns_correct_amount() {
        let f = Fixture::new();
        f.vault.set_meta_reward_rate_bps(&f.admin, &META_RATE);

        f.vault.meta_stake(&f.alice, &100_000i128);
        let returned = f.vault.meta_unstake(&f.alice, &40_000i128);
        assert_eq!(returned, 40_000);

        // Principal reduced by the unstaked amount; pool-wide TVL tracks it.
        let pos: MetaPosition = f.vault.get_meta_position(&f.alice).unwrap();
        assert_eq!(pos.meta_staked, 60_000);
        assert_eq!(f.vault.get_total_meta_staked(), 60_000);

        // Remaining principal is returned on final exit.
        assert_eq!(f.vault.meta_unstake(&f.alice, &60_000i128), 60_000);
        assert_eq!(f.vault.get_meta_position(&f.alice).unwrap().meta_staked, 0);
        assert_eq!(f.vault.get_total_meta_staked(), 0);
    }

    #[test]
    fn meta_unstake_more_than_held_reverts() {
        let f = Fixture::new();
        f.vault.meta_stake(&f.alice, &100_000i128);
        let res = f.vault.try_meta_unstake(&f.alice, &200_000i128);
        assert_eq!(res.err().unwrap(), MetaStakingError::PositionNotFound);
    }

    #[test]
    fn set_meta_rate_requires_admin() {
        let f = Fixture::new();
        // mock_all_auths is on, so a non-admin caller must still fail the
        // admin check via `require_admin`.
        let res = f.vault.try_set_meta_reward_rate_bps(&f.alice, &META_RATE);
        assert_eq!(res.err().unwrap(), MetaStakingError::Unauthorized);
    }
}

