use soroban_sdk::contracterror;

/// # Policy: error codes are never reused
///
/// Every variant's numeric discriminant is part of this contract's public
/// ABI — callers match on that number, not the Rust variant name. Once a
/// number is assigned to a shipped variant, it must never be reassigned,
/// renumbered, or freed for reuse by deleting the variant, even if the
/// variant is later deprecated. New cases get the next unused number;
/// `VaultError` is already at Soroban's 50-variant cap (see the comment
/// above `VaultExtError` below), so anything new goes into one of the
/// satellite enums instead. `DepositorCapReached = 27` is the one
/// documented exception, and only because it reuses a slot that was
/// planned but never actually shipped under a different meaning — see
/// its own doc comment.
///
/// Enforced by `src/test_error_codes.rs`'s snapshot test: it fails if any
/// existing code here is renumbered or deleted, and fails to *compile* if
/// a new variant is added without also being added to that file.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultError {
    NotInitialized = 1,
    /// Returned by initialize() when the vault has already been initialized,
    /// and by `enact_proposal()` when the proposal has already been enacted.
    AlreadyInitialized = 2,
    /// Returned by admin-only entrypoints that call `admin::require_admin()`
    /// and by `rescue_token()` / `slash()` when the supplied admin address does
    /// not match the stored admin.
    Unauthorized = 3,
    /// Returned by staking, unstaking, and amount-setting calls when a
    /// caller supplies zero or a negative amount where that is not allowed.
    ZeroAmount = 4,
    /// Returned by `withdraw()`, `unstake()`, and `unstake_all()` when the
    /// caller tries to burn more shares than they own.
    InsufficientShares = 5,
    /// Returned by staking, unstaking, and admin-yield entrypoints that require
    /// the pool to be unpaused.
    VaultPaused = 6,
    /// Returned when a token address does not implement the expected SEP-41 token
    /// interface during initialization or token validation fails (issue #598).
    InvalidTokenInterface = 7,
    /// Returned by staking, unstaking, claim, slash, preview, and reward math
    /// helpers when checked arithmetic or share conversion fails.
    ArithmeticError = 8,
    /// Returned by `withdraw()`, `unstake()`, and `unstake_all()` when the
    /// requested share amount exceeds the configured per-transaction limit.
    WithdrawalLimitExceeded = 9,
    /// Returned by `set_early_exit_penalty_bps()` when the admin sets a value
    /// above the supported cap.
    InvalidPenaltyBps = 10,
    /// Returned by `deposit()`, `stake()`, and `stake_for()` when the resulting
    /// position would fall below the configured minimum stake.
    BelowMinimumStake = 11,
    /// Returned by `set_boost_schedule()` when more than five boost tiers are
    /// supplied.
    TooManyBoostTiers = 12,
    /// Returned by `set_boost_schedule()` when a tier multiplier is below the
    /// base rate or the tier ledgers are not strictly increasing.
    InvalidBoostSchedule = 13,
    /// Returned by `claim()`, `stake_and_claim()`, and `claim_epoch_rewards()`
    /// when the reward pool does not hold enough tokens to pay the claim, and
    /// by `withdraw_from_yield()` when the requested amount exceeds what is
    /// currently tracked as deployed to the yield protocol.
    InsufficientRewardPool = 14,
    /// Returned by `revoke_delegate()` when the caller revokes the wrong
    /// delegate, and by `stake_for()` when the caller is not the approved
    /// delegate for the beneficiary.
    NotADelegate = 15,
    /// Returned by `rescue_token()` when the admin tries to rescue the stake
    /// token itself.
    CannotRescueStakeToken = 16,
    /// Returned by `rescue_token()` when the admin tries to rescue the
    /// registered reward token.
    CannotRescueRewardToken = 17,
    /// Returned by position-dependent flows such as `unstake_all()`,
    /// `claimable_since()`, `position_age_ledgers()`, `time_since_last_claim()`,
    /// `request_unstake()`, `execute_unstake()`, `slash()`, `transfer_position()`,
    /// `merge_positions()`, and `flag_frozen()` when the user has no active
    /// stake or unbonding position. Also returned by `create_proposal()` and
    /// `vote()` when the caller has no active position, and by `vote()` /
    /// `enact_proposal()` when the given proposal id does not exist.
    PositionNotFound = 18,
    /// Returned by `deposit()`, `stake()`, `stake_for()`, and `stake_and_claim()`
    /// when whitelist enforcement is enabled and the staker or beneficiary is
    /// not approved.
    NotWhitelisted = 19,
    /// Returned by `withdraw()` and `unstake()` when cooldown is enabled, and
    /// by `execute_unstake()` when the cooldown has not finished yet.
    UseCooldownFlow = 20,
    /// Returned by `set_unstake_fee_bps()` when the fee exceeds 500 bps (5%).
    UnstakeFeeTooHigh = 21,
    /// Returned by `batch_position_query()` when more than 20 addresses are supplied.
    BatchTooLarge = 22,
    /// Returned by `vote()` when the caller has already voted on the given
    /// proposal.
    TooManyStakers = 23,
    /// Returned by `transfer_position()` when the recipient already has an
    /// active staking position.
    RecipientAlreadyStaking = 24,
    /// Returned by `start_boost_campaign()` when a boost campaign is already active.
    CampaignAlreadyActive = 25,
    /// Returned by `end_boost_campaign()` when there is no active boost campaign
    /// to cancel.
    NoCampaignActive = 26,
    /// Returned by `deposit()`, `stake()`, and `stake_and_claim()` when the
    /// admin-configured unique-depositor cap (issue #568) has already been
    /// reached and the caller has never deposited before. Reuses the numeric
    /// slot of the never-implemented `LeaderboardSizeTooLarge` case because
    /// Soroban caps `#[contracterror]` enums at 50 variants and all other slots
    /// are live.
    DepositorCapReached = 27,
    /// Returned by `view_all_positions()` when `page_size` is 0 or greater than 20.
    PageSizeTooLarge = 28,
    /// Returned by staking entrypoints when KYC enforcement is enabled and the
    /// staker is not approved.
    KycNotApproved = 29,
    /// Returned by `deposit()`, `stake()`, `stake_for()`, `stake_and_claim()`,
    /// `pause()`, and `unpause()` after `emergency_stop()` has permanently
    /// stopped the contract.
    ContractStopped = 30,
    /// Returned by staking entrypoints when the new deposit would exceed the
    /// configured pool cap, and by `deploy_to_yield()` when the requested
    /// amount exceeds `available_for_yield()` (the 20% liquidity buffer).
    PoolCapReached = 31,
    /// Returned by `set_pool_description()` when the description exceeds 200
    /// characters.
    DescriptionTooLong = 32,
    /// Returned by `record_wave_activity()` when the supplied wave id is not
    /// greater than the last recorded wave.
    NonMonotonicWaveId = 33,
    /// Returned by `record_wave_activity()` when more than 50 active users are
    /// supplied in one call.
    TooManyActiveUsers = 34,
    /// Returned by `initialize()` when the admin or token address is invalid
    /// for this contract, such as matching the contract's own address.
    InvalidAddress = 35,
    /// Returned by `initialize()` and `set_reward_rate_bps()` when the reward
    /// APR exceeds the configured cap.
    RateTooHigh = 36,
    /// Returned by staking entrypoints when the user already holds the
    /// configured maximum number of active positions, and by
    /// `create_proposal()` when 10 governance proposals are already open.
    MaxPositionsReached = 37,
    /// Returned by `set_max_positions_per_user()` when the requested cap exceeds 10.
    MaxPositionsTooHigh = 38,
    /// Returned by `vote()` when the proposal's voting period has already
    /// ended, or the proposal has already been enacted. Also returned by
    /// `enact_proposal()` when the proposal has been vetoed (issue #241).
    BatchKycTooLarge = 39,
    /// Returned by `set_dynamic_fee_config()` when `base_fee_bps > max_fee_bps`
    /// or `utilization_threshold_bps` exceeds 10 000 (100%).
    InvalidRate = 40,
    /// Custom error message exceeds MAX_ERROR_MESSAGE_LENGTH (150 characters).
    MessageTooLong = 41,

    /// Returned by epoch-mode entrypoints when the contract is in the wrong mode.
    EpochModeConflict = 42,
    /// Returned when a vesting queue already holds the maximum supported entries.
    VestingQueueFull = 43,
    /// Returned when a vesting withdrawal is requested but nothing has matured yet.
    NothingToWithdraw = 44,
    /// Returned when an epoch cannot be finalized because the configured
    /// window has not elapsed, and by `enact_proposal()` when the voting
    /// period has not ended yet.
    EpochNotFinalized = 45,
    /// Caller is not an approved relayer for the target user (issue #118).
    RelayerNotApproved = 46,
    /// Caller is not on the yield source whitelist (issue #126).
    NotYieldSource = 47,
    /// notify_reward_added called with a zero or negative amount (issue #126).
    InvalidRewardAmount = 48,
    /// Returned when a new stake is attempted after `start_graceful_shutdown` has been called.
    PoolShuttingDown = 49,
    /// Returned by every admin-gated entrypoint after admin renouncement.
    NoAdmin = 50,
}

impl VaultError {
    /// Backwards compatibility alias for `InvalidTokenInterface`.
    pub const InvalidToken: VaultError = VaultError::InvalidTokenInterface;
}

/// Typed validation failures for public APIs that historically panicked.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PublicApiError {
    Unauthorized = 1,
    NotInitialized = 2,
    InvalidAllocation = 3,
    UnsupportedToken = 4,
    ActionNotYetExecutable = 5,
    ActionNotFound = 6,
    AutoCompoundDisabled = 7,
    PositionNotFound = 8,
    NotNftOwner = 9,
    ArithmeticError = 10,
    NoAdmin = 11,
}

impl From<VaultError> for PublicApiError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => Self::Unauthorized,
            VaultError::NotInitialized => Self::NotInitialized,
            VaultError::NoAdmin => Self::NoAdmin,
            VaultError::PositionNotFound => Self::PositionNotFound,
            VaultError::ArithmeticError => Self::ArithmeticError,
            _ => Self::Unauthorized,
        }
    }
}

/// Errors for voluntarily locked positions. Kept separate because `VaultError`
/// has reached Soroban's 50-variant contract error limit.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultLockError {
    NotInitialized = 1,
    Unauthorized = 3,
    ZeroAmount = 4,
    InsufficientShares = 5,
    VaultPaused = 6,
    ArithmeticError = 8,
    WithdrawalLimitExceeded = 9,
    PositionNotFound = 18,
    UseCooldownFlow = 20,
    PositionLocked = 51,
    LockDurationTooLong = 52,
    InvalidLockBoostSchedule = 53,
    TooManyLockBoostTiers = 54,
    LockAlreadyActive = 55,
    /// Returned by `lock_position()` while the pool is in withdraw-only mode
    /// (issue #705).
    WithdrawOnlyMode = 56,
}

/// Soroban caps every `#[contracterror]`/`#[contracttype]` enum at 50 variants
/// (`ScSpecUdtUnionV0::cases` is a `VecM<_, 50>` in stellar-xdr) ΓÇö `VaultError`
/// above is already at exactly that cap, so new error cases for issues #205,
/// #206, and #209 can't be added to it. This second, separate error enum
/// holds just those new cases, plus mirrors of the handful of `VaultError`
/// cases the new functions can also hit (via the `From` impl below, so `?`
/// still works normally at call sites).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultExtError {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Mirrors `VaultError::ZeroAmount`.
    ZeroAmount = 3,
    /// Mirrors `VaultError::ArithmeticError`.
    ArithmeticError = 4,
    /// Mirrors `VaultError::AlreadyInitialized` ΓÇö returned by `import_state()`.
    AlreadyInitialized = 5,
    /// Mirrors `VaultError::PositionNotFound`.
    PositionNotFound = 6,
    /// Returned by `set_insurance_rate_bps()` when `bps` exceeds 500 (5%)
    /// (issue #199).
    InvalidInsuranceRate = 7,
    /// Returned by `export_state()` when more than 100 positions would be
    /// exported (issue #203).
    TooManyPositions = 8,
    /// Returned by `set_fee_recipients()` when `recipients.len() > 5`
    /// (issue #197), and by `add_output_token()` when the output-token
    /// whitelist is already full (issue #244).
    TooManyRecipients = 9,
    /// Returned by `set_fee_recipients()` when the recipients' `share_bps`
    /// values don't sum to exactly 10 000 (issue #197).
    InvalidFeeAllocation = 10,
    /// Returned by `queue_action()` when 5 actions are already pending
    /// (issue #195).
    TooManyPendingActions = 11,
    /// Returned by `execute_action()`/`cancel_action()` when the given
    /// action id doesn't exist or was already executed/cancelled (issue
    /// #195).
    ActionNotFound = 12,
    /// Returned by `execute_action()` when called before `executable_at`
    /// (issue #195).
    ActionNotYetExecutable = 13,
    /// Returned by `initialize_multisig()` when `admins.len()` is 0 or > 5,
    /// or `threshold` is 0 or greater than `admins.len()` (issue #196).
    InvalidMultisigConfig = 14,
    /// Returned by `propose_action()` when 10 open proposals already exist
    /// (issue #196).
    TooManyProposals = 15,
    /// Returned by `propose_action()`/`approve_action()` when the caller is
    /// not one of the configured multisig admins (issue #196).
    NotAMultisigAdmin = 16,
    /// Returned by `approve_action()` when the caller already approved this
    /// proposal (issue #196).
    AlreadyApproved = 17,
    /// Returned by `execute_proposal()` when approvals are below the
    /// configured threshold, or the proposal id doesn't exist / was already
    /// executed (issue #196).
    ProposalNotReady = 18,
    /// Returned by `rollback_last_rate_change()` when there is no previous
    /// rate to restore, or it was already rolled back (issue #206).
    RollbackUnavailable = 19,
    /// Returned by `swap_and_stake()` when the DEX swap's output amount is
    /// below the caller-supplied `min_stake_amount` (issue #205).
    SlippageExceeded = 20,
    /// Returned by `swap_and_stake()` when `input_token` is not registered
    /// with a DEX router capable of swapping it to the stake token, or when
    /// no DEX router has been configured at all (issue #205). Also returned
    /// by `claim_in_token()` for an `output_token` that is not on the
    /// admin-managed whitelist (issue #244).
    UnsupportedInputToken = 21,
    /// Returned by `position_split()` when `split_amount` is not strictly
    /// between 0 and the caller's current position amount (issue #209).
    InvalidSplitAmount = 22,
    /// Returned by `claim_merkle_reward()` when the Merkle proof is invalid.
    MerkleInvalidProof = 23,
    /// Returned by `claim_merkle_reward()` when the user has already claimed
    /// for the given epoch.
    MerkleAlreadyClaimed = 24,
    /// Returned by `create_tournament()` when a tournament is already active.
    TournamentAlreadyExists = 25,
    /// Returned by `finalize_tournament()` when the tournament has not ended yet.
    TournamentNotEnded = 26,
    /// Returned by `finalize_tournament()` when no tournament exists.
    TournamentNotFound = 27,
    /// Returned by `compare_pools()` when more than 5 external pools are supplied.
    /// Returned by `compare_pools()` when more than 5 external pools are supplied.
    TooManyPools = 28,
    /// Returned by `execute_buyback()` when buyback is not enabled.
    BuybackNotEnabled = 29,
    /// Returned by `execute_buyback()` when accrued fees are below the threshold.
    BuybackThresholdNotMet = 30,
    /// Returned by stake/claim rate-limit checks (issue #201).
    RateLimitExceeded = 31,
    /// Returned by `start_bootstrap()` when `initial_rate < base_rate`.
    InvalidBootstrapConfig = 32,
    /// Returned by delegation chain operations when a cycle is detected (issue #200).
    CircularDelegation = 33,
    /// Returned when a delegation chain would exceed the maximum length (issue #200).
    ChainTooLong = 34,
    /// Returned by `issue_certificate` when the user's stake is below the minimum (issue #222).
    IneligibleForCertificate = 35,
    /// Returned by `set_reward_smoothing()` when the smoothing period exceeds
    /// `MAX_SMOOTHING_PERIOD_LEDGERS`, or when `min_amount` is negative
    /// (issue #235).
    InvalidSmoothingConfig = 36,
    /// Returned by `referral_tree_data()` when `max_level` exceeds
    /// `MAX_REFERRAL_TREE_DEPTH` (issue #236).
    ReferralDepthTooDeep = 37,
    /// Returned by `start_capacity_auction()` when an auction is already open,
    /// and by `place_bid()` / `finalize_capacity_auction()` when no auction
    /// exists (issue #237).
    AuctionNotFound = 38,
    /// Returned by `start_capacity_auction()` when `spots` is 0 or above
    /// `MAX_AUCTION_SPOTS`, or `duration_ledgers` is 0 (issue #237).
    InvalidAuctionConfig = 39,
    /// Returned by `start_capacity_auction()` when the previous auction has not
    /// been finalized yet (issue #237).
    AuctionAlreadyActive = 40,
    /// Returned by `place_bid()` after the auction window has closed, and by
    /// `place_bid()` / `finalize_capacity_auction()` on an already-finalized
    /// auction (issue #237).
    AuctionClosed = 41,
    /// Returned by `finalize_capacity_auction()` when called before the auction
    /// window has elapsed (issue #237).
    AuctionNotEnded = 42,
    /// Returned by `place_bid()` when the resulting bid is below the auction's
    /// `min_bid` (issue #237).
    BidBelowMinimum = 43,
    /// Returned by `place_bid()` when the auction already holds
    /// `MAX_AUCTION_BIDS` distinct bidders (issue #237).
    TooManyBids = 44,
    /// Returned by `create_lottery()` when an undrawn lottery already exists,
    /// and by `draw_lottery()` when no lottery is configured or it was
    /// already drawn (issue #239).
    LotteryAlreadyActive = 45,
    /// Returned by `draw_lottery()` when called before `draw_at_ledger`
    /// (issue #239).
    LotteryNotReady = 46,
    /// Returned by `add_milestone()` when 10 milestones are already
    /// configured (issue #238).
    TooManyMilestones = 47,
    /// Returned by `check_and_release()` when no oracle contract has been
    /// registered via `set_oracle_contract()` (issue #240).
    NoOracleConfigured = 48,
    /// Returned by `veto_proposal()` when the caller's pool share is below
    /// the configured veto threshold, or the veto threshold is unset (0),
    /// which disables the feature entirely (issue #241).
    BelowVetoThreshold = 49,
    /// Returned by `veto_proposal()` when the proposal already has a vetoer,
    /// or has already been enacted and so can no longer be vetoed (issue
    /// #241).
    AlreadyVetoed = 50,
}

/// Third error enum, for the same 50-variant reason `VaultExtError` exists:
/// both `VaultError` and `VaultExtError` are now at exactly the cap, so the
/// cases introduced by issues #258 (branding), #259 (staking insurance),
/// #260 (flash stake), #261 (stake-backed loans), #275 (reward Gini
/// coefficient), #276 (seasonal reward multiplier), #274 (staker bio), and
/// #298 (pool sunsetting workflow) live here, plus mirrors of the handful of
/// `VaultError` cases those functions can also hit (via the `From` impl
/// below, so `?` still works normally at call sites).
///
/// Note on `InvalidBrandingField`: a `#[contracterror]` variant cannot carry a
/// payload, so the offending field name is encoded in the variant itself ΓÇö
/// `InvalidBrandingDisplayName`, `InvalidBrandingLogoHash`,
/// `InvalidBrandingWebsiteUrl`, `InvalidBrandingTwitterHandle` ΓÇö rather than
/// as an inner `String` on a single generic variant.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultFeatureError {
    Unauthorized = 1,
    NotInitialized = 2,
    ZeroAmount = 3,
    ArithmeticError = 4,
    PositionNotFound = 5,
    VaultPaused = 6,
    InsufficientRewardPool = 7,
    InvalidBrandingField = 8,
    InvalidInsuranceConfig = 9,
    InsuranceProductNotSet = 10,
    InsuranceAlreadyActive = 11,
    InsurancePolicyNotFound = 12,
    InsuranceFundInsufficient = 13,
    TooManyAffectedUsers = 14,
    TooManyStakers = 15,
    InvalidSeasonConfig = 16,
    TooManySeasons = 17,
    SeasonOverlap = 18,
    SeasonNotFound = 19,
    BioTooLong = 20,
    InvalidSunsetTransition = 21,
    PositionsStillActive = 22,
    InvalidFlashStakeFee = 23,
    InvalidLoanConfig = 24,
    LoanConfigNotSet = 25,
    LoanNotFound = 26,
    ExceedsMaxLtv = 27,
    LoanNotLiquidatable = 28,
    InvalidRevenueShareConfig = 29,
    RevenueShareInvalidProof = 30,
    RevenueShareAlreadyClaimed = 31,
    TooManyAccessTiers = 32,
    IneligibleForAccessTier = 33,
    UserStillEligible = 34,
    InvalidLotSize = 35,
    FeeBuybackNotEnabled = 36,
    NoDexRouterConfigured = 37,
    NotAContractDelegate = 38,
    ContractDelegateCapExceeded = 39,
    ContractDelegatePerCallExceeded = 40,
    PositionCollateralized = 41,
    FaceValueExceedsPosition = 42,
    DebtNftNotFound = 43,
    NotNftHolder = 44,
    TooManyCompetitors = 45,
    TooManyOpenOffers = 46,
    OfferNotFound = 47,
    OfferExpired = 48,
    NotRequestedCounterparty = 49,
    InsufficientAmount = 50,
}

// Fourth error enum for overflow errors beyond the Soroban 50-variant cap.
// VaultFeatureError exceeded the cap; new errors go here.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultOverflowError {
    Unauthorized = 1,
    NotInitialized = 2,
    ZeroAmount = 3,
    ArithmeticError = 4,
    PositionNotFound = 5,
    VaultPaused = 6,
    InsufficientRewardPool = 7,
    InvalidBrandingDisplayName = 8,
    InvalidBrandingLogoHash = 9,
    InvalidBrandingWebsiteUrl = 10,
    InvalidBrandingTwitterHandle = 11,
    InvalidInsuranceProduct = 12,
    InsuranceProductNotSet = 13,
    InsuranceAlreadyActive = 14,
    InsurancePolicyNotFound = 15,
    InsuranceFundInsufficient = 16,
    TooManyAffectedUsers = 17,
    TooManyStakers = 18,
    InvalidSeasonConfig = 19,
    TooManySeasons = 20,
    SeasonOverlap = 21,
    SeasonNotFound = 22,
    BioTooLong = 23,
    InvalidSunsetTransition = 24,
    PositionsStillActive = 25,
    InvalidFlashStakeFee = 26,
    InvalidLoanConfig = 27,
    LoanConfigNotSet = 28,
    LoanNotFound = 29,
    ExceedsMaxLtv = 30,
    LoanNotLiquidatable = 31,
    InvalidRevenueShareConfig = 32,
    RevenueShareInvalidProof = 33,
    RevenueShareAlreadyClaimed = 34,
    TooManyAccessTiers = 35,
    IneligibleForAccessTier = 36,
    UserStillEligible = 37,
    InvalidLotSize = 38,
    FeeBuybackNotEnabled = 39,
    NoDexRouterConfigured = 40,
    NotAContractDelegate = 41,
    ContractDelegateCapExceeded = 42,
    ContractDelegatePerCallExceeded = 43,
    PositionCollateralized = 44,
    FaceValueExceedsPosition = 45,
    DebtNftNotFound = 46,
    NotNftHolder = 47,
    TooManyCompetitors = 48,
    TooManyOpenOffers = 49,
    /// Returned by staking entrypoints when the stake amount is insufficient.
    InsufficientStake = 50,
}

impl From<VaultError> for VaultOverflowError {
    fn from(e: VaultError) -> Self {
        match e {
            VaultError::Unauthorized => VaultOverflowError::Unauthorized,
            VaultError::NotInitialized => VaultOverflowError::NotInitialized,
            VaultError::ZeroAmount => VaultOverflowError::ZeroAmount,
            VaultError::ArithmeticError => VaultOverflowError::ArithmeticError,
            VaultError::PositionNotFound => VaultOverflowError::PositionNotFound,
            VaultError::VaultPaused => VaultOverflowError::VaultPaused,
            VaultError::InsufficientRewardPool => VaultOverflowError::InsufficientRewardPool,
            _ => VaultOverflowError::Unauthorized,
        }
    }
}

impl From<VaultError> for VaultExtError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultExtError::Unauthorized,
            VaultError::NotInitialized => VaultExtError::NotInitialized,
            VaultError::ZeroAmount => VaultExtError::ZeroAmount,
            VaultError::ArithmeticError => VaultExtError::ArithmeticError,
            VaultError::PositionNotFound => VaultExtError::PositionNotFound,
            // Any other VaultError reaching here (shouldn't happen given how
            // these functions are written) maps to the closest generic case.
            _ => VaultExtError::Unauthorized,
        }
    }
}

impl From<VaultError> for VaultFeatureError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultFeatureError::Unauthorized,
            VaultError::NotInitialized => VaultFeatureError::NotInitialized,
            VaultError::ZeroAmount => VaultFeatureError::ZeroAmount,
            VaultError::ArithmeticError => VaultFeatureError::ArithmeticError,
            VaultError::PositionNotFound => VaultFeatureError::PositionNotFound,
            VaultError::VaultPaused => VaultFeatureError::VaultPaused,
            VaultError::InsufficientRewardPool => VaultFeatureError::InsufficientRewardPool,
            // Any other VaultError reaching here (shouldn't happen given how
            // these functions are written) maps to the closest generic case.
            _ => VaultFeatureError::Unauthorized,
        }
    }
}

/// Fifth error enum for issue #391 (stake_to_learn). Both `VaultError`,
/// `VaultExtError`, `VaultFeatureError`, and `VaultOverflowError` are at
/// Soroban's 50-variant cap, so quiz-specific errors live here.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultQuizError {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Mirrors `VaultError::ZeroAmount`.
    ZeroAmount = 3,
    /// Mirrors `VaultError::ArithmeticError`.
    ArithmeticError = 4,
    /// Mirrors `VaultError::VaultPaused`.
    VaultPaused = 5,
    /// Returned by `submit_quiz_answer()` when the given `quiz_id` does not
    /// correspond to any stored quiz.
    QuizNotFound = 6,
    /// Returned by `submit_quiz_answer()` when the user has already
    /// successfully completed this quiz.
    QuizAlreadyCompleted = 7,
    /// Returned by `submit_quiz_answer()` when the user has exhausted all
    /// allowed attempts for this quiz.
    QuizMaxAttemptsReached = 8,
    /// Returned by `add_quiz()` when the contract already holds the maximum
    /// of 20 quizzes.
    TooManyQuizzes = 9,
    /// Returned by `submit_daily_tip()` (issue #458) when `content` exceeds
    /// `MAX_TIP_CONTENT_LEN` characters.
    TipTooLong = 10,
    /// Returned by `submit_daily_tip()` (issue #458) when the caller has
    /// already submitted a tip for the current day.
    AlreadySubmittedToday = 11,
    /// Returned by `submit_daily_tip()` (issue #458) when the current day
    /// already holds `MAX_TIPS_PER_DAY` candidates.
    TooManyTipsToday = 12,
    /// Returned by `vote_daily_tip()` (issue #458) when the given `tip_id`
    /// does not correspond to a candidate submitted for the current day, and
    /// by `finalize_daily_tip()` when the given day has no candidates.
    TipNotFound = 13,
    /// Returned by `vote_daily_tip()` (issue #458) when the caller has
    /// already voted on a tip for the current day.
    AlreadyVotedToday = 14,
    /// Returned by `cancel_config_change()` / `execute_config_change()` /
    /// `get_admin_proposal()` (issue #455) when the given `proposal_id` does
    /// not exist.
    AdminProposalNotFound = 15,
    /// Returned by `cancel_config_change()` / `execute_config_change()`
    /// (issue #455) when the proposal was already cancelled.
    AdminProposalAlreadyCancelled = 16,
    /// Returned by `cancel_config_change()` / `execute_config_change()`
    /// (issue #455) when the proposal was already executed.
    AdminProposalAlreadyExecuted = 17,
    /// Returned by `execute_config_change()` (issue #455) when called
    /// before the proposal's `executes_at` ledger has been reached.
    AdminProposalNotYetExecutable = 18,
}

impl From<VaultError> for VaultQuizError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultQuizError::Unauthorized,
            VaultError::NotInitialized => VaultQuizError::NotInitialized,
            VaultError::ZeroAmount => VaultQuizError::ZeroAmount,
            VaultError::ArithmeticError => VaultQuizError::ArithmeticError,
            VaultError::VaultPaused => VaultQuizError::VaultPaused,
            _ => VaultQuizError::Unauthorized,
        }
    }
}

/// Sixth error enum, added for the same 50-variant reason the earlier
/// `Vault*Error` enums exist: every prior `#[contracterror]` enum is already at
/// Soroban's cap. Holds the cases introduced by issues #459 (position health
/// auto-recovery), #460 (lockdrop campaign), #461 (proof-of-humanity hook), and
/// #462 (roadmap voting), plus mirrors of the handful of `VaultError` cases
/// those functions can also hit (via the `From` impl below, so `?` keeps
/// working at call sites that mix the two).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultCampaignError {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Mirrors `VaultError::ZeroAmount`.
    ZeroAmount = 3,
    /// Mirrors `VaultError::ArithmeticError`.
    ArithmeticError = 4,
    /// Mirrors `VaultError::PositionNotFound`.
    PositionNotFound = 5,
    /// Mirrors `VaultError::VaultPaused`.
    VaultPaused = 6,

    // ── Issue #459: position health auto-recovery ────────────────────────────
    /// Returned by `check_and_recover()` / `cancel_recovery_config()` when the
    /// user has no active recovery configuration.
    RecoveryNotConfigured = 7,
    /// Returned by `check_and_recover()` when the user's current position
    /// health is at or above their configured trigger threshold (or the
    /// position carries no loan, so health is unbounded).
    RecoveryNotTriggered = 8,
    /// Returned by `check_and_recover()` when the per-user daily cooldown has
    /// not elapsed since the last recovery.
    RecoveryOnCooldown = 9,
    /// Returned by `set_recovery_config()` when `trigger_health_bps` is 0, or
    /// `action_amount` is not positive for an action that needs one.
    InvalidRecoveryConfig = 10,
    /// Returned by `check_and_recover()` when an `AutoRepayLoan` action is
    /// configured but the user has no outstanding loan.
    NoActiveLoan = 11,

    // ── Issue #460: lockdrop campaign ───────────────────────────────────────
    /// Returned by `start_lockdrop()` when a campaign that has not been
    /// finalized already exists.
    LockdropAlreadyActive = 12,
    /// Returned by lockdrop entrypoints when no campaign has been started.
    LockdropNotActive = 13,
    /// Returned by `commit_to_lockdrop()` after the commitment window
    /// (`ends_at`) has passed.
    LockdropEnded = 14,
    /// Returned by `finalize_lockdrop()` before `ends_at`.
    LockdropNotEnded = 15,
    /// Returned by `claim_lockdrop_reward()` before the campaign is finalized.
    LockdropNotFinalized = 16,
    /// Returned by `finalize_lockdrop()` when the campaign is already finalized.
    LockdropAlreadyFinalized = 17,
    /// Returned by `commit_to_lockdrop()` when the caller already has a
    /// commitment in the active campaign.
    AlreadyCommitted = 18,
    /// Returned by `exit_lockdrop()` / `claim_lockdrop_reward()` when the
    /// caller has no commitment.
    CommitmentNotFound = 19,
    /// Returned by `exit_lockdrop()` before the caller's chosen lock duration
    /// has elapsed.
    LockStillActive = 20,
    /// Returned by `claim_lockdrop_reward()` when the caller already claimed.
    AlreadyClaimed = 21,
    /// Returned by `claim_lockdrop_reward()` when the caller's proportional
    /// allocation is zero.
    NothingToClaim = 22,
    /// Returned by `commit_to_lockdrop()` when the campaign already holds the
    /// maximum supported number of committers.
    LockdropFull = 23,
    /// Returned by `commit_to_lockdrop()` when `lock_duration_ledgers` is 0 or
    /// exceeds the campaign's `max_lock_ledgers`.
    InvalidLockDuration = 24,

    // ── Issue #461: proof-of-humanity hook ─────────────────────────────────
    /// Returned by `stake_verified()` when no humanity oracle / config has
    /// been registered.
    OracleNotConfigured = 25,
    /// Returned by `stake_verified()` when the staked amount is below the
    /// minimum that applies to the caller's verification status.
    BelowHumanityMinStake = 26,
    /// Returned by `set_humanity_config()` when a bps field is out of range.
    InvalidHumanityConfig = 27,

    // ── Issue #462: roadmap voting ────────────────────────────────────────
    /// Returned by `add_roadmap_item()` when 20 items already exist.
    TooManyRoadmapItems = 28,
    /// Returned by `remove_roadmap_item()` / `vote_roadmap_item()` for an
    /// unknown item id.
    RoadmapItemNotFound = 29,
    /// Returned by `add_roadmap_item()` when the title exceeds 80 characters.
    TitleTooLong = 30,
    /// Returned by `vote_roadmap_item()` when the caller's allocations for the
    /// current epoch would exceed the 100-point budget.
    VoteBudgetExceeded = 31,
    /// Returned by `vote_roadmap_item()` when `weight` alone exceeds 100.
    InvalidVoteWeight = 32,

    // ── Issue #430: staker region tags ───────────────────────────────────
    /// Returned by `set_region_tag()` when the code exceeds 10 characters.
    RegionCodeTooLong = 33,
    /// Returned by `set_region_tag()` when the code is empty or contains a
    /// non-alphanumeric character.
    InvalidRegionCode = 34,
    /// Returned by `set_region_tag()` when a new tag would exceed
    /// `MAX_REGION_TAGGED_STAKERS`.
    TooManyRegionTags = 35,
}

impl From<VaultError> for VaultCampaignError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultCampaignError::Unauthorized,
            VaultError::NotInitialized => VaultCampaignError::NotInitialized,
            VaultError::ZeroAmount => VaultCampaignError::ZeroAmount,
            VaultError::ArithmeticError => VaultCampaignError::ArithmeticError,
            VaultError::PositionNotFound => VaultCampaignError::PositionNotFound,
            VaultError::VaultPaused => VaultCampaignError::VaultPaused,
            // Any other VaultError reaching here maps to the closest generic case.
            _ => VaultCampaignError::Unauthorized,
        }
    }
}

/// Seventh error enum, added for the same 50-variant reason the earlier
/// `Vault*Error` enums exist: every prior `#[contracterror]` enum is at
/// Soroban's cap. Holds the cases for the pool-insights / runway-guard /
/// admin-recovery issue batch, plus mirrors of the `VaultError` cases those
/// functions can hit (via the `From` impl below, so `?` still works).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultOpsError {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Mirrors `VaultError::ZeroAmount`.
    ZeroAmount = 3,
    /// Mirrors `VaultError::ArithmeticError`.
    ArithmeticError = 4,
    /// Mirrors `VaultError::RateTooHigh` — a reward rate above
    /// `balance::MAX_RATE_BPS` was supplied.
    RateTooHigh = 5,
    /// Returned by `set_reward_rate_bps` when the new rate would exhaust the
    /// reward pool before the configured minimum runway
    /// (`set_min_runway_ledgers`).
    InsufficientRunway = 6,
    /// Returned by `set_min_runway_ledgers` when `ledgers` is non-zero but
    /// below the supported floor.
    InvalidRunway = 7,
    /// Returned by `propose_admin_recovery` when a recovery proposal is
    /// already active.
    RecoveryAlreadyPending = 8,
    /// Returned by `execute_admin_recovery` / `cancel_admin_recovery` when no
    /// recovery proposal is active.
    RecoveryNotPending = 9,
    /// Returned by `execute_admin_recovery` before the recovery delay has
    /// elapsed.
    RecoveryDelayNotElapsed = 10,
    /// Returned by `propose_admin_recovery` when `new_admin` equals the
    /// current admin.
    InvalidRecoveryConfig = 11,
    /// Returned by `set_reward_rate_bps` while the algorithmic reward rate
    /// is enabled (issue #510) — the rate is derived from utilization.
    DynamicRateActive = 12,
    /// Returned by `fund_reward_pool` when `amount` is below the configured
    /// minimum (issue #549). Disabled when the minimum is 0.
    FundingBelowMinimum = 13,
}

impl From<VaultError> for VaultOpsError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultOpsError::Unauthorized,
            VaultError::NotInitialized => VaultOpsError::NotInitialized,
            VaultError::ZeroAmount => VaultOpsError::ZeroAmount,
            VaultError::ArithmeticError => VaultOpsError::ArithmeticError,
            VaultError::RateTooHigh => VaultOpsError::RateTooHigh,
            // Any other VaultError reaching here maps to the closest generic case.
            _ => VaultOpsError::Unauthorized,
        }
    }
}

/// Eighth error enum for issues #526-#529 (scheduled exit, snapshot airdrop,
/// external price oracle, co-sponsor). All prior `#[contracterror]` enums
/// are at Soroban's 50-variant cap.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultFeature2Error {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Mirrors `VaultError::ZeroAmount`.
    ZeroAmount = 3,
    /// Mirrors `VaultError::ArithmeticError`.
    ArithmeticError = 4,
    /// Mirrors `VaultError::PositionNotFound`.
    PositionNotFound = 5,
    /// Mirrors `VaultError::VaultPaused`.
    VaultPaused = 6,
    /// Mirrors `VaultError::InsufficientRewardPool`.
    InsufficientRewardPool = 7,
    /// Returned by `create_airdrop()` / `execute_scheduled_exit()` when the
    /// supplied ledger or config is invalid.
    InvalidRecoveryConfig = 8,
    /// Returned by `register_co_sponsor()` when the sponsor is already
    /// registered and active.
    AlreadyRegistered = 9,
    /// Returned by `fund_co_sponsor_rewards()` when the sponsor's
    /// registration has expired.
    SponsorExpired = 10,
    /// Returned by `claim_airdrop()` when the user already claimed.
    AlreadyClaimed = 11,
    /// Returned by `get_position_value_usd()` when no oracle is configured.
    NoOracleConfigured = 12,
    /// Returned by `claim_airdrop()` when the user has no weight at the
    /// snapshot ledger.
    InsufficientStake = 13,
}

/// Ninth error enum, for issues #519 (vote-weight delegation) and #520
/// (tiered fee discounts). All eight prior `#[contracterror]` enums are at
/// Soroban's 50-variant cap. Issues #518 (`transfer_position`) and #521
/// (guardian pause) don't need new cases here — every error they can return
/// already exists on `VaultError` itself (`PositionNotFound`,
/// `RecipientAlreadyStaking`, `Unauthorized`, `ContractStopped`, etc.), so
/// those two entrypoints just return `VaultError` directly.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultFeature3Error {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Mirrors `VaultError::ZeroAmount`.
    ZeroAmount = 3,
    /// Mirrors `VaultError::ArithmeticError`.
    ArithmeticError = 4,
    /// Mirrors `VaultError::PositionNotFound`.
    PositionNotFound = 5,
    /// Mirrors `VaultError::InsufficientShares`.
    InsufficientShares = 6,
    /// Returned by `set_fee_tiers()` (issue #520) when more than
    /// `MAX_FEE_TIERS` tiers are supplied.
    TooManyFeeTiers = 7,
    /// Returned by `set_fee_tiers()` (issue #520) when a tier's
    /// `discount_bps` exceeds 10 000 (100%) or `min_position_amount` is
    /// negative.
    InvalidFeeTierConfig = 8,
    /// Returned by `delegate_vote_weight()` (issue #519) when a user tries
    /// to delegate to themselves.
    SelfDelegationNotAllowed = 9,
    /// Returned by `delegate_vote_weight()` (issue #519) when the target
    /// delegate already has `MAX_DELEGATORS_PER_DELEGATE` delegators.
    TooManyDelegators = 10,
    /// Returned by `revoke_vote_delegation()` (issue #519) when the caller
    /// has no active delegation to revoke.
    NoDelegationSet = 11,
}

impl From<VaultError> for VaultFeature3Error {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultFeature3Error::Unauthorized,
            VaultError::NotInitialized => VaultFeature3Error::NotInitialized,
            VaultError::ZeroAmount => VaultFeature3Error::ZeroAmount,
            VaultError::ArithmeticError => VaultFeature3Error::ArithmeticError,
            VaultError::PositionNotFound => VaultFeature3Error::PositionNotFound,
            VaultError::InsufficientShares => VaultFeature3Error::InsufficientShares,
            _ => VaultFeature3Error::Unauthorized,
        }
    }
}

impl From<VaultError> for VaultFeature2Error {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultFeature2Error::Unauthorized,
            VaultError::NotInitialized => VaultFeature2Error::NotInitialized,
            VaultError::ZeroAmount => VaultFeature2Error::ZeroAmount,
            VaultError::ArithmeticError => VaultFeature2Error::ArithmeticError,
            VaultError::PositionNotFound => VaultFeature2Error::PositionNotFound,
            VaultError::VaultPaused => VaultFeature2Error::VaultPaused,
            VaultError::InsufficientRewardPool => VaultFeature2Error::InsufficientRewardPool,
            _ => VaultFeature2Error::Unauthorized,
        }
    }
}

/// Tenth error enum for issues #530-#533 (activity log, invariant checker,
/// reward-rate ceiling, pause grace period).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultFeature4Error {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Returned by `force_unpause()` when the vault is not paused.
    NotPaused = 3,
    /// Returned by `force_unpause()` before the maximum pause duration has
    /// elapsed.
    GracePeriodNotElapsed = 4,
    /// Returned by `set_max_pause_duration()` while the vault is paused, or
    /// when the supplied duration is below `MIN_MAX_PAUSE_LEDGERS`.
    InvalidPauseDuration = 5,
    /// Returned by `set_max_reward_rate()` when the new ceiling is zero,
    /// higher than the current ceiling, or below the current reward rate.
    InvalidRateCeiling = 6,
    /// Returned by `assert_invariants()` when a core accounting total
    /// (shares, deposits, reward pool) is negative.
    NegativeAccounting = 7,
    /// Returned by `assert_invariants()` when the sum of all stakers' share
    /// balances differs from total shares outstanding.
    SharesMismatch = 8,
    /// Returned by `assert_invariants()` when the vault's actual token
    /// balance is below what its accounting says it holds.
    Undercollateralized = 9,
}

impl From<VaultError> for VaultFeature4Error {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultFeature4Error::Unauthorized,
            VaultError::NotInitialized => VaultFeature4Error::NotInitialized,
            _ => VaultFeature4Error::Unauthorized,
        }
    }
}

/// Error enum for issue #513 (role-based access control) and issue #510
/// (utilization-driven dynamic reward rate). All ten prior `#[contracterror]`
/// enums are either at Soroban's 50-variant cap or scoped to another feature
/// batch, so the access-control cases live here.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultAccessError {
    /// Mirrors `VaultError::Unauthorized` — the caller is not the admin and
    /// does not hold the required role.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Returned by `require_role()` when the caller is neither the admin nor
    /// an explicit grantee of the required `Role` (issue #513).
    MissingRole = 3,
    /// Mirrors `VaultError::UnstakeFeeTooHigh` — returned by
    /// `role_set_unstake_fee_bps()` when the fee exceeds 500 bps.
    UnstakeFeeTooHigh = 4,
    /// Mirrors `VaultError::RateTooHigh`.
    RateTooHigh = 5,
    /// Returned by `set_dynamic_rate_config()` when the curve is invalid:
    /// `target_tvl <= 0`, or `min_rate_bps > target_rate_bps`, or
    /// `target_rate_bps > max_rate_bps` (issue #510).
    InvalidDynamicRateConfig = 6,
    /// Mirrors `VaultOpsError::InsufficientRunway`, surfaced through the
    /// role-gated rate setter.
    InsufficientRunway = 7,
    /// Mirrors `VaultOpsError::DynamicRateActive`, surfaced through the
    /// role-gated rate setter.
    DynamicRateActive = 8,
}

impl From<VaultError> for VaultAccessError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultAccessError::Unauthorized,
            VaultError::NotInitialized => VaultAccessError::NotInitialized,
            VaultError::RateTooHigh => VaultAccessError::RateTooHigh,
            VaultError::UnstakeFeeTooHigh => VaultAccessError::UnstakeFeeTooHigh,
            _ => VaultAccessError::Unauthorized,
        }
    }
}

impl From<VaultOpsError> for VaultAccessError {
    fn from(err: VaultOpsError) -> Self {
        match err {
            VaultOpsError::Unauthorized => VaultAccessError::Unauthorized,
            VaultOpsError::NotInitialized => VaultAccessError::NotInitialized,
            VaultOpsError::RateTooHigh => VaultAccessError::RateTooHigh,
            VaultOpsError::InsufficientRunway => VaultAccessError::InsufficientRunway,
            VaultOpsError::DynamicRateActive => VaultAccessError::DynamicRateActive,
            _ => VaultAccessError::Unauthorized,
        }
    }
}

/// Twelfth error enum, added for issue #593: replaces the last ad-hoc string
/// `panic!`s in the issue #498–#505 extension modules with typed, matchable
/// error codes. Every prior `#[contracterror]` enum is either at Soroban's
/// 50-variant cap or scoped to another feature batch. The variant names keep
/// the exact wording of the string panics they replace so integrators can map
/// old revert messages to stable numeric codes one-to-one.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultFeature5Error {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Replaces `panic!("InvalidSplitRecipients")` — returned by
    /// `set_treasury_split()` when more than 3 recipients are supplied or the
    /// `recipients` / `bps_shares` lengths differ (issue #499).
    InvalidSplitRecipients = 3,
    /// Replaces `panic!("InvalidSplitBpsSum")` — returned by
    /// `set_treasury_split()` when the `bps_shares` values do not sum to
    /// exactly 10 000 (issue #499).
    InvalidSplitBpsSum = 4,
    /// Replaces `panic!("UnregisteredToken")` — returned by
    /// `swap_secondary_reward()` when no DEX router is registered for the
    /// supplied source token (issue #501).
    UnregisteredToken = 5,
    /// Replaces `panic!("TimelockNotExpired")` — returned by
    /// `execute_admin_action()` when the queued action's `executable_at`
    /// ledger has not been reached yet (issue #503).
    TimelockNotExpired = 6,
    /// Replaces `panic!("ActionNotFound")` — returned by
    /// `execute_admin_action()` / `cancel_admin_action()` when the given
    /// action id does not exist (issue #503).
    ActionNotFound = 7,
    /// Replaces `panic!("AutoCompoundNotEnabled")` — returned by `compound()`
    /// when the user has not opted into auto-compounding (issue #504).
    AutoCompoundNotEnabled = 8,
    /// Replaces `panic!("NoSharesToTokenize")` — returned by
    /// `tokenize_position()` when the caller holds no shares (issue #505).
    NoSharesToTokenize = 9,
    /// Replaces `panic!("NotNFTOwner")` — returned by `redeem_position_nft()`
    /// when the caller is not the recorded owner of the token id (issue #505).
    NotNftOwner = 10,
    /// Returned by `compound()` while the pool is in withdraw-only mode
    /// (issue #705).
    WithdrawOnlyMode = 11,
}

impl From<VaultError> for VaultFeature5Error {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultFeature5Error::Unauthorized,
            VaultError::NotInitialized => VaultFeature5Error::NotInitialized,
            _ => VaultFeature5Error::Unauthorized,
        }
    }
}

/// Error enum for issues #697-#700 (share allowances, keeper authorization,
/// extra reward tokens, reward epochs). `VaultError` is at Soroban's
/// 50-variant cap, so these cases get their own enum, matching
/// `VaultFeature2Error`..`VaultFeature5Error`.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultFeature6Error {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Mirrors `VaultError::ZeroAmount`.
    ZeroAmount = 3,
    /// Mirrors `VaultError::InsufficientShares`.
    InsufficientShares = 4,
    /// Mirrors `VaultError::VaultPaused`.
    VaultPaused = 5,
    /// Mirrors `VaultError::ArithmeticError`.
    ArithmeticError = 6,
    /// Mirrors `VaultError::WithdrawalLimitExceeded`.
    WithdrawalLimitExceeded = 7,
    /// Mirrors `VaultError::UseCooldownFlow`.
    UseCooldownFlow = 8,
    /// Mirrors `VaultError::BelowMinimumStake`.
    BelowMinimumStake = 9,
    /// Returned by `approve_withdrawal()` for a negative share amount (issue #697).
    InvalidAmount = 10,
    /// Returned by `approve_withdrawal()` when `expires_at` is not in the
    /// future (issue #697).
    InvalidExpiry = 11,
    /// Returned by `withdraw_from()` when the allowance has expired (issue #697).
    AllowanceExpired = 12,
    /// Returned by `withdraw_from()` when no allowance exists or it is smaller
    /// than the requested shares (issue #697).
    InsufficientAllowance = 13,
    /// Returned by `authorize_keeper()` when the user already has three
    /// authorized keepers (issue #698).
    KeeperLimitReached = 14,
    /// Returned by `claim_for()` / `compound_for()` when the keeper is not
    /// authorized by the user (issue #698).
    KeeperNotAuthorized = 15,
    /// Returned by `compound_for()` when the user has not enabled
    /// auto-compounding (issue #698).
    AutoCompoundNotEnabled = 16,
    /// Returned by `add_reward_token()` when three extra tokens are already
    /// configured (issue #699).
    RewardTokenLimitReached = 17,
    /// Returned by `add_reward_token()` for a token that is already configured
    /// (issue #699).
    RewardTokenAlreadyAdded = 18,
    /// Returned when the token is not a configured extra reward token (issue #699).
    RewardTokenNotFound = 19,
    /// Returned by `add_reward_token()` for a zero rate or one above
    /// `MAX_RATE_BPS` (issue #699).
    InvalidRate = 20,
    /// Returned by `set_epoch_config()` for a zero length or negative budget
    /// (issue #700).
    InvalidEpochConfig = 21,
}

impl From<VaultError> for VaultFeature6Error {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::NotInitialized => VaultFeature6Error::NotInitialized,
            VaultError::ZeroAmount => VaultFeature6Error::ZeroAmount,
            VaultError::InsufficientShares => VaultFeature6Error::InsufficientShares,
            VaultError::VaultPaused => VaultFeature6Error::VaultPaused,
            VaultError::ArithmeticError => VaultFeature6Error::ArithmeticError,
            VaultError::WithdrawalLimitExceeded => VaultFeature6Error::WithdrawalLimitExceeded,
            VaultError::UseCooldownFlow => VaultFeature6Error::UseCooldownFlow,
            VaultError::BelowMinimumStake => VaultFeature6Error::BelowMinimumStake,
            _ => VaultFeature6Error::Unauthorized,
        }
    }
}

/// Issue #621: negative-balance impossibility backstop.
///
/// `VaultError`, `VaultExtError`, and `VaultFeatureError` are all already at
/// Soroban's 50-variant cap for `#[contracterror]` enums (see the note on
/// `DataKey` in `storage.rs`), so this gets its own enum rather than adding a
/// variant to one of those, matching the established pattern for handling an
/// exhausted error enum (e.g. `VaultFeature2Error`..`VaultFeature5Error`).
///
/// Every balance-decrementing call site (withdraw, fee deduction, penalty,
/// slash, etc.) is expected to validate the requested amount against the
/// caller's current balance up front and return its own typed error (e.g.
/// `VaultError::InsufficientShares`) before ever reaching a `balance::set_*`
/// helper with a value that would go negative. `balance::assert_non_negative`
/// (see `balance.rs`) is the last line of defense in those setters: it fires
/// only if that upstream check was missing or wrong, aborting the transaction
/// and reverting all state changes so no negative balance is ever committed
/// to storage.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultInvariantError {
    /// A `balance::set_*` accounting field would have been written as
    /// negative; the write was rejected and the call reverted instead.
    NegativeBalance = 1,
}

/// Thirteenth error enum, for issues #705-#708 (withdraw-only mode, position
/// split/merge, beneficiary inheritance, dust cleanup). Every earlier
/// `#[contracterror]` enum is at Soroban's 50-variant cap or scoped to another
/// feature batch, so these cases get their own enum, plus mirrors of the
/// `VaultError` cases the new entrypoints can also hit (via the `From` impl
/// below, so `?` still works at call sites).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultSafetyError {
    /// Mirrors `VaultError::Unauthorized`.
    Unauthorized = 1,
    /// Mirrors `VaultError::NotInitialized`.
    NotInitialized = 2,
    /// Returned when a ledger count or amount that must be positive is not.
    ZeroAmount = 3,
    /// Mirrors `VaultError::ArithmeticError`.
    ArithmeticError = 4,
    /// Mirrors `VaultError::PositionNotFound`; also returned for an unknown
    /// position id (issue #706).
    PositionNotFound = 5,
    /// Mirrors `VaultError::VaultPaused`.
    VaultPaused = 6,
    /// Mirrors `VaultError::MaxPositionsReached` (issue #706).
    MaxPositionsReached = 7,
    /// Returned by `split_position()` when `shares` is not strictly between 0
    /// and the size of the position being split (issue #706).
    InvalidSplitAmount = 8,
    /// Returned by `merge_positions()` when the two ids are equal or the
    /// positions carry different lock/boost parameters (issue #706).
    IncompatiblePositions = 9,
    /// Returned by `set_beneficiary()` when the beneficiary is the owner
    /// (issue #707).
    InvalidBeneficiary = 10,
    /// Returned by `clear_beneficiary()` / `claim_inheritance()` when no
    /// beneficiary is set (issue #707).
    NoBeneficiary = 11,
    /// Returned by `claim_inheritance()` before `last_interaction +
    /// inactivity_ledgers` has been reached (issue #707).
    InactivityPeriodNotElapsed = 12,
    /// Mirrors `VaultError::RecipientAlreadyStaking` (issue #707).
    RecipientAlreadyStaking = 13,
    /// Returned by `close_dust_position()` when no dust threshold is set
    /// (issue #708).
    DustThresholdNotSet = 14,
    /// Returned by `close_dust_position()` when the position is at or above
    /// the dust threshold (issue #708).
    NotDustPosition = 15,
    /// Returned by `close_dust_position()` while the position is locked
    /// (issue #708).
    PositionLocked = 16,
    /// Returned by `set_dust_keeper_tip_bps()` above the 1 000 bps cap
    /// (issue #708).
    InvalidTipBps = 17,
    /// Returned by `close_dust_position()` while the position has escrowed
    /// rewards pending release, which closing would forfeit (issue #708).
    EscrowPending = 18,
}

impl From<VaultError> for VaultSafetyError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => VaultSafetyError::Unauthorized,
            VaultError::NotInitialized => VaultSafetyError::NotInitialized,
            VaultError::ArithmeticError => VaultSafetyError::ArithmeticError,
            VaultError::PositionNotFound => VaultSafetyError::PositionNotFound,
            VaultError::VaultPaused => VaultSafetyError::VaultPaused,
            VaultError::MaxPositionsReached => VaultSafetyError::MaxPositionsReached,
            VaultError::RecipientAlreadyStaking => VaultSafetyError::RecipientAlreadyStaking,
            _ => VaultSafetyError::Unauthorized,
        }
    }
}

/// Errors for proposal categories, expedited proposals, vote delegation expiry,
/// and strategy adapter management (issues #761-#764).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GovernanceStrategyError {
    Unauthorized = 1,
    NotInitialized = 2,
    ArithmeticError = 3,
    ZeroAmount = 4,
    InvalidBps = 5,
    InvalidPeriod = 6,
    ProposalNotFound = 7,
    ProposalAlreadyEnacted = 8,
    VotingStillActive = 9,
    QuorumNotMet = 10,
    ThresholdNotMet = 11,
    ProposalVetoed = 12,
    ExpeditedLimitReached = 13,
    InsufficientBond = 14,
    DelegationExpired = 15,
    SelfDelegationNotAllowed = 16,
    MaxStrategiesReached = 17,
    StrategyAlreadyExists = 18,
    StrategyNotFound = 19,
    StrategyCapExceeded = 20,
    AdapterCallFailed = 21,
    PositionNotFound = 22,
    AlreadyVoted = 23,
    DelayNotElapsed = 24,
}

impl From<VaultError> for GovernanceStrategyError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::Unauthorized => GovernanceStrategyError::Unauthorized,
            VaultError::NotInitialized => GovernanceStrategyError::NotInitialized,
            VaultError::ArithmeticError => GovernanceStrategyError::ArithmeticError,
            VaultError::ZeroAmount => GovernanceStrategyError::ZeroAmount,
            VaultError::PositionNotFound => GovernanceStrategyError::PositionNotFound,
            _ => GovernanceStrategyError::Unauthorized,
        }
    }
}

