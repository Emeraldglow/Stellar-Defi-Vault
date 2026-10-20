//! Pool health insurance (issue #289).
//!
//! An external guarantor deposits a reserve that covers staker losses if the
//! admin misbehaves â€” for example by draining reward tokens. The guarantor can
//! only withdraw their reserve once the pool has been demonstrably solvent for
//! 90 days and no insolvency has been declared.
//!
//! The reserve is denominated in reward tokens, the same denomination as the
//! staker losses it exists to cover.
//!
//! # The trust asymmetry
//!
//! `declare_insolvency` is admin-only and **irreversible**. That is deliberate:
//! a reversible declaration would let an admin open the claim window, watch who
//! claims, and close it again. But it also means a hostile admin can strand a
//! guarantor's reserve by declaring insolvency for no reason. Pair it with the
//! timelock from issue #137 where that is available â€” the guarantor's
//! protection against the admin is procedural, not enforced here.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Ledgers the pool must stay solvent before a guarantor may withdraw.
///
/// 90 days at ~5s per ledger.
pub const SOLVENCY_PERIOD_LEDGERS: u32 = 1_555_200;

/// Instance key: the registered guarantor address.
const GUARANTOR_KEY: Symbol = symbol_short!("gtor");
/// Instance key: the coverage amount the guarantor committed to.
const COVERAGE_KEY: Symbol = symbol_short!("gtor_cov");
/// Instance key: reserve currently held, tracked separately from the pool.
const RESERVE_KEY: Symbol = symbol_short!("gtor_res");
/// Instance key: ledger at which the guarantor was registered.
const REGISTERED_KEY: Symbol = symbol_short!("gtor_reg");
/// Instance key: ledger at which insolvency was declared, if it was.
const INSOLVENT_KEY: Symbol = symbol_short!("gtor_ins");

/// The registered guarantor, if one exists.
pub fn get_guarantor(env: &Env) -> Option<Address> {
    env.storage().instance().get(&GUARANTOR_KEY)
}

/// The coverage amount the guarantor committed to.
pub fn get_coverage_amount(env: &Env) -> i128 {
    env.storage().instance().get(&COVERAGE_KEY).unwrap_or(0)
}

/// The reserve currently available to cover claims.
///
/// Held separately from `RewardPoolBalance` on purpose: a reserve counted as
/// part of the reward pool would be payable as ordinary yield, which is
/// precisely the failure it exists to insure against.
pub fn get_reserve(env: &Env) -> i128 {
    env.storage().instance().get(&RESERVE_KEY).unwrap_or(0)
}

fn set_reserve(env: &Env, amount: i128) {
    env.storage().instance().set(&RESERVE_KEY, &amount);
}

/// The ledger at which insolvency was declared, if it has been.
pub fn insolvency_ledger(env: &Env) -> Option<u32> {
    env.storage().instance().get(&INSOLVENT_KEY)
}

/// Whether the pool has been declared insolvent.
pub fn is_insolvent(env: &Env) -> bool {
    insolvency_ledger(env).is_some()
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Register a guarantor and the coverage they commit to. Admin only.
    ///
    /// Registering does not move funds â€” the guarantor funds the reserve
    /// themselves via `deposit_guarantee`, so the admin cannot register an
    /// address and drain it.
    pub fn register_guarantor(
        env: Env,
        guarantor: Address,
        coverage_amount: i128,
    ) -> Result<(), VaultError> {
        admin::require_admin(&env)?;
        if guarantor == env.current_contract_address() {
            return Err(VaultError::InvalidAddress);
        }

        if coverage_amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        if is_insolvent(&env) {
            // Registering a new guarantor after insolvency would let an admin
            // reset the arrangement on top of an unpaid claim window.
            return Err(VaultError::ContractStopped);
        }

        env.storage().instance().set(&GUARANTOR_KEY, &guarantor);
        env.storage()
            .instance()
            .set(&COVERAGE_KEY, &coverage_amount);
        env.storage()
            .instance()
            .set(&REGISTERED_KEY, &env.ledger().sequence());

        env.events().publish(
            (symbol_short!("gtor_reg"), guarantor),
            (coverage_amount, env.ledger().sequence()),
        );
        Ok(())
    }

    /// The registered guarantor, if any.
    pub fn get_guarantor_address(env: Env) -> Option<Address> {
        get_guarantor(&env)
    }

    /// The reserve currently available to cover claims.
    pub fn get_guarantee_reserve(env: Env) -> i128 {
        get_reserve(&env)
    }

    /// Deposit into the coverage reserve. Guarantor only.
    pub fn deposit_guarantee(env: Env, guarantor: Address, amount: i128) -> Result<(), VaultError> {
        guarantor.require_auth();

        let registered = get_guarantor(&env).ok_or(VaultError::NotInitialized)?;
        if registered != guarantor {
            return Err(VaultError::RelayerNotApproved);
        }
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let updated = get_reserve(&env)
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticError)?;
        set_reserve(&env, updated);

        env.events().publish(
            (symbol_short!("gtor_dep"), guarantor),
            (amount, updated, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Declare the pool insolvent, opening the reserve to staker claims.
    ///
    /// Admin only and **irreversible** â€” see the module docs for why, and for
    /// the risk that carries for the guarantor.
    pub fn declare_insolvency(env: Env) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if is_insolvent(&env) {
            return Err(VaultError::ContractStopped);
        }

        let ledger = env.ledger().sequence();
        env.storage().instance().set(&INSOLVENT_KEY, &ledger);

        let admin = admin::get_admin(&env)?;
        env.events()
            .publish((symbol_short!("gtor_isv"), admin), ledger);
        Ok(())
    }

    /// Whether the pool has been declared insolvent.
    pub fn is_pool_insolvent(env: Env) -> bool {
        is_insolvent(&env)
    }

    /// Claim from the coverage reserve after an insolvency declaration.
    ///
    /// Claims are first-come-first-served against a finite reserve: it covers
    /// what the guarantor committed, which may be less than total losses.
    pub fn claim_guarantee(env: Env, user: Address, amount: i128) -> Result<i128, VaultError> {
        user.require_auth();

        if !is_insolvent(&env) {
            return Err(VaultError::NotInitialized);
        }
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let reserve = get_reserve(&env);
        if reserve < amount {
            return Err(VaultError::InsufficientRewardPool);
        }

        let remaining = reserve
            .checked_sub(amount)
            .ok_or(VaultError::ArithmeticError)?;
        set_reserve(&env, remaining);

        env.events().publish(
            (symbol_short!("gtor_clm"), user),
            (amount, remaining, env.ledger().sequence()),
        );
        Ok(remaining)
    }

    /// Withdraw the remaining reserve. Guarantor only.
    ///
    /// Permitted only once the pool has been solvent for
    /// [`SOLVENCY_PERIOD_LEDGERS`] since registration, and never after an
    /// insolvency declaration â€” otherwise a guarantor could pull the reserve
    /// out from under the claims it exists to cover.
    pub fn withdraw_guarantee(env: Env, guarantor: Address) -> Result<i128, VaultError> {
        guarantor.require_auth();

        let registered = get_guarantor(&env).ok_or(VaultError::NotInitialized)?;
        if registered != guarantor {
            return Err(VaultError::RelayerNotApproved);
        }
        if is_insolvent(&env) {
            return Err(VaultError::ContractStopped);
        }

        let registered_at: u32 = env
            .storage()
            .instance()
            .get(&REGISTERED_KEY)
            .ok_or(VaultError::NotInitialized)?;

        let unlock_at = registered_at.saturating_add(SOLVENCY_PERIOD_LEDGERS);
        if env.ledger().sequence() < unlock_at {
            return Err(VaultError::EpochNotFinalized);
        }

        let amount = get_reserve(&env);
        if amount <= 0 {
            return Err(VaultError::NothingToWithdraw);
        }
        set_reserve(&env, 0);

        env.events().publish(
            (symbol_short!("gtor_wdr"), guarantor),
            (amount, env.ledger().sequence()),
        );
        Ok(amount)
    }

    /// The ledger at which the guarantor may withdraw, `0` if unregistered.
    pub fn guarantee_unlock_ledger(env: Env) -> u32 {
        env.storage()
            .instance()
            .get::<_, u32>(&REGISTERED_KEY)
            .map(|at| at.saturating_add(SOLVENCY_PERIOD_LEDGERS))
            .unwrap_or(0)
    }
}















