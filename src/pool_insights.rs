//! Pool-wide read-only insights.
//!
//! Two aggregators that let dashboards render a pool overview and document the
//! vault's rounding behaviour without issuing a round-trip per metric.
//!
//! # Storage
//!
//! Both entrypoints are pure views over existing state — no new storage keys,
//! no auth, no state changes.

use soroban_sdk::{contractimpl, contracttype, Env};

use crate::balance;
use crate::vault::{VaultContract, VaultContractClient};

/// Aggregated, pool-wide statistics returned by [`VaultContract::get_pool_summary`].
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolSummary {
    /// Total principal currently tracked as deposited (stake-token units).
    pub total_deposited: i128,
    /// Number of registered depositors/stakers.
    pub depositor_count: u32,
    /// Current annual reward rate in basis points.
    pub current_rate_bps: u32,
    /// Pool utilisation in basis points: `total_deposited / pool_cap * 10_000`.
    /// Zero when no pool cap is configured.
    pub utilization_bps: u32,
    /// Tokens held in the reward pool available to pay future rewards.
    pub reward_pool_balance: i128,
}

/// Direction that sub-unit division rounds in a given operation.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RoundingDirection {
    /// Fractional units are dropped (floor / truncation toward zero).
    Down,
    /// Fractional units are rounded up (ceiling).
    Up,
}

/// Documents the rounding direction used by each core share/token conversion.
/// Purely descriptive — it does not change behaviour.
///
/// Named `PoolRoundingPolicy` rather than `RoundingPolicy` because the existing
/// issue-#220 `RoundingPolicy` enum (Floor/Ceiling/Nearest) already occupies
/// that type name; a second UDT with the same name would collide in the
/// contract spec.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolRoundingPolicy {
    /// `deposit`: `amount -> shares` (`amount * total_shares / total_deposited`).
    pub deposit: RoundingDirection,
    /// `withdraw`/`unstake`: `shares -> amount` (`shares * total_deposited / total_shares`).
    pub withdraw: RoundingDirection,
    /// `preview_redeem`: same conversion as `withdraw`.
    pub preview_redeem: RoundingDirection,
}

#[contractimpl]
impl VaultContract {
    /// Read-only aggregate of the pool's headline metrics in a single call.
    /// No auth required; performs no state changes.
    pub fn get_pool_summary(env: Env) -> PoolSummary {
        let total_deposited = balance::get_total_deposited(&env);
        let pool_cap = balance::get_pool_cap(&env);
        let utilization_bps = if pool_cap > 0 && total_deposited > 0 {
            let scaled = total_deposited
                .checked_mul(10_000)
                .and_then(|v| v.checked_div(pool_cap))
                .unwrap_or(0);
            if scaled > u32::MAX as i128 {
                u32::MAX
            } else if scaled < 0 {
                0
            } else {
                scaled as u32
            }
        } else {
            0
        };

        PoolSummary {
            total_deposited,
            depositor_count: balance::get_total_stakers(&env),
            current_rate_bps: balance::get_reward_rate_bps(&env),
            utilization_bps,
            reward_pool_balance: balance::get_reward_pool_balance(&env),
        }
    }

    /// Read-only documentation of the rounding direction used by `deposit`,
    /// `withdraw`/`unstake`, and `preview_redeem`. No auth required; performs
    /// no state changes.
    ///
    /// All three conversions use integer division on non-negative operands, so
    /// every step truncates toward zero (rounds **down**). That deliberately
    /// favours the pool over the individual staker by at most one sub-unit, and
    /// any dust is retained as pool value rather than minted or paid out.
    pub fn get_rounding_policy(_env: Env) -> PoolRoundingPolicy {
        PoolRoundingPolicy {
            deposit: RoundingDirection::Down,
            withdraw: RoundingDirection::Down,
            preview_redeem: RoundingDirection::Down,
        }
    }
}
