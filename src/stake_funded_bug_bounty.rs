//! Stake-funded bug bounty (issue #468).
//!
//! Community-funded bug bounty pool where stakers voluntarily contribute a
//! small percentage of their rewards (up to 10% / 1000 bps) to a security
//! fund. When a valid vulnerability is reported and confirmed by admin,
//! the reporter receives a payout from the accumulated fund.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! persistent and instance storage, matching `balance.rs` and other feature modules.
//!
//! Storage keys:
//! - Per-user contribution bps: `(symbol_short!("bb_bps"), user: Address)` (persistent)
//! - BugBountyFund balance: `symbol_short!("bb_fund")` -> `i128` (instance)

use soroban_sdk::{symbol_short, Address, Env, Symbol};

const BB_CONTRIBUTION_KEY: Symbol = symbol_short!("bb_bps");
const BB_FUND_KEY: Symbol = symbol_short!("bb_fund");

/// Maximum allowed contribution: 1000 bps (10% of rewards).
pub const MAX_BUG_BOUNTY_BPS: u32 = 1000;

/// Read user's configured bug bounty contribution in bps (0..1000).
pub fn get_contribution_bps(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&(BB_CONTRIBUTION_KEY, user.clone()))
        .unwrap_or(0)
}

/// Set user's configured bug bounty contribution in bps.
pub fn set_contribution_bps(env: &Env, user: &Address, bps: u32) {
    env.storage()
        .persistent()
        .set(&(BB_CONTRIBUTION_KEY, user.clone()), &bps);
}

/// Read accumulated BugBountyFund balance from instance storage.
pub fn get_fund_balance(env: &Env) -> i128 {
    env.storage().instance().get(&BB_FUND_KEY).unwrap_or(0)
}

/// Set BugBountyFund balance in instance storage.
pub fn set_fund_balance(env: &Env, balance: i128) {
    env.storage().instance().set(&BB_FUND_KEY, &balance);
}

/// Deduct bug bounty contribution during reward claim, route to BugBountyFund,
/// and emit `bug_bounty_contributed` event.
/// Returns the deducted contribution amount.
pub fn deduct_bounty_contribution(env: &Env, user: &Address, reward: i128) -> i128 {
    if reward <= 0 {
        return 0;
    }

    let bps = get_contribution_bps(env, user);
    if bps == 0 {
        return 0;
    }

    let contribution = reward.saturating_mul(bps as i128) / 10_000;
    if contribution <= 0 {
        return 0;
    }

    let current_fund = get_fund_balance(env);
    let new_fund = current_fund.saturating_add(contribution);
    set_fund_balance(env, new_fund);

    // Emit bug_bounty_contributed event: (user, contribution_amount, fund_total, ledger)
    env.events().publish(
        (symbol_short!("bb_cnt"), user.clone()),
        (contribution, new_fund, env.ledger().sequence()),
    );

    contribution
}
