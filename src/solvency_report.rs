//! On-chain solvency report (issue #753).
//!
//! `get_solvency_report` compares the tokens the contract actually holds
//! against the two things it owes users:
//!
//! - `total_principal`: the staked principal backing all outstanding shares
//!   (`balance::get_total_deposited`).
//! - `accrued_unclaimed`: rewards already settled into users' accrued balances
//!   but not yet claimed.
//!
//! `reward_pool` is reported separately because it is *reward* money, not user
//! principal. It is not added to the liabilities a second time: the portion of
//! the pool that has been accrued is already counted in `accrued_unclaimed`,
//! and the rest is the unaccrued reward budget. Keeping the two fields apart
//! makes it obvious how much of the balance is principal and how much is
//! reward funding.
//!
//! `surplus_or_deficit` is:
//!
//! ```text
//! token_balance - (total_principal + accrued_unclaimed)
//! ```
//!
//! so `>= 0` means the contract can cover every principal withdrawal and every
//! accrued reward, while `< 0` is a deficit. Because `token_balance` is read
//! from the token contract itself, direct donations to the vault increase both
//! `token_balance` and the surplus even though no internal counter changed.
//!
//! # Accrued rewards
//!
//! `accrued_unclaimed` is read from a pool-wide accumulator maintained by
//! [`crate::balance::set_accrued_reward`] as rewards are accrued and cleared,
//! so this report never scans the staker registry. Rewards earned since a
//! position's last checkpoint are realized into `accrued_reward` on that
//! position's next stake, unstake, or claim (see `VaultContract::pending_reward`)
//! and are picked up by the accumulator then.
//!
//! The accumulator is new with this issue and starts at zero, so a deployment
//! upgrading with outstanding accrued rewards will under-report them until the
//! next accrual/claim cycle rewrites those positions.
//!
//! # Scope
//!
//! The token balance may also hold other internal earmarks (insurance reserve,
//! revenue-share pool, claim-fee reserve, treasury). The report deliberately
//! measures total assets against user principal plus accrued rewards, as the
//! issue specifies; it does not net out those earmarks.
//!
//! `token_balance` is the stake token's balance. The default setup pays rewards
//! in that same token, so all five fields are directly comparable. When a
//! distinct reward token is configured (issue #524), `accrued_unclaimed` and
//! `reward_pool` are denominated in the reward token and should be checked
//! against that token's balance separately.

use soroban_sdk::{contractimpl, contracttype, token, Address, Env};

use crate::balance;
use crate::storage::DataKey;
use crate::vault::{VaultContract, VaultContractArgs, VaultContractClient};

/// Point-in-time solvency snapshot (issue #753).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SolvencyReport {
    /// Tokens held by the vault contract, read from the token contract.
    pub token_balance: i128,
    /// Staked principal backing all outstanding shares.
    pub total_principal: i128,
    /// Reward-token balance earmarked for reward payouts.
    pub reward_pool: i128,
    /// Rewards accrued to users but not yet claimed.
    pub accrued_unclaimed: i128,
    /// `token_balance - (total_principal + accrued_unclaimed)`. Negative means
    /// the vault cannot cover every principal and accrued reward obligation.
    pub surplus_or_deficit: i128,
}

#[contractimpl]
impl VaultContract {
    /// Read-only: current solvency snapshot comparing held tokens to user
    /// principal plus accrued-but-unclaimed rewards (issue #753).
    ///
    /// Works before `initialize` too, reporting a zero token balance; all
    /// arithmetic saturates rather than panicking on extreme values.
    pub fn get_solvency_report(env: Env) -> SolvencyReport {
        let token_addr: Option<Address> = env.storage().instance().get(&DataKey::Token);
        let token_balance = match token_addr {
            Some(addr) => token::Client::new(&env, &addr).balance(&env.current_contract_address()),
            None => 0,
        };

        let total_principal = balance::get_total_deposited(&env);
        let reward_pool = balance::get_reward_pool_balance(&env);
        let accrued_unclaimed = balance::get_total_accrued_unclaimed(&env);
        let surplus_or_deficit = token_balance
            .saturating_sub(total_principal)
            .saturating_sub(accrued_unclaimed);

        SolvencyReport {
            token_balance,
            total_principal,
            reward_pool,
            accrued_unclaimed,
            surplus_or_deficit,
        }
    }
}
