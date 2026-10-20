//! Minimum position age before boost multipliers activate (issue #401).
//!
//! Prevents stakers from front-running a boost campaign by staking just
//! before it starts and immediately benefiting. Age is measured from
//! `StakedAtLedger`, so a full unstake-then-restake resets the clock.
//!
//! # Wiring
//!
//! Splicing this into the live boost-multiplier computation inside
//! `calc_pending_reward` isn't done here â€” matching the same documented gap
//! `governance_power_decay.rs` and `transfer_cooldown.rs` leave for their own
//! entrypoints. `is_boost_eligible` is the read a boost-multiplier
//! computation should gate on (apply the multiplier only when it returns
//! `true`), and `check_boost_activation` is the call a `claim()` path should
//! make to emit `boost_activated` lazily, exactly once, the first time a
//! user is found eligible. Both are directly callable and tested on their
//! own in the meantime.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

const MIN_AGE_KEY: Symbol = symbol_short!("bam_age");
const ACTIVATED_KEY: Symbol = symbol_short!("bam_act");

fn get_min_age(env: &Env) -> u32 {
    env.storage().instance().get(&MIN_AGE_KEY).unwrap_or(0)
}

fn staked_at_ledger(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&crate::storage::DataKey::StakedAtLedger(user.clone()))
        .unwrap_or(0)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Set the minimum position age (in ledgers) before any boost
    /// multiplier applies. Admin only. `0` disables the gate (default).
    pub fn set_boost_activation_minimum_age(env: Env, ledgers: u32) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        env.storage().instance().set(&MIN_AGE_KEY, &ledgers);
        Ok(())
    }

    /// Read-only query: the current minimum boost-activation age.
    pub fn get_boost_activation_minimum_age(env: Env) -> u32 {
        get_min_age(&env)
    }

    /// True when `user`'s position is old enough for boost multipliers to
    /// apply: `current_ledger - staked_at_ledger >= minimum_age`. Always
    /// true when the minimum age is `0` (disabled) or the user has never
    /// staked is treated as not yet eligible.
    pub fn is_boost_eligible(env: Env, user: Address) -> bool {
        let min_age = get_min_age(&env);
        if min_age == 0 {
            return true;
        }

        let staked_at = staked_at_ledger(&env, &user);
        if staked_at == 0 {
            return false;
        }

        env.ledger().sequence().saturating_sub(staked_at) >= min_age
    }

    /// Ledgers remaining until `user` becomes boost-eligible; `0` if already
    /// eligible.
    pub fn get_ledgers_until_boost(env: Env, user: Address) -> u32 {
        let min_age = get_min_age(&env);
        if min_age == 0 {
            return 0;
        }

        let staked_at = staked_at_ledger(&env, &user);
        let elapsed = env.ledger().sequence().saturating_sub(staked_at);
        if elapsed >= min_age {
            0
        } else {
            min_age - elapsed
        }
    }

    /// Checks `user`'s boost eligibility and, the first time they cross the
    /// age threshold, emits `boost_activated` and records that it fired so
    /// it never fires again for the same position. Returns the eligibility
    /// result. See the module-level "Wiring" note for where this is meant
    /// to be called from.
    pub fn check_boost_activation(env: Env, user: Address) -> bool {
        let eligible = Self::is_boost_eligible(env.clone(), user.clone());
        if eligible {
            let key = (ACTIVATED_KEY, user.clone());
            let already: bool = env.storage().persistent().get(&key).unwrap_or(false);
            if !already {
                env.storage().persistent().set(&key, &true);
                env.events()
                    .publish((symbol_short!("bam_on"), user), env.ledger().sequence());
            }
        }
        eligible
    }
}















