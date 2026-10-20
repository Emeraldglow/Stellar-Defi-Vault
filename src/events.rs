use crate::storage::AdminAction;
use soroban_sdk::{symbol_short, Address, Env, Symbol};

pub fn deposit(env: &Env, depositor: &Address, amount: i128, shares_minted: i128, ledger: u32) {
    let topics = (symbol_short!("deposit"), depositor);
    env.events()
        .publish(topics, (amount, shares_minted, ledger));
}

pub fn referral_bonus_paid(
    env: &Env,
    referrer: &Address,
    referee: &Address,
    referrer_bonus: i128,
    referee_bonus: i128,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("ref_bns"), referrer.clone(), referee.clone()),
        (referrer_bonus, referee_bonus, ledger),
    );
}

pub fn referral_earnings_claimed(
    env: &Env,
    referrer: &Address,
    amount: i128,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("ref_clm"), referrer.clone()),
        (amount, ledger),
    );
}

pub fn ve_boost_locked(
    env: &Env,
    user: &Address,
    amount: i128,
    duration_ledgers: u32,
    unlock_ledger: u32,
) {
    env.events().publish(
        (symbol_short!("ve_lck"), user.clone()),
        (amount, duration_ledgers, unlock_ledger),
    );
}

pub fn ve_boost_unlocked(
    env: &Env,
    user: &Address,
    amount: i128,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("ve_unl"), user.clone()),
        (amount, ledger),
    );
}

pub fn gauge_voted(
    env: &Env,
    user: &Address,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("g_voted"), user.clone()),
        ledger,
    );
}

pub fn proposal_vote_changed(
    env: &Env,
    user: &Address,
    proposal_id: u32,
    support: bool,
    weight: i128,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("v_chg"), user.clone(), proposal_id),
        (support, weight, ledger),
    );
}

pub fn withdraw(
    env: &Env,
    withdrawer: &Address,
    shares_burned: i128,
    amount_returned: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("withdraw"), withdrawer);
    env.events()
        .publish(topics, (shares_burned, amount_returned, ledger));
}

/// Emitted when a user exits through the paused-vault break-glass path.
pub fn emergency_withdrawal(
    env: &Env,
    user: &Address,
    amount_returned: i128,
    rewards_forfeited: i128,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("emg_wdraw"), user.clone()),
        (amount_returned, rewards_forfeited, ledger),
    );
}

pub fn position_locked(
    env: &Env,
    user: &Address,
    lock_duration_ledgers: u32,
    boost_bps: u32,
    unlocks_at: u32,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("pos_lock"), user.clone()),
        (lock_duration_ledgers, boost_bps, unlocks_at, ledger),
    );
}

pub fn unpaused(env: &Env, admin: &Address, ledger: u32) {
    let topics = (symbol_short!("unpaused"), admin);
    env.events().publish(topics, (ledger,));
}

/// Emitted when a `pause_until` schedule lazily lifts a pause on some
/// unrelated call, rather than via an explicit admin `unpause()` (issue
/// #556) — no admin address is relevant here since no one authorized this
/// specific transaction to unpause the pool; it just happened to be the
/// first call after `target_ledger`.
pub fn auto_unpaused(env: &Env, ledger: u32) {
    let topics = (symbol_short!("auto_unp"),);
    env.events().publish(topics, (ledger,));
}

pub fn yield_added(env: &Env, admin: &Address, amount: i128) {
    let topics = (symbol_short!("yield_add"), admin);
    env.events()
        .publish(topics, (amount, env.ledger().sequence()));
}

pub fn admin_changed(env: &Env, old_admin: &Address, new_admin: &Address) {
    let topics = (symbol_short!("admin_set"), old_admin);
    env.events()
        .publish(topics, (new_admin, env.ledger().sequence()));
}

pub fn withdrawal_limit_updated(env: &Env, admin: &Address, new_limit: i128) {
    let topics = (symbol_short!("wd_limit"), admin);
    env.events()
        .publish(topics, (new_limit, env.ledger().sequence()));
}

/// Issue #202: emitted by `start_bootstrap()`.
pub fn bootstrap_started(
    env: &Env,
    initial_rate_bps: u32,
    base_rate_bps: u32,
    duration_ledgers: u32,
) {
    let topics = (symbol_short!("boot_str"), env.current_contract_address());
    env.events().publish(
        topics,
        (
            initial_rate_bps,
            base_rate_bps,
            duration_ledgers,
            env.ledger().sequence(),
        ),
    );
}

/// Issue #202: emitted the first time any call notices the bootstrap period
/// has elapsed and settles the rate to `base_rate_bps` permanently.
pub fn bootstrap_ended(env: &Env, base_rate_bps: u32) {
    let topics = (symbol_short!("boot_end"), env.current_contract_address());
    env.events()
        .publish(topics, (base_rate_bps, env.ledger().sequence()));
}

/// Issue #197: emitted once per recipient during a fee split.
pub fn fee_distributed(env: &Env, recipient: &Address, amount: i128, ledger: u32) {
    let topics = (symbol_short!("fee_dist"), recipient);
    env.events().publish(topics, (amount, ledger));
}

/// Issue #195: emitted by `queue_action()`.
pub fn action_queued(env: &Env, action_id: u32, executable_at: u32) {
    let topics = (symbol_short!("act_queue"), env.current_contract_address());
    env.events()
        .publish(topics, (action_id, executable_at, env.ledger().sequence()));
}

/// Issue #195: emitted by `execute_action()`.
pub fn action_executed(env: &Env, action_id: u32) {
    let topics = (symbol_short!("act_exec"), env.current_contract_address());
    env.events()
        .publish(topics, (action_id, env.ledger().sequence()));
}

pub fn rate_changed(env: &Env, old_rate_bps: u32, new_rate_bps: u32) {
    let topics = (symbol_short!("rate_chg"), env.current_contract_address());
    env.events().publish(
        topics,
        (old_rate_bps, new_rate_bps, env.ledger().sequence()),
    );
}

/// Issue #206: emitted by `rollback_last_rate_change()`.
pub fn rate_rolled_back(env: &Env, restored_rate: u32, discarded_rate: u32) {
    let topics = (symbol_short!("rate_rbk"), env.current_contract_address());
    env.events().publish(
        topics,
        (restored_rate, discarded_rate, env.ledger().sequence()),
    );
}

/// Issue #207: emitted alongside `stake`/`unstake` for cross-chain bridge
/// relayers, when bridge event emission is enabled.
pub fn bridge_packet(env: &Env, packet: &crate::storage::BridgePacket) {
    let topics = (symbol_short!("bridge_pk"), packet.source_address.clone());
    env.events().publish(
        topics,
        (
            packet.sequence,
            packet.amount,
            packet.action.clone(),
            packet.timestamp_ledger,
        ),
    );
}

/// Issue #209: emitted by `position_split()`.
pub fn position_split(env: &Env, user: &Address, original_amount: i128, split_amount: i128) {
    let topics = (symbol_short!("pos_split"), user);
    env.events().publish(
        topics,
        (original_amount, split_amount, env.ledger().sequence()),
    );
}

/// Issue #205: emitted by `swap_and_stake()` after the swap and stake both
/// succeed.
pub fn swapped_and_staked(
    env: &Env,
    user: &Address,
    input_token: &Address,
    input_amount: i128,
    stake_amount_received: i128,
) {
    let topics = (symbol_short!("swap_stk"), user);
    env.events().publish(
        topics,
        (
            input_token,
            input_amount,
            stake_amount_received,
            env.ledger().sequence(),
        ),
    );
}

pub fn pool_cap_updated(env: &Env, admin: &Address, new_cap: i128) {
    let topics = (symbol_short!("cap_upd"), admin);
    env.events()
        .publish(topics, (new_cap, env.ledger().sequence()));
}

pub fn position_opened(env: &Env, user: &Address, amount: i128) {
    let topics = (symbol_short!("pos_open"), user);
    env.events()
        .publish(topics, (amount, env.ledger().sequence()));
}

pub fn position_closed(env: &Env, user: &Address) {
    let topics = (symbol_short!("pos_clos"), user);
    env.events().publish(topics, (env.ledger().sequence(),));
}

// ── Issue #39: rescue token event ────────────────────────────────────────────

pub fn token_rescued(env: &Env, token: &Address, amount: i128, recipient: &Address) {
    let topics = (symbol_short!("tk_rescue"), recipient);
    env.events().publish(
        topics,
        (
            token.clone(),
            amount,
            recipient.clone(),
            env.ledger().sequence(),
        ),
    );
}

// ── Issue #42: admin action audit events ─────────────────────────────────────

pub fn admin_action_set_reward_rate(env: &Env, actor: &Address, old_rate: u32, new_rate: u32) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::SetRewardRate),
        (actor.clone(), env.ledger().sequence(), old_rate, new_rate),
    );
}

pub fn admin_action_pause(env: &Env, actor: &Address) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::Pause),
        (actor.clone(), env.ledger().sequence()),
    );
}

pub fn admin_action_unpause(env: &Env, actor: &Address) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::Unpause),
        (actor.clone(), env.ledger().sequence()),
    );
}

pub fn admin_action_transfer_admin(env: &Env, actor: &Address, new_admin: &Address) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::TransferAdmin),
        (actor.clone(), env.ledger().sequence(), new_admin.clone()),
    );
}

pub fn admin_action_set_lock_period(env: &Env, actor: &Address, new_ledgers: u32) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::SetLockPeriod),
        (actor.clone(), env.ledger().sequence(), new_ledgers),
    );
}

pub fn admin_action_set_cap(env: &Env, actor: &Address, new_limit: i128) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::SetCap),
        (actor.clone(), env.ledger().sequence(), new_limit),
    );
}

pub fn admin_action_rescue_token(
    env: &Env,
    actor: &Address,
    token: &Address,
    amount: i128,
    recipient: &Address,
) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::RescueToken),
        (
            actor.clone(),
            env.ledger().sequence(),
            token.clone(),
            amount,
            recipient.clone(),
        ),
    );
}

pub fn admin_action_set_early_exit_penalty(env: &Env, actor: &Address, new_bps: u32) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::SetEarlyExitPenalty),
        (actor.clone(), env.ledger().sequence(), new_bps),
    );
}

pub fn admin_action_set_min_stake(env: &Env, actor: &Address, new_amount: i128) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::SetMinStake),
        (actor.clone(), env.ledger().sequence(), new_amount),
    );
}

pub fn admin_action_fund_reward_pool(env: &Env, actor: &Address, amount: i128) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::FundRewardPool),
        (actor.clone(), env.ledger().sequence(), amount),
    );
}

pub fn admin_action_add_yield(env: &Env, actor: &Address, amount: i128) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::AddYield),
        (actor.clone(), env.ledger().sequence(), amount),
    );
}

pub fn admin_action_set_boost_schedule(env: &Env, actor: &Address, num_tiers: u32) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::SetBoostSchedule),
        (actor.clone(), env.ledger().sequence(), num_tiers),
    );
}

pub fn admin_action_set_nft_contract(env: &Env, actor: &Address, nft_addr: &Address) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::SetNftContract),
        (actor.clone(), env.ledger().sequence(), nft_addr.clone()),
    );
}

pub fn admin_action_set_restake_window(env: &Env, actor: &Address, window: u32) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::SetRestakeWindow),
        (actor.clone(), env.ledger().sequence(), window),
    );
}

pub fn admin_action_set_reward_token(env: &Env, actor: &Address, token: &Address) {
    env.events().publish(
        (symbol_short!("adm_act"), actor, AdminAction::SetRewardToken),
        (actor.clone(), env.ledger().sequence(), token.clone()),
    );
}

pub fn slash(env: &Env, admin: &Address, user: &Address, amount: i128) {
    let topics = (symbol_short!("slash"), admin);
    env.events()
        .publish(topics, (user.clone(), amount, env.ledger().sequence()));
}

pub fn position_transferred(env: &Env, from: &Address, to: &Address, amount: i128) {
    let topics = (symbol_short!("pos_xfer"), from);
    env.events()
        .publish(topics, (to.clone(), amount, env.ledger().sequence()));
}

/// Emitted by `transfer_position_with_rewards` (issue #134).
///
/// `pending_reward_estimate` is the pending reward computed at transfer time —
/// it is informational only; no settlement occurs.
pub fn position_transferred_with_rewards(
    env: &Env,
    from: &Address,
    to: &Address,
    amount: i128,
    pending_reward_estimate: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("pos_xf_rw"), from);
    env.events().publish(
        topics,
        (to.clone(), amount, pending_reward_estimate, ledger),
    );
}

pub fn campaign_started(env: &Env, admin: &Address, multiplier_bps: u32, ends_at_ledger: u32) {
    let topics = (symbol_short!("camp_on"), admin);
    env.events().publish(
        topics,
        (multiplier_bps, ends_at_ledger, env.ledger().sequence()),
    );
}

pub fn campaign_ended(env: &Env, admin: &Address) {
    let topics = (symbol_short!("camp_off"), admin);
    env.events().publish(topics, (env.ledger().sequence(),));
}

/// Emitted when accrued rewards are automatically compounded back into stake.
pub fn auto_restaked(env: &Env, user: &Address, amount: i128) {
    let topics = (symbol_short!("auto_rst"), user);
    env.events()
        .publish(topics, (amount, env.ledger().sequence()));
}

/// Emitted when the contract automatically pauses because reward funding dropped too low.
pub fn auto_paused(env: &Env, reward_balance: i128, threshold: i128) {
    let topics = (symbol_short!("auto_ps"), env.current_contract_address());
    env.events()
        .publish(topics, (reward_balance, threshold, env.ledger().sequence()));
}

/// Emitted when a user claims staking rewards (via `claim` or `stake_and_claim`).
pub fn claimed(env: &Env, user: &Address, reward: i128, ledger: u32) {
    let topics = (symbol_short!("claimed"), user);
    env.events().publish(topics, (reward, ledger));
}

/// Emitted by `claim()` when a configured gas rebate is paid out of the
/// dedicated gas-rebate pool alongside the ordinary reward payout (issue #569).
pub fn gas_rebate_paid(env: &Env, user: &Address, amount: i128) {
    let topics = (symbol_short!("gas_rbt"), user);
    env.events()
        .publish(topics, (amount, env.ledger().sequence()));
}

/// Emitted by `initialize` so indexers can detect new pool deployments on-chain.
pub fn pool_initialized(
    env: &Env,
    admin: &Address,
    stake_token: &Address,
    reward_token: &Address,
    reward_rate_bps: u32,
) {
    let topics = (symbol_short!("init"), admin);
    env.events().publish(
        topics,
        (
            admin.clone(),
            stake_token.clone(),
            reward_token.clone(),
            reward_rate_bps,
        ),
    );
}

// ── Issue #101: frozen position event ─────────────────────────────────────────

/// Emitted when an admin flags a user's position as frozen via `flag_frozen`.
pub fn frozen_position(env: &Env, admin: &Address, user: &Address, frozen_at: u32) {
    let topics = (symbol_short!("frz_pos"), admin);
    env.events()
        .publish(topics, (user.clone(), frozen_at, env.ledger().sequence()));
}

// ── Issue #107: emergency stop event ─────────────────────────────────────────

/// Emitted once when `emergency_stop` is triggered. This event is permanent
/// and irreversible — the contract will never accept new stakes again.
pub fn stopped(env: &Env, admin: &Address) {
    let topics = (symbol_short!("stopped"), admin);
    env.events().publish(topics, env.ledger().sequence());
}

/// Emitted once when `start_graceful_shutdown` is triggered. New stakes are
/// permanently blocked; existing stakers can still unstake, claim, and
/// withdraw vested rewards normally.
pub fn shutdown_started(env: &Env, admin: &Address) {
    let topics = (symbol_short!("shutdown"), admin);
    env.events().publish(topics, env.ledger().sequence());
}

// ── Issue #97: pool description event ────────────────────────────────────────

/// Emitted when the admin updates the pool description.
pub fn description_updated(env: &Env, admin: &Address, description: &soroban_sdk::String) {
    let topics = (symbol_short!("desc_upd"), admin);
    env.events()
        .publish(topics, (description.clone(), env.ledger().sequence()));
}

/// Emitted when a user merges their staking positions.
pub fn positions_merged(env: &Env, user: &Address, count_merged: u32, total_amount: i128) {
    let topics = (symbol_short!("merge"), user);
    env.events().publish(
        topics,
        (count_merged, total_amount, env.ledger().sequence()),
    );
}

// ── Issue #118: relayer approval events ───────────────────────────────────────

/// Emitted when a user approves a relayer.
pub fn relayer_approved(env: &Env, user: &Address, relayer: &Address) {
    let topics = (symbol_short!("rlyr_set"), user);
    env.events()
        .publish(topics, (relayer.clone(), env.ledger().sequence()));
}

/// Emitted when a user revokes a relayer.
pub fn relayer_revoked(env: &Env, user: &Address, relayer: &Address) {
    let topics = (symbol_short!("rlyr_rev"), user);
    env.events()
        .publish(topics, (relayer.clone(), env.ledger().sequence()));
}

/// Emitted when a relayer claims rewards on behalf of a user.
pub fn claimed_on_behalf(env: &Env, relayer: &Address, user: &Address, reward: i128) {
    let topics = (symbol_short!("claimed"), user);
    env.events()
        .publish(topics, (reward, relayer.clone(), env.ledger().sequence()));
}

// ── Issue #126: yield source events ───────────────────────────────────────────

/// Emitted when a yield source notifies the contract of a new reward deposit.
pub fn reward_added(env: &Env, source: &Address, amount: i128) {
    let topics = (symbol_short!("rwd_add"), source);
    env.events()
        .publish(topics, (amount, env.ledger().sequence()));
}

/// Emitted when admin adds an address to the yield source whitelist.
pub fn yield_source_added(env: &Env, admin: &Address, source: &Address) {
    let topics = (symbol_short!("ys_add"), admin);
    env.events()
        .publish(topics, (source.clone(), env.ledger().sequence()));
}

/// Emitted when admin removes an address from the yield source whitelist.
pub fn yield_source_removed(env: &Env, admin: &Address, source: &Address) {
    let topics = (symbol_short!("ys_rem"), admin);
    env.events()
        .publish(topics, (source.clone(), env.ledger().sequence()));
}

// ── Issue #157: pool name events ──────────────────────────────────────────────

/// Emitted when the admin sets or updates the pool name via `set_pool_name`.
// `set_pool_name` itself is not currently wired up; kept for a future feature.
#[allow(dead_code)]
pub fn pool_name_updated(env: &Env, admin: &Address, name: &soroban_sdk::String) {
    let topics = (symbol_short!("nm_upd"), admin);
    env.events()
        .publish(topics, (name.clone(), env.ledger().sequence()));
}

// ── Reward refill alert ───────────────────────────────────────────────────────

/// Emitted when the reward pool runway drops below 30 days after a claim.
///
/// Topics: `("rfil_alt", vault_address)`.
/// Data: `(reward_balance: i128, ledgers_until_empty: u32, ledger: u32)`.
pub fn refill_alert(env: &Env, reward_balance: i128, ledgers_until_empty: u32, ledger: u32) {
    let topics = (symbol_short!("rfil_alt"), env.current_contract_address());
    env.events()
        .publish(topics, (reward_balance, ledgers_until_empty, ledger));
}

/// Emitted when the admin sets or updates the emergency contact string.
pub fn emergency_contact_updated(env: &Env, admin: &Address, contact: &soroban_sdk::String) {
    let topics = (symbol_short!("emg_cnt"), admin);
    env.events()
        .publish(topics, (contact.clone(), env.ledger().sequence()));
}

// ── Issue #219: pause with reason ────────────────────────────────────────────

/// Emitted when the admin pauses the pool with a reason and message.
pub fn paused_with_reason(
    env: &Env,
    admin: &Address,
    reason: &crate::storage::PauseReason,
    message: &soroban_sdk::String,
    ledger: u32,
) {
    let topics = (symbol_short!("paused"), admin);
    env.events()
        .publish(topics, (*reason, message.clone(), ledger));
}

// ── Issue #218: pool-to-pool migration ──────────────────────────────────────

/// Emitted when a user migrates their position to a target pool.
pub fn migrated(
    env: &Env,
    user: &Address,
    amount: i128,
    rewards_settled: i128,
    target_pool: &Address,
    ledger: u32,
) {
    let topics = (symbol_short!("migrated"), user);
    env.events().publish(
        topics,
        (amount, rewards_settled, target_pool.clone(), ledger),
    );
}

/// Emitted when the migration target pool is set.
pub fn migration_target_set(env: &Env, admin: &Address, target_pool: &Address, ledger: u32) {
    let topics = (symbol_short!("mig_tgt"), admin);
    env.events().publish(topics, (target_pool.clone(), ledger));
}

// ── Issue #215: yield farming hook ────────────────────────────────────────────

/// Emitted when the admin registers the external yield protocol.
pub fn yield_protocol_set(env: &Env, admin: &Address, protocol: &Address) {
    let topics = (symbol_short!("yld_p_set"), admin);
    env.events()
        .publish(topics, (protocol.clone(), env.ledger().sequence()));
}

/// Emitted when idle stake tokens are deployed to the yield protocol.
pub fn yield_deployed(env: &Env, admin: &Address, amount: i128, ledger: u32) {
    let topics = (symbol_short!("yld_dep"), admin);
    env.events().publish(topics, (amount, ledger));
}

/// Emitted when tokens (plus any accrued yield) are withdrawn from the yield protocol.
pub fn yield_withdrawn(env: &Env, admin: &Address, amount_returned: i128, ledger: u32) {
    let topics = (symbol_short!("yld_wdrw"), admin);
    env.events().publish(topics, (amount_returned, ledger));
}

// ── Issue #216: governance voting ─────────────────────────────────────────────

/// Emitted when a staker creates a new governance proposal.
pub fn proposal_created(env: &Env, proposer: &Address, proposal_id: u32, ends_at: u32) {
    let topics = (symbol_short!("prop_new"), proposer);
    env.events().publish(topics, (proposal_id, ends_at));
}

/// Emitted when a proposal's bond is refunded to its proposer at
/// `enact_proposal` time because the proposal reached quorum (win or
/// lose). Distinct from `proposal_cancelled`'s refund, which happens
/// earlier, when the proposer withdraws the proposal themselves.
pub fn proposal_bond_refunded(
    env: &Env,
    proposer: &Address,
    proposal_id: u32,
    amount: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("bond_rfnd"), proposer);
    env.events().publish(topics, (proposal_id, amount, ledger));
}

/// Emitted when a proposal's bond is slashed to the treasury at
/// `enact_proposal` time because the proposal failed to reach quorum.
pub fn proposal_bond_slashed(
    env: &Env,
    proposer: &Address,
    proposal_id: u32,
    amount: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("bond_slsh"), proposer);
    env.events().publish(topics, (proposal_id, amount, ledger));
}

/// Emitted when the original proposer cancels their own proposal
/// (`cancel_proposal`) before the window closes. `bond_refunded` is `0`
/// when no bond was collected at creation.
pub fn proposal_cancelled(
    env: &Env,
    proposer: &Address,
    proposal_id: u32,
    bond_refunded: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("prop_cncl"), proposer);
    env.events()
        .publish(topics, (proposal_id, bond_refunded, ledger));
}

/// Emitted when a staker votes on a governance proposal.
pub fn proposal_voted(
    env: &Env,
    voter: &Address,
    proposal_id: u32,
    support: bool,
    weight: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("prop_vote"), voter);
    env.events()
        .publish(topics, (proposal_id, support, weight, ledger));
}

/// Emitted when a voter changes their side on an open proposal
/// (`change_vote`). `weight` is the voter's originally recorded weight
/// (unchanged by the switch) and `change_count` is how many times they have
/// now changed their vote on this proposal.
pub fn vote_changed(
    env: &Env,
    voter: &Address,
    proposal_id: u32,
    new_support: bool,
    weight: i128,
    change_count: u32,
    ledger: u32,
) {
    let topics = (symbol_short!("vote_chg"), voter);
    env.events().publish(
        topics,
        (proposal_id, new_support, weight, change_count, ledger),
    );
}

/// Emitted when a governance proposal is enacted (whether or not it passed).
///
/// Topics: `("prop_enct", vault_address)`.
/// Data: `(id, parameter, new_value, total_votes, ledger)`.
pub fn proposal_enacted(
    env: &Env,
    id: u32,
    parameter: crate::storage::ProposableParam,
    new_value: i128,
    total_votes: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("prop_enct"), env.current_contract_address());
    env.events()
        .publish(topics, (id, parameter, new_value, total_votes, ledger));
}

// ── Issue #182: webhook URL config ───────────────────────────────────────────

/// Emitted when the admin sets or clears the stake-event webhook URL.
///
/// `url` is `None` when the webhook has just been cleared.
pub fn webhook_url_updated(
    env: &Env,
    admin: &Address,
    url: &Option<soroban_sdk::String>,
    ledger: u32,
) {
    let topics = (symbol_short!("whk_url"), admin);
    env.events().publish(topics, (url.clone(), ledger));
}

// ── Issue #210: Merkle Reward Distribution ────────────────────────────────────

pub fn merkle_claimed(env: &Env, user: &Address, amount: i128, epoch: u32, ledger: u32) {
    let topics = (symbol_short!("mrk_clm"), user);
    env.events().publish(topics, (amount, epoch, ledger));
}

// ── Issue #211: Staking Tournament Competition ─────────────────────────────────

pub fn tournament_winner(env: &Env, winner: &Address, score: i128, prize_paid: i128, ledger: u32) {
    let topics = (symbol_short!("tour_win"), winner);
    env.events()
        .publish(topics, (winner, score, prize_paid, ledger));
}

// ── Issue #212: Buyback & Burn ────────────────────────────────────────────────

pub fn buyback_executed(env: &Env, fee_amount_used: i128, reward_tokens_burned: i128, ledger: u32) {
    let topics = (symbol_short!("buyback"), env.current_contract_address());
    env.events()
        .publish(topics, (fee_amount_used, reward_tokens_burned, ledger));
}

pub fn buyback_toggled(env: &Env, admin: &Address, enabled: bool) {
    let topics = (symbol_short!("buy_tog"), admin);
    env.events()
        .publish(topics, (enabled, env.ledger().sequence()));
}

pub fn merkle_root_published(env: &Env, admin: &Address, epoch: u32, total_claimable: i128) {
    let topics = (symbol_short!("mrk_pub"), admin);
    env.events()
        .publish(topics, (epoch, total_claimable, env.ledger().sequence()));
}

// ── Events referenced by existing feature branches ────────────────────────────

pub fn state_exported(env: &Env, admin: &Address, num_positions: u32, ledger: u32) {
    let topics = (symbol_short!("st_exp"), admin);
    env.events().publish(topics, (num_positions, ledger));
}

pub fn penalty_redistributed(env: &Env, total_amount: i128, recipient_count: u32, ledger: u32) {
    let topics = (symbol_short!("pen_rdst"), env.current_contract_address());
    env.events()
        .publish(topics, (total_amount, recipient_count, ledger));
}

pub fn insurance_deployed(env: &Env, admin: &Address, amount: i128, new_balance: i128) {
    let topics = (symbol_short!("ins_dep"), admin);
    env.events()
        .publish(topics, (amount, new_balance, env.ledger().sequence()));
}

// ── Issue #231: halving occurred ──────────────────────────────────────────────

/// Emitted when a reward calculation crosses a halving boundary during
/// claim or stake operations.
///
/// Not yet wired into `reward_between_ledgers`: that walk splits segments at
/// halving boundaries without a hook to emit from. Kept so the halving schedule
/// has an event to publish once one exists.
#[allow(dead_code)]
pub fn halving_occurred(env: &Env, halving_count: u32, effective_rate_bps: i128, ledger: u32) {
    let topics = (symbol_short!("halving"), env.current_contract_address());
    env.events()
        .publish(topics, (halving_count, effective_rate_bps, ledger));
}

// ── Issue #222: staking certificate events ────────────────────────────────────

pub fn certificate_issued(
    env: &Env,
    user: &Address,
    certificate_id: u32,
    valid_until: u32,
    ledger: u32,
) {
    let topics = (symbol_short!("cert_iss"), user);
    env.events()
        .publish(topics, (certificate_id, valid_until, ledger));
}

// ── Issue #233: pool activation events ────────────────────────────────────────

pub fn pool_activated(env: &Env, total_staked: i128, threshold: i128, ledger: u32) {
    let topics = (symbol_short!("pool_act"), env.current_contract_address());
    env.events()
        .publish(topics, (total_staked, threshold, ledger));
}

pub fn pool_deactivated(env: &Env, total_staked: i128, threshold: i128, ledger: u32) {
    let topics = (symbol_short!("pool_dact"), env.current_contract_address());
    env.events()
        .publish(topics, (total_staked, threshold, ledger));
}

// ── Issue #234: reward activation events ──────────────────────────────────────

/// Emitted the first time pool TVL reaches the configured minimum pool size,
/// which is the ledger reward accrual starts from.
pub fn rewards_activated(env: &Env, total_staked: i128, min_pool_size: i128, ledger: u32) {
    let topics = (symbol_short!("rwd_act"), env.current_contract_address());
    env.events()
        .publish(topics, (total_staked, min_pool_size, ledger));
}

/// Emitted when the admin changes the minimum pool size required for rewards.
pub fn min_pool_size_set(env: &Env, admin: &Address, old_amount: i128, new_amount: i128) {
    let topics = (symbol_short!("minpool"), admin);
    env.events()
        .publish(topics, (old_amount, new_amount, env.ledger().sequence()));
}

// ── Issue #235: reward smoothing events ───────────────────────────────────────

/// Emitted when a large yield addition is scheduled for linear release instead
/// of being credited to the pool immediately.
pub fn smoothing_scheduled(
    env: &Env,
    total_amount: i128,
    start_ledger: u32,
    duration_ledgers: u32,
) {
    let topics = (symbol_short!("smth_sch"), env.current_contract_address());
    env.events()
        .publish(topics, (total_amount, start_ledger, duration_ledgers));
}

/// Emitted when a slice of a smoothing schedule is credited to the pool.
pub fn smoothing_released(env: &Env, amount: i128, cumulative_released: i128, total_amount: i128) {
    let topics = (symbol_short!("smth_rel"), env.current_contract_address());
    env.events().publish(
        topics,
        (
            amount,
            cumulative_released,
            total_amount,
            env.ledger().sequence(),
        ),
    );
}

// ── Issue #237: capacity auction events ───────────────────────────────────────

pub fn auction_started(env: &Env, spots: u32, min_bid: i128, ends_at: u32) {
    let topics = (symbol_short!("auct_st"), env.current_contract_address());
    env.events()
        .publish(topics, (spots, min_bid, ends_at, env.ledger().sequence()));
}

pub fn bid_placed(env: &Env, bidder: &Address, amount: i128, total_bid: i128) {
    let topics = (symbol_short!("bid_plcd"), bidder);
    env.events()
        .publish(topics, (amount, total_bid, env.ledger().sequence()));
}

pub fn auction_won(env: &Env, bidder: &Address, amount: i128, rank: u32) {
    let topics = (symbol_short!("auct_won"), bidder);
    env.events()
        .publish(topics, (amount, rank, env.ledger().sequence()));
}

pub fn bid_refunded(env: &Env, bidder: &Address, amount: i128) {
    let topics = (symbol_short!("bid_rfnd"), bidder);
    env.events()
        .publish(topics, (amount, env.ledger().sequence()));
}

pub fn auction_finalized(env: &Env, winner_count: u32, total_staked: i128, refunded: i128) {
    let topics = (symbol_short!("auct_fin"), env.current_contract_address());
    env.events().publish(
        topics,
        (
            winner_count,
            total_staked,
            refunded,
            env.ledger().sequence(),
        ),
    );
}

// ── Issue #232: position expired event ────────────────────────────────────────

pub fn position_expired(env: &Env, user: &Address, expired_at_ledger: u32, current_ledger: u32) {
    let topics = (symbol_short!("pos_exp"), user);
    env.events()
        .publish(topics, (expired_at_ledger, current_ledger));
}

pub fn dynamic_fee_config_updated(
    env: &Env,
    admin: &Address,
    base_fee_bps: u32,
    max_fee_bps: u32,
    threshold_bps: u32,
) {
    let topics = (symbol_short!("dfeecfg"), admin);
    env.events().publish(
        topics,
        (
            base_fee_bps,
            max_fee_bps,
            threshold_bps,
            env.ledger().sequence(),
        ),
    );
}

// ── Issue #239: stake-weighted lottery ────────────────────────────────────────

pub fn lottery_drawn(
    env: &Env,
    winner: &Address,
    prize_amount: i128,
    random_seed: u64,
    ledger: u32,
) {
    let topics = (symbol_short!("lot_drwn"), winner);
    env.events()
        .publish(topics, (prize_amount, random_seed, ledger));
}

// ── Issue #238: loyalty milestone badges ──────────────────────────────────────

pub fn milestone_achieved(
    env: &Env,
    user: &Address,
    milestone_id: u32,
    milestone_name: &soroban_sdk::String,
    ledger: u32,
) {
    let topics = (symbol_short!("mstn_ach"), user);
    env.events()
        .publish(topics, (milestone_id, milestone_name.clone(), ledger));
}

/// Emitted when a user successfully completes a quiz and unlocks a reward tier.
pub fn quiz_completed(env: &Env, user: &Address, quiz_id: u32, tier_unlocked: u32, ledger: u32) {
    let topics = (symbol_short!("quiz_comp"), user);
    env.events()
        .publish(topics, (quiz_id, tier_unlocked, ledger));
}

/// Emitted when a user fails a quiz attempt.
pub fn quiz_attempt_failed(
    env: &Env,
    user: &Address,
    quiz_id: u32,
    remaining_attempts: u32,
    ledger: u32,
) {
    let topics = (symbol_short!("quiz_fail"), user);
    env.events()
        .publish(topics, (quiz_id, remaining_attempts, ledger));
}

/// Emitted when an admin adds a new quiz.
pub fn quiz_added(env: &Env, admin: &Address, quiz_id: u32, tier_unlocked: u32, ledger: u32) {
    let topics = (symbol_short!("quiz_add"), admin);
    env.events()
        .publish(topics, (quiz_id, tier_unlocked, ledger));
}

// ── Issue #240: oracle-triggered lock-up release ──────────────────────────────

pub fn condition_triggered(
    env: &Env,
    user: &Address,
    asset_id: &soroban_sdk::String,
    oracle_price: i128,
    trigger_price: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("cond_trg"), user);
    env.events().publish(
        topics,
        (asset_id.clone(), oracle_price, trigger_price, ledger),
    );
}

// ── Issue #241: governance proposal veto ──────────────────────────────────────

pub fn proposal_vetoed(
    env: &Env,
    user: &Address,
    proposal_id: u32,
    user_stake_bps: u32,
    ledger: u32,
) {
    let topics = (symbol_short!("prop_veto"), user);
    env.events()
        .publish(topics, (proposal_id, user_stake_bps, ledger));
}

/// Emitted when the guardian vetoes a proposal that passed its vote, during
/// its execution delay. Distinct from `proposal_vetoed` above (the
/// stake-weighted veto, issue #241): this one carries a `reason_hash`
/// instead of a stake percentage, since the guardian's authority to veto
/// comes from its role, not from holding a share of the pool.
pub fn guardian_proposal_vetoed(
    env: &Env,
    guardian: &Address,
    proposal_id: u32,
    reason_hash: &soroban_sdk::BytesN<32>,
    ledger: u32,
) {
    let topics = (Symbol::new(env, "proposal_vetoed"), guardian);
    env.events()
        .publish(topics, (proposal_id, reason_hash.clone(), ledger));
}

// ── Issue #256: governance vote weight delegation ──────────────────────────────

pub fn vote_delegated(
    env: &Env,
    delegator: &Address,
    delegate: &Address,
    weight: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("votedeleg"), delegator);
    env.events()
        .publish(topics, (delegate.clone(), weight, ledger));
}

// ── Issue #257: auto-convert reward on claim ────────────────────────────────────

pub fn reward_converted(
    env: &Env,
    user: &Address,
    reward_amount: i128,
    converted_amount: i128,
    target_token: &Address,
    ledger: u32,
) {
    let topics = (symbol_short!("rwd_conv"), user);
    env.events().publish(
        topics,
        (
            reward_amount,
            converted_amount,
            target_token.clone(),
            ledger,
        ),
    );
}

// ── Issue #251: exit-queue priority bidding ─────────────────────────────────────

pub fn priority_bid_placed(
    env: &Env,
    user: &Address,
    bid_amount: i128,
    previous_position: u32,
    ledger: u32,
) {
    let topics = (symbol_short!("pbid_plc"), user);
    env.events()
        .publish(topics, (bid_amount, previous_position, ledger));
}

// ── Issue #258: pool whitelabel branding ──────────────────────────────────────

pub fn branding_updated(
    env: &Env,
    admin: &Address,
    display_name: &soroban_sdk::String,
    website_url: &soroban_sdk::String,
    ledger: u32,
) {
    let topics = (symbol_short!("brand_upd"), admin);
    env.events()
        .publish(topics, (display_name.clone(), website_url.clone(), ledger));
}

// ── Issue #259: staking insurance ─────────────────────────────────────────────

pub fn insurance_activated(
    env: &Env,
    user: &Address,
    premium_bps: u32,
    coverage_amount: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("ins_act"), user);
    env.events()
        .publish(topics, (premium_bps, coverage_amount, ledger));
}

pub fn insurance_cancelled(env: &Env, user: &Address, coverage_amount: i128, ledger: u32) {
    let topics = (symbol_short!("ins_canc"), user);
    env.events().publish(topics, (coverage_amount, ledger));
}

pub fn insurance_premium_paid(env: &Env, user: &Address, premium: i128, ledger: u32) {
    let topics = (symbol_short!("ins_prem"), user);
    env.events().publish(topics, (premium, ledger));
}

pub fn shortfall_paid(env: &Env, user: &Address, payout: i128, ledger: u32) {
    let topics = (symbol_short!("shortfal"), user);
    env.events().publish(topics, (payout, ledger));
}

// ── Issue #260: flash stake ───────────────────────────────────────────────────

pub fn flash_staked(env: &Env, user: &Address, amount: i128, receipt_id: u64, ledger: u32) {
    let topics = (symbol_short!("flash_st"), user);
    env.events().publish(topics, (amount, receipt_id, ledger));
}

// ── Issue #261: stake-backed loans ────────────────────────────────────────────

pub fn loan_opened(env: &Env, user: &Address, amount: i128, total_principal: i128, ledger: u32) {
    let topics = (symbol_short!("loan_opn"), user);
    env.events()
        .publish(topics, (amount, total_principal, ledger));
}

pub fn loan_repaid(
    env: &Env,
    user: &Address,
    amount: i128,
    remaining_principal: i128,
    remaining_interest: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("loan_rpy"), user);
    env.events().publish(
        topics,
        (amount, remaining_principal, remaining_interest, ledger),
    );
}

pub fn loan_liquidated(
    env: &Env,
    user: &Address,
    debt_covered: i128,
    shares_slashed: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("loan_liq"), user);
    env.events()
        .publish(topics, (debt_covered, shares_slashed, ledger));
}

// ── Issue #275: reward Gini coefficient ───────────────────────────────────────

pub fn gini_computed(env: &Env, result_bps: u32, staker_count: u32, ledger: u32) {
    let topics = (symbol_short!("gini_cmp"), env.current_contract_address());
    env.events()
        .publish(topics, (result_bps, staker_count, ledger));
}

// ── Issue #276: seasonal reward multiplier ────────────────────────────────────

/// `starts_at` identifies the season (stable across `remove_season()` calls
/// on other entries, unlike a Vec index).
pub fn season_started(
    env: &Env,
    starts_at: u32,
    name: &soroban_sdk::String,
    multiplier_bps: u32,
    ledger: u32,
) {
    let topics = (symbol_short!("seas_str"), env.current_contract_address());
    env.events()
        .publish(topics, (starts_at, name.clone(), multiplier_bps, ledger));
}

pub fn season_ended(env: &Env, starts_at: u32, ledger: u32) {
    let topics = (symbol_short!("seas_end"), env.current_contract_address());
    env.events().publish(topics, (starts_at, ledger));
}

// ── Issue #274: staker bio ────────────────────────────────────────────────────

/// No bio content in the event payload by design — keeps the event lean and
/// avoids duplicating free-text content on-chain in two places.
pub fn bio_updated(env: &Env, user: &Address, ledger: u32) {
    let topics = (symbol_short!("bio_upd"), user);
    env.events().publish(topics, (ledger,));
}

// ── Issue #298: pool sunsetting workflow ──────────────────────────────────────

pub fn sunset_announced(env: &Env, admin: &Address, grace_period_end: u32, ledger: u32) {
    let topics = (symbol_short!("snst_ann"), admin);
    env.events().publish(topics, (grace_period_end, ledger));
}

pub fn sunset_stage_changed(env: &Env, new_state: crate::storage::SunsetState, ledger: u32) {
    let topics = (symbol_short!("snst_chg"), env.current_contract_address());
    env.events().publish(topics, (new_state, ledger));
}

/// Emitted once when `initiate_sunset` starts a graceful pool sunset
/// (issue #525). `exit_deadline` is the ledger by which existing users are
/// asked to have exited; deposits stay blocked for good afterwards, while
/// withdrawals and claims remain open.
pub fn sunset_initiated(env: &Env, admin: &Address, exit_deadline: u32) {
    let topics = (symbol_short!("snst_ini"), admin);
    env.events()
        .publish(topics, (exit_deadline, env.ledger().sequence()));
}

pub fn force_resolved(
    env: &Env,
    user: &Address,
    principal: i128,
    accrued_reward: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("frc_res"), user);
    env.events()
        .publish(topics, (principal, accrued_reward, ledger));
}

// ── Issue #286: debt NFT collateral ─────────────────────────────────────────

pub fn debt_nft_minted(env: &Env, issuer: &Address, nft_id: u32, face_value: i128, ledger: u32) {
    let topics = (symbol_short!("dbt_mnt"), issuer);
    env.events().publish(topics, (nft_id, face_value, ledger));
}

pub fn debt_nft_burned(env: &Env, holder: &Address, nft_id: u32, ledger: u32) {
    let topics = (symbol_short!("dbt_brn"), holder);
    env.events().publish(topics, (nft_id, ledger));
}

pub fn debt_nft_transferred(env: &Env, from: &Address, to: &Address, nft_id: u32, ledger: u32) {
    let topics = (symbol_short!("dbt_trn"), from);
    env.events().publish(topics, (to, nft_id, ledger));
}

// ── Issue #283: position AMM ─────────────────────────────────────────────────

pub fn swap_executed(
    env: &Env,
    offerer: &Address,
    counterparty: &Address,
    offerer_amount: i128,
    counterparty_amount: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("swap_exe"), offerer);
    env.events().publish(
        topics,
        (counterparty, offerer_amount, counterparty_amount, ledger),
    );
}

// ── Issue #284: reward prediction market ─────────────────────────────────────

pub fn market_resolved(
    env: &Env,
    outcome: bool,
    winning_side_total: i128,
    losing_side_total: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("mkt_rsl"), env.current_contract_address());
    env.events()
        .publish(topics, (outcome, winning_side_total, losing_side_total, ledger));
    let topics = (symbol_short!("mkt_rsl"),);
    env.events().publish(
        topics,
        (outcome, winning_side_total, losing_side_total, ledger),
    );
}

// ── Issue #285: cross-pool yield detector ────────────────────────────────────

pub fn higher_yield_detected(
    env: &Env,
    competitor: &Address,
    their_rate: i128,
    our_rate: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("hi_yld"), competitor);
    env.events().publish(topics, (their_rate, our_rate, ledger));
}
// ── Issue #281: Fee Revenue Sharing ──────────────────────────────────────────

pub fn revenue_distributed(
    env: &Env,
    merkle_root: &soroban_sdk::Bytes,
    total_amount: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("rev_dist"), env.current_contract_address());
    env.events()
        .publish(topics, (merkle_root.clone(), total_amount, ledger));
}

pub fn revenue_share_claimed(env: &Env, user: &Address, amount: i128, epoch: u32, ledger: u32) {
    let topics = (symbol_short!("rev_clm"), user);
    env.events().publish(topics, (amount, epoch, ledger));
}

// ── Issue #280: New Staker Reward Escrow ────────────────────────────────────

pub fn escrow_released(env: &Env, user: &Address, escrowed_amount: i128, ledger: u32) {
    let topics = (symbol_short!("esc_rel"), user);
    env.events()
        .publish(topics, (user.clone(), escrowed_amount, ledger));
}

// ── Issue #282: Stake-Gated Access ───────────────────────────────────────────

pub fn access_token_issued(env: &Env, user: &Address, tier_index: u32, ledger: u32) {
    let topics = (symbol_short!("acc_iss"), user);
    env.events().publish(topics, (tier_index, ledger));
}

pub fn access_token_revoked(
    env: &Env,
    user: &Address,
    tier_index: u32,
    reason: Symbol,
    ledger: u32,
) {
    let topics = (symbol_short!("acc_rev"), user);
    env.events().publish(topics, (tier_index, reason, ledger));
}

// ── Issue #312: gift staking position ────────────────────────────────────────

pub fn gift_staked(env: &Env, sender: &Address, recipient: &Address, amount: i128, ledger: u32) {
    let topics = (symbol_short!("gift_stk"), sender);
    env.events()
        .publish(topics, (recipient.clone(), amount, ledger));
}

// ── Issue #313: anniversary bonus ────────────────────────────────────────────

pub fn anniversary_bonus_paid(
    env: &Env,
    user: &Address,
    anniversary_number: u32,
    bonus_amount: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("anniv_bp"), user);
    env.events()
        .publish(topics, (anniversary_number, bonus_amount, ledger));
}

// ── Issue #314: withdrawal receipt NFT ───────────────────────────────────────

pub fn withdrawal_receipt_minted(
    env: &Env,
    user: &Address,
    receipt_id: u64,
    amount_returned: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("wd_rcpt"), user);
    env.events()
        .publish(topics, (receipt_id, amount_returned, ledger));
}


pub fn fee_buyback_executed(
    env: &Env,
    admin: &Address,
    stake_fees_used: i128,
    reward_tokens_burned: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("fee_buyb"), admin);
    env.events()
        .publish(topics, (stake_fees_used, reward_tokens_burned, ledger));
}

pub fn onboarding_completed(env: &Env, user: &Address, ledger: u32) {
    let topics = (symbol_short!("onb_done"), user);
    env.events().publish(topics, (ledger,));
}
/// Issue #242: emitted when a stake receives a matching contribution.
pub fn stake_matched(
    env: &Env,
    user: &Address,
    stake_amount: i128,
    match_amount: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("stk_match"), user);
    env.events()
        .publish(topics, (stake_amount, match_amount, ledger));
}

/// Issue #243: emitted when a user purchases unstake insurance.
pub fn insurance_purchased(env: &Env, user: &Address, premium: i128, coverage: i128, ledger: u32) {
    let topics = (symbol_short!("ins_buy"), user);
    env.events().publish(topics, (premium, coverage, ledger));
}

// ── Keeper registry ───────────────────────────────────────────────────────────

pub fn keeper_registered(env: &Env, keeper: &Address, ledger: u32) {
    let topics = (symbol_short!("kpr_reg"), keeper);
    env.events().publish(topics, (ledger,));
}

pub fn keeper_deregistered(env: &Env, keeper: &Address, ledger: u32) {
    let topics = (symbol_short!("kpr_dreg"), keeper);
    env.events().publish(topics, (ledger,));
}

// ── Epoch reward outflow cap ─────────────────────────────────────────────────

/// Emitted when a claim exceeds the remaining epoch cap headroom and the
/// excess is queued as a `DeferredReward`, claimable once `next_epoch_start`
/// is reached.
pub fn reward_deferred(env: &Env, user: &Address, deferred_amount: i128, next_epoch_start: u32) {
    let topics = (symbol_short!("rwd_defr"), user);
    env.events().publish(
        topics,
        (deferred_amount, next_epoch_start, env.ledger().sequence()),
    );
}

/// Issue #244: emitted when rewards are claimed in a non-stake output token.
// ── Issue #374: admin action nonce (replay protection) ───────────────────────

/// Emitted by `execute_admin_action_with_nonce()` after a nonce-gated admin
/// action successfully executes. `nonce` is the value that was just consumed
/// — the admin's next call must supply `nonce + 1`.
pub fn admin_action_nonce_consumed(env: &Env, admin: &Address, nonce: u64, ledger: u32) {
    let topics = (symbol_short!("adm_nonce"), admin);
    env.events().publish(topics, (nonce, ledger));
}

// ── Issue #376: halving countdown ─────────────────────────────────────────────
// Read-only advisory (like `reward_smoothing()`'s `SmoothingStatus`) — no
// event to emit, nothing about pool state changes when it's queried.

// ── Issue #375: governance proposal comment thread ────────────────────────────

/// Emitted by `post_proposal_comment()` when a stake-weighted comment is
/// added to a proposal's thread.
pub fn proposal_comment_posted(
    env: &Env,
    author: &Address,
    proposal_id: u32,
    stake_weight: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("prop_cmt"), author);
    env.events()
        .publish(topics, (proposal_id, stake_weight, ledger));
}

// ── Issue #377: position health alert ─────────────────────────────────────────

/// Emitted by `position_health_alert()` when at least one attention-worthy
/// condition is true for the checked position.
pub fn position_health_alert(
    env: &Env,
    user: &Address,
    approaching_expiry: bool,
    lock_ending_soon: bool,
    loan_at_risk: bool,
    rewards_near_cap: bool,
    ledger: u32,
) {
    let topics = (symbol_short!("pos_hlth"), user);
    env.events().publish(
        topics,
        (
            approaching_expiry,
            lock_ending_soon,
            loan_at_risk,
            rewards_near_cap,
            ledger,
        ),
    );
}

pub fn reward_claimed_in_token(
    env: &Env,
    user: &Address,
    reward_amount: i128,
    output_token: &Address,
    output_amount: i128,
    ledger: u32,
) {
    let topics = (symbol_short!("rw_in_tok"), user);
    env.events().publish(
        topics,
        (reward_amount, output_token.clone(), output_amount, ledger),
    );
}

// ── Issue #392: loyalty points events ───────────────────────────────────────

pub fn points_awarded(env: &Env, user: &Address, amount: u32, new_balance: u32, ledger: u32) {
    let topics = (symbol_short!("loy_awd"), user.clone());
    env.events().publish(topics, (amount, new_balance, ledger));
}

pub fn points_redeemed(
    env: &Env,
    user: &Address,
    amount: u32,
    benefit: crate::storage::PointsBenefit,
    new_balance: u32,
    ledger: u32,
) {
    let topics = (symbol_short!("loy_rdm"), user.clone());
    env.events()
        .publish(topics, (amount, benefit, new_balance, ledger));
}

pub fn loyalty_config_updated(env: &Env, rules_len: u32, ledger: u32) {
    let topics = (symbol_short!("loy_cfg"),);
    env.events().publish(topics, (rules_len, ledger));
}

/// Emitted when an address's KYC approval status changes.
pub fn kyc_status_changed(env: &Env, user: &Address, approved: bool) {
    let topics = (symbol_short!("kyc_set"), user);
    env.events().publish(topics, approved);
}

// ── Issue #541: deposit receipt memo ─────────────────────────────────────────

/// Emitted by `deposit_with_memo()` (issue #541).
pub fn deposit_completed(
    env: &Env,
    depositor: &Address,
    amount: i128,
    shares_minted: i128,
    memo: &soroban_sdk::String,
    ledger: u32,
) {
    let topics = (Symbol::new(env, "deposit_completed"), depositor);
    env.events()
        .publish(topics, (amount, shares_minted, memo.clone(), ledger));
}

// ── Issue #540: scheduled reward-rate ramp ───────────────────────────────────

/// Emitted when a linear reward-rate ramp begins (issue #540).
pub fn rate_ramp_started(
    env: &Env,
    start_rate_bps: u32,
    target_rate_bps: u32,
    duration_ledgers: u32,
    start_ledger: u32,
) {
    let topics = (Symbol::new(env, "rate_ramp_started"),);
    env.events().publish(
        topics,
        (
            start_rate_bps,
            target_rate_bps,
            duration_ledgers,
            start_ledger,
        ),
    );
}

/// Emitted when a linear reward-rate ramp completes or is settled (issue #540).
pub fn rate_ramp_completed(env: &Env, final_rate_bps: u32, ledger: u32) {
    let topics = (Symbol::new(env, "rate_ramp_completed"),);
    env.events()
        .publish(topics, (final_rate_bps, ledger));
}

// ── Issue #539: per-token fee override ───────────────────────────────────────

/// Emitted when a per-token fee override is configured or cleared (issue #539).
pub fn token_fee_override_set(
    env: &Env,
    token: &Address,
    deposit_fee_bps: Option<u32>,
    unstake_fee_bps: Option<u32>,
    ledger: u32,
) {
    let topics = (Symbol::new(env, "token_fee_set"), token);
    env.events()
        .publish(topics, (deposit_fee_bps, unstake_fee_bps, ledger));
}

// ── Issues #542-545 ────────────────────────────────────────────────────────

/// Emitted once per low episode when the reward pool drops below the
/// admin-configured threshold relative to outstanding obligations (#544).
/// The notified flag resets on recovery, so a new episode emits again —
/// but repeated calls while continuously low never re-emit.
pub fn reward_pool_low(
    env: &Env,
    pool_balance: i128,
    outstanding_obligations: i128,
    threshold_bps: u32,
    ledger: u32,
) {
    let topics = (symbol_short!("rwpl_low"),);
    env.events().publish(
        topics,
        (pool_balance, outstanding_obligations, threshold_bps, ledger),
    );
}

/// Emitted when a keeper records an APY snapshot into the rolling history (#543).
pub fn apy_snapshot_recorded(env: &Env, apy_bps: u32, ledger: u32) {
    let topics = (symbol_short!("apy_snap"),);
    env.events().publish(topics, (apy_bps, ledger));
}

// ── Issues #701-#704: fee holiday and insurance reserve ──────────────────────

/// Emitted when the admin starts a fee holiday (issue #703).
pub fn fee_holiday_started(env: &Env, ends_at_ledger: u32, ledger: u32) {
    let topics = (Symbol::new(env, "fee_holiday_started"),);
    env.events().publish(topics, (ends_at_ledger, ledger));
}

/// Emitted when a fee holiday ends, either through `end_fee_holiday` or lazily
/// the first time a fee calculation sees that it has expired (issue #703).
pub fn fee_holiday_ended(env: &Env, ends_at_ledger: u32, ledger: u32) {
    let topics = (Symbol::new(env, "fee_holiday_ended"),);
    env.events().publish(topics, (ends_at_ledger, ledger));
}

/// Emitted when the admin releases funds from the insurance reserve (issue #704).
pub fn insurance_reserve_used(
    env: &Env,
    admin: &Address,
    amount: i128,
    reason_hash: &soroban_sdk::BytesN<32>,
    ledger: u32,
) {
    let topics = (Symbol::new(env, "insurance_reserve_used"), admin);
    env.events()
        .publish(topics, (amount, reason_hash.clone(), ledger));
}

/// Emitted when an admin proposes a new administrator address (Issue #738).
pub fn admin_proposal_created(env: &Env, admin: &Address, new_admin: &Address) {
    env.events().publish(
        (symbol_short!("adm_prop"), admin.clone()),
        (new_admin.clone(), env.ledger().sequence()),
    );
}

/// Emitted when a proposed admin accepts and completes the transfer (Issue #738).
pub fn admin_proposal_accepted(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (symbol_short!("adm_acpt"), old_admin.clone()),
        (new_admin.clone(), env.ledger().sequence()),
    );
}

/// Emitted when an admin cancels an active admin transfer proposal (Issue #738).
pub fn admin_proposal_cancelled(env: &Env, admin: &Address, cancelled_admin: &Address) {
    env.events().publish(
        (symbol_short!("adm_canc"), admin.clone()),
        (cancelled_admin.clone(), env.ledger().sequence()),
    );
}

/// Emitted when the global reward accumulator index updates (Issue #650).
pub fn reward_accumulator_updated(env: &Env, global_index: i128, ledger: u32) {
    env.events().publish(
        (symbol_short!("rw_acc"),),
        (global_index, ledger),
    );
}

/// Emitted when parameters for a proposal category are updated (issue #761).
pub fn category_params_set(
    env: &Env,
    admin: &Address,
    category: u32,
    quorum_bps: u32,
    threshold_bps: u32,
    period_ledgers: u32,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("cat_prm"), admin.clone()),
        (category, quorum_bps, threshold_bps, period_ledgers, ledger),
    );
}

/// Emitted when an expedited emergency proposal is created (issue #762).
pub fn expedited_proposal_created(
    env: &Env,
    proposer: &Address,
    proposal_id: u32,
    bond: i128,
    ends_at: u32,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("exp_prop"), proposer.clone()),
        (proposal_id, bond, ends_at, ledger),
    );
}

/// Emitted when an expedited proposal bond is refunded (issue #762).
pub fn expedited_bond_refunded(
    env: &Env,
    proposer: &Address,
    proposal_id: u32,
    bond: i128,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("exp_rfnd"), proposer.clone()),
        (proposal_id, bond, ledger),
    );
}

/// Emitted when an expedited proposal bond is slashed for failing quorum (issue #762).
pub fn expedited_bond_slashed(
    env: &Env,
    proposer: &Address,
    proposal_id: u32,
    bond: i128,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("exp_slsh"), proposer.clone()),
        (proposal_id, bond, ledger),
    );
}

/// Emitted when vote delegation with optional expiry is configured (issue #763).
pub fn vote_delegation_with_expiry_set(
    env: &Env,
    delegator: &Address,
    delegate: &Address,
    expires_at: Option<u32>,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("vdel_exp"), delegator.clone()),
        (delegate.clone(), expires_at, ledger),
    );
}

/// Emitted when an external yield strategy adapter is added (issue #764).
pub fn strategy_added(
    env: &Env,
    admin: &Address,
    adapter: &Address,
    cap_bps: u32,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("str_add"), admin.clone()),
        (adapter.clone(), cap_bps, ledger),
    );
}

/// Emitted when idle funds are allocated to a strategy adapter (issue #764).
pub fn strategy_allocated(
    env: &Env,
    keeper: &Address,
    adapter: &Address,
    amount: i128,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("str_alc"), keeper.clone()),
        (adapter.clone(), amount, ledger),
    );
}

/// Emitted when funds are recalled or withdrawn from a strategy adapter (issue #764).
pub fn strategy_withdrawn(
    env: &Env,
    adapter: &Address,
    amount: i128,
    ledger: u32,
) {
    env.events().publish(
        (symbol_short!("str_wdr"), adapter.clone()),
        (amount, ledger),
    );
}


