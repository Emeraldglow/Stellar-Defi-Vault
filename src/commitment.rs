//! Commitâ€“reveal stake commitments (issue #288).
//!
//! Lets a staker commit to an amount without revealing it, then reveal and
//! stake later. A large stake intention is no longer telegraphed in the
//! mempool, which removes the front-running window and gives whale stakers a
//! privacy layer.
//!
//! The commitment is `SHA256(amount_bytes || salt)`. The salt is what makes it
//! meaningful: without one, the space of plausible stake amounts is small
//! enough to brute-force straight off the hash.
//!
//! # What this does and does not hide
//!
//! It hides the amount *between commit and reveal*. It is not zero-knowledge:
//! the reveal publishes the amount, and the resulting stake is as visible as
//! any other. The `commitment_revealed` event deliberately carries no amount,
//! but the balance change it accompanies is public regardless.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Bytes, Env, Symbol};

use crate::admin;
use crate::errors::VaultError;
use crate::VaultContract;
use crate::vault::VaultContractClient;

/// Default window, in ledgers, within which a commitment must be revealed.
///
/// Roughly a day at ~5s ledgers. Long enough to survive a stuck transaction,
/// short enough that an abandoned commitment does not lock the slot for long.
pub const DEFAULT_COMMITMENT_WINDOW: u32 = 17_280;

/// Instance-storage key for the configured reveal window.
const WINDOW_KEY: Symbol = symbol_short!("cmt_wnd");

/// Persistent-storage key prefix for a user's outstanding commitment.
const COMMITMENT_KEY: Symbol = symbol_short!("cmt_rec");

/// An outstanding commitâ€“reveal record.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CommitmentRecord {
    pub hash: Bytes,
    pub committed_at: u32,
    pub revealed: bool,
}

/// The configured reveal window, falling back to the default.
pub fn get_window(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&WINDOW_KEY)
        .unwrap_or(DEFAULT_COMMITMENT_WINDOW)
}

/// A user's outstanding commitment, if any.
pub fn commitment_for(env: &Env, user: &Address) -> Option<CommitmentRecord> {
    env.storage()
        .persistent()
        .get(&(COMMITMENT_KEY, user.clone()))
}

/// Whether a record is past its reveal window and may be replaced.
fn is_expired(env: &Env, record: &CommitmentRecord) -> bool {
    env.ledger().sequence() > record.committed_at.saturating_add(get_window(env))
}

/// The preimage a commitment hashes: the amount's big-endian bytes, then salt.
///
/// Fixed-width, big-endian amount encoding matters. A variable-length encoding
/// would let two different `(amount, salt)` pairs produce the same byte
/// sequence by shifting bytes across the boundary, which would let a committer
/// reveal an amount they never committed to.
fn preimage(env: &Env, amount: i128, salt: &Bytes) -> Bytes {
    let mut buffer = Bytes::new(env);
    for byte in amount.to_be_bytes().iter() {
        buffer.push_back(*byte);
    }
    buffer.append(salt);
    buffer
}

/// Compute the commitment hash for an `(amount, salt)` pair.
///
/// Exposed so a caller can build the commitment off-chain with exactly the
/// encoding the contract will verify against, rather than reimplementing it
/// and discovering the mismatch at reveal time.
pub fn compute_hash(env: &Env, amount: i128, salt: &Bytes) -> Bytes {
    env.crypto().sha256(&preimage(env, amount, salt)).into()
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Set the reveal window, in ledgers. Admin only.
    pub fn set_commitment_window(env: Env, ledgers: u32) -> Result<(), VaultError> {
        admin::require_admin(&env)?;

        if ledgers == 0 {
            // A zero window would expire every commitment in the same ledger
            // it was made, making reveal impossible.
            return Err(VaultError::InvalidRate);
        }

        env.storage().instance().set(&WINDOW_KEY, &ledgers);

        let admin = admin::get_admin(&env)?;
        env.events().publish(
            (symbol_short!("cmt_wset"), admin),
            (ledgers, env.ledger().sequence()),
        );
        Ok(())
    }

    /// The configured reveal window, in ledgers.
    pub fn get_commitment_window(env: Env) -> u32 {
        get_window(&env)
    }

    /// Commit to staking an amount, revealing only its hash.
    ///
    /// One commitment per user at a time: while an unrevealed, unexpired
    /// commitment stands, a second call is rejected. Allowing several would let
    /// a committer prepare a range of amounts and reveal whichever suits them
    /// once the market moved â€” exactly the optionality commitâ€“reveal exists to
    /// remove.
    pub fn commit_to_stake(
        env: Env,
        user: Address,
        commitment_hash: Bytes,
    ) -> Result<(), VaultError> {
        user.require_auth();

        if commitment_hash.len() != 32 {
            // A SHA-256 digest is 32 bytes; anything else can never match a
            // reveal, so reject it now rather than locking the slot with a
            // commitment that is impossible to satisfy.
            return Err(VaultError::InvalidAddress);
        }

        if let Some(existing) = commitment_for(&env, &user) {
            if !existing.revealed && !is_expired(&env, &existing) {
                return Err(VaultError::MaxPositionsReached);
            }
        }

        let record = CommitmentRecord {
            hash: commitment_hash,
            committed_at: env.ledger().sequence(),
            revealed: false,
        };
        env.storage()
            .persistent()
            .set(&(COMMITMENT_KEY, user.clone()), &record);

        // No amount, and no hash: the event marks that a commitment exists.
        env.events()
            .publish((symbol_short!("cmt_made"), user), record.committed_at);
        Ok(())
    }

    /// A user's outstanding commitment, if any.
    pub fn get_commitment(env: Env, user: Address) -> Option<CommitmentRecord> {
        commitment_for(&env, &user)
    }

    /// Reveal a commitment and stake the committed amount.
    ///
    /// Verifies `hash == SHA256(amount || salt)` before staking. On success the
    /// commitment is consumed, so the same reveal cannot be replayed.
    ///
    /// Note this does **not** perform the stake itself â€” it validates and
    /// clears the commitment, then defers to the contract's own `stake`
    /// entrypoint so that every staking rule (minimum, pause, whitelist,
    /// position cap) applies exactly as it does to a direct stake. Duplicating
    /// that logic here is how the two paths would drift apart.
    pub fn reveal_and_stake(
        env: Env,
        user: Address,
        amount: i128,
        salt: Bytes,
    ) -> Result<(), VaultError> {
        user.require_auth();

        let record = commitment_for(&env, &user).ok_or(VaultError::NotInitialized)?;

        if record.revealed {
            return Err(VaultError::NothingToWithdraw);
        }
        if is_expired(&env, &record) {
            return Err(VaultError::EpochNotFinalized);
        }

        let expected = compute_hash(&env, amount, &salt);
        if expected != record.hash {
            return Err(VaultError::InvalidAddress);
        }

        // Consume the commitment before staking. If the stake then fails the
        // whole transaction reverts, so this cannot leave a consumed
        // commitment with no stake behind it.
        env.storage()
            .persistent()
            .remove(&(COMMITMENT_KEY, user.clone()));

        // Deliberately amount-free, so an observer learns that a reveal
        // happened but not what was staked from this event alone.
        env.events().publish(
            (symbol_short!("cmt_rvl"), user.clone()),
            env.ledger().sequence(),
        );

        // `stake` returns the shares minted; the reveal path has no use for
        // that figure, so it is discarded rather than widening this signature.
        Self::stake(env, user, amount).map(|_shares| ())
    }
}















