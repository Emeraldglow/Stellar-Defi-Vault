

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::balance;
use crate::errors::VaultError;
use crate::events;
use crate::VaultContract;
use crate::vault::VaultContractClient;

const CAP_CONFIG_KEY: Symbol = symbol_short!("epc_cap");
const TRACKER_KEY: Symbol = symbol_short!("epc_trk");
const DEFERRED_KEY: Symbol = symbol_short!("epc_dfr");

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EpochRewardCapConfig {
    pub cap_per_epoch: i128,
    pub epoch_ledgers: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EpochRewardTracker {
    pub epoch_start: u32,
    pub rewards_paid_this_epoch: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeferredReward {
    pub amount: i128,
    pub next_epoch_start: u32,
}

pub fn get_cap_config(env: &Env) -> Option<EpochRewardCapConfig> {
    env.storage().instance().get(&CAP_CONFIG_KEY)
}

fn set_cap_config(env: &Env, config: &EpochRewardCapConfig) {
    env.storage().instance().set(&CAP_CONFIG_KEY, config);
}

fn get_raw_tracker(env: &Env) -> Option<EpochRewardTracker> {
    env.storage().instance().get(&TRACKER_KEY)
}

fn set_tracker(env: &Env, tracker: &EpochRewardTracker) {
    env.storage().instance().set(&TRACKER_KEY, tracker);
}

pub fn get_deferred(env: &Env, user: &Address) -> Option<DeferredReward> {
    env.storage()
        .persistent()
        .get(&(DEFERRED_KEY, user.clone()))
}

fn set_deferred(env: &Env, user: &Address, deferred: &DeferredReward) {
    env.storage()
        .persistent()
        .set(&(DEFERRED_KEY, user.clone()), deferred);
}

fn remove_deferred(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&(DEFERRED_KEY, user.clone()));
}

/// Add `amount` to `user`'s deferred-reward bucket, claimable via
/// `claim_deferred_reward` once `next_epoch_start` is reached. Shared by any
/// claim path that needs to queue an overflow amount rather than lose it â€”
/// currently this module's own `claim_epoch_capped_reward` and
/// `minimum_reserve_ratio.rs`'s `claim_with_reserve_floor` (issue #405),
/// which reuses this same bucket per that issue's own notes.
pub(crate) fn queue_deferred(env: &Env, user: &Address, amount: i128, next_epoch_start: u32) {
    if amount <= 0 {
        return;
    }
    let existing = get_deferred(env, user).map(|d| d.amount).unwrap_or(0);
    set_deferred(
        env,
        user,
        &DeferredReward {
            amount: existing.saturating_add(amount),
            next_epoch_start,
        },
    );
    events::reward_deferred(env, user, amount, next_epoch_start);
}

/// The current tracker, rolling over to a fresh epoch window if the
/// configured `epoch_ledgers` window has elapsed since `epoch_start`.
fn current_tracker(env: &Env, config: &EpochRewardCapConfig) -> EpochRewardTracker {
    let now = env.ledger().sequence();
    match get_raw_tracker(env) {
        Some(tracker) if now.saturating_sub(tracker.epoch_start) < config.epoch_ledgers => tracker,
        _ => EpochRewardTracker {
            epoch_start: now,
            rewards_paid_this_epoch: 0,
        },
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Configure the per-epoch reward outflow cap. Admin only. Starts a fresh
    /// tracking window from the current ledger.
    pub fn set_epoch_reward_cap(
        env: Env,
        cap_per_epoch: i128,
        epoch_ledgers: u32,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        if cap_per_epoch < 0 || epoch_ledgers == 0 {
            return Err(VaultError::ZeroAmount);
        }

        crate::epoch_reward_cap::set_cap_config(
            &env,
            &EpochRewardCapConfig {
                cap_per_epoch,
                epoch_ledgers,
            },
        );
        crate::epoch_reward_cap::set_tracker(
            &env,
            &EpochRewardTracker {
                epoch_start: env.ledger().sequence(),
                rewards_paid_this_epoch: 0,
            },
        );
        Ok(())
    }

    /// The configured cap and epoch window length, as `(cap, epoch_ledgers)`.
    pub fn get_epoch_reward_cap(env: Env) -> Result<(i128, u32), VaultError> {
        let config =
            crate::epoch_reward_cap::get_cap_config(&env).ok_or(VaultError::NotInitialized)?;
        Ok((config.cap_per_epoch, config.epoch_ledgers))
    }

    /// Rewards paid out so far in the current epoch window.
    pub fn get_rewards_paid_this_epoch(env: Env) -> Result<i128, VaultError> {
        let config =
            crate::epoch_reward_cap::get_cap_config(&env).ok_or(VaultError::NotInitialized)?;
        Ok(crate::epoch_reward_cap::current_tracker(&env, &config).rewards_paid_this_epoch)
    }

    /// Cap headroom remaining in the current epoch window.
    pub fn get_epoch_cap_remaining(env: Env) -> Result<i128, VaultError> {
        let config =
            crate::epoch_reward_cap::get_cap_config(&env).ok_or(VaultError::NotInitialized)?;
        let tracker = crate::epoch_reward_cap::current_tracker(&env, &config);
        Ok(config
            .cap_per_epoch
            .saturating_sub(tracker.rewards_paid_this_epoch))
    }

    /// A user's currently queued deferred reward, if any.
    pub fn get_deferred_reward(env: Env, user: Address) -> Option<DeferredReward> {
        crate::epoch_reward_cap::get_deferred(&env, &user)
    }

    /// Claim accrued rewards subject to the configured epoch cap.
    ///
    /// If the payout would exceed the cap's remaining headroom, only the
    /// remaining headroom is paid now; the rest is queued as a
    /// `DeferredReward` for `user`, claimable next epoch via
    /// `claim_deferred_reward`. Returns the amount actually paid now.
    pub fn claim_epoch_capped_reward(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let config =
            crate::epoch_reward_cap::get_cap_config(&env).ok_or(VaultError::NotInitialized)?;
        let accrued = balance::get_accrued_reward(&env, &user);
        if accrued <= 0 {
            return Ok(0);
        }

        let mut tracker = crate::epoch_reward_cap::current_tracker(&env, &config);
        let remaining_cap = config
            .cap_per_epoch
            .saturating_sub(tracker.rewards_paid_this_epoch)
            .max(0);

        let payable = accrued.min(remaining_cap);
        let deferred_amount = accrued - payable;

        // The whole accrued balance is settled here: the payable part is
        // transferred below, the rest moves into the deferred bucket.
        balance::set_accrued_reward(&env, &user, 0);

        if payable > 0 {
            let pool_balance = balance::get_reward_pool_balance(&env);
            if pool_balance < payable {
                return Err(VaultError::InsufficientRewardPool);
            }
            balance::set_reward_pool_balance(&env, pool_balance - payable);

            let token_addr: Address = env
                .storage()
                .instance()
                .get(&crate::storage::DataKey::Token)
                .ok_or(VaultError::NotInitialized)?;
            soroban_sdk::token::Client::new(&env, &token_addr).transfer(
                &env.current_contract_address(),
                &user,
                &payable,
            );

            tracker.rewards_paid_this_epoch =
                tracker.rewards_paid_this_epoch.saturating_add(payable);
            events::claimed(&env, &user, payable, env.ledger().sequence());
        }
        crate::epoch_reward_cap::set_tracker(&env, &tracker);

        if deferred_amount > 0 {
            let next_epoch_start = tracker.epoch_start.saturating_add(config.epoch_ledgers);
            crate::epoch_reward_cap::queue_deferred(&env, &user, deferred_amount, next_epoch_start);
        }

        Ok(payable)
    }

    /// Collect a previously queued deferred reward. Callable once the epoch
    /// it was deferred into has started.
    pub fn claim_deferred_reward(env: Env, user: Address) -> Result<i128, VaultError> {
        user.require_auth();

        let deferred = crate::epoch_reward_cap::get_deferred(&env, &user)
            .ok_or(VaultError::NothingToWithdraw)?;
        if env.ledger().sequence() < deferred.next_epoch_start {
            return Err(VaultError::EpochNotFinalized);
        }
        crate::epoch_reward_cap::remove_deferred(&env, &user);

        let pool_balance = balance::get_reward_pool_balance(&env);
        if pool_balance < deferred.amount {
            return Err(VaultError::InsufficientRewardPool);
        }
        balance::set_reward_pool_balance(&env, pool_balance - deferred.amount);

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&crate::storage::DataKey::Token)
            .ok_or(VaultError::NotInitialized)?;
        soroban_sdk::token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &user,
            &deferred.amount,
        );

        events::claimed(&env, &user, deferred.amount, env.ledger().sequence());
        Ok(deferred.amount)
    }
}















