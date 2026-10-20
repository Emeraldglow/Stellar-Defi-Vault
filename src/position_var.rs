//! Position Value-at-Risk estimate (issue #408).
//!
//! A deterministic scenario analysis â€” not a statistical VaR in the
//! probabilistic sense â€” that quantifies a staking position's exposure to
//! four adverse scenarios: unstaking before the lock-up expires, worst-case
//! slashing, the reward opportunity cost of exiting early, and a
//! caller-supplied reward-token price drop.

use soroban_sdk::{contractimpl, contracttype, Address, Env};

use crate::balance;
use crate::storage::DataKey;
use crate::VaultContract;
use crate::vault::{STELLAR_LEDGERS_PER_YEAR};
use crate::vault::VaultContractClient;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VaRReport {
    pub position_amount: i128,
    pub early_exit_loss: i128,
    pub max_slash_exposure: i128,
    pub lock_opportunity_cost: i128,
    pub reward_price_drop_impact: i128,
    pub total_var_bps: u32,
}

impl VaRReport {
    fn zero() -> Self {
        VaRReport {
            position_amount: 0,
            early_exit_loss: 0,
            max_slash_exposure: 0,
            lock_opportunity_cost: 0,
            reward_price_drop_impact: 0,
            total_var_bps: 0,
        }
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Estimate potential loss for `user`'s position under adverse
    /// conditions (issue #408). `reward_price_drop_bps` is caller-supplied â€”
    /// this contract has no price oracle of its own.
    ///
    /// Read-only: no auth required, no state changes. Returns a
    /// zero-valued report if the user has no active position.
    pub fn get_position_var(env: Env, user: Address, reward_price_drop_bps: u32) -> VaRReport {
        let shares = balance::get_shares(&env, &user);
        if shares <= 0 {
            return VaRReport::zero();
        }

        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let position_amount =
            balance::shares_to_amount(total_shares, total_deposited, shares).unwrap_or(shares);

        // Worst case: the entire position is slashed.
        let max_slash_exposure = position_amount;

        // Lock-up state, used for both `early_exit_loss` and
        // `lock_opportunity_cost`.
        let lock_period: u32 = env.storage().instance().get(&DataKey::LockPeriod).unwrap_or(0);
        let staked_at: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StakedAtLedger(user.clone()))
            .unwrap_or(0);
        let current_ledger = env.ledger().sequence();
        let unlock_ledger = staked_at.saturating_add(lock_period);
        let still_locked = lock_period > 0 && current_ledger < unlock_ledger;

        // Penalty amount if the user unstakes right now, from the
        // early-exit penalty config. Zero if already unlocked.
        let early_exit_loss = if still_locked {
            let penalty_bps: u32 = env
                .storage()
                .instance()
                .get(&DataKey::EarlyExitPenaltyBps)
                .unwrap_or(0);
            (position_amount * penalty_bps as i128) / 10_000
        } else {
            0
        };

        // Rewards foregone by unstaking now instead of waiting for the lock
        // to expire, estimated from the pool's current reward rate over the
        // remaining locked ledgers.
        let lock_opportunity_cost = if still_locked {
            let remaining_ledgers = (unlock_ledger - current_ledger) as i128;
            let reward_rate_bps = balance::get_reward_rate_bps(&env) as i128;
            position_amount
                .saturating_mul(reward_rate_bps)
                .saturating_mul(remaining_ledgers)
                / (10_000i128.saturating_mul(STELLAR_LEDGERS_PER_YEAR as i128))
        } else {
            0
        };

        // Impact of a caller-supplied reward-token price drop on the
        // position's currently pending (unclaimed) reward.
        let pending_reward = balance::get_accrued_reward(&env, &user);
        let reward_price_drop_impact =
            (pending_reward * reward_price_drop_bps as i128) / 10_000;

        let total_var_bps = if position_amount > 0 {
            (((early_exit_loss + reward_price_drop_impact) * 10_000) / position_amount)
                .clamp(0, u32::MAX as i128) as u32
        } else {
            0
        };

        VaRReport {
            position_amount,
            early_exit_loss,
            max_slash_exposure,
            lock_opportunity_cost,
            reward_price_drop_impact,
            total_var_bps,
        }
    }
}















