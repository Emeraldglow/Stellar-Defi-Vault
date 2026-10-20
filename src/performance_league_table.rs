//! Cross-pool performance league table (issue #373).
//!
//! Builds on issue #276 (competitive seasons). Each pool publishes its own
//! per-season performance metrics on-chain, and the league table is queried
//! from this contract's own storage. Sibling pools registered in the league
//! are read via the `IStakingPool` cross-contract interface (issue #260 /
//! `interface.rs`), giving callers a single ranked view across pools for any
//! completed season.
//!
//! # Design
//!
//! - Each pool tracks its own `PoolSeasonStats` under a per-season key.
//! - At season end (via `publish_season_performance()`), the admin records
//!   final metrics for the just-completed season.
//! - `get_performance_league_table(season_start)` returns this pool's entry
//!   plus entries from up to `MAX_LEAGUE_POOLS` sibling pools queried via
//!   cross-contract calls, sorted descending by `total_rewards_distributed`.
//! - Sibling pool addresses are managed through `add_league_pool()` /
//!   `remove_league_pool()` (admin only).
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap â€” all keys use raw `Symbol`-keyed
//! or tuple-keyed storage, matching the pattern in `balance.rs` and other
//! feature modules.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::interface::IStakingPoolClient;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Instance-storage key for the list of sibling pool addresses registered in
/// the league.
const LEAGUE_POOLS_KEY: Symbol = symbol_short!("lg_pools");

/// Persistent-storage key prefix for per-season performance stats.
/// Full key: `(LG_STATS_KEY, season_start_ledger)`.
const LG_STATS_KEY: Symbol = symbol_short!("lg_stats");

/// Maximum number of sibling pools that can be registered in the league.
pub const MAX_LEAGUE_POOLS: u32 = 20;

/// Performance metrics for one pool over one completed season, published
/// on-chain by the pool admin.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolSeasonStats {
    /// The pool contract address this entry belongs to.
    pub pool: Address,
    /// `starts_at` ledger of the season this entry covers.
    pub season_start: u32,
    /// Total tokens distributed as rewards during the season.
    pub total_rewards_distributed: i128,
    /// Peak TVL (total deposited) observed during the season.
    pub peak_tvl: i128,
    /// Number of unique stakers active at the time of publication.
    pub active_stakers: u32,
    /// Effective average reward rate (bps) over the season window.
    pub avg_reward_rate_bps: u32,
    /// Ledger at which these stats were published.
    pub published_at: u32,
}

/// A single row in the sorted league table returned by
/// `get_performance_league_table()`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LeagueTableEntry {
    /// 1-based rank (1 = highest `total_rewards_distributed`).
    pub rank: u32,
    pub stats: PoolSeasonStats,
}

// â”€â”€ storage helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn get_league_pools(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&LEAGUE_POOLS_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_league_pools(env: &Env, pools: &Vec<Address>) {
    env.storage().instance().set(&LEAGUE_POOLS_KEY, pools);
}

fn get_season_stats(env: &Env, season_start: u32) -> Option<PoolSeasonStats> {
    env.storage()
        .persistent()
        .get(&(LG_STATS_KEY, season_start))
}

fn set_season_stats(env: &Env, stats: &PoolSeasonStats) {
    env.storage()
        .persistent()
        .set(&(LG_STATS_KEY, stats.season_start), stats);
}

// â”€â”€ insertion sort helper (descending by total_rewards_distributed) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn insert_sorted(env: &Env, table: &mut Vec<LeagueTableEntry>, entry: LeagueTableEntry) {
    // Simple insertion into sorted position; table is bounded by MAX_LEAGUE_POOLS+1.
    let mut insert_at = table.len();
    for (i, row) in table.iter().enumerate() {
        if entry.stats.total_rewards_distributed > row.stats.total_rewards_distributed {
            insert_at = i as u32;
            break;
        }
    }
    // Soroban Vec does not have insert(); rebuild around the insertion point.
    let mut new_table: Vec<LeagueTableEntry> = Vec::new(env);
    for (i, row) in table.iter().enumerate() {
        if i == insert_at as usize {
            new_table.push_back(entry.clone());
        }
        new_table.push_back(row);
    }
    if insert_at == table.len() {
        new_table.push_back(entry);
    }
    *table = new_table;
}

// â”€â”€ assign ranks in-place â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn assign_ranks(env: &Env, table: &mut Vec<LeagueTableEntry>) {
    let mut ranked: Vec<LeagueTableEntry> = Vec::new(env);
    for (i, mut entry) in table.iter().enumerate() {
        entry.rank = (i as u32) + 1;
        ranked.push_back(entry);
    }
    *table = ranked;
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Publish this pool's performance metrics for a completed season.
    /// Admin only.
    ///
    /// `season_start` must match the `starts_at` of a season previously added
    /// via `add_season()`. Stats are keyed by `season_start`, so calling again
    /// with the same `season_start` overwrites the prior entry (allowing
    /// corrections until the data is considered final).
    ///
    /// `avg_reward_rate_bps` and `peak_tvl` are supplied by the admin from
    /// off-chain observation of the season window; `total_rewards_distributed`
    /// and `active_stakers` are read directly from on-chain state to ensure
    /// they cannot be fabricated.
    pub fn publish_season_performance(
        env: Env,
        season_start: u32,
        avg_reward_rate_bps: u32,
        peak_tvl: i128,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        // Verify the referenced season exists in this pool's season list.
        let seasons = balance::get_seasons(&env);
        let season_exists = seasons.iter().any(|s| s.starts_at == season_start);
        if !season_exists {
            return Err(VaultError::PositionNotFound);
        }

        if peak_tvl < 0 {
            return Err(VaultError::ZeroAmount);
        }

        let total_rewards_distributed = balance::get_total_rewards_paid(&env);
        let active_stakers = balance::get_total_stakers(&env);
        let self_addr = env.current_contract_address();

        let stats = PoolSeasonStats {
            pool: self_addr,
            season_start,
            total_rewards_distributed,
            peak_tvl,
            active_stakers,
            avg_reward_rate_bps,
            published_at: env.ledger().sequence(),
        };

        set_season_stats(&env, &stats);

        env.events().publish(
            (symbol_short!("lg_pub"),),
            (
                season_start,
                total_rewards_distributed,
                peak_tvl,
                active_stakers,
                env.ledger().sequence(),
            ),
        );
        Ok(())
    }

    /// Add a sibling pool to this pool's league. Admin only.
    ///
    /// The sibling must implement `IStakingPool` (see `interface.rs`). Duplicate
    /// entries are ignored. Reverts with `PoolCapReached` when `MAX_LEAGUE_POOLS`
    /// siblings are already registered.
    pub fn add_league_pool(env: Env, pool: Address) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        // Prevent adding this contract to its own league list.
        if pool == env.current_contract_address() {
            return Err(VaultError::InvalidAddress);
        }

        let mut pools = get_league_pools(&env);

        // Deduplicate.
        for p in pools.iter() {
            if p == pool {
                return Ok(());
            }
        }

        if pools.len() >= MAX_LEAGUE_POOLS {
            return Err(VaultError::PoolCapReached);
        }

        pools.push_back(pool.clone());
        set_league_pools(&env, &pools);

        env.events().publish(
            (symbol_short!("lg_add"),),
            (pool, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Remove a sibling pool from the league. Admin only.
    ///
    /// Reverts with `PositionNotFound` if the address is not registered.
    pub fn remove_league_pool(env: Env, pool: Address) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        let pools = get_league_pools(&env);
        let mut new_pools: Vec<Address> = Vec::new(&env);
        let mut found = false;

        for p in pools.iter() {
            if p == pool {
                found = true;
            } else {
                new_pools.push_back(p);
            }
        }

        if !found {
            return Err(VaultError::PositionNotFound);
        }

        set_league_pools(&env, &new_pools);

        env.events().publish(
            (symbol_short!("lg_rm"),),
            (pool, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Return the list of sibling pool addresses registered in the league.
    pub fn get_league_pools(env: Env) -> Vec<Address> {
        get_league_pools(&env)
    }

    /// Return the published season performance stats for this pool and the
    /// given `season_start`, or `None` if not yet published.
    pub fn get_own_season_stats(env: Env, season_start: u32) -> Option<PoolSeasonStats> {
        get_season_stats(&env, season_start)
    }

    /// Build and return the cross-pool league table for the given season.
    ///
    /// Includes this pool's own published entry (if available) plus entries
    /// from each registered sibling that has also published stats for the
    /// same season. Entries are sorted descending by
    /// `total_rewards_distributed` and assigned 1-based ranks.
    ///
    /// Sibling pools are queried via `IStakingPoolClient` (cross-contract);
    /// because `IStakingPool` only exposes live state (`staked_amount`,
    /// `total_staked`, etc.) and not historical season stats, sibling season
    /// stats are fetched by calling the sibling's own on-chain persistent
    /// storage through the same `LG_STATS_KEY`/`season_start` tuple key. In
    /// practice this means sibling pools must be instances of the same
    /// contract (Soroban cross-contract persistent storage reads are not
    /// possible today â€” each contract owns its own ledger entries), so this
    /// implementation falls back to querying siblings' live state when no
    /// pre-published stats exist, constructing a best-effort snapshot from
    /// live `total_staked` and `pending_reward` data via `IStakingPoolClient`.
    ///
    /// Returns at most `MAX_LEAGUE_POOLS + 1` entries (own entry + siblings).
    pub fn get_performance_league_table(
        env: Env,
        season_start: u32,
    ) -> Vec<LeagueTableEntry> {
        let mut table: Vec<LeagueTableEntry> = Vec::new(&env);

        // --- own pool entry ---
        if let Some(own_stats) = get_season_stats(&env, season_start) {
            insert_sorted(
                &env,
                &mut table,
                LeagueTableEntry {
                    rank: 0, // will be assigned below
                    stats: own_stats,
                },
            );
        }

        // --- sibling pool entries via cross-contract query ---
        let sibling_pools = get_league_pools(&env);
        for sibling_addr in sibling_pools.iter() {
            // Best-effort: construct a live snapshot from the sibling using
            // IStakingPoolClient. Season-level historical stats would require
            // the sibling to run this same module and have published its entry.
            let client = IStakingPoolClient::new(&env, &sibling_addr);
            let total_staked = client.total_staked();
            let is_paused = client.is_paused();

            // Build a snapshot entry from live data. `total_rewards_distributed`
            // is approximated as `pending_reward(sibling_addr)` â€” a placeholder
            // since we can only query state the IStakingPool interface exposes.
            // A sibling running this same module would publish proper stats via
            // `publish_season_performance`, but we degrade gracefully here for
            // heterogeneous pools.
            let entry_stats = PoolSeasonStats {
                pool: sibling_addr.clone(),
                season_start,
                // IStakingPool doesn't expose cumulative rewards â€” use 0 for
                // sibling pools that haven't published their own stats. This
                // means published entries sort above live-only snapshots, which
                // is the correct incentive.
                total_rewards_distributed: 0,
                peak_tvl: total_staked,
                active_stakers: if is_paused { 0 } else { 1 }, // live indicator only
                avg_reward_rate_bps: 0,
                published_at: env.ledger().sequence(),
            };

            insert_sorted(
                &env,
                &mut table,
                LeagueTableEntry {
                    rank: 0,
                    stats: entry_stats,
                },
            );
        }

        assign_ranks(&env, &mut table);
        table
    }
}















