use soroban_sdk::{contracttype, Address, String, Vec, Env, Symbol, symbol_short};

/// Consolidated per-user settings (issue #796).
///
/// Stores all user preferences in a single storage entry to reduce rent costs.
/// Uses versioned-enum pattern so new settings can be added later.
///
/// - `version`: struct version for future migrations
/// - `claim_destination`: address to receive claimed rewards (None = self)
/// - `auto_compound`: whether to automatically compound rewards
/// - `keepers`: list of addresses authorized to act on behalf of user
/// - `notifications_enabled`: whether user wants notifications
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserSettings {
    pub version: u32,
    pub claim_destination: Option<Address>,
    pub auto_compound: bool,
    pub keepers: Vec<Address>,
    pub notifications_enabled: bool,
}

impl UserSettings {
    /// Create default settings for a new user.
    pub fn default() -> Self {
        Self {
            version: 1,
            claim_destination: None,
            auto_compound: false,
            keepers: Vec::new(&soroban_sdk::Env::default()),
            notifications_enabled: true,
        }
    }
}

/// Storage keys for all persistent and instance state in the vault.
///
/// Instance keys (fast, small): Admin, Token, TotalShares, TotalDeposited,
/// MinStake, RewardRateBps, RewardPoolBalance, BoostSchedule, Paused,
/// WithdrawalLimit, LockPeriod, EarlyExitPenaltyBps, TotalStakers,
/// TotalRewardsPaid, WhitelistEnabled, CooldownPeriod,
/// UnstakeFeeBps, AllStakers, InactivityThreshold, Changelog,
/// LastRateChangeLedger, InitializedAtLedger.
///
/// Persistent keys (per-user, long-lived): ShareBalance, StakeHistory,
/// RewardCheckpointLedger, LastClaimLedger, AccruedReward, StakedAtLedger,
/// Delegate, Whitelisted, UnbondingPosition, UserClaimWindow, FrozenAt,
/// VestingEntries.
/// Domain sub-enum for administrator and protocol-wide configuration keys (Issue #646).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdminKey {
    Admin,
    PendingAdmin,
    EmergencyAdmin,
    AdminRenounced,
    Deployer,
    SlashTreasury,
    Token,
    TotalShares,
    TotalDeposited,
    MinStake,
    Paused,
    WithdrawalLimit,
    LockPeriod,
    EarlyExitPenaltyBps,
    TotalStakers,
    CooldownPeriod,
    UnstakeFeeBps,
}

/// Domain sub-enum for staker positions, delegation, and user-specific accounting (Issue #646).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionKey {
    ShareBalance(Address),
    StakeHistory(Address),
    StakedAtLedger(Address),
    LastUnstakeLedger(Address),
    UnbondingPosition(Address),
    FrozenAt(Address),
    FirstStakedAt(Address),
    Restaked(Address),
    Delegate(Address),
    RewardCheckpointLedger(Address),
    LastClaimLedger(Address),
    AccruedReward(Address),
    RewardRemainder(Address),
    UserClaimWindow(Address),
    VestingEntries(Address),
    RewardAccumulatorCheckpoint(Address),
}

/// Domain sub-enum for reward distribution, emissions, and boost campaigns (Issue #646).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewardKey {
    RewardRateBps,
    RewardPoolBalance,
    RewardToken,
    TotalRewardsPaid,
    TotalEverClaimed,
    RateHistory,
    BoostSchedule,
    BoostCampaign,
    VestingPeriod,
    EpochMode,
    CurrentEpoch,
    EpochLedgers,
    EpochRewardPerEpoch,
    EpochRewardFactor(u32),
    GlobalRewardIndex,
    LastAccumulatorLedger,
}

/// Domain sub-enum for governance, access control, and pool lifecycle (Issue #646).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GovernanceKey {
    WhitelistEnabled,
    Whitelisted(Address),
    AllStakers,
    Leaderboard,
    LeaderboardSize,
    InactivityThreshold,
    KycRequired,
    KycApproved(Address),
    Stopped,
    ShuttingDown,
}

/// Top-level key enum wrapping namespaced domain sub-enums (Issue #646).
///
/// This removes Soroban's 50-variant limit on a single `#[contracttype]` enum
/// by dividing keys into domain-specific sub-enums while preserving type safety.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageKey {
    Admin(AdminKey),
    Position(PositionKey),
    Reward(RewardKey),
    Governance(GovernanceKey),
}

/// Storage keys for all persistent and instance state in the vault.
///
/// Instance keys (fast, small): Admin, Token, TotalShares, TotalDeposited,
/// MinStake, RewardRateBps, RewardPoolBalance, BoostSchedule, Paused,
/// WithdrawalLimit, LockPeriod, EarlyExitPenaltyBps, TotalStakers,
/// TotalRewardsPaid, WhitelistEnabled, CooldownPeriod,
/// UnstakeFeeBps, AllStakers, InactivityThreshold, Changelog,
/// LastRateChangeLedger, InitializedAtLedger.
///
/// Persistent keys (per-user, long-lived): ShareBalance, StakeHistory,
/// RewardCheckpointLedger, LastClaimLedger, AccruedReward, StakedAtLedger,
/// Delegate, Whitelisted, UnbondingPosition, UserClaimWindow, FrozenAt,
/// VestingEntries.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Token,
    TotalShares,
    TotalDeposited,
    MinStake,
    RewardRateBps,
    RewardPoolBalance,
    BoostSchedule,
    ShareBalance(Address),
    StakeHistory(Address),
    RewardCheckpointLedger(Address),
    LastClaimLedger(Address),
    AccruedReward(Address),
    Paused,
    WithdrawalLimit,
    LockPeriod,
    EarlyExitPenaltyBps,
    StakedAtLedger(Address),
    TotalStakers,
    TotalRewardsPaid,
    Delegate(Address),
    LastUnstakeLedger(Address),
    TotalEverClaimed,
    Restaked(Address),
    WhitelistEnabled,
    Whitelisted(Address),
    CooldownPeriod,
    UnbondingPosition(Address),
    RewardRemainder(Address),
    UserClaimWindow(Address),
    UnstakeFeeBps,
    AllStakers,
    RateHistory,
    BoostCampaign,
    Leaderboard,
    LeaderboardSize,
    // Issue #101: frozen positions
    InactivityThreshold,
    FrozenAt(Address),
    KycRequired,
    KycApproved(Address),
    Stopped,
    ShuttingDown,
    FirstStakedAt(Address),
    // Task 2: Vesting
    VestingPeriod,
    VestingEntries(Address),
    // Task 3: Epoch Mode
    EpochMode,
    CurrentEpoch,
    EpochLedgers,
    EpochRewardPerEpoch,
    EpochRewardFactor(u32),
    // NOTE: DataKey is at Soroban's 50-variant cap. New keys should be
    // added to the appropriate domain sub-enum (AdminKey, PositionKey,
    // RewardKey, GovernanceKey) wrapped by StorageKey (issue #646).
}

impl DataKey {
    /// Maps a legacy DataKey variant to its namespaced domain sub-enum within StorageKey (Issue #646).
    pub fn to_storage_key(&self) -> StorageKey {
        match self {
            DataKey::Admin => StorageKey::Admin(AdminKey::Admin),
            DataKey::Token => StorageKey::Admin(AdminKey::Token),
            DataKey::TotalShares => StorageKey::Admin(AdminKey::TotalShares),
            DataKey::TotalDeposited => StorageKey::Admin(AdminKey::TotalDeposited),
            DataKey::MinStake => StorageKey::Admin(AdminKey::MinStake),
            DataKey::Paused => StorageKey::Admin(AdminKey::Paused),
            DataKey::WithdrawalLimit => StorageKey::Admin(AdminKey::WithdrawalLimit),
            DataKey::LockPeriod => StorageKey::Admin(AdminKey::LockPeriod),
            DataKey::EarlyExitPenaltyBps => StorageKey::Admin(AdminKey::EarlyExitPenaltyBps),
            DataKey::TotalStakers => StorageKey::Admin(AdminKey::TotalStakers),
            DataKey::CooldownPeriod => StorageKey::Admin(AdminKey::CooldownPeriod),
            DataKey::UnstakeFeeBps => StorageKey::Admin(AdminKey::UnstakeFeeBps),

            DataKey::ShareBalance(addr) => StorageKey::Position(PositionKey::ShareBalance(addr.clone())),
            DataKey::StakeHistory(addr) => StorageKey::Position(PositionKey::StakeHistory(addr.clone())),
            DataKey::RewardCheckpointLedger(addr) => StorageKey::Position(PositionKey::RewardCheckpointLedger(addr.clone())),
            DataKey::LastClaimLedger(addr) => StorageKey::Position(PositionKey::LastClaimLedger(addr.clone())),
            DataKey::AccruedReward(addr) => StorageKey::Position(PositionKey::AccruedReward(addr.clone())),
            DataKey::StakedAtLedger(addr) => StorageKey::Position(PositionKey::StakedAtLedger(addr.clone())),
            DataKey::LastUnstakeLedger(addr) => StorageKey::Position(PositionKey::LastUnstakeLedger(addr.clone())),
            DataKey::Restaked(addr) => StorageKey::Position(PositionKey::Restaked(addr.clone())),
            DataKey::Delegate(addr) => StorageKey::Position(PositionKey::Delegate(addr.clone())),
            DataKey::UnbondingPosition(addr) => StorageKey::Position(PositionKey::UnbondingPosition(addr.clone())),
            DataKey::RewardRemainder(addr) => StorageKey::Position(PositionKey::RewardRemainder(addr.clone())),
            DataKey::UserClaimWindow(addr) => StorageKey::Position(PositionKey::UserClaimWindow(addr.clone())),
            DataKey::FrozenAt(addr) => StorageKey::Position(PositionKey::FrozenAt(addr.clone())),
            DataKey::FirstStakedAt(addr) => StorageKey::Position(PositionKey::FirstStakedAt(addr.clone())),
            DataKey::VestingEntries(addr) => StorageKey::Position(PositionKey::VestingEntries(addr.clone())),

            DataKey::RewardRateBps => StorageKey::Reward(RewardKey::RewardRateBps),
            DataKey::RewardPoolBalance => StorageKey::Reward(RewardKey::RewardPoolBalance),
            DataKey::TotalRewardsPaid => StorageKey::Reward(RewardKey::TotalRewardsPaid),
            DataKey::TotalEverClaimed => StorageKey::Reward(RewardKey::TotalEverClaimed),
            DataKey::RateHistory => StorageKey::Reward(RewardKey::RateHistory),
            DataKey::BoostSchedule => StorageKey::Reward(RewardKey::BoostSchedule),
            DataKey::BoostCampaign => StorageKey::Reward(RewardKey::BoostCampaign),
            DataKey::VestingPeriod => StorageKey::Reward(RewardKey::VestingPeriod),
            DataKey::EpochMode => StorageKey::Reward(RewardKey::EpochMode),
            DataKey::CurrentEpoch => StorageKey::Reward(RewardKey::CurrentEpoch),
            DataKey::EpochLedgers => StorageKey::Reward(RewardKey::EpochLedgers),
            DataKey::EpochRewardPerEpoch => StorageKey::Reward(RewardKey::EpochRewardPerEpoch),
            DataKey::EpochRewardFactor(f) => StorageKey::Reward(RewardKey::EpochRewardFactor(*f)),

            DataKey::WhitelistEnabled => StorageKey::Governance(GovernanceKey::WhitelistEnabled),
            DataKey::Whitelisted(addr) => StorageKey::Governance(GovernanceKey::Whitelisted(addr.clone())),
            DataKey::AllStakers => StorageKey::Governance(GovernanceKey::AllStakers),
            DataKey::Leaderboard => StorageKey::Governance(GovernanceKey::Leaderboard),
            DataKey::LeaderboardSize => StorageKey::Governance(GovernanceKey::LeaderboardSize),
            DataKey::InactivityThreshold => StorageKey::Governance(GovernanceKey::InactivityThreshold),
            DataKey::KycRequired => StorageKey::Governance(GovernanceKey::KycRequired),
            DataKey::KycApproved(addr) => StorageKey::Governance(GovernanceKey::KycApproved(addr.clone())),
            DataKey::Stopped => StorageKey::Governance(GovernanceKey::Stopped),
            DataKey::ShuttingDown => StorageKey::Governance(GovernanceKey::ShuttingDown),
        }
    }
}

/// Pool-wide share price at a particular ledger (issue #494).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharePriceSnapshot {
    pub price_numerator: i128,
    pub price_denominator: i128,
    pub ledger: u32,
}

/// Storage key for an individual epoch snapshot.
///
/// Soroban's enum contracttype support is stricter for tuple variants, so we
/// keep the address and epoch together in a dedicated struct instead of a
/// multi-field enum variant.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserEpochSnapshotKey {
    pub user: Address,
    pub epoch: u32,
}

/// Issue #42: enum of all admin actions for the audit log.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum AdminAction {
    SetRewardRate,
    Pause,
    Unpause,
    TransferAdmin,
    SetLockPeriod,
    SetCap,
    Slash,
    RescueToken,
    SetEarlyExitPenalty,
    SetMinStake,
    FundRewardPool,
    AddYield,
    SetBoostSchedule,
    SetNftContract,
    SetRestakeWindow,
    SetRewardToken,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UnbondingPosition {
    pub amount: i128,
    pub unbonding_since: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VaultState {
    pub total_shares: i128,
    pub total_deposited: i128,
    pub paused: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolStats {
    pub total_staked: i128,
    pub total_stakers: u32,
    pub reward_rate_bps: i128,
    pub reward_token_balance: i128,
    pub paused: bool,
    pub total_rewards_paid: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RewardTier {
    pub max_amount: i128,
    pub rate_bps: i128,
}

/// Aggregate user stats used by `user_stats`.
///
/// - `position_amount`: the user's current position size expressed in token units.
/// - `pending_reward`: rewards accrued but not yet claimed.
/// - `staked_at_ledger`: the ledger sequence when the position was first opened.
/// - `last_claim_ledger`: the most recent ledger at which rewards were claimed.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserStats {
    pub position_amount: i128,
    pub pending_reward: i128,
    pub staked_at_ledger: u32,
    pub last_claim_ledger: u32,
}

/// Matching program configuration (issue #242).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MatchingProgram {
    pub match_rate_bps: u32,
    pub per_user_cap: i128,
    pub total_budget: i128,
    pub budget_used: i128,
    pub active: bool,
}

/// Per-user matching stats (issue #242).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserMatchingStats {
    pub total_matched: i128,
}

/// Cohort statistics for weekly staker cohorts.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CohortStats {
    pub cohort_id: u32,
    pub member_count: u32,
    pub total_staked: i128,
    pub avg_position: i128,
    pub total_rewards_claimed: i128,
    pub active_members: u32,
}

/// Active boost campaign set by admin (#48).
///
/// - `multiplier_bps`: reward multiplier stacked on top of tier multipliers (10000 = 1x).
/// - `starts_at_ledger`: ledger when the campaign was activated.
/// - `ends_at_ledger`: ledger after which the campaign no longer applies.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CampaignInfo {
    pub multiplier_bps: u32,
    pub starts_at_ledger: u32,
    pub ends_at_ledger: u32,
}

/// A single entry in the staking leaderboard (#46).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LeaderboardEntry {
    pub staker: Address,
    pub amount: i128,
}

/// Type alias for the leaderboard vector used in storage and queries.
#[allow(dead_code)]
pub type Leaderboard = Vec<LeaderboardEntry>;

/// Current stake position for a user.
///
/// - `amount`: the user's current position size expressed in token units.
/// - `staked_at_ledger`: the ledger sequence when the position was first opened.
/// - `last_claim_ledger`: the most recent ledger at which rewards were claimed.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StakePosition {
    pub amount: i128,
    pub staked_at_ledger: u32,
    pub last_claim_ledger: u32,
}

/// Snapshot of all pool-level configuration returned by `get_pool_config`.
///
/// Allows frontends to fetch all settings in a single RPC call instead of
/// querying each key individually.
///
/// - `admin`: current admin address.
/// - `stake_token`: token accepted for staking and used to pay rewards.
/// - `reward_token`: same as `stake_token` (single-token vault).
/// - `reward_rate_bps`: annual reward rate in basis points.
/// - `paused`: whether deposits and withdrawals are currently paused.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolConfig {
    pub admin: Address,
    pub stake_token: Address,
    pub reward_token: Address,
    pub reward_rate_bps: u32,
    pub paused: bool,
}

/// Contract metadata returned by `contract_metadata`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ContractMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
}

/// Per-user reward claim window used to enforce the optional claim cap.
///
/// - `claimed_in_window`: cumulative rewards claimed by this user in the current window.
/// - `window_started_at`: ledger sequence at which the current window began.
///
/// The window resets automatically when `current_ledger > window_started_at + window_ledgers`.
/// Any unclaimed remainder is deferred to the next window — it is not lost.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimWindow {
    pub claimed_in_window: i128,
    pub window_started_at: u32,
}

/// Dynamic unstake fee model configured via `set_dynamic_fee_config` (issue #213).
///
/// Below `utilization_threshold_bps` pool utilization, the unstake fee is
/// `base_fee_bps`. Above it, the fee interpolates linearly up to
/// `max_fee_bps` at 100% utilization. While a config is set, `unstake` uses
/// this dynamic fee instead of the static fee from `set_unstake_fee_bps`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicFeeConfig {
    pub base_fee_bps: u32,
    pub max_fee_bps: u32,
    pub utilization_threshold_bps: u32,
}

/// Composite staker reputation score returned by `get_reputation_score` (issue #214).
///
/// Each sub-score is bounded to 2500 basis points; `total_score` is the sum,
/// bounded to 10 000 (100%). All zero for an address with no active position.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReputationScore {
    pub duration_score: u32,
    pub consistency_score: u32,
    pub size_score: u32,
    pub streak_score: u32,
    pub total_score: u32,
}

/// Single entry in the on-chain changelog exposed by `get_changelog`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ChangelogEntry {
    pub change_type: String,
    pub old_value: i128,
    pub new_value: i128,
}

/// One entry in the on-chain reward-rate changelog exposed by
/// `get_rate_history` (issue #522).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RateChange {
    pub old_rate_bps: u32,
    pub new_rate_bps: u32,
    /// Ledger sequence number of the update, matching the timestamp carried by
    /// the `rate_changed` event.
    pub changed_at: u32,
}

/// One entry in the rich reward-rate history exposed by `get_reward_rate_history` (issue #124).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RateHistoryEntry {
    pub old_rate_bps: i128,
    pub new_rate_bps: i128,
    pub changed_at_ledger: u32,
    pub changed_by: Address,
}

/// Accumulated referral stats stored per referrer (issue #757).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReferralStats {
    pub referee_count: u32,
    pub total_referred_deposits: i128,
    pub earned: i128,
    pub claimed: i128,
}

/// Receipt for a cast governance vote (issue #760).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VoteReceipt {
    pub support: bool,
    pub weight: i128,
    pub ledger: u32,
}

/// Vote-escrow style reward boost lock (issue #758).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VeBoostLock {
    pub amount: i128,
    pub unlock_ledger: u32,
    pub duration_ledgers: u32,
}

/// Result of `get_boost_tier_progress` showing where a user is toward the next boost tier.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BoostTierProgress {
    pub current_tier: u32,
    pub current_multiplier_bps: i128,
    pub next_tier_in_ledgers: Option<u32>,
    pub next_multiplier_bps: Option<i128>,
}

/// One entry in the referral leaderboard returned by `referral_leaderboard`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReferralLeaderboardEntry {
    pub referrer: Address,
    pub total_referred_stake: i128,
    pub referral_count: u32,
}

/// Comprehensive health snapshot of the pool returned by `pool_health_report`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolHealthReport {
    pub reward_token_balance: i128,
    pub total_staked: i128,
    pub total_stakers: u32,
    pub total_rewards_paid: i128,
    pub reward_rate_bps: i128,
    pub is_paused: bool,
    pub is_stopped: bool,
    pub uptime_ledgers: u32,
    pub estimated_daily_obligations: i128,
    pub is_solvent_7_days: bool,
}

/// Admin-only operational snapshot for monitoring and alerting dashboards.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OperatorDashboard {
    pub pool_health: PoolHealthReport,
    pub staker_count: u32,
    pub inactive_staker_count: u32,
    pub pending_exit_queue_count: u32,
    pub total_ever_staked: i128,
    pub total_ever_claimed: i128,
    pub largest_position: i128,
    pub smallest_active_position: i128,
    pub sunset_state: SunsetState,
    pub open_governance_proposals: u32,
    pub reward_token_runway_days: u32,
}

/// Effective reward rate breakdown returned by `reward_multiplier_preview` (issue #181).
///
/// Each `*_multiplier_bps` field defaults to 10000 (1x) if that boost mechanism
/// is not active for the user. `effective_rate_bps` is the fully stacked rate.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RewardMultiplierBreakdown {
    pub base_rate_bps: i128,
    pub tier_multiplier_bps: i128,
    pub campaign_multiplier_bps: i128,
    pub effective_rate_bps: i128,
}

/// Aggregate score used by `staking_efficiency_score`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StakingEfficiencyScore {
    pub total_claimed: i128,
    pub estimated_if_compounded: i128,
    pub efficiency_bps: i128,
}

/// Aggregated user state returned by `user_summary` (issue #103).
///
/// - `position`: 0 or 1 `StakePosition` entries; empty when user has no stake.
/// - `pending_reward`: rewards accrued but not yet claimed.
/// - `pool_share_bps`: user's share of the total pool in basis points (10000 = 100%).
///
/// Note: `position` uses `Vec<StakePosition>` (0-or-1 elements) because
/// `Option<ContractType>` is not supported in `#[contracttype]` structs in
/// soroban-sdk 21.x testutils mode.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserSummary {
    pub position: Vec<StakePosition>,
    pub pending_reward: i128,
    pub pool_share_bps: i128,
}

// ── Issue #105: stake/unstake history ────────────────────────────────────────

/// Discriminant for a stake history entry.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StakeAction {
    Stake,
    Unstake,
}

/// One entry in a user's recent staking activity log.
///
/// - `action`: whether the user staked or unstaked.
/// - `amount`: token amount involved (not shares).
/// - `ledger`: ledger sequence number at which the action was recorded.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StakeHistoryEntry {
    pub action: StakeAction,
    pub amount: i128,
    pub ledger: u32,
}

// ── Issue #104: interface detection ──────────────────────────────────────────

/// Feature interface identifiers for `supports_interface`.
///
/// `Base` is always supported. All others are only true when the corresponding
/// feature is compiled into this deployment.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum InterfaceId {
    Base,
    Lockup,
    Whitelist,
    Compounding,
    EpochMode,
    VestingSchedule,
}

/// Result of a `can_unstake` pre-flight check (issue #98).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum UnstakeCheckResult {
    /// The unstake would succeed.
    Ok,
    /// The user has no active staking position.
    NoPosition,
    /// The user's position is smaller than the requested amount (in token units).
    InsufficientAmount,
    /// The pool is currently paused.
    PoolPaused,
    /// The lock-up period has not yet elapsed (early exit penalty would apply).
    StillLocked,
}

/// Per-user staking streak data (issue #99).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StakeStreak {
    pub current_streak: u32,
    pub longest_streak: u32,
    pub last_active_wave: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VestingEntry {
    pub amount: i128,
    pub claimable_at_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EpochState {
    pub epoch_number: u32,
    pub started_at: u32,
    pub reward_pool: i128,
    pub total_staked_snapshot: i128,
}

/// Governance checkpoint: total staked recorded at a specific ledger (issue snapshot_total_staked).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TotalStakedSnapshot {
    pub total_staked: i128,
    pub ledger: u32,
}

/// Issue #166: aggregated contract/token addresses for integration setup.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractAddresses {
    pub contract: Address,
    pub admin: Address,
    pub stake_token: Address,
    pub reward_token: Address,
}

/// One day-bucket in the rolling 7-day activity heatmap.
///
/// `day_index` = `current_ledger / LEDGERS_PER_DAY` (see `vault::LEDGERS_PER_DAY`).
/// Each counter is incremented once per matching user action within that day.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DayBucket {
    pub day_index: u32,
    pub stake_count: u32,
    pub unstake_count: u32,
    pub claim_count: u32,
}

// ── Issue #219: pause reason ─────────────────────────────────────────────────

/// Reason categories for pool pauses (issue #219).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PauseReason {
    Maintenance,
    SecurityIncident,
    RateReconfiguration,
    CapAdjustment,
    Other,
}

/// Full pause state stored when the pool is paused (issue #219).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PauseInfo {
    pub reason: PauseReason,
    pub message: String,
    pub paused_at: u32,
}

// ── Issue #217: tax reporting ────────────────────────────────────────────────

/// Annual tax report summary for a user (issue #217).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TaxReport {
    pub user: Address,
    pub ledger_from: u32,
    pub ledger_to: u32,
    pub total_rewards_claimed: i128,
    pub average_stake_amount: i128,
    pub claim_count: u32,
}

// ── Issue #220: rounding policy ──────────────────────────────────────────────

/// Rounding behaviour for sub-unit division in share and reward math (issue #220).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RoundingPolicy {
    Floor,
    Ceiling,
    Nearest,
}
// ── Issue #220: rounding dust handling ────────────────────────────────────────

use crate::errors::VaultSafetyError;

/// Storage key for accumulated rounding dust (issue #220).
pub const ROUNDING_DUST_KEY: Symbol = symbol_short!("rounding_dust");

/// Retrieve the current rounding dust amount (0 if none).
pub fn get_rounding_dust(env: &Env) -> i128 {
    env.storage().instance().get(&ROUNDING_DUST_KEY).unwrap_or(0)
}

/// Add amount to rounding dust counter (checked arithmetic).
pub fn add_rounding_dust(env: &Env, amount: i128) -> Result<(), VaultSafetyError> {
    let cur = get_rounding_dust(env);
    let new = cur.checked_add(amount).ok_or(VaultSafetyError::ArithmeticError)?;
    env.storage().instance().set(&ROUNDING_DUST_KEY, &new);
    Ok(())
}

/// Sweep accumulated rounding dust into reward pool, clearing the counter.
pub fn sweep_rounding_dust(env: &Env) -> Result<i128, VaultSafetyError> {
    let dust = get_rounding_dust(env);
    env.storage().instance().remove(&ROUNDING_DUST_KEY);
    Ok(dust)
}


// ── Issue #216: governance voting ────────────────────────────────────────────

/// Pool parameter a governance proposal can change (issue #216).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ProposableParam {
    RewardRate,
    UnstakeFee,
    PoolCap,
    MinStake,
}

/// How vote weight is computed for a governance proposal.
///
/// The mode is **snapshotted at proposal-creation time** so that a mid-vote
/// mode change by the admin cannot alter the weighting rules of an already-open
/// proposal.
///
/// # Quadratic weighting caveat
///
/// Quadratic weighting reduces whale dominance: a holder with 10 000 tokens
/// gets `sqrt(10 000) = 100` weight instead of 10 000.  However, it is easy
/// to bypass by splitting stake across multiple wallets, so meaningful
/// protection requires a complementary identity/sybil-resistance layer
/// (e.g. the existing KYC or proof-of-humanity hook).  Use with awareness.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VoteWeightMode {
    /// Weight = staked token amount.  This is the default.
    Linear,
    /// Weight = integer square root of staked token amount.
    /// Rounds down; a position worth 0 still has 0 weight.
    Quadratic,
}

/// A governance proposal to change a pool parameter, created via
/// `create_proposal` and resolved via `enact_proposal` (issue #216).
///
/// Voting power is the staker's token amount at the time of the `vote` call,
/// not adjusted retroactively if the staker's position changes afterward.
/// The `weight_mode` is fixed at creation time from the pool-wide setting.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GovernanceProposal {
    pub id: u32,
    pub parameter: ProposableParam,
    pub new_value: i128,
    pub votes_for: i128,
    pub votes_against: i128,
    pub ends_at: u32,
    pub enacted: bool,
    /// Weighting mode active when this proposal was created.
    /// Immutable after creation so mode changes don't affect open proposals.
    pub weight_mode: VoteWeightMode,
}

// ── Issue #207: cross-chain bridge relayer hook ──────────────────────────────

/// Packet data emitted alongside `stake`/`unstake`, shaped for cross-chain
/// bridge relayers (issue #207). Supplementary only — never read by, or
/// required for, core stake/unstake logic.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BridgePacket {
    pub sequence: u64,
    pub source_address: Address,
    pub amount: i128,
    pub action: String,
    pub timestamp_ledger: u32,
}

// ── Issue #197: fee splitting ─────────────────────────────────────────────────

/// One recipient of a proportional fee split (issue #197).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeRecipient {
    pub address: Address,
    pub share_bps: u32,
}

// ── Issue #195: timelocked admin actions ─────────────────────────────────────

/// A queued admin action awaiting its timelock delay (issue #195).
///
/// Execution support (see `execute_action()` in vault.rs) is implemented for
/// `AdminAction::SetRewardRate` and `AdminAction::Pause`/`Unpause` — the
/// issue's own notes call these out as "the primary candidates" for
/// timelock. Queuing any other `AdminAction` is allowed (the queue/cancel
/// lifecycle itself is generic), but executing one reverts with
/// `ActionNotFound` rather than silently doing nothing, since safely
/// decoding+dispatching arbitrary opaque `Bytes` for every admin action in
/// this contract is a much larger dispatcher than this issue's stated scope.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingAction {
    pub id: u32,
    pub action_type: AdminAction,
    pub params: soroban_sdk::Bytes,
    pub queued_at: u32,
    pub executable_at: u32,
}

// ── Issue #196: multi-sig admin ──────────────────────────────────────────────

/// Multisig configuration: up to 5 admin addresses and an M-of-N approval
/// threshold (issue #196). Additive and optional — see the scope note on
/// `initialize_multisig()` in vault.rs for why this does not replace the
/// existing single `Admin` key.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MultisigConfig {
    pub admins: Vec<Address>,
    pub threshold: u32,
}

/// A multisig proposal awaiting approvals (issue #196). Named
/// `AdminProposal` rather than the issue's literal `Proposal` to avoid
/// confusion with the existing, unrelated `GovernanceProposal` (staker
/// voting on pool parameters, issue #216) already in this file.
///
/// Same execution-dispatch scope note as `PendingAction` above: only
/// `AdminAction::SetRewardRate` and `Pause`/`Unpause` are actually
/// executable once threshold is reached; other action types can be
/// proposed and approved but revert on execution.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AdminProposal {
    pub id: u32,
    pub action_type: AdminAction,
    pub params: soroban_sdk::Bytes,
    pub approvals: Vec<Address>,
    pub executed: bool,
}

// ── Missing types for existing feature branches ──────────────────────────────

/// Export payload for migrating pool state to a new contract (issue #203).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MigrationExport {
    pub admin: Address,
    pub stake_token: Address,
    pub reward_token: Address,
    pub reward_rate_bps: u32,
    pub total_staked: i128,
    pub total_stakers: u32,
    pub paused: bool,
    pub all_positions: Vec<(Address, StakePosition)>,
}

/// Bootstrap configuration for phased reward rate (issue #202).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BootstrapConfig {
    pub initial_rate: u32,
    pub base_rate: u32,
    pub started_at: u32,
    pub duration: u32,
}

/// A delegation chain: beneficiary + list of delegates (issue #200).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DelegationChain {
    pub beneficiary: Address,
    pub delegates: Vec<Address>,
}

/// Storage footprint report (issue #208).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StorageUsageReport {
    pub instance_keys: u32,
    pub persistent_position_count: u32,
    pub persistent_other_keys: u32,
    pub estimated_total_bytes: u32,
}

// ── Issue #210: Merkle Reward Distribution ────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MerkleRoot {
    pub root: soroban_sdk::Bytes,
    pub epoch: u32,
    pub total_claimable: i128,
    pub published_at: u32,
}

// ── Issue #231: Halving Schedule ──────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HalvingConfig {
    pub interval_ledgers: u32,
    pub started_at: u32,
    pub halving_count: u32,
    pub floor_rate_bps: i128,
}

// ── Issue #222: Staking Certificate ───────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StakingCertificate {
    pub holder: Address,
    pub min_amount_staked: i128,
    pub staked_since: u32,
    pub issued_at: u32,
    pub valid_until: u32,
    pub certificate_id: u32,
}

// ── Issue #211: Staking Tournament Competition ─────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Tournament {
    pub prize_pool: i128,
    pub starts_at: u32,
    pub ends_at: u32,
    pub winner: soroban_sdk::Bytes,
    pub finalized: bool,
}

// ── Issue #221: Cross-Pool Reward Comparison ──────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolComparison {
    pub pool_address: Address,
    pub reward_rate_bps: i128,
    pub reachable: bool,
}

// ── Issue #235: Reward Smoothing ──────────────────────────────────────────────

/// A large reward addition being released linearly over `duration_ledgers`
/// (issue #235).
///
/// `released` is the cumulative amount already credited to `total_deposited` by
/// `release_smoothed_yield()`. The schedule is finished once
/// `released == total_amount`; the tokens themselves are transferred into the
/// contract up front when the schedule is created, so a finished schedule owes
/// nothing.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SmoothingSchedule {
    pub total_amount: i128,
    pub released: i128,
    pub start_ledger: u32,
    pub duration_ledgers: u32,
}

/// Read-only snapshot of the active smoothing schedule returned by
/// `reward_smoothing()` (issue #235).
///
/// All fields are zero when no schedule has ever been created. `releasable_now`
/// is what a `release_smoothed_yield()` call at this ledger would credit;
/// `unreleased` is everything still locked, including `releasable_now`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SmoothingStatus {
    pub total_amount: i128,
    pub released: i128,
    pub releasable_now: i128,
    pub unreleased: i128,
    pub start_ledger: u32,
    pub duration_ledgers: u32,
    pub ledgers_remaining: u32,
}

// ── Issue #236: Referral Tree Visualization ───────────────────────────────────

/// One node in the flattened referral hierarchy returned by
/// `referral_tree_data()` (issue #236).
///
/// The tree is returned as a flat, breadth-first `Vec` rather than a nested
/// structure because `#[contracttype]` cannot express a recursive type. Rebuild
/// the hierarchy client-side by joining `parent` against `address`.
///
/// - `level`: 0 for the queried root, 1 for its direct referrals, up to 3.
/// - `parent`: the address that referred this node. For the root (level 0) this
///   is the root's own address, since `Option<Address>` is not usable inside a
///   `#[contracttype]` struct in soroban-sdk 21.x testutils mode (same
///   constraint that shapes `UserSummary::position`).
/// - `referred_count` / `total_referred_stake`: this node's own referral stats.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReferralTreeNode {
    pub address: Address,
    pub parent: Address,
    pub level: u32,
    pub referred_count: u32,
    pub total_referred_stake: i128,
}

// ── Issue #237: Capacity Auction ──────────────────────────────────────────────

/// An auction allocating a limited number of pool spots to the highest bidders
/// (issue #237).
///
/// A bid is denominated in stake tokens and is escrowed in the contract when
/// placed. On `finalize_capacity_auction()` the top `spots` bids are converted
/// into real staking positions and every losing bid is refunded in full.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CapacityAuction {
    pub spots: u32,
    pub min_bid: i128,
    pub started_at: u32,
    pub ends_at: u32,
    pub finalized: bool,
    pub total_escrowed: i128,
}

/// A single bid in the capacity auction (issue #237).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AuctionBid {
    pub bidder: Address,
    pub amount: i128,
    pub placed_at: u32,
}

// ── Issue #239: stake-weighted lottery ────────────────────────────────────────

/// A prize pool funded by the admin and drawn among active stakers, weighted
/// by their staked amount (issue #239).
///
/// `winner` uses `Vec<Address>` (0 or 1 elements) rather than `Option<Address>`
/// since `Option<Address>` is not usable inside a `#[contracttype]` struct in
/// soroban-sdk 21.x testutils mode (same constraint noted on `UserSummary`
/// and `ReferralTreeNode` above).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LotteryConfig {
    pub prize_amount: i128,
    pub draw_at_ledger: u32,
    pub drawn: bool,
    pub winner: Vec<Address>,
}

// ── Issue #238: loyalty milestone badges ──────────────────────────────────────

/// The staker metric a milestone is evaluated against (issue #238).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MilestoneCondition {
    StakeDurationLedgers,
    TotalStakedAmount,
    ClaimCount,
    TotalRewardsClaimed,
}

/// An admin-configured staking achievement. Evaluated lazily by
/// `check_milestones()`, which records a non-transferable badge in
/// `UserMilestones` the first time a staker crosses `threshold` (issue #238).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Milestone {
    pub id: u32,
    pub name: String,
    pub condition_type: MilestoneCondition,
    pub threshold: i128,
}

/// A single entry in the achievement leaderboard (issue #173 extension).
///
/// - `user`: the staker's address.
/// - `milestone_count`: number of milestones achieved by this staker.
/// - `latest_achievement_ledger`: most recent ledger when a milestone was achieved.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AchievementLeaderboardEntry {
    pub user: Address,
    pub milestone_count: u32,
    pub latest_achievement_ledger: u32,
}

// ── Issue #240: oracle-triggered lock-up release ──────────────────────────────

/// Direction the oracle price must move relative to `trigger_price` to
/// satisfy a `PriceCondition` (issue #240).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TriggerDirection {
    Below,
    Above,
}

/// A user-configured price condition that, once satisfied, waives their
/// position's early-exit lock-up penalty (issue #240).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PriceCondition {
    pub trigger_price: i128,
    pub asset_id: String,
    pub direction: TriggerDirection,
}

// ── Issue #250: optimal claim frequency advisory ───────────────────────────────

/// Advisory result from `get_optimal_claim_frequency()` (issue #250). Every
/// field is an approximation at the current reward rate/boost/campaign — see
/// that function's doc comment for the exact assumptions.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OptimalClaimAdvice {
    pub recommended_interval_ledgers: u32,
    pub recommended_interval_days: u32,
    /// Estimated annual reward gain from compounding at
    /// `recommended_interval_ledgers` vs. simple (non-compounding) accrual.
    /// Named `annual_compounding_gain` rather than the longer
    /// `estimated_annual_gain_from_compounding` — Soroban's contracttype
    /// struct field names are capped at 30 characters.
    pub annual_compounding_gain: i128,
    pub break_even_reward_per_claim: i128,
}

// ── Issue #257: auto-convert reward on claim ────────────────────────────────────

/// A user's auto-convert-on-claim configuration (issue #257): reward tokens
/// are swapped to `target_token` via the configured DEX router (issue #205)
/// on every `claim`, subject to `min_output_bps` slippage protection.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AutoConvertConfig {
    pub target_token: Address,
    pub min_output_bps: u32,
}

// ── Issue #251: exit-queue priority bidding ─────────────────────────────────────

/// A single recorded call to `bid_for_queue_priority()` (issue #251).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PriorityBidRecord {
    pub user: Address,
    pub bid_amount: i128,
    pub previous_position: u32,
    pub ledger: u32,
}

// ── Issue #258: pool whitelabel branding ────────────────────────────────────────

/// Whitelabel identity for the pool, set by the admin via `set_branding()` and
/// read by frontends via `get_branding()` without any off-chain lookup.
///
/// Every field is length-capped (see `MAX_BRANDING_*` in vault.rs); exceeding a
/// cap reverts with the `VaultBrandError` variant naming that field.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BrandingConfig {
    pub display_name: String,
    pub logo_hash: String,
    pub website_url: String,
    pub twitter_handle: String,
}

// ── Issue #259: staking insurance (principal protection) ────────────────────────

/// Admin-configured insurance product terms (issue #259).
///
/// `premium_bps` is the share of every reward claim redirected into the
/// insurance fund while a policy is active. `max_coverage_per_user` caps the
/// principal any single policy may cover.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct InsuranceProduct {
    pub premium_bps: u32,
    pub max_coverage_per_user: i128,
}

/// A user's active principal-protection policy (issue #259).
///
/// `coverage_amount` is snapshotted from the position at purchase time and is
/// what `declare_shortfall()` pays out from the insurance fund (issue #141).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct InsurancePolicy {
    pub premium_bps: u32,
    pub coverage_amount: i128,
    pub active_since: u32,
    pub last_premium_at: u32,
}

// ── Issue #260: flash stake ─────────────────────────────────────────────────────

/// Permanent on-chain proof that `user` held `amount` at ledger `ledger`,
/// produced by `flash_stake()` (issue #260).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct FlashStakeReceipt {
    pub user: Address,
    pub amount: i128,
    pub ledger: u32,
    pub receipt_id: u64,
}

// ── Issue #261: stake-backed loans ──────────────────────────────────────────────

/// Admin-set borrowing terms (issue #261). `max_ltv_bps` bounds the loan
/// against the borrower's staked principal; `interest_rate_bps` is a simple
/// (non-compounding) annual rate accrued per ledger.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LoanConfig {
    pub max_ltv_bps: u32,
    pub interest_rate_bps: u32,
}

/// A user's outstanding loan against their staking position (issue #261).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Loan {
    pub principal: i128,
    pub interest_accrued: i128,
    pub opened_at: u32,
    pub last_interest_at: u32,
}

// ── Issue #276: seasonal reward multiplier ────────────────────────────────────

/// An admin-configured calendar window during which the reward rate is
/// multiplied by `multiplier_bps` (issue #276). At most one season may be
/// active at any given ledger — `add_season()` rejects overlapping ranges.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Season {
    pub name: String,
    pub starts_at: u32,
    pub ends_at: u32,
    pub multiplier_bps: u32,
}

// ── Issue #298: pool sunsetting workflow ──────────────────────────────────────

/// Lifecycle stage of a pool being formally deprecated (issue #298).
/// Transitions are one-way: `Active` -> `SunsetAnnounced` -> `GracePeriodActive`
/// -> `ForceResolutionActive` -> `Closed`.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SunsetState {
    Active,
    SunsetAnnounced,
    GracePeriodActive,
    ForceResolutionActive,
    Closed,
}

// ── Issue #286: debt NFT collateral ─────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DebtNFT {
    pub id: u32,
    pub issuer: Address,
    pub holder: Address,
    pub face_value: i128,
    pub expires_at: u32,
    pub minted_at: u32,
}

// ── Issue #283: position AMM (atomic position swaps) ─────────────────────────

/// A peer-to-peer swap offer between two stakers (issue #283).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SwapOffer {
    pub offerer: Address,
    pub offered_amount: i128,
    pub requested_address: Address,
    pub requested_amount: i128,
    pub expires_at: u32,
}

// ── Issue #284: reward prediction market ────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PredictionMarket {
    pub closes_at: u32,
    pub target_ledger: u32,
    pub current_rate_bps: i128,
    pub bets_higher: i128,
    pub bets_lower: i128,
    pub resolved: bool,
    pub outcome: Option<bool>,
}

// ── Issue #285: cross-pool yield detector ────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct YieldComparison {
    pub pool: Address,
    pub rate_bps: i128,
    pub reachable: bool,
    pub is_higher: bool,
}
// ── Issue #281: Fee Revenue Sharing ──────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RevenueSharingConfig {
    pub governance_token: Address,
    pub share_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RevenueShareMerkleRoot {
    pub root: soroban_sdk::Bytes,
    pub epoch: u32,
    pub total_amount: i128,
}

// ── Issue #282: Stake-Gated Access ───────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AccessTier {
    pub min_stake: i128,
    pub min_duration_ledgers: u32,
    pub access_token_contract: Address,
}

// ── Issue #314: withdrawal receipt ───────────────────────────────────────────

/// Immutable on-chain proof of an unstake operation (issue #314).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct WithdrawalReceipt {
    pub receipt_id: u64,
    pub user: Address,
    pub amount_returned: i128,
    pub rewards_claimed: i128,
    pub unstaked_at: u32,
    pub lock_penalty_paid: i128,
}

/// Tracks completion of recommended onboarding steps for a staker.





























// ── Issue #105: stake/unstake history ────────────────────────────────────────



// ── Issue #104: interface detection ──────────────────────────────────────────









// ── Issue #219: pause reason ─────────────────────────────────────────────────



// ── Issue #217: tax reporting ────────────────────────────────────────────────


// ── Issue #220: rounding policy ──────────────────────────────────────────────


// ── Issue #216: governance voting ────────────────────────────────────────────



// ── Issue #207: cross-chain bridge relayer hook ──────────────────────────────


// ── Issue #197: fee splitting ─────────────────────────────────────────────────


// ── Issue #195: timelocked admin actions ─────────────────────────────────────


// ── Issue #196: multi-sig admin ──────────────────────────────────────────────



// ── Missing types for existing feature branches ──────────────────────────────





// ── Issue #210: Merkle Reward Distribution ────────────────────────────────────


// ── Issue #231: Halving Schedule ──────────────────────────────────────────────


// ── Issue #222: Staking Certificate ───────────────────────────────────────────


// ── Issue #211: Staking Tournament Competition ─────────────────────────────────


// ── Issue #221: Cross-Pool Reward Comparison ──────────────────────────────────


// ── Issue #235: Reward Smoothing ──────────────────────────────────────────────



// ── Issue #236: Referral Tree Visualization ───────────────────────────────────


// ── Issue #237: Capacity Auction ──────────────────────────────────────────────



// ── Issue #239: stake-weighted lottery ────────────────────────────────────────


// ── Issue #238: loyalty milestone badges ──────────────────────────────────────



// ── Issue #240: oracle-triggered lock-up release ──────────────────────────────



// ── Issue #250: optimal claim frequency advisory ───────────────────────────────


// ── Issue #257: auto-convert reward on claim ────────────────────────────────────


// ── Issue #251: exit-queue priority bidding ─────────────────────────────────────


// ── Issue #258: pool whitelabel branding ────────────────────────────────────────


// ── Issue #259: staking insurance (principal protection) ────────────────────────



// ── Issue #260: flash stake ─────────────────────────────────────────────────────


// ── Issue #261: stake-backed loans ──────────────────────────────────────────────



// ── Issue #276: seasonal reward multiplier ────────────────────────────────────


// ── Issue #298: pool sunsetting workflow ──────────────────────────────────────


// ── Issue #281: Fee Revenue Sharing ──────────────────────────────────────────



// ── Issue #282: Stake-Gated Access ───────────────────────────────────────────


// ── Issue #314: withdrawal receipt ───────────────────────────────────────────


// ── Issue #309: staker onboarding checklist ───────────────────────────────────

/// Tracks completion of the recommended onboarding steps for a staker (issue
/// #309). Append-only: flags never reset to false even after the user
/// unstakes. `completed_at` is set once, to the ledger at which the last
/// remaining flag first flipped true.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OnboardingChecklist {
    pub has_staked: bool,
    pub has_claimed: bool,
    pub has_set_bio: bool,
    pub has_enabled_streaming: bool,
    pub has_set_auto_restake: bool,
    pub completed_at: Option<u32>,
}

/// A contract address approved to stake on a user's behalf.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ContractDelegate {
    pub contract_address: Address,
    pub max_stake_per_call: i128,
    pub total_authorized: i128,
    pub total_used: i128,
}

// ── Issue #392: loyalty points system ───────────────────────────────────────

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PointsAction {
    PerLedgerStaked,
    PerClaim,
    PerGovernanceVote,
    PerMilestone,
    PerReferral,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PointsRule {
    pub action: PointsAction,
    pub points_per_action: u32,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PointsBenefit {
    FeeWaiver,
    BoostUnlock,
    EarlyAccess,
}

/// Quiz structure for stake_to_learn feature (issue #391).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Quiz {
    pub id: u32,
    pub question_hash: soroban_sdk::Bytes,
    pub answer_hash: soroban_sdk::Bytes,
    pub reward_tier_unlocked: u32,
    pub attempts_allowed: u32,
}

// ── Issue #377: position health alert ────────────────────────────────────────

/// Result of `position_health_alert()`: a unified check across every
/// attention-worthy condition a staker's position can be in (issue #377).
///
/// Each `Option` field is `None` when that condition doesn't apply to the
/// position (e.g. no lock period configured, or no outstanding loan) rather
/// than a false alarm. `needs_attention` is the OR of the four boolean flags,
/// so a frontend can branch on it alone without inspecting every field.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PositionHealthAlert {
    pub user: Address,
    pub needs_attention: bool,
    pub approaching_expiry: bool,
    pub ledgers_until_expiry: Option<u32>,
    pub lock_ending_soon: bool,
    pub ledgers_until_unlock: Option<u32>,
    pub loan_at_risk: bool,
    pub loan_health_factor_bps: Option<u32>,
    pub rewards_near_cap: bool,
}

// ── Issue #376: halving countdown ────────────────────────────────────────────

/// Countdown to the next reward halving, returned by `halving_countdown()`
/// (issue #376). Builds on the halving schedule from issue #231
/// (`HalvingConfig`). All fields are zero when no halving schedule has been
/// configured via `set_halving_config()`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HalvingCountdown {
    pub next_halving_ledger: u32,
    pub ledgers_remaining: u32,
    pub estimated_days_remaining: u32,
    pub halvings_so_far: u32,
    pub current_rate_bps: i128,
    pub post_halving_rate_bps: i128,
}

// ── Issue #375: governance proposal comment thread ───────────────────────────

/// A single stake-weighted comment on a governance proposal (issue #375).
///
/// `stake_weight` is the author's staked shares snapshotted at post time —
/// same convention `GovernanceProposal` voting already uses ("not adjusted
/// retroactively if the staker's position changes afterward").
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProposalComment {
    pub author: Address,
    pub text: String,
    pub stake_weight: i128,
    pub posted_at: u32,
}

