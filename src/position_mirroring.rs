//! Position mirroring (issue #453).
//!
//! Social trading feature where a follower designates a leader to mirror.
//!
//! ## Performance fee (mirror-fee extension)
//!
//! A follower may optionally set `leader_fee_bps` (max 1 000 bps = 10%) when
//! calling `set_mirror_config`. When non-zero, every reward claim by the
//! follower triggers `apply_mirror_perf_fee`, which:
//!
//! 1. Deducts `leader_fee_bps / 10 000` of the follower's post-claim reward.
//! 2. Transfers that amount directly to the leader's address.
//! 3. Emits a `mirror_fee_paid` event.
//!
//! Setting `leader_fee_bps = 0` means no fee — existing configs and all
//! callers that don't set a fee are unaffected. The fee is strictly opt-in;
//! it is never applied by default.

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, Vec};

use crate::balance;
use crate::errors::{VaultError, VaultExtError};
use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractClient};

const MIRROR_CFG_KEY: Symbol = symbol_short!("mirr_cfg");
const MIRROR_FOLLOWER_LIST_KEY: Symbol = symbol_short!("mirr_flw");

/// Hard cap on the leader performance fee a follower may opt into.
pub const MAX_LEADER_FEE_BPS: u32 = 1_000; // 10 %

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MirrorConfig {
    pub leader: Address,
    pub mirror_ratio_bps: u32,
    pub max_mirror_amount: i128,
    pub active: bool,
    /// Basis points of the follower's reward claim paid to the leader as a
    /// performance fee. `0` disables the fee. Max `MAX_LEADER_FEE_BPS`.
    pub leader_fee_bps: u32,
}

fn get_mirror_config_inner(env: &Env, follower: &Address) -> Option<MirrorConfig> {
    env.storage().persistent().get(&(MIRROR_CFG_KEY, follower.clone()))
}

fn set_mirror_config_inner(env: &Env, follower: &Address, cfg: &MirrorConfig) {
    env.storage()
        .persistent()
        .set(&(MIRROR_CFG_KEY, follower.clone()), cfg);
}

fn remove_mirror_config_inner(env: &Env, follower: &Address) {
    env.storage().persistent().remove(&(MIRROR_CFG_KEY, follower.clone()));
}

fn get_followers(env: &Env, leader: &Address) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&(MIRROR_FOLLOWER_LIST_KEY, leader.clone()))
        .unwrap_or(Vec::new(env))
}

fn set_followers(env: &Env, leader: &Address, list: &Vec<Address>) {
    env.storage()
        .persistent()
        .set(&(MIRROR_FOLLOWER_LIST_KEY, leader.clone()), list);
}

fn add_follower(env: &Env, leader: &Address, follower: &Address) -> Result<(), VaultExtError> {
    let mut list = get_followers(env, leader);
    // check already present
    for a in list.iter() {
        if a == *follower {
            return Ok(());
        }
    }
    if list.len() >= 10 {
        return Err(VaultExtError::TooManyRecipients); // mapped to TooManyFollowers
    }
    list.push_back(follower.clone());
    set_followers(env, leader, &list);
    Ok(())
}

fn remove_follower(env: &Env, leader: &Address, follower: &Address) {
    let list = get_followers(env, leader);
    let mut new_list = Vec::new(env);
    for a in list.iter() {
        if a != *follower {
            new_list.push_back(a);
        }
    }
    set_followers(env, leader, &new_list);
}

/// Try to mirror a stake/unstake for all followers of leader.
/// Called from vault stake/unstake paths.
pub fn maybe_mirror_action(env: &Env, leader: &Address, action: Symbol, amount: i128) {
    let followers = get_followers(env, leader);
    if followers.is_empty() {
        return;
    }
    for follower in followers.iter() {
        let cfg_opt = get_mirror_config_inner(env, &follower);
        if cfg_opt.is_none() {
            continue;
        }
        let cfg = cfg_opt.unwrap();
        if !cfg.active || cfg.leader != *leader {
            continue;
        }
        let mirror_amount = amount.saturating_mul(cfg.mirror_ratio_bps as i128) / 10_000;
        if mirror_amount <= 0 {
            continue;
        }
        let capped = if cfg.max_mirror_amount > 0 && mirror_amount > cfg.max_mirror_amount {
            cfg.max_mirror_amount
        } else {
            mirror_amount
        };
        // Execute action — share accounting only; token transfer is simulated via balance check
        if action == symbol_short!("stake") {
            // Check follower has enough token balance (simulates allowance check)
            // If we cannot read token, treat as failed
            let token_addr: Option<Address> = env.storage().instance().get(&DataKey::Token);
            if token_addr.is_none() {
                env.events().publish(
                    (symbol_short!("mir_fail"), leader.clone()),
                    (follower.clone(), symbol_short!("stake"), capped, env.ledger().sequence()),
                );
                continue;
            }
            let token_client = token::Client::new(env, &token_addr.unwrap());
            let follower_balance = token_client.balance(&follower);
            if follower_balance < capped {
                env.events().publish(
                    (symbol_short!("mir_fail"), leader.clone()),
                    (follower.clone(), symbol_short!("stake"), capped, env.ledger().sequence()),
                );
                continue;
            }
            // Simulate transfer by directly adjusting shares without moving tokens for follower
            // In a real deployment this would use allowance; we avoid revert by not calling transfer
            let total_shares = balance::get_total_shares(env);
            let total_deposited = balance::get_total_deposited(env);
            let shares = balance::amount_to_shares(total_shares, total_deposited, capped).unwrap_or(capped);
            let cur = balance::get_shares(env, &follower);
            balance::set_shares(env, &follower, cur + shares);
            balance::set_total_shares(env, total_shares + shares);
            balance::set_total_deposited(env, total_deposited + capped);
            env.events().publish(
                (symbol_short!("mir_exec"), leader.clone()),
                (follower.clone(), symbol_short!("stake"), capped, env.ledger().sequence()),
            );
        } else if action == symbol_short!("unstake") {
            let cur_shares = balance::get_shares(env, &follower);
            if cur_shares == 0 || cur_shares < capped {
                env.events().publish(
                    (symbol_short!("mir_fail"), leader.clone()),
                    (follower.clone(), symbol_short!("unstake"), capped, env.ledger().sequence()),
                );
                continue;
            }
            let total_shares = balance::get_total_shares(env);
            let total_deposited = balance::get_total_deposited(env);
            let shares_to_burn = capped.min(cur_shares);
            let out_amount = balance::shares_to_amount(total_shares, total_deposited, shares_to_burn).unwrap_or(shares_to_burn);
            balance::set_shares(env, &follower, cur_shares - shares_to_burn);
            balance::set_total_shares(env, total_shares - shares_to_burn);
            balance::set_total_deposited(env, total_deposited - out_amount);
            env.events().publish(
                (symbol_short!("mir_exec"), leader.clone()),
                (follower.clone(), symbol_short!("unstake"), capped, env.ledger().sequence()),
            );
        }
    }
}

/// Apply the leader performance fee to a follower's reward payout.
///
/// Called from both `do_claim` sites in vault.rs after `apply_claim_fee` has
/// already reduced the payout.  If the follower has no active mirror config or
/// their `leader_fee_bps` is zero, this is a cheap no-op.
///
/// # Fee mechanics
/// - `fee = reward_after_claim_fee * leader_fee_bps / 10_000`
/// - The fee is transferred directly from the contract to the leader via the
///   reward token.  No reserve accumulates; the leader receives it immediately.
/// - Returns the net payout after the fee, i.e. what the follower will receive.
///
/// # Error handling
/// Arithmetic errors return `VaultError::ArithmeticError`.  Any token-transfer
/// failure propagates as-is.  A missing token address returns
/// `VaultError::NotInitialized`.
pub fn apply_mirror_perf_fee(
    env: &Env,
    follower: &Address,
    reward_after_claim_fee: i128,
) -> Result<i128, VaultError> {
    // Fast path: nothing to do for non-followers or zero reward.
    if reward_after_claim_fee <= 0 {
        return Ok(reward_after_claim_fee);
    }

    let cfg = match get_mirror_config_inner(env, follower) {
        Some(c) if c.active && c.leader_fee_bps > 0 => c,
        _ => return Ok(reward_after_claim_fee),
    };

    let fee_amount = reward_after_claim_fee
        .checked_mul(cfg.leader_fee_bps as i128)
        .ok_or(VaultError::ArithmeticError)?
        .checked_div(10_000)
        .ok_or(VaultError::ArithmeticError)?;

    if fee_amount <= 0 {
        return Ok(reward_after_claim_fee);
    }

    let net_payout = reward_after_claim_fee
        .checked_sub(fee_amount)
        .ok_or(VaultError::ArithmeticError)?;

    // Resolve the reward token (same fallback logic as VaultContract::reward_token_address).
    let reward_token_addr: Address = env
        .storage()
        .instance()
        .get(&symbol_short!("rwd_tok"))
        .or_else(|| env.storage().instance().get(&DataKey::Token))
        .ok_or(VaultError::NotInitialized)?;

    token::Client::new(env, &reward_token_addr).transfer(
        &env.current_contract_address(),
        &cfg.leader,
        &fee_amount,
    );

    crate::events::mirror_fee_paid(
        env,
        follower,
        &cfg.leader,
        fee_amount,
        net_payout,
        env.ledger().sequence(),
    );

    Ok(net_payout)
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Follower sets mirror config.
    ///
    /// `leader_fee_bps` is the share of the follower's reward claims that will
    /// be forwarded to the leader as a performance fee (0 = no fee, max 1 000).
    /// The follower explicitly opts in; no fee is ever applied by default.
    pub fn set_mirror_config(
        env: Env,
        follower: Address,
        leader: Address,
        mirror_ratio_bps: u32,
        max_mirror_amount: i128,
        leader_fee_bps: u32,
    ) -> Result<(), VaultExtError> {
        follower.require_auth();
        if follower == leader {
            return Err(VaultExtError::InvalidMultisigConfig);
        }
        if mirror_ratio_bps == 0 || mirror_ratio_bps > 10_000 {
            return Err(VaultExtError::InvalidFeeAllocation);
        }
        if max_mirror_amount <= 0 {
            return Err(VaultExtError::ZeroAmount);
        }
        if leader_fee_bps > MAX_LEADER_FEE_BPS {
            return Err(VaultExtError::InvalidFeeAllocation);
        }
        // Remove old follower entry if switching leaders.
        if let Some(old_cfg) = get_mirror_config_inner(&env, &follower) {
            if old_cfg.leader != leader {
                remove_follower(&env, &old_cfg.leader, &follower);
            }
        }
        let cfg = MirrorConfig {
            leader: leader.clone(),
            mirror_ratio_bps,
            max_mirror_amount,
            active: true,
            leader_fee_bps,
        };
        set_mirror_config_inner(&env, &follower, &cfg);
        add_follower(&env, &leader, &follower)?;
        env.events().publish(
            (symbol_short!("mir_cfg"), follower.clone()),
            (leader, mirror_ratio_bps, max_mirror_amount, leader_fee_bps, env.ledger().sequence()),
        );
        Ok(())
    }

    pub fn cancel_mirroring(env: Env, follower: Address) -> Result<(), VaultExtError> {
        follower.require_auth();
        let cfg = get_mirror_config_inner(&env, &follower).ok_or(VaultExtError::PositionNotFound)?;
        remove_follower(&env, &cfg.leader, &follower);
        remove_mirror_config_inner(&env, &follower);
        env.events().publish(
            (symbol_short!("mir_canc"), follower),
            (env.ledger().sequence(),),
        );
        Ok(())
    }

    pub fn get_mirror_config(env: Env, follower: Address) -> Option<MirrorConfig> {
        get_mirror_config_inner(&env, &follower)
    }

    pub fn get_mirror_followers(env: Env, leader: Address) -> Vec<Address> {
        get_followers(&env, &leader)
    }
}
