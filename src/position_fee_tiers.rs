//! Tiered fee discounts for large depositors (issue #520).
//!
//! Lets the admin configure a small ladder of position-size tiers, each
//! granting a discount (in basis points) off the pool's unstake fee. Larger,
//! stickier positions pay a lower effective fee, incentivizing bigger
//! deposits and discouraging churn.
//!
//! This contract currently only charges a fee on `unstake` (there is no
//! separate deposit fee to discount), so the tier ladder is applied there.
//!
//! # Wiring
//!
//! `vault.rs`'s real unstake flow (`do_unstake`) is private to that module
//! and applies the flat `balance::get_unstake_fee_bps()` fee. Rather than
//! editing it, this exposes its own `unstake_with_tier_discount` entrypoint
//! — mirroring `do_unstake`'s share/token accounting — that substitutes the
//! caller's tier-discounted fee. To keep that mirror honest about scope: fee
//! revenue routing here is simplified to crediting the reward pool directly
//! (skipping `community_treasury`/fee-recipient distribution/buyback, all of
//! which are private wiring inside `do_unstake` itself).
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::{VaultError, VaultFeature3Error};
use crate::storage::DataKey;
use crate::vault::VaultContract;

/// Most fee tiers `set_fee_tiers()` accepts at once.
pub const MAX_FEE_TIERS: u32 = 5;

const TIERS_KEY: Symbol = symbol_short!("fee_tiers");

/// One rung of the tier ladder: positions at or above `min_position_amount`
/// (in stake-token units) get `discount_bps` off the pool's unstake fee.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct FeeTier {
    pub min_position_amount: i128,
    pub discount_bps: u32,
}

fn get_tiers(env: &Env) -> Vec<FeeTier> {
    env.storage()
        .instance()
        .get(&TIERS_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_tiers(env: &Env, tiers: &Vec<FeeTier>) {
    env.storage().instance().set(&TIERS_KEY, tiers);
}

/// The best (highest `min_position_amount`) qualifying tier's discount, `0`
/// if none qualify or no tiers are configured.
fn best_discount_bps(env: &Env, position_amount: i128) -> u32 {
    let tiers = get_tiers(env);
    let mut best_min = -1i128;
    let mut best_discount = 0u32;
    for tier in tiers.iter() {
        if position_amount >= tier.min_position_amount && tier.min_position_amount > best_min {
            best_min = tier.min_position_amount;
            best_discount = tier.discount_bps;
        }
    }
    best_discount
}

fn position_amount_of(env: &Env, user: &Address) -> i128 {
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    let shares = balance::get_shares(env, user);
    balance::shares_to_amount(total_shares, total_deposited, shares).unwrap_or(0)
}

#[contractimpl]
impl VaultContract {
    /// Configure the fee-discount tier ladder. Admin only. Replaces any
    /// previously configured tiers. Order does not matter — the highest
    /// qualifying `min_position_amount` always wins.
    pub fn set_fee_tiers(env: Env, tiers: Vec<FeeTier>) -> Result<(), VaultFeature3Error> {
        admin::require_admin(&env).map_err(VaultFeature3Error::from)?;

        if tiers.len() > MAX_FEE_TIERS {
            return Err(VaultFeature3Error::TooManyFeeTiers);
        }
        for tier in tiers.iter() {
            if tier.discount_bps > 10_000 || tier.min_position_amount < 0 {
                return Err(VaultFeature3Error::InvalidFeeTierConfig);
            }
        }

        crate::position_fee_tiers::set_tiers(&env, &tiers);
        env.events()
            .publish((symbol_short!("fee_tier"),), (tiers.len(), env.ledger().sequence()));
        Ok(())
    }

    /// The currently configured fee-discount tiers.
    pub fn get_fee_tiers(env: Env) -> Vec<FeeTier> {
        crate::position_fee_tiers::get_tiers(&env)
    }

    /// The unstake fee, in basis points, `user` would actually pay right now
    /// after applying their best-qualifying tier discount.
    pub fn get_effective_unstake_fee_bps(env: Env, user: Address) -> u32 {
        let base_bps = balance::get_unstake_fee_bps(&env) as i128;
        let position_amount = crate::position_fee_tiers::position_amount_of(&env, &user);
        let discount_bps = crate::position_fee_tiers::best_discount_bps(&env, position_amount) as i128;

        let effective = base_bps
            .saturating_mul(10_000i128.saturating_sub(discount_bps).max(0))
            / 10_000;
        effective.max(0) as u32
    }

    /// Unstake `shares`, charging the caller's tier-discounted fee instead of
    /// the pool's flat unstake fee. See the module doc for the scope note on
    /// simplified fee routing versus `unstake()`. Returns the gross token
    /// amount the shares converted to (matching `unstake()`'s own return
    /// convention), before the fee was deducted.
    pub fn unstake_with_tier_discount(
        env: Env,
        staker: Address,
        shares: i128,
    ) -> Result<i128, VaultFeature3Error> {
        staker.require_auth();

        if shares <= 0 {
            return Err(VaultFeature3Error::ZeroAmount);
        }
        let user_shares = balance::get_shares(&env, &staker);
        if user_shares < shares {
            return Err(VaultFeature3Error::InsufficientShares);
        }

        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let amount = balance::shares_to_amount(total_shares, total_deposited, shares)
            .ok_or(VaultFeature3Error::ArithmeticError)?;

        let position_amount = crate::position_fee_tiers::position_amount_of(&env, &staker);
        let discount_bps =
            crate::position_fee_tiers::best_discount_bps(&env, position_amount) as i128;
        let base_bps = balance::get_unstake_fee_bps(&env) as i128;
        let effective_bps = base_bps
            .saturating_mul(10_000i128.saturating_sub(discount_bps).max(0))
            / 10_000;

        let fee = amount
            .checked_mul(effective_bps)
            .and_then(|v| v.checked_div(10_000))
            .ok_or(VaultFeature3Error::ArithmeticError)?;
        let payout = amount
            .checked_sub(fee)
            .ok_or(VaultFeature3Error::ArithmeticError)?;

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(VaultFeature3Error::NotInitialized)?;
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &staker,
            &payout,
        );

        balance::set_shares(&env, &staker, user_shares - shares);
        balance::set_total_shares(&env, total_shares - shares);
        balance::set_total_deposited(&env, total_deposited - amount);

        if fee > 0 {
            let reward_pool = balance::get_reward_pool_balance(&env);
            balance::set_reward_pool_balance(&env, reward_pool + fee);
        }

        env.events().publish(
            (symbol_short!("tier_uns"), staker),
            (amount, fee, discount_bps, env.ledger().sequence()),
        );
        Ok(amount)
    }
}
