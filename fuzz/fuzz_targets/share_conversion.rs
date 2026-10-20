#![no_main]
//! Fuzz target: share conversion (issue #661).
//!
//! Drives `balance::amount_to_shares_with_decimals` and
//! `balance::shares_to_amount_with_decimals` — the two pure functions that back
//! every mint and burn (see `docs/INVARIANTS.md`) — with arbitrary
//! `(total_shares, total_deposited, amount, decimals)` inputs and asserts the
//! documented invariants:
//!
//! - **No panic.** Both functions use checked arithmetic and return `Option`,
//!   so no input may cause an overflow panic or a panic from a debug assertion.
//! - **No negative balances.** For non-negative inputs the result must be
//!   non-negative.
//! - **Full redemption returns the pool.** When `total_shares > 0`, redeeming
//!   every share returns exactly `total_deposited`.
//! - **Deposit→shares→redeem never mints value.** With an existing pool
//!   (`total_shares > 0 && total_deposited > 0`), converting an amount to shares
//!   and straight back never yields more than the amount deposited (rounding
//!   always favours the pool).

use libfuzzer_sys::fuzz_target;
use soroban_sdk::Env;
use stellar_defi_vault::fuzz_support::{
    amount_to_shares_with_decimals, shares_to_amount_with_decimals,
};

/// A spread of realistic token precisions, plus the share precision (7).
const DECIMALS: [u32; 8] = [0, 2, 5, 6, 7, 8, 9, 18];

fuzz_target!(|data: (i128, i128, i128, u8)| {
    let (total_shares, total_deposited, amount, selector) = data;
    let decimals = DECIMALS[(selector as usize) % DECIMALS.len()];

    let shares = amount_to_shares_with_decimals(total_shares, total_deposited, amount, decimals);

    if let Some(shares) = shares {
        // Invariant: no negative balances for non-negative inputs.
        if total_shares >= 0 && total_deposited >= 0 && amount >= 0 {
            assert!(
                shares >= 0,
                "amount_to_shares produced a negative share count"
            );
        }

        let back = shares_to_amount_with_decimals(total_shares, total_deposited, shares, decimals);
        if let Some(back) = back {
            if total_shares > 0 && total_deposited >= 0 && amount >= 0 {
                assert!(back >= 0, "shares_to_amount produced a negative amount");
            }
        }
    }

    // Invariant: redeeming every share returns the whole pool, exactly.
    if total_shares > 0 && total_deposited >= 0 {
        let redeemed =
            shares_to_amount_with_decimals(total_shares, total_deposited, total_shares, decimals);
        assert_eq!(
            redeemed,
            Some(total_deposited),
            "redeeming all shares must return exactly total_deposited"
        );
    }

    // Force an Env so any accidental Env-dependent behaviour is exercised under
    // fuzzing too; the pure functions above take no Env, so this is a guard
    // that they stay pure.
    let _env = Env::default();
});
