//! Lockdrop campaign (issue #460).
//!
//! A one-time event where users lock the stake token for a chosen duration and
//! receive a share of a fixed reward pool proportional to a time-weighted
//! score: `score = locked_amount * lock_duration_ledgers`. Locking for twice as
//! long earns twice the allocation for the same amount.
//!
//! Distinct from regular staking: lockdrop deposits are held in their own
//! per-user commitment records, are fully locked until
//! `committed_at + lock_duration_ledgers`, and never mint vault shares.
//!
//! Only one campaign exists at a time. `exit_lockdrop()` (unlock principal) and
//! `claim_lockdrop_reward()` (collect the reward allocation) are independent.
//!
//! # Storage (`DataKey` is at Soroban's 50-variant cap — raw `Symbol` keys)
//!
//! - Campaign config: `symbol_short!("ldrp_cfg")` -> `LockdropConfig`
//! - Sum of all scores: `symbol_short!("ldrp_scr")` -> `i128`
//! - Committer list: `symbol_short!("ldrp_usr")` -> `Vec<Address>`
//! - Per-user commitment: `(Symbol::new(env, "ldrp_cmt"), user)` -> `LockdropCommitment`
//! - Per-user reward allocation: `(Symbol::new(env, "ldrp_alc"), user)` -> `i128`
//! - Per-user claimed flag: `(Symbol::new(env, "ldrp_clm"), user)` -> `bool`

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, Vec};

use crate::admin;
use crate::errors::VaultCampaignError;
use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient};

const CONFIG_KEY: Symbol = symbol_short!("ldrp_cfg");
const TOTAL_SCORE_KEY: Symbol = symbol_short!("ldrp_scr");
const COMMITTERS_KEY: Symbol = symbol_short!("ldrp_usr");

/// Upper bound on distinct committers so `finalize_lockdrop()` stays within a
/// bounded loop.
pub const MAX_COMMITTERS: u32 = 500;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LockdropConfig {
    pub total_reward_pool: i128,
    pub max_lock_ledgers: u32,
    pub ends_at: u32,
    pub finalized: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LockdropCommitment {
    pub locked_amount: i128,
    pub lock_duration_ledgers: u32,
    pub score: i128,
    pub committed_at: u32,
}

fn commitment_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "ldrp_cmt"), user.clone())
}

fn allocation_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "ldrp_alc"), user.clone())
}

fn claimed_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "ldrp_clm"), user.clone())
}

pub fn get_config(env: &Env) -> Option<LockdropConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

fn set_config(env: &Env, config: &LockdropConfig) {
    env.storage().instance().set(&CONFIG_KEY, config);
}

pub fn get_total_score(env: &Env) -> i128 {
    env.storage().instance().get(&TOTAL_SCORE_KEY).unwrap_or(0)
}

fn set_total_score(env: &Env, score: i128) {
    env.storage().instance().set(&TOTAL_SCORE_KEY, &score);
}

fn get_committers(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&COMMITTERS_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_committers(env: &Env, committers: &Vec<Address>) {
    env.storage().instance().set(&COMMITTERS_KEY, committers);
}

pub fn get_commitment(env: &Env, user: &Address) -> Option<LockdropCommitment> {
    env.storage().persistent().get(&commitment_key(env, user))
}

pub fn get_allocation(env: &Env, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&allocation_key(env, user))
        .unwrap_or(0)
}

fn is_claimed(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&claimed_key(env, user))
        .unwrap_or(false)
}

fn token_address(env: &Env) -> Result<Address, VaultCampaignError> {
    env.storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(VaultCampaignError::NotInitialized)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #460: admin starts the lockdrop. Funds the fixed reward pool by
    /// transferring `total_reward_pool` of the stake token from the admin into
    /// the contract. Reverts if a non-finalized campaign already exists.
    pub fn start_lockdrop(
        env: Env,
        admin_addr: Address,
        total_reward_pool: i128,
        max_lock_ledgers: u32,
        duration_ledgers: u32,
    ) -> Result<(), VaultCampaignError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        if let Some(cfg) = get_config(&env) {
            if !cfg.finalized {
                return Err(VaultCampaignError::LockdropAlreadyActive);
            }
        }
        if total_reward_pool <= 0 {
            return Err(VaultCampaignError::ZeroAmount);
        }
        if max_lock_ledgers == 0 || duration_ledgers == 0 {
            return Err(VaultCampaignError::InvalidLockDuration);
        }

        let token_addr = token_address(&env)?;
        token::Client::new(&env, &token_addr).transfer(
            &admin_addr,
            &env.current_contract_address(),
            &total_reward_pool,
        );

        let ends_at = env.ledger().sequence().saturating_add(duration_ledgers);
        set_config(
            &env,
            &LockdropConfig {
                total_reward_pool,
                max_lock_ledgers,
                ends_at,
                finalized: false,
            },
        );
        set_total_score(&env, 0);
        set_committers(&env, &Vec::new(&env));

        env.events().publish(
            (symbol_short!("ldrp_srt"),),
            (total_reward_pool, max_lock_ledgers, ends_at),
        );
        Ok(())
    }

    /// Issue #460: user locks `amount` of the stake token for
    /// `lock_duration_ledgers`. Tokens are locked until
    /// `committed_at + lock_duration_ledgers`. Returns the commitment score.
    pub fn commit_to_lockdrop(
        env: Env,
        user: Address,
        amount: i128,
        lock_duration_ledgers: u32,
    ) -> Result<i128, VaultCampaignError> {
        user.require_auth();

        let cfg = get_config(&env).ok_or(VaultCampaignError::LockdropNotActive)?;
        if cfg.finalized {
            return Err(VaultCampaignError::LockdropAlreadyFinalized);
        }
        let current_ledger = env.ledger().sequence();
        if current_ledger > cfg.ends_at {
            return Err(VaultCampaignError::LockdropEnded);
        }
        if amount <= 0 {
            return Err(VaultCampaignError::ZeroAmount);
        }
        if lock_duration_ledgers == 0 || lock_duration_ledgers > cfg.max_lock_ledgers {
            return Err(VaultCampaignError::InvalidLockDuration);
        }
        if get_commitment(&env, &user).is_some() {
            return Err(VaultCampaignError::AlreadyCommitted);
        }

        let mut committers = get_committers(&env);
        if committers.len() >= MAX_COMMITTERS {
            return Err(VaultCampaignError::LockdropFull);
        }

        let token_addr = token_address(&env)?;
        token::Client::new(&env, &token_addr).transfer(
            &user,
            &env.current_contract_address(),
            &amount,
        );

        let score = amount
            .checked_mul(lock_duration_ledgers as i128)
            .ok_or(VaultCampaignError::ArithmeticError)?;

        env.storage().persistent().set(
            &commitment_key(&env, &user),
            &LockdropCommitment {
                locked_amount: amount,
                lock_duration_ledgers,
                score,
                committed_at: current_ledger,
            },
        );
        committers.push_back(user.clone());
        set_committers(&env, &committers);
        set_total_score(&env, get_total_score(&env).saturating_add(score));

        env.events().publish(
            (symbol_short!("ldrp_cmt"), user),
            (amount, lock_duration_ledgers, score),
        );
        Ok(score)
    }

    /// Issue #460: admin finalizes the campaign after `ends_at`, splitting the
    /// reward pool across committers in proportion to their score share.
    pub fn finalize_lockdrop(env: Env, admin_addr: Address) -> Result<(), VaultCampaignError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        let mut cfg = get_config(&env).ok_or(VaultCampaignError::LockdropNotActive)?;
        if cfg.finalized {
            return Err(VaultCampaignError::LockdropAlreadyFinalized);
        }
        if env.ledger().sequence() <= cfg.ends_at {
            return Err(VaultCampaignError::LockdropNotEnded);
        }

        let total_score = get_total_score(&env);
        let committers = get_committers(&env);
        if total_score > 0 {
            for user in committers.iter() {
                if let Some(commitment) = get_commitment(&env, &user) {
                    let allocation = cfg
                        .total_reward_pool
                        .saturating_mul(commitment.score)
                        / total_score;
                    env.storage()
                        .persistent()
                        .set(&allocation_key(&env, &user), &allocation);
                }
            }
        }

        cfg.finalized = true;
        set_config(&env, &cfg);

        env.events().publish(
            (symbol_short!("ldrp_fin"),),
            (cfg.total_reward_pool, total_score, committers.len()),
        );
        Ok(())
    }

    /// Issue #460: claim the proportional reward after the campaign is
    /// finalized. Independent of `exit_lockdrop()`.
    pub fn claim_lockdrop_reward(
        env: Env,
        user: Address,
    ) -> Result<i128, VaultCampaignError> {
        user.require_auth();

        let cfg = get_config(&env).ok_or(VaultCampaignError::LockdropNotActive)?;
        if !cfg.finalized {
            return Err(VaultCampaignError::LockdropNotFinalized);
        }
        if get_commitment(&env, &user).is_none() {
            return Err(VaultCampaignError::CommitmentNotFound);
        }
        if is_claimed(&env, &user) {
            return Err(VaultCampaignError::AlreadyClaimed);
        }

        let allocation = get_allocation(&env, &user);
        if allocation <= 0 {
            return Err(VaultCampaignError::NothingToClaim);
        }

        let token_addr = token_address(&env)?;
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &user,
            &allocation,
        );
        env.storage()
            .persistent()
            .set(&claimed_key(&env, &user), &true);

        env.events()
            .publish((symbol_short!("ldrp_clm"), user), allocation);
        Ok(allocation)
    }

    /// Issue #460: unlock and withdraw the locked principal once the caller's
    /// chosen lock duration has elapsed. Separate from the reward claim.
    pub fn exit_lockdrop(env: Env, user: Address) -> Result<i128, VaultCampaignError> {
        user.require_auth();

        let commitment =
            get_commitment(&env, &user).ok_or(VaultCampaignError::CommitmentNotFound)?;
        let unlock_at = commitment
            .committed_at
            .saturating_add(commitment.lock_duration_ledgers);
        if env.ledger().sequence() < unlock_at {
            return Err(VaultCampaignError::LockStillActive);
        }

        let token_addr = token_address(&env)?;
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &user,
            &commitment.locked_amount,
        );
        env.storage()
            .persistent()
            .remove(&commitment_key(&env, &user));

        env.events()
            .publish((symbol_short!("ldrp_ext"), user), commitment.locked_amount);
        Ok(commitment.locked_amount)
    }

    /// Issue #460: read the current campaign config.
    pub fn get_lockdrop_config(env: Env) -> Option<LockdropConfig> {
        get_config(&env)
    }

    /// Issue #460: read a user's commitment.
    pub fn get_lockdrop_commitment(env: Env, user: Address) -> Option<LockdropCommitment> {
        get_commitment(&env, &user)
    }

    /// Issue #460: read a user's finalized reward allocation (0 before finalize).
    pub fn get_lockdrop_allocation(env: Env, user: Address) -> i128 {
        get_allocation(&env, &user)
    }

    /// Issue #460: sum of all commitment scores in the campaign.
    pub fn get_lockdrop_total_score(env: Env) -> i128 {
        get_total_score(&env)
    }
}
