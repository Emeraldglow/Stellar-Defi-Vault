//! Coordinated stake/unstake pattern detector (issue #406).
//!
//! Heuristically flags groups of addresses whose staking activity is
//! suspiciously correlated in timing and amount â€” a signal of possible
//! governance manipulation, wash trading, or Sybil attacks. Flagging is
//! purely advisory: it never blocks a stake, unstake, or claim, and false
//! positives are expected â€” the alerts exist for admin review, not
//! enforcement.
//!
//! # Data source
//!
//! There is no dedicated stake/unstake action log on `main` (`StakeAction`/
//! `StakeHistoryEntry` in `storage.rs` are declared but never populated).
//! What *is* populated is `balance::get_stake_history`, a per-user
//! `Vec<(ledger, amount)>` of position snapshots recorded across recent
//! stake/unstake activity (also what `vote_weight_at` reads). This module
//! scans that: each user's most recent snapshot is treated as their latest
//! activity event for cross-address coordination detection, and a user's
//! last few snapshots are used to detect wash-pattern self-correlation.
//! Since the snapshot doesn't carry a stake-vs-unstake direction, every
//! cross-address match is classified as `CoordinatedStake`.
//! `CollusionPattern::CoordinatedUnstake` is kept in the enum for when a
//! directional activity log exists to drive it.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! instance storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;
use crate::vault::{ MAX_GINI_STAKERS};

/// Instance-storage key for the rolling alert list.
const ALERTS_KEY: Symbol = symbol_short!("cld_alrt");

/// Max rolling alerts retained; oldest is evicted once full.
const MAX_ALERTS: u32 = 20;

/// Window, in ledgers, within which 3+ addresses staking/unstaking similar
/// amounts is flagged as coordinated.
const COORDINATION_WINDOW_LEDGERS: u32 = 1_000;

/// Amount similarity tolerance for coordination detection, in basis points
/// (500 = 5%).
const COORDINATION_TOLERANCE_BPS: i128 = 500;

/// Minimum distinct addresses for a coordinated-pattern alert.
const COORDINATION_MIN_ADDRESSES: u32 = 3;

/// Window, in ledgers, within which 3+ activity events from the same address
/// is flagged as a wash pattern.
const WASH_WINDOW_LEDGERS: u32 = 10_000;

/// Minimum activity events for a wash-pattern alert.
const WASH_MIN_EVENTS: u32 = 3;

/// The kind of coordinated behavior a `CollusionAlert` was raised for.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CollusionPattern {
    CoordinatedStake,
    CoordinatedUnstake,
    WashPattern,
}

/// A raised collusion alert, retained for admin review.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CollusionAlert {
    pub addresses: Vec<Address>,
    pub pattern_type: CollusionPattern,
    pub detected_at: u32,
    pub total_coordinated_amount: i128,
}

fn get_alerts(env: &Env) -> Vec<CollusionAlert> {
    env.storage()
        .instance()
        .get(&ALERTS_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_alerts(env: &Env, alerts: &Vec<CollusionAlert>) {
    env.storage().instance().set(&ALERTS_KEY, alerts);
}

/// Append `alert` to the rolling list, evicting the oldest entry once full.
fn push_alert(env: &Env, alerts: &mut Vec<CollusionAlert>, alert: CollusionAlert) {
    if alerts.len() >= MAX_ALERTS {
        alerts.remove(0);
    }
    alerts.push_back(alert);
}

/// Whether `a` and `b` are within `COORDINATION_TOLERANCE_BPS` of each other.
fn within_amount_tolerance(a: i128, b: i128) -> bool {
    let diff = (a - b).abs();
    let base = a.max(b);
    if base <= 0 {
        return a == b;
    }
    diff.saturating_mul(10_000) <= base.saturating_mul(COORDINATION_TOLERANCE_BPS)
}

/// One user's latest recorded staking activity, if any.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct LatestActivity {
    staker: Address,
    ledger: u32,
    amount: i128,
}

/// Scan for coordinated cross-address activity and same-address wash
/// patterns, and return any newly-raised alerts (also persisted into the
/// rolling alert list). Bounded to the first `MAX_GINI_STAKERS` registered
/// stakers, matching `get_reward_gini_coefficient`'s scan bound (issue
/// #275) â€” this is a heuristic, best-effort scan, not an exhaustive audit.
fn scan_for_collusion(env: &Env) -> Vec<CollusionAlert> {
    let all_stakers = balance::get_all_stakers(env);
    let scan_len = all_stakers.len().min(MAX_GINI_STAKERS);
    let now = env.ledger().sequence();

    let mut new_alerts: Vec<CollusionAlert> = Vec::new(env);
    let mut latest_events: Vec<LatestActivity> = Vec::new(env);

    // â”€â”€ wash pattern: same address, 3+ activity snapshots within the window â”€â”€
    for i in 0..scan_len {
        let staker = all_stakers.get(i).unwrap();
        let Some(history) = balance::get_stake_history(env, &staker) else {
            continue;
        };
        if history.is_empty() {
            continue;
        }

        let last_idx = history.len() - 1;
        let (last_ledger, last_amount) = history.get(last_idx).unwrap();
        latest_events.push_back(LatestActivity {
            staker: staker.clone(),
            ledger: last_ledger,
            amount: last_amount,
        });

        if history.len() >= WASH_MIN_EVENTS {
            let window_start_idx = history.len() - WASH_MIN_EVENTS;
            let (oldest_ledger, _) = history.get(window_start_idx).unwrap();
            if last_ledger.saturating_sub(oldest_ledger) <= WASH_WINDOW_LEDGERS {
                let mut total: i128 = 0;
                let mut j = window_start_idx;
                while j < history.len() {
                    let (_, amt) = history.get(j).unwrap();
                    total = total.saturating_add(amt);
                    j += 1;
                }
                let mut addresses = Vec::new(env);
                addresses.push_back(staker.clone());
                new_alerts.push_back(CollusionAlert {
                    addresses,
                    pattern_type: CollusionPattern::WashPattern,
                    detected_at: now,
                    total_coordinated_amount: total,
                });
            }
        }
    }

    // â”€â”€ coordinated stake/unstake: 3+ addresses, similar time + amount â”€â”€â”€â”€â”€â”€
    let n = latest_events.len();
    let mut used = Vec::new(env);
    for _ in 0..n {
        used.push_back(false);
    }

    for i in 0..n {
        if used.get(i).unwrap() {
            continue;
        }
        let anchor = latest_events.get(i).unwrap();

        let mut group_addresses: Vec<Address> = Vec::new(env);
        group_addresses.push_back(anchor.staker.clone());
        let mut group_total: i128 = anchor.amount;
        let mut group_indices: Vec<u32> = Vec::new(env);
        group_indices.push_back(i as u32);

        let mut j = i + 1;
        while j < n {
            if !used.get(j).unwrap() {
                let candidate = latest_events.get(j).unwrap();
                let ledger_diff = if candidate.ledger >= anchor.ledger {
                    candidate.ledger - anchor.ledger
                } else {
                    anchor.ledger - candidate.ledger
                };
                if ledger_diff <= COORDINATION_WINDOW_LEDGERS
                    && within_amount_tolerance(anchor.amount, candidate.amount)
                {
                    group_addresses.push_back(candidate.staker.clone());
                    group_total = group_total.saturating_add(candidate.amount);
                    group_indices.push_back(j as u32);
                }
            }
            j += 1;
        }

        if group_addresses.len() >= COORDINATION_MIN_ADDRESSES {
            for idx in group_indices.iter() {
                used.set(idx, true);
            }
            new_alerts.push_back(CollusionAlert {
                addresses: group_addresses,
                pattern_type: CollusionPattern::CoordinatedStake,
                detected_at: now,
                total_coordinated_amount: group_total,
            });
        }
    }

    new_alerts
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Scan recent staking activity for coordinated stake/unstake and wash
    /// patterns. Admin only. Any newly-detected alerts are appended to the
    /// rolling alert list (max 20, oldest evicted) and returned; each also
    /// emits a `collusion_alert_raised` event.
    pub fn check_collusion(env: Env) -> Result<Vec<CollusionAlert>, VaultError> {
        admin::require_admin(&env)?;

        let found = scan_for_collusion(&env);
        if found.is_empty() {
            return Ok(found);
        }

        let mut alerts = get_alerts(&env);
        for alert in found.iter() {
            push_alert(&env, &mut alerts, alert.clone());
            env.events().publish(
                (symbol_short!("cld_flag"),),
                (
                    alert.pattern_type,
                    alert.addresses.len(),
                    alert.total_coordinated_amount,
                    alert.detected_at,
                ),
            );
        }
        set_alerts(&env, &alerts);

        Ok(found)
    }

    /// Read-only query: the current rolling list of raised collusion alerts
    /// (max 20, oldest evicted). Admin only.
    pub fn get_collusion_alerts(env: Env) -> Result<Vec<CollusionAlert>, VaultError> {
        admin::require_admin(&env)?;
        Ok(get_alerts(&env))
    }

    /// Dismiss a raised alert as a false positive. Admin only.
    pub fn dismiss_alert(env: Env, index: u32) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let mut alerts = get_alerts(&env);
        if index >= alerts.len() {
            return Err(VaultError::InvalidRate);
        }
        alerts.remove(index);
        set_alerts(&env, &alerts);

        env.events().publish(
            (symbol_short!("cld_dsms"),),
            (index, env.ledger().sequence()),
        );
        Ok(())
    }
}










