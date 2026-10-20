//! Direct position transfer (issue #518).
//!
//! Lets a staker move their whole vault position — shares, stake-start
//! ledger, last-claim ledger, and any pending (unclaimed) reward — to another
//! address in one call, instead of unstaking and having the recipient
//! re-deposit. Useful for gifting a position or consolidating accounts.
//!
//! Every error case this needs already exists on the main `VaultError` enum
//! (`PositionNotFound`, `RecipientAlreadyStaking`, both already documented
//! there as belonging to `transfer_position()`), so this returns `VaultError`
//! directly rather than introducing a new error enum.
//!
//! # Wiring
//!
//! `vault.rs`'s per-user position fields (`ShareBalance`, `StakedAtLedger`,
//! `LastClaimLedger`, `AccruedReward`) are all keyed directly by `Address`,
//! so a transfer is implemented here as moving each of those four values
//! from `from` to `to` — no different, mechanically, from what `unstake` +
//! `stake` would do, minus the token round-trip and fee.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap; the fields this touches
//! (`StakedAtLedger`) are read/written directly via `DataKey`, matching how
//! `vault.rs`'s own `position_split()` does it.

use soroban_sdk::{contractimpl, symbol_short, Address, Env};

use crate::balance;
use crate::errors::VaultError;
use crate::storage::DataKey;
use crate::vault::VaultContract;

#[contractimpl]
impl VaultContract {
    /// Transfer the caller's entire staking position to `to` in one call.
    ///
    /// Reverts with `PositionNotFound` if the caller has no active position,
    /// and with `RecipientAlreadyStaking` if `to` already has one (including
    /// the degenerate case of `to == from`). Returns the number of shares
    /// transferred.
    pub fn transfer_position(env: Env, from: Address, to: Address) -> Result<i128, VaultError> {
        from.require_auth();

        let from_shares = balance::get_shares(&env, &from);
        if from_shares <= 0 {
            return Err(VaultError::PositionNotFound);
        }
        let to_shares = balance::get_shares(&env, &to);
        if to_shares > 0 {
            return Err(VaultError::RecipientAlreadyStaking);
        }

        let staked_at_ledger: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StakedAtLedger(from.clone()))
            .unwrap_or_else(|| env.ledger().sequence());
        let last_claim_ledger = balance::get_last_claim_ledger(&env, &from);
        let accrued = balance::get_accrued_reward(&env, &from);

        balance::set_shares(&env, &from, 0);
        balance::set_shares(&env, &to, from_shares);

        env.storage()
            .persistent()
            .set(&DataKey::StakedAtLedger(to.clone()), &staked_at_ledger);
        env.storage()
            .persistent()
            .remove(&DataKey::StakedAtLedger(from.clone()));

        balance::set_last_claim_ledger(&env, &to, last_claim_ledger);
        balance::set_last_claim_ledger(&env, &from, 0);
        balance::set_accrued_reward(&env, &to, accrued);
        balance::set_accrued_reward(&env, &from, 0);

        env.events().publish(
            (symbol_short!("pos_xfer"), from),
            (to, from_shares, env.ledger().sequence()),
        );
        Ok(from_shares)
    }
}
