//! Issue #512: fee-on-transfer / rebasing token safety.
//!
//! The vault used to credit the *stated* transfer amount on every inbound
//! transfer. With a fee-on-transfer or rebasing token the vault would receive
//! less than that, inflating `total_deposited` and diluting every staker's
//! shares. `pull_tokens` measures the vault's balance before and after the
//! transfer and returns what actually arrived, which callers credit instead.

use soroban_sdk::{symbol_short, token, Address, Env};

use crate::errors::VaultError;

/// Transfers `amount` of `token_addr` from `from` into the vault and returns
/// the amount the vault actually received (`balance_after - balance_before`).
///
/// Emits `short_rcv` (`from`) -> `(stated, received)` when the token delivered
/// less than requested. Reverts with `ZeroAmount` when nothing arrived.
pub(crate) fn pull_tokens(
    env: &Env,
    token_addr: &Address,
    from: &Address,
    amount: i128,
) -> Result<i128, VaultError> {
    let client = token::Client::new(env, token_addr);
    let vault = env.current_contract_address();

    let before = client.balance(&vault);
    client.transfer(from, &vault, &amount);
    let after = client.balance(&vault);

    let received = after.checked_sub(before).ok_or(VaultError::ArithmeticError)?;
    if received <= 0 {
        return Err(VaultError::ZeroAmount);
    }
    if received < amount {
        env.events()
            .publish((symbol_short!("short_rcv"), from.clone()), (amount, received));
    }
    // Never credit more than was requested, even if a rebasing token
    // happened to rebase upward mid-transfer.
    Ok(received.min(amount))
}
