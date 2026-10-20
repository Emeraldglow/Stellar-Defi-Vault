// Issue #463: Position clawback for fraudulent stakes
// Issue #464: NFT-based yield boost
// Issue #465: Milestone countdown tracker
// Issue #466: Pool parameter change log

use crate::{
    admin, balance,
    errors::VaultExtError,
    storage::DataKey,
    vault::{VaultContract, VaultContractClient},
};
use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Vec};

// ── Issue #463: Clawback window storage ──────────────────────────────────────

/// Clawback window in ledgers (admin configurable, defaults to 1 day = 17280 ledgers)
pub fn get_clawback_window(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("cbk_win"))
        .unwrap_or(17_280) // ~1 day default
}

pub fn set_clawback_window(env: &Env, ledgers: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("cbk_win"), &ledgers);
}

// ── Issue #464: NFT boost multiplier ─────────────────────────────────────────

/// Returns the yield boost multiplier (in bps, 10000 = 1x) for NFT holders
/// Returns 10000 if NFT contract not configured or user doesn't hold NFT
pub fn get_nft_yield_boost(env: &Env, user: &Address) -> u32 {
    let _nft_contract = match balance::get_nft_contract(env) {
        Some(addr) => addr,
        None => return 10_000, // No boost
    };

    // Simple check: if user has any NFT receipt, grant 20% boost (12000 bps)
    // In production this would call the NFT contract's balance_of function
    let nft_key = (symbol_short!("nft_bal"), user.clone());
    let has_nft: bool = env.storage().persistent().get(&nft_key).unwrap_or(false);

    if has_nft {
        12_000 // 20% boost
    } else {
        10_000 // No boost
    }
}

// Simple helper to mark user as NFT holder (for testing/demo purposes)
pub fn set_nft_holder(env: &Env, user: &Address, holder: bool) {
    let nft_key = (symbol_short!("nft_bal"), user.clone());
    env.storage().persistent().set(&nft_key, &holder);
}

// ── Issue #465: Milestone progress tracking ──────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MilestoneProgress {
    pub milestone_id: u32,
    pub milestone_name: String,
    pub current_value: i128,
    pub target_value: i128,
    pub progress_pct: u32, // 0-10000 (basis points)
    pub ledgers_to_target: u32,
}

/// Get user's progress toward a specific milestone
pub fn get_user_milestone_progress(
    env: &Env,
    user: &Address,
    milestone_id: u32,
) -> MilestoneProgress {
    // Simplified: track stake duration milestone as example
    let current_stake = balance::get_shares(env, user);

    let staked_at = env
        .storage()
        .persistent()
        .get::<_, u32>(&DataKey::StakedAtLedger(user.clone()))
        .unwrap_or(0);

    let current_ledger = env.ledger().sequence();
    let duration = current_ledger.saturating_sub(staked_at);

    // Example milestones: 1 week = 120960 ledgers, 1 month = 518400 ledgers
    let (target, name) = match milestone_id {
        1 => (120_960u32, String::from_str(env, "1 Week Staker")),
        2 => (518_400u32, String::from_str(env, "1 Month Staker")),
        _ => (120_960u32, String::from_str(env, "1 Week Staker")),
    };

    let progress_pct = if target > 0 && current_stake > 0 {
        ((duration as u64 * 10_000u64) / target as u64).min(10_000) as u32
    } else {
        0
    };

    let ledgers_to_target = if duration < target {
        target - duration
    } else {
        0
    };

    MilestoneProgress {
        milestone_id,
        milestone_name: name,
        current_value: duration as i128,
        target_value: target as i128,
        progress_pct,
        ledgers_to_target,
    }
}

// ── Issue #466: Parameter change log ─────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterChange {
    pub ledger: u32,
    pub changed_by: Address,
    pub parameter: String,
    pub old_value: i128,
    pub new_value: i128,
}

const MAX_PARAM_LOG_ENTRIES: u32 = 100;

pub fn get_parameter_change_log(env: &Env) -> Vec<ParameterChange> {
    env.storage()
        .instance()
        .get(&symbol_short!("prm_log"))
        .unwrap_or(Vec::new(env))
}

pub fn set_parameter_change_log(env: &Env, log: &Vec<ParameterChange>) {
    env.storage().instance().set(&symbol_short!("prm_log"), log);
}

/// Append a parameter change to the immutable log
pub fn log_parameter_change(
    env: &Env,
    changed_by: &Address,
    parameter: &str,
    old_value: i128,
    new_value: i128,
) {
    let mut log = get_parameter_change_log(env);

    let entry = ParameterChange {
        ledger: env.ledger().sequence(),
        changed_by: changed_by.clone(),
        parameter: String::from_str(env, parameter),
        old_value,
        new_value,
    };

    log.push_back(entry);

    // Keep only last MAX_PARAM_LOG_ENTRIES
    while log.len() > MAX_PARAM_LOG_ENTRIES {
        log.remove(0);
    }

    set_parameter_change_log(env, &log);
}

// ── Contract implementation ───────────────────────────────────────────────────

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #463: Clawback a position within the fraud window
    /// Only admin can call this to reverse fraudulent stakes
    pub fn position_clawback(
        env: Env,
        admin_addr: Address,
        user: Address,
    ) -> Result<i128, VaultExtError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        let staked_at = env
            .storage()
            .persistent()
            .get::<_, u32>(&DataKey::StakedAtLedger(user.clone()))
            .ok_or(VaultExtError::PositionNotFound)?;

        let current_ledger = env.ledger().sequence();
        let clawback_window = get_clawback_window(&env);

        if current_ledger > staked_at + clawback_window {
            return Err(VaultExtError::ActionNotYetExecutable);
        }

        let shares = balance::get_shares(&env, &user);
        if shares == 0 {
            return Err(VaultExtError::PositionNotFound);
        }

        // Calculate token amount
        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);
        let amount = shares
            .checked_mul(total_deposited)
            .and_then(|v| v.checked_div(total_shares))
            .ok_or(VaultExtError::ArithmeticError)?;

        // Remove position
        balance::set_shares(&env, &user, 0);
        balance::set_total_shares(&env, total_shares - shares);
        balance::set_total_deposited(&env, total_deposited - amount);

        // Transfer tokens to slash treasury
        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(VaultExtError::NotInitialized)?;
        let treasury = balance::get_slash_treasury(&env).unwrap_or(admin_addr.clone());

        soroban_sdk::token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &treasury,
            &amount,
        );

        // Emit event
        env.events().publish(
            (symbol_short!("clawback"), admin_addr),
            (user, amount, current_ledger),
        );

        Ok(amount)
    }

    /// Issue #464: Check if user gets NFT yield boost
    pub fn yield_boost_nft(env: Env, user: Address) -> u32 {
        get_nft_yield_boost(&env, &user)
    }

    /// Issue #465: Get countdown to next milestone
    pub fn milestone_countdown(env: Env, user: Address, milestone_id: u32) -> MilestoneProgress {
        get_user_milestone_progress(&env, &user, milestone_id)
    }

    /// Issue #466: Get parameter change history (paginated)
    pub fn pool_parameter_change_log(env: Env, offset: u32, limit: u32) -> Vec<ParameterChange> {
        let log = get_parameter_change_log(&env);
        let total = log.len();

        if offset >= total {
            return Vec::new(&env);
        }

        let end = (offset + limit).min(total);
        let mut result = Vec::new(&env);

        for i in offset..end {
            if let Some(entry) = log.get(i) {
                result.push_back(entry);
            }
        }

        result
    }

    /// Issue #463: Admin configures clawback window
    pub fn set_clawback_window_ledgers(
        env: Env,
        admin_addr: Address,
        ledgers: u32,
    ) -> Result<(), VaultExtError> {
        admin_addr.require_auth();
        admin::require_admin(&env)?;

        let old_value = get_clawback_window(&env);
        set_clawback_window(&env, ledgers);

        log_parameter_change(
            &env,
            &admin_addr,
            "clawback_window",
            old_value as i128,
            ledgers as i128,
        );

        Ok(())
    }
}
