//! Optional quadratic vote weighting (governance feature).
//!
//! When enabled, each voter's weight is the **integer square root** of their
//! staked token amount rather than the raw amount.  This reduces whale
//! dominance: a 10 000-token holder gets weight 100 instead of 10 000, while a
//! 100-token holder gets weight 10 — a 100× stake advantage collapses to 10×.
//!
//! # Mode lifecycle
//!
//! The pool-wide mode (`Linear` | `Quadratic`) is stored in instance storage
//! and read by every `vote()` and `batch_vote()` call.  Crucially, **the mode
//! is snapshotted into each `GovernanceProposal` at creation time** (see
//! `create_proposal` in vault.rs).  A subsequent admin mode-change therefore
//! never alters the weighting rules for any already-open proposal — voters
//! always operate under the rules that were advertised when they chose to
//! engage with a given proposal.
//!
//! # `isqrt` — integer square root
//!
//! We use the standard Newton–Raphson / Babylonian integer square root.
//!
//! **Rounding:** always rounds *down* (floor).  For a staked amount `n`:
//! - `isqrt(0) = 0`
//! - `isqrt(1) = 1`
//! - `isqrt(3) = 1`   (floor of 1.73)
//! - `isqrt(4) = 2`
//! - `isqrt(99) = 9`  (floor of 9.94)
//! - `isqrt(100) = 10`
//!
//! Negative inputs (impossible for a staked amount but theoretically reachable
//! from a delegated-weight accumulator) return `0`.
//!
//! # Sybil-resistance note
//!
//! Quadratic weighting is trivially bypassed by splitting stake across multiple
//! wallets unless a sybil-resistance / identity layer is in place (e.g. the
//! existing KYC whitelist or proof-of-humanity hook).  Document this prominently
//! in pool UIs when enabling quadratic mode.

use soroban_sdk::{contractimpl, symbol_short, Address, Env};

use crate::admin;
use crate::errors::VaultError;
use crate::storage::VoteWeightMode;
use crate::vault::VaultContract;

// ── Integer square root ───────────────────────────────────────────────────────

/// Integer square root of `n`, rounded down (floor).
///
/// Uses the Babylonian / Newton–Raphson iteration which converges in
/// O(log n) steps.  Safe for all non-negative `i128` values up to
/// `i128::MAX` (the result fits in a `u64` since sqrt(2^127) < 2^64).
///
/// Returns `0` for any `n ≤ 0`.
pub fn isqrt(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    if n == 1 {
        return 1;
    }
    // Initial estimate: shift right by half the bit-length.
    let mut x: i128 = n;
    let mut y: i128 = (x + 1) >> 1;
    while y < x {
        x = y;
        y = (x + n / x) >> 1;
    }
    // x is now floor(sqrt(n)).
    // Postcondition: x*x <= n < (x+1)*(x+1)
    x
}

// ── Contract entrypoints ──────────────────────────────────────────────────────

#[contractimpl]
impl VaultContract {
    /// Admin: set the pool-wide vote weight mode.
    ///
    /// `Linear` (default) — weight = staked token amount.
    /// `Quadratic`        — weight = floor(sqrt(staked token amount)).
    ///
    /// The change takes effect for proposals created **after** this call.
    /// Open proposals are unaffected: each proposal stores the mode that was
    /// active when it was created.
    ///
    /// # Authorization
    /// Requires admin.
    ///
    /// # Events
    /// Emits `vote_weight_mode_changed`.
    pub fn set_vote_weight_mode(
        env: Env,
        admin: Address,
        mode: VoteWeightMode,
    ) -> Result<(), VaultError> {
        admin::require_admin_as(&env, &admin)?;
        crate::balance::set_vote_weight_mode(&env, mode);
        crate::events::vote_weight_mode_changed(&env, &admin, mode, env.ledger().sequence());
        Ok(())
    }

    /// Read-only: the current pool-wide vote weight mode.
    pub fn get_vote_weight_mode(env: Env) -> VoteWeightMode {
        crate::balance::get_vote_weight_mode(&env)
    }
}
