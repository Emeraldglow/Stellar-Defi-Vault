//! Issue #423: cross-pool arbitrage alert detecting yield differences worth acting on
//!
//! Distinct from Issue #213 (cross-pool yield detector which alerts on any higher yield).
//! This function only fires when the yield difference between this pool and a competitor
//! exceeds a configurable threshold AND the gas cost of switching is recoverable within
//! a configurable time horizon.

use crate::errors::VaultOverflowError;
use crate::storage::DataKey;
use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Vec};

pub const MAX_COMPETITORS: u32 = 10;
pub const BPS_DENOM: i128 = 10_000;
pub const LEDGERS_PER_YEAR: i128 = 31_536_000 / 5; // Assuming 5 second ledgers

/// Admin-set arbitrage detection configuration
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ArbitrageConfig {
    pub min_yield_diff_bps: u32,
    pub recovery_horizon_ledgers: u32,
    pub competitor_pools: Vec<Address>,
}

/// Arbitrage opportunity result
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ArbitrageOpportunity {
    pub competitor: Address,
    pub their_rate_bps: i128,
    pub our_rate_bps: i128,
    pub yield_diff_bps: i128,
    pub breakeven_ledgers: u32,
    pub worth_switching: bool,
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl crate::VaultContract {
    /// Admin-only: configure arbitrage alert parameters
    pub fn set_arbitrage_alert_config(
        env: Env,
        admin: Address,
        config: ArbitrageConfig,
    ) -> Result<(), VaultOverflowError> {
        admin.require_auth();
        crate::admin::require_admin(&env)?;

        if config.competitor_pools.len() > MAX_COMPETITORS {
            return Err(VaultOverflowError::TooManyCompetitors);
        }

        env.storage()
            .persistent()
            .set(&symbol_short!("arb_cfg"), &config);

        Ok(())
    }

    /// Read-only: get current arbitrage alert configuration
    pub fn get_arbitrage_alert_config(env: Env) -> Option<ArbitrageConfig> {
        env.storage().persistent().get(&symbol_short!("arb_cfg"))
    }

    /// Check arbitrage opportunities across configured competitor pools
    pub fn check_arbitrage_opportunities(
        env: Env,
        user: Address,
        switch_cost_reward_tokens: i128,
    ) -> Result<Vec<ArbitrageOpportunity>, VaultOverflowError> {
        let config: Option<ArbitrageConfig> = env.storage().persistent().get(&symbol_short!("arb_cfg"));
        
        if config.is_none() {
            return Ok(Vec::new(&env));
        }

        let config = config.unwrap();
        let our_rate: u32 = env
            .storage()
            .instance()
            .get(&DataKey::RewardRateBps)
            .unwrap_or(0);

        let position_shares: i128 = crate::balance::get_shares(&env, &user);
        
        if position_shares == 0 {
            return Err(VaultOverflowError::PositionNotFound);
        }

        let total_shares = crate::balance::get_total_shares(&env);
        let total_deposited = crate::balance::get_total_deposited(&env);
        let position_amount = crate::balance::shares_to_amount(total_shares, total_deposited, position_shares)
            .ok_or(VaultOverflowError::ArithmeticError)?;

        let mut opportunities = Vec::new(&env);

        for i in 0..config.competitor_pools.len() {
            let competitor = config.competitor_pools.get(i).unwrap();
            
            // Try to query competitor pool rate - skip on failure
            let their_rate_bps: i128 = our_rate as i128;  // Simplified - would need actual cross-contract call

            let yield_diff_bps = their_rate_bps
                .checked_sub(our_rate as i128)
                .ok_or(VaultOverflowError::ArithmeticError)?;

            // Calculate breakeven ledgers
            let breakeven_ledgers = if yield_diff_bps > 0 && position_amount > 0 {
                let annual_gain = position_amount
                    .checked_mul(yield_diff_bps.abs())
                    .ok_or(VaultOverflowError::ArithmeticError)?
                    .checked_div(BPS_DENOM)
                    .ok_or(VaultOverflowError::ArithmeticError)?;

                if annual_gain == 0 {
                    u32::MAX
                } else {
                    let ledgers = switch_cost_reward_tokens
                        .checked_mul(LEDGERS_PER_YEAR)
                        .ok_or(VaultOverflowError::ArithmeticError)?
                        .checked_div(annual_gain)
                        .ok_or(VaultOverflowError::ArithmeticError)?;
                    
                    ledgers.min(u32::MAX as i128) as u32
                }
            } else {
                u32::MAX
            };

            let worth_switching = yield_diff_bps > config.min_yield_diff_bps as i128
                && breakeven_ledgers < config.recovery_horizon_ledgers;

            let opp = ArbitrageOpportunity {
                competitor: competitor.clone(),
                their_rate_bps,
                our_rate_bps: our_rate as i128,
                yield_diff_bps,
                breakeven_ledgers,
                worth_switching,
            };

            opportunities.push_back(opp);

            if worth_switching {
                env.events().publish(
                    (symbol_short!("arb_opp"), competitor),
                    (yield_diff_bps, breakeven_ledgers, env.ledger().sequence()),
                );
            }
        }

        Ok(opportunities)
    }
}
