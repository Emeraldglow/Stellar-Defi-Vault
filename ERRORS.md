# Error Codes

All contract entrypoints revert with typed `#[contracterror]` enums — never
with raw string `panic!` messages — so callers and integrators can match on
stable numeric codes instead of parsing strings. Each enum is `#[repr(u32)]`;
the numeric value on the wire is the discriminant listed below.

> Soroban caps every `#[contracterror]` enum at 50 variants
> (`ScSpecUdtUnionV0::cases` is a `VecM<_, 50>` in stellar-xdr), so the codes
> are split across several enums. Codes are stable within an enum; never
> renumber an existing variant.

## `VaultError` — core vault (src/errors.rs)

| Code | Variant | Returned when |
|---:|---|---|
| 1 | `NotInitialized` | Entrypoint requires `initialize()` first |
| 2 | `AlreadyInitialized` | `initialize()` called twice; proposal already enacted |
| 3 | `Unauthorized` | Caller is not the stored admin |
| 4 | `ZeroAmount` | Zero/negative amount where not allowed |
| 5 | `InsufficientShares` | Burning more shares than owned |
| 6 | `VaultPaused` | Pool is paused |
| 7 | `InvalidTokenInterface` | Provided token does not implement SEP-41 token interface (decimals/balance/transfer) |
| 8 | `ArithmeticError` | Checked arithmetic / share conversion failed |
| 9 | `WithdrawalLimitExceeded` | Share amount over per-transaction limit |
| 10 | `InvalidPenaltyBps` | Early-exit penalty above cap |
| 11 | `BelowMinimumStake` | Resulting position below minimum stake |
| 12 | `TooManyBoostTiers` | More than five boost tiers |
| 13 | `InvalidBoostSchedule` | Bad tier multiplier / non-increasing ledgers |
| 14 | `InsufficientRewardPool` | Reward pool cannot cover the claim |
| 15 | `NotADelegate` | Wrong / unapproved delegate |
| 16 | `CannotRescueStakeToken` | `rescue_token()` on the stake token |
| 17 | `CannotRescueRewardToken` | `rescue_token()` on the reward token |
| 18 | `PositionNotFound` | No active stake / unbonding / proposal |
| 19 | `NotWhitelisted` | Whitelist enforcement rejects the caller |
| 20 | `UseCooldownFlow` | Cooldown-gated withdrawal path required |
| 21 | `UnstakeFeeTooHigh` | Unstake fee above 500 bps |
| 22 | `BatchTooLarge` | More than 20 addresses in a batch query |
| 23 | `TooManyStakers` | Already voted on the proposal |
| 24 | `RecipientAlreadyStaking` | Transfer target already has a position |
| 25 | `CampaignAlreadyActive` | Boost campaign already running |
| 26 | `NoCampaignActive` | No boost campaign to end |
| 27 | `DepositorCapReached` | Unique-depositor cap reached |
| 28 | `PageSizeTooLarge` | `page_size` 0 or above 20 |
| 29 | `KycNotApproved` | KYC enforcement rejects the staker |
| 30 | `ContractStopped` | `emergency_stop()` was called |
| 31 | `PoolCapReached` | Pool cap / yield-liquidity buffer exceeded |
| 32 | `DescriptionTooLong` | Pool description over 200 chars |
| 33 | `NonMonotonicWaveId` | Wave id not greater than the last one |
| 34 | `TooManyActiveUsers` | Over 50 active users in one call |
| 35 | `InvalidAddress` | Admin/token address invalid (e.g. self) |
| 36 | `RateTooHigh` | Reward APR above the configured cap |
| 37 | `MaxPositionsReached` | Per-user position cap / proposal cap hit |
| 38 | `MaxPositionsTooHigh` | Requested cap above 10 |
| 39 | `BatchKycTooLarge` | Voting ended / proposal enacted or vetoed |
| 40 | `InvalidRate` | Dynamic-fee config out of range |
| 41 | `MessageTooLong` | Custom message over 150 chars |
| 42 | `EpochModeConflict` | Wrong epoch mode for the entrypoint |
| 43 | `VestingQueueFull` | Vesting queue at max entries |
| 44 | `NothingToWithdraw` | No matured vesting to withdraw |
| 45 | `EpochNotFinalized` | Epoch window not elapsed / voting not ended |
| 46 | `RelayerNotApproved` | Caller not an approved relayer |
| 47 | `NotYieldSource` | Caller not on the yield-source whitelist |
| 48 | `InvalidRewardAmount` | Zero/negative reward amount |
| 49 | `PoolShuttingDown` | New stake after `start_graceful_shutdown` |
| 50 | `NotInEpochMode` | Pool not configured for epoch mode |

## `VaultFeature5Error` — issues #498–#505 panic replacement (issue #593)

These variants replace the last ad-hoc string `panic!`s in the codebase. The
variant names deliberately match the legacy panic strings so existing
integrators can map them one-to-one.

| Code | Variant | Entrypoint | Replaces legacy panic |
|---:|---|---|---|
| 1 | `Unauthorized` | any admin-gated call | — (mirrors `VaultError::Unauthorized`) |
| 2 | `NotInitialized` | any call before `initialize()` | — (mirrors `VaultError::NotInitialized`) |
| 3 | `InvalidSplitRecipients` | `set_treasury_split()` — >3 recipients or `recipients`/`bps_shares` length mismatch | `panic!("InvalidSplitRecipients")` |
| 4 | `InvalidSplitBpsSum` | `set_treasury_split()` — bps shares don't sum to 10 000 | `panic!("InvalidSplitBpsSum")` |
| 5 | `UnregisteredToken` | `swap_secondary_reward()` — no DEX router for the source token | `panic!("UnregisteredToken")` |
| 6 | `TimelockNotExpired` | `execute_admin_action()` — `executable_at` not reached | `panic!("TimelockNotExpired")` |
| 7 | `ActionNotFound` | `execute_admin_action()` / `cancel_admin_action()` — unknown action id | `panic!("ActionNotFound")` |
| 8 | `AutoCompoundNotEnabled` | `compound()` — user has not opted in | `panic!("AutoCompoundNotEnabled")` |
| 9 | `NoSharesToTokenize` | `tokenize_position()` — caller holds no shares | `panic!("NoSharesToTokenize")` |
| 10 | `NotNftOwner` | `redeem_position_nft()` — caller does not own the token id | `panic!("NotNFTOwner")` |

## Other enums

`VaultExtError`, `VaultFeatureError`, `VaultOverflowError`, `VaultQuizError`,
`VaultCampaignError`, `VaultOpsError`, `VaultFeature2Error`,
`VaultFeature3Error`, `VaultFeature4Error`, and `VaultAccessError` follow the
same convention: codes start at 1, are stable, and are documented with doc
comments next to each variant in `src/errors.rs`. The first variants of each
overflow enum mirror the matching `VaultError` codes, and `From<VaultError>`
impls let internal helpers propagate errors without losing the code.
