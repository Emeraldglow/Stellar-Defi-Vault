//! Predicts when the pool will reach its TVL capacity cap, based on recent
//! stake inflow (issue #402).

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

const INFLOW_KEY: Symbol = symbol_short!("cuf_infl");
const LAST_WARN_KEY: Symbol = symbol_short!("cuf_warn");

const MAX_INFLOW_ENTRIES: u32 = 100;
/// 7 days at ~5s/ledger (matches `vault::LEDGERS_PER_DAY` * 7).
const SEVEN_DAY_LEDGERS: u32 = 120_960;
const LEDGERS_PER_DAY: u32 = 17_280;
const WARNING_DAYS_THRESHOLD: u32 = 7;
const WARNING_COOLDOWN_LEDGERS: u32 = LEDGERS_PER_DAY;

/// Forecast returned by `get_capacity_forecast`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CapacityForecast {
    pub current_tvl: i128,
    pub pool_cap: i128,
    pub remaining_capacity: i128,
    pub days_until_full: Option<u32>,
}

fn get_inflow(env: &Env) -> Vec<(u32, i128)> {
    env.storage()
        .instance()
        .get(&INFLOW_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_inflow(env: &Env, inflow: &Vec<(u32, i128)>) {
    env.storage().instance().set(&INFLOW_KEY, inflow);
}

fn sum_7day_inflow(env: &Env) -> i128 {
    let inflow = get_inflow(env);
    let cutoff = env.ledger().sequence().saturating_sub(SEVEN_DAY_LEDGERS);

    let mut total: i128 = 0;
    let n = inflow.len();
    let mut i = 0u32;
    while i < n {
        let (ledger, amount) = inflow.get(i).unwrap();
        if ledger >= cutoff {
            total = total.saturating_add(amount);
        }
        i += 1;
    }
    total
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Record a stake inflow entry for the rolling 7-day forecast window.
    /// Requires the staker's own auth, matching `stake()`'s auth model.
    ///
    /// # Wiring
    /// There is no hook into the existing `stake()` to call this
    /// automatically, so â€” matching the same documented gap
    /// `governance_power_decay.rs` leaves for its own entrypoints â€” this is
    /// the call a modified `stake()` would make; it is directly callable and
    /// tested on its own in the meantime.
    pub fn record_stake_inflow(env: Env, user: Address, amount: i128) -> Result<(), VaultError> {
        user.require_auth();
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let mut inflow = get_inflow(&env);
        inflow.push_back((env.ledger().sequence(), amount));

        if inflow.len() > MAX_INFLOW_ENTRIES {
            inflow.remove(0);
        }

        set_inflow(&env, &inflow);
        Ok(())
    }

    /// Sum of recorded stake inflow within the last 7 days (120 960 ledgers).
    pub fn get_7day_stake_inflow(env: Env) -> i128 {
        sum_7day_inflow(&env)
    }

    /// Average daily stake inflow over the last 7 days: `7day_inflow / 7`.
    pub fn get_daily_stake_rate(env: Env) -> i128 {
        sum_7day_inflow(&env) / 7
    }

    /// Predicts when the pool will reach its TVL cap based on the recent
    /// stake inflow trend. `days_until_full` is `None` when no cap is set
    /// or the daily inflow rate is zero. Emits `capacity_warning` (at most
    /// once per `LEDGERS_PER_DAY`) when the forecast is under 7 days out.
    pub fn get_capacity_forecast(env: Env) -> CapacityForecast {
        let current_tvl = balance::get_total_deposited(&env);
        let pool_cap = balance::get_pool_cap(&env);
        let daily_rate = sum_7day_inflow(&env) / 7;

        let remaining_capacity = if pool_cap > 0 {
            (pool_cap - current_tvl).max(0)
        } else {
            0
        };

        let days_until_full = if pool_cap <= 0 || daily_rate <= 0 {
            None
        } else {
            Some((remaining_capacity / daily_rate) as u32)
        };

        if let Some(days) = days_until_full {
            if days < WARNING_DAYS_THRESHOLD {
                let current_ledger = env.ledger().sequence();
                let last_warned: u32 = env.storage().instance().get(&LAST_WARN_KEY).unwrap_or(0);
                if current_ledger.saturating_sub(last_warned) >= WARNING_COOLDOWN_LEDGERS {
                    env.storage()
                        .instance()
                        .set(&LAST_WARN_KEY, &current_ledger);
                    env.events().publish(
                        (symbol_short!("cap_warn"),),
                        (days, current_tvl, pool_cap, current_ledger),
                    );
                }
            }
        }

        CapacityForecast {
            current_tvl,
            pool_cap,
            remaining_capacity,
            days_until_full,
        }
    }
}















