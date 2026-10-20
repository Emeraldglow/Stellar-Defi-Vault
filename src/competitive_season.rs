//! Competitive staking seasons.
//!
//! Resets competitive standing on a periodic cycle instead of leaving a
//! single static leaderboard permanently dominated by early large stakers.
//! An admin opens a season with a duration and prize pool; once its window
//! elapses, `end_season()` snapshots the top 10 stakers by current stake,
//! records a winner, and archives the season permanently.
//!
//! Named `CompetitiveSeason` (rather than the issue's literal `Season`) to
//! avoid colliding with the existing `Season` type in `storage.rs`, which is
//! an unrelated calendar reward-rate window (issue #276).
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so season state is kept under
//! raw `Symbol`-keyed storage, matching the pattern already established in
//! `balance.rs` / `price_oracle.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Number of top stakers snapshotted at season end.
pub const TOP_N: u32 = 10;
/// Number of past seasons retained in `get_season_history()`.
const MAX_HISTORY: u32 = 20;

const CURRENT_SEASON_KEY: Symbol = symbol_short!("cur_seas");
const SEASON_HISTORY_KEY: Symbol = symbol_short!("seas_hst");
const NEXT_SEASON_ID_KEY: Symbol = symbol_short!("seas_nid");

/// A single competitive season, open or archived.
///
/// `winner` uses `Vec<Address>` (0 or 1 elements) rather than `Option<Address>`
/// since `Option<Address>` is not usable inside a `#[contracttype]` struct in
/// soroban-sdk 21.x testutils mode (same constraint noted on `LotteryConfig`
/// in storage.rs).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CompetitiveSeason {
    pub season_id: u32,
    pub started_at: u32,
    pub ended_at: u32,
    pub winner: Vec<Address>,
    pub prize_amount: i128,
    pub top_10: Vec<(Address, i128)>,
}

/// Ranks all stakers by current share balance and returns the top `n`,
/// highest first.
fn top_stakers(env: &Env, n: u32) -> Vec<(Address, i128)> {
    let all_stakers = balance::get_all_stakers(env);
    let mut ranked: Vec<(Address, i128)> = Vec::new(env);

    for staker in all_stakers.iter() {
        let amount = balance::get_shares(env, &staker);
        if amount <= 0 {
            continue;
        }

        let mut inserted = false;
        for i in 0..ranked.len() {
            let (_, existing_amount) = ranked.get(i).unwrap();
            if amount > existing_amount {
                ranked.insert(i, (staker.clone(), amount));
                inserted = true;
                break;
            }
        }
        if !inserted {
            ranked.push_back((staker.clone(), amount));
        }
        while ranked.len() > n {
            ranked.pop_back();
        }
    }

    ranked
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Starts a new competitive season lasting `duration_ledgers`, with
    /// `prize_amount` set aside for the eventual winner. Admin only.
    pub fn start_season(
        env: Env,
        admin: Address,
        duration_ledgers: u32,
        prize_amount: i128,
    ) -> Result<u32, VaultError> {
        let stored_admin = crate::admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        if duration_ledgers == 0 {
            return Err(VaultError::ZeroAmount);
        }
        if prize_amount < 0 {
            return Err(VaultError::ZeroAmount);
        }

        let season_id: u32 = env
            .storage()
            .instance()
            .get(&NEXT_SEASON_ID_KEY)
            .unwrap_or(0);
        let started_at = env.ledger().sequence();

        let season = CompetitiveSeason {
            season_id,
            started_at,
            ended_at: started_at + duration_ledgers,
            winner: Vec::new(&env),
            prize_amount,
            top_10: Vec::new(&env),
        };

        env.storage().instance().set(&CURRENT_SEASON_KEY, &season);
        env.storage()
            .instance()
            .set(&NEXT_SEASON_ID_KEY, &(season_id + 1));

        env.events().publish(
            (symbol_short!("seas_str"), season_id),
            (started_at, season.ended_at, prize_amount),
        );
        Ok(season_id)
    }

    /// Read-only lookup of the currently open season, if any.
    pub fn get_current_season(env: Env) -> Option<CompetitiveSeason> {
        env.storage().instance().get(&CURRENT_SEASON_KEY)
    }

    /// Ends the current season once its ledger window has elapsed:
    /// snapshots the top 10 stakers by current stake, records the winner,
    /// and archives the season to permanent history. Admin only.
    pub fn end_season(env: Env, admin: Address) -> Result<CompetitiveSeason, VaultError> {
        let stored_admin = crate::admin::get_admin(&env)?;
        if admin != stored_admin {
            return Err(VaultError::Unauthorized);
        }
        admin.require_auth();

        let mut season: CompetitiveSeason = env
            .storage()
            .instance()
            .get(&CURRENT_SEASON_KEY)
            .ok_or(VaultError::NotInitialized)?;

        let current_ledger = env.ledger().sequence();
        if current_ledger < season.ended_at {
            return Err(VaultError::EpochNotFinalized);
        }

        let ranked = top_stakers(&env, TOP_N);
        season.top_10 = ranked.clone();
        season.winner = match ranked.get(0) {
            Some((winner, _)) => {
                let mut v = Vec::new(&env);
                v.push_back(winner);
                v
            }
            None => Vec::new(&env),
        };
        season.ended_at = current_ledger;

        let mut history: Vec<CompetitiveSeason> = env
            .storage()
            .persistent()
            .get(&SEASON_HISTORY_KEY)
            .unwrap_or(Vec::new(&env));
        while history.len() >= MAX_HISTORY {
            history.remove(0);
        }
        history.push_back(season.clone());
        env.storage().persistent().set(&SEASON_HISTORY_KEY, &history);
        env.storage().instance().remove(&CURRENT_SEASON_KEY);

        env.events().publish(
            (symbol_short!("seas_end"), season.season_id),
            season.prize_amount,
        );
        Ok(season)
    }

    /// Archived seasons, oldest first (most recent `MAX_HISTORY` retained).
    pub fn get_season_history(env: Env) -> Vec<CompetitiveSeason> {
        env.storage()
            .persistent()
            .get(&SEASON_HISTORY_KEY)
            .unwrap_or(Vec::new(&env))
    }
}















