//! Share-price conversion math.
//!
//! Shares use a fixed seven-decimal precision regardless of the underlying
//! deposit token's precision. On the first deposit the token amount is scaled
//! into that precision; afterwards shares are minted and burned at the current
//! `total_shares / total_deposited` ratio.
//!
//! These helpers were lost when `balance.rs` was split into submodules in
//! commit `2c268e4`; restoring them here keeps the public re-exports in
//! `lib.rs` compiling.

use super::rewards::DEFAULT_TOKEN_DECIMALS;

/// Convert a deposit amount to shares using the current vault ratio.
///
/// The first deposit scales the token amount into the vault's fixed share
/// precision; subsequent deposits are proportional to the existing pool.
pub fn amount_to_shares(total_shares: i128, total_deposited: i128, amount: i128) -> Option<i128> {
    amount_to_shares_with_decimals(total_shares, total_deposited, amount, DEFAULT_TOKEN_DECIMALS)
}

/// Convert token base units to shares using an explicit token precision.
///
/// `token_decimals` is the precision queried from the configured token. It is
/// only used for the first deposit, when there is no existing ratio to use.
pub fn amount_to_shares_with_decimals(
    total_shares: i128,
    total_deposited: i128,
    amount: i128,
    token_decimals: u32,
) -> Option<i128> {
    if total_shares == 0 || total_deposited == 0 {
        scale_amount(amount, token_decimals, DEFAULT_TOKEN_DECIMALS)
    } else {
        amount.checked_mul(total_shares)?.checked_div(total_deposited)
    }
}

/// Convert shares to the underlying token amount at the current ratio.
pub fn shares_to_amount(total_shares: i128, total_deposited: i128, shares: i128) -> Option<i128> {
    shares_to_amount_with_decimals(total_shares, total_deposited, shares, DEFAULT_TOKEN_DECIMALS)
}

/// Convert shares in fixed seven-decimal precision to token base units.
///
/// The ratio already includes the initial decimal scaling, so
/// `token_decimals` is accepted for API symmetry but does not alter the
/// calculation.
pub fn shares_to_amount_with_decimals(
    total_shares: i128,
    total_deposited: i128,
    shares: i128,
    _token_decimals: u32,
) -> Option<i128> {
    if total_shares == 0 {
        Some(0)
    } else {
        shares.checked_mul(total_deposited)?.checked_div(total_shares)
    }
}

fn scale_amount(amount: i128, from_decimals: u32, to_decimals: u32) -> Option<i128> {
    if from_decimals == to_decimals {
        return Some(amount);
    }

    if from_decimals < to_decimals {
        amount.checked_mul(10_i128.checked_pow(to_decimals - from_decimals)?)
    } else {
        amount.checked_div(10_i128.checked_pow(from_decimals - to_decimals)?)
    }
}
