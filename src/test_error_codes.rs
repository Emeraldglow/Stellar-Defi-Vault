//! Stable error-code snapshot test for `VaultError`.
//!
//! # Policy: error codes are never reused
//!
//! `VaultError`'s numeric discriminants are part of this contract's public
//! ABI: a failed call surfaces this number (not the Rust variant name,
//! which never crosses the Wasm boundary) to every caller — the Soroban
//! CLI, generated SDK bindings, and any off-chain indexer matching on
//! error codes. Once a number has been assigned to a shipped variant:
//!
//! - it must never be reassigned to a different variant or meaning,
//! - the variant must never be renumbered,
//! - deleting the variant does **not** free its number for reuse by
//!   anything else, ever.
//!
//! The one documented exception already in this codebase: `errors.rs`
//! notes that `DepositorCapReached = 27` reuses the slot of a
//! `LeaderboardSizeTooLarge` case that was planned but never actually
//! implemented or shipped — no released build ever returned `27` for
//! that other meaning, so nothing is being reassigned out from under a
//! caller. That is not a precedent for reusing a number that *has*
//! shipped under its current meaning.
//!
//! New error cases get the next unused number. `VaultError` is already at
//! Soroban's 50-variant cap (`ScSpecUdtUnionV0::cases` is a
//! `VecM<_, 50>` in stellar-xdr — see the comment above `VaultExtError`
//! in `errors.rs`), so from here on, new cases belong in one of the
//! satellite enums instead (`VaultExtError`, `VaultFeatureError`, ...),
//! never in `VaultError` itself.
//!
//! # What this test enforces
//!
//! `vault_error_codes_match_snapshot` below is a table of every current
//! variant against its frozen numeric code, written as a literal rather
//! than derived from the variant itself — so renumbering an existing
//! variant changes `variant as u32` but not the literal next to it, and
//! the assertion fails. Deleting an existing variant is caught even
//! earlier, at compile time: the row referencing it stops compiling.
//!
//! `_every_variant_has_a_snapshot_row` is a wildcard-free exhaustive
//! match whose only purpose is to make the compiler refuse to build this
//! file if `VaultError` ever gains a new variant that isn't also added
//! here — that is what actually enforces "adding a new variant requires
//! appending to the snapshot," rather than relying on a contributor
//! remembering to update a count by hand.
//!
//! **Extending this file:** add the new variant's row to `CODES` with the
//! next unused number, and add the variant to the match in
//! `_every_variant_has_a_snapshot_row`. Never edit or remove an existing
//! row or match arm.

use crate::errors::VaultError;

/// `(variant, frozen numeric code)` for every `VaultError` variant that
/// currently exists, in the order declared in `errors.rs`. Append new
/// rows to the end; never edit or remove an existing one.
const CODES: &[(VaultError, u32)] = &[
    (VaultError::NotInitialized, 1),
    (VaultError::AlreadyInitialized, 2),
    (VaultError::Unauthorized, 3),
    (VaultError::ZeroAmount, 4),
    (VaultError::InsufficientShares, 5),
    (VaultError::VaultPaused, 6),
    (VaultError::InvalidTokenInterface, 7),
    (VaultError::ArithmeticError, 8),
    (VaultError::WithdrawalLimitExceeded, 9),
    (VaultError::InvalidPenaltyBps, 10),
    (VaultError::BelowMinimumStake, 11),
    (VaultError::TooManyBoostTiers, 12),
    (VaultError::InvalidBoostSchedule, 13),
    (VaultError::InsufficientRewardPool, 14),
    (VaultError::NotADelegate, 15),
    (VaultError::CannotRescueStakeToken, 16),
    (VaultError::CannotRescueRewardToken, 17),
    (VaultError::PositionNotFound, 18),
    (VaultError::NotWhitelisted, 19),
    (VaultError::UseCooldownFlow, 20),
    (VaultError::UnstakeFeeTooHigh, 21),
    (VaultError::BatchTooLarge, 22),
    (VaultError::TooManyStakers, 23),
    (VaultError::RecipientAlreadyStaking, 24),
    (VaultError::CampaignAlreadyActive, 25),
    (VaultError::NoCampaignActive, 26),
    (VaultError::DepositorCapReached, 27),
    (VaultError::PageSizeTooLarge, 28),
    (VaultError::KycNotApproved, 29),
    (VaultError::ContractStopped, 30),
    (VaultError::PoolCapReached, 31),
    (VaultError::DescriptionTooLong, 32),
    (VaultError::NonMonotonicWaveId, 33),
    (VaultError::TooManyActiveUsers, 34),
    (VaultError::InvalidAddress, 35),
    (VaultError::RateTooHigh, 36),
    (VaultError::MaxPositionsReached, 37),
    (VaultError::MaxPositionsTooHigh, 38),
    (VaultError::BatchKycTooLarge, 39),
    (VaultError::InvalidRate, 40),
    (VaultError::MessageTooLong, 41),
    (VaultError::EpochModeConflict, 42),
    (VaultError::VestingQueueFull, 43),
    (VaultError::NothingToWithdraw, 44),
    (VaultError::EpochNotFinalized, 45),
    (VaultError::RelayerNotApproved, 46),
    (VaultError::NotYieldSource, 47),
    (VaultError::InvalidRewardAmount, 48),
    (VaultError::PoolShuttingDown, 49),
    (VaultError::NoAdmin, 50),
];

/// Table-driven snapshot check: every variant's live `as u32` value must
/// still equal the frozen literal recorded for it above.
#[test]
fn vault_error_codes_match_snapshot() {
    for (variant, expected_code) in CODES {
        assert_eq!(
            *variant as u32,
            *expected_code,
            "VaultError::{variant:?} must keep numeric code {expected_code} (got {}). \
             Error codes are part of the public ABI and are never renumbered or reused \
             once shipped — see this file's module docs.",
            *variant as u32,
        );
    }
}

/// Cross-check that `CODES` hasn't silently fallen behind the enum. The
/// actual enforcement is `_every_variant_has_a_snapshot_row`'s exhaustive
/// match below, which won't compile if it has — this just turns that
/// compile-time fact into a readable assertion message too.
#[test]
fn vault_error_snapshot_covers_every_known_variant() {
    assert_eq!(
        CODES.len(),
        50,
        "VaultError currently has 50 variants (it is at Soroban's 50-variant cap); \
         this snapshot covers {} of them. If you added a new variant, append a row \
         to CODES with the next unused number.",
        CODES.len(),
    );
    // Using the guard here (rather than leaving it uncalled) keeps it from
    // being flagged as dead code, without needing an `#[allow]`.
    _every_variant_has_a_snapshot_row(VaultError::NoAdmin);
}

/// Wildcard-free exhaustive match over every `VaultError` variant. This
/// does not run any logic — its only job is to fail to *compile* the
/// moment `VaultError` gains a variant that isn't listed here, which is
/// what actually enforces "adding a new variant requires appending to the
/// snapshot" (see `CODES` above, which must also gain a row for it).
fn _every_variant_has_a_snapshot_row(err: VaultError) {
    match err {
        VaultError::NotInitialized
        | VaultError::AlreadyInitialized
        | VaultError::Unauthorized
        | VaultError::ZeroAmount
        | VaultError::InsufficientShares
        | VaultError::VaultPaused
        | VaultError::InvalidTokenInterface
        | VaultError::ArithmeticError
        | VaultError::WithdrawalLimitExceeded
        | VaultError::InvalidPenaltyBps
        | VaultError::BelowMinimumStake
        | VaultError::TooManyBoostTiers
        | VaultError::InvalidBoostSchedule
        | VaultError::InsufficientRewardPool
        | VaultError::NotADelegate
        | VaultError::CannotRescueStakeToken
        | VaultError::CannotRescueRewardToken
        | VaultError::PositionNotFound
        | VaultError::NotWhitelisted
        | VaultError::UseCooldownFlow
        | VaultError::UnstakeFeeTooHigh
        | VaultError::BatchTooLarge
        | VaultError::TooManyStakers
        | VaultError::RecipientAlreadyStaking
        | VaultError::CampaignAlreadyActive
        | VaultError::NoCampaignActive
        | VaultError::DepositorCapReached
        | VaultError::PageSizeTooLarge
        | VaultError::KycNotApproved
        | VaultError::ContractStopped
        | VaultError::PoolCapReached
        | VaultError::DescriptionTooLong
        | VaultError::NonMonotonicWaveId
        | VaultError::TooManyActiveUsers
        | VaultError::InvalidAddress
        | VaultError::RateTooHigh
        | VaultError::MaxPositionsReached
        | VaultError::MaxPositionsTooHigh
        | VaultError::BatchKycTooLarge
        | VaultError::InvalidRate
        | VaultError::MessageTooLong
        | VaultError::EpochModeConflict
        | VaultError::VestingQueueFull
        | VaultError::NothingToWithdraw
        | VaultError::EpochNotFinalized
        | VaultError::RelayerNotApproved
        | VaultError::NotYieldSource
        | VaultError::InvalidRewardAmount
        | VaultError::PoolShuttingDown
        | VaultError::NoAdmin => {}
    }
}
