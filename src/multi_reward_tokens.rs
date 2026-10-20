//! Multiple simultaneous reward tokens (issue #699).
//!
//! Besides the primary reward, the admin can configure up to three extra
//! reward tokens (for partner incentives). Each has its own rate, its own
//! funding pool, and accrues independently of the primary reward and of the
//! other extra tokens.
//!
//! # Accounting
//!
//! Each token keeps a global reward-per-share accumulator that grows with time
//! at `rate_bps` (annualised like the primary rate, see `reward_math.rs`).
//! A user's earnings for a token are `shares * (accumulator - checkpoint)`,
//! where the checkpoint is the accumulator value when the user was last
//! settled. `balance::set_shares()` calls [`settle_user`] before any share
//! change, so earnings are always computed at the share count that was
//! actually held. Users already staked when a token is added earn from the
//! moment it is added.
//!
//! # Funding
//!
//! Accrual does not depend on funding. Claims pay at most what the token's
//! pool holds and the rest stays pending, so a token that has run dry never
//! blocks the other tokens or reverts `claim_all()`.
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultFeature6Error;
use crate::vault::{VaultContractClient, BOOST_BPS_BASE, STELLAR_LEDGERS_PER_YEAR};
use crate::VaultContract;

/// Maximum number of extra reward tokens.
pub const MAX_EXTRA_REWARD_TOKENS: u32 = 3;

/// Fixed-point scale of the reward-per-share accumulator.
const PRECISION: i128 = 1_000_000_000_000_000_000;

const TOKENS_KEY: Symbol = symbol_short!("xr_toks");
const CHECKPOINT_KEY: Symbol = symbol_short!("xr_ucp");
const PENDING_KEY: Symbol = symbol_short!("xr_upd");

/// Configuration and global state of one extra reward token.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ExtraRewardToken {
    /// The reward token contract.
    pub token: Address,
    /// Annual reward rate in basis points, per share.
    pub rate_bps: u32,
    /// Reward earned per share since the token was added, scaled by `PRECISION`.
    pub acc_per_share: i128,
    /// Ledger at which `acc_per_share` was last advanced.
    pub last_update: u32,
    /// Funded balance still available to pay claims.
    pub pool: i128,
}

fn load(env: &Env) -> Vec<ExtraRewardToken> {
    env.storage()
        .instance()
        .get(&TOKENS_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn save(env: &Env, tokens: &Vec<ExtraRewardToken>) {
    env.storage().instance().set(&TOKENS_KEY, tokens);
}

fn find(tokens: &Vec<ExtraRewardToken>, token: &Address) -> Option<u32> {
    (0..tokens.len()).find(|i| tokens.get(*i).map(|t| &t.token == token).unwrap_or(false))
}

/// Advance `t.acc_per_share` to the current ledger.
fn advance(env: &Env, t: &mut ExtraRewardToken) {
    let now = env.ledger().sequence();
    if now <= t.last_update {
        return;
    }
    let elapsed = (now - t.last_update) as i128;
    let increase = elapsed
        .saturating_mul(t.rate_bps as i128)
        .saturating_mul(PRECISION)
        / (BOOST_BPS_BASE as i128 * STELLAR_LEDGERS_PER_YEAR as i128);
    t.acc_per_share = t.acc_per_share.saturating_add(increase);
    t.last_update = now;
}

fn stored_pending(env: &Env, token: &Address, user: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&(PENDING_KEY, token.clone(), user.clone()))
        .unwrap_or(0)
}

fn earned_since_checkpoint(env: &Env, t: &ExtraRewardToken, user: &Address, shares: i128) -> i128 {
    let checkpoint: i128 = env
        .storage()
        .persistent()
        .get(&(CHECKPOINT_KEY, t.token.clone(), user.clone()))
        .unwrap_or(0);
    if shares <= 0 || t.acc_per_share <= checkpoint {
        return 0;
    }
    shares.saturating_mul(t.acc_per_share - checkpoint) / PRECISION
}

/// Move what `user` earned on `t` into their stored pending balance and
/// checkpoint them at the current accumulator.
fn settle_one(env: &Env, t: &ExtraRewardToken, user: &Address, shares: i128) {
    let earned = earned_since_checkpoint(env, t, user, shares);
    if earned > 0 {
        env.storage().persistent().set(
            &(PENDING_KEY, t.token.clone(), user.clone()),
            &stored_pending(env, &t.token, user).saturating_add(earned),
        );
    }
    env.storage().persistent().set(
        &(CHECKPOINT_KEY, t.token.clone(), user.clone()),
        &t.acc_per_share,
    );
}

/// Settle `user`'s earnings on every extra reward token at their current
/// share count. Called before any change to that count; does nothing when no
/// extra token is configured.
pub fn settle_user(env: &Env, user: &Address) {
    let mut tokens = load(env);
    if tokens.is_empty() {
        return;
    }
    let shares = balance::get_shares(env, user);
    for i in 0..tokens.len() {
        let mut t = tokens.get(i).unwrap();
        advance(env, &mut t);
        settle_one(env, &t, user, shares);
        tokens.set(i, t);
    }
    save(env, &tokens);
}

/// Take what the pool can cover of `user`'s pending balance on token `index`,
/// updating the pool and pending balance. Returns the amount to transfer.
fn take_payable(
    env: &Env,
    tokens: &mut Vec<ExtraRewardToken>,
    index: u32,
    user: &Address,
) -> i128 {
    let mut t = tokens.get(index).unwrap();
    let pending = stored_pending(env, &t.token, user);
    let pay = pending.min(t.pool);
    if pay > 0 {
        t.pool -= pay;
        env.storage().persistent().set(
            &(PENDING_KEY, t.token.clone(), user.clone()),
            &(pending - pay),
        );
        tokens.set(index, t);
    }
    pay
}

fn pay_out(env: &Env, token_addr: &Address, user: &Address, amount: i128) {
    if amount > 0 {
        token::Client::new(env, token_addr).transfer(
            &env.current_contract_address(),
            user,
            &amount,
        );
        env.events().publish(
            (symbol_short!("xr_claim"), token_addr.clone(), user.clone()),
            amount,
        );
    }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Adds an extra reward token that accrues at `rate_bps` per share per
    /// year, alongside the primary reward. At most three extra tokens.
    ///
    /// # Authorization
    /// `admin` must sign and be the pool admin.
    ///
    /// # Errors
    /// - `InvalidRate`: `rate_bps` is zero or above `MAX_RATE_BPS`.
    /// - `RewardTokenLimitReached`: three extra tokens are already configured.
    /// - `RewardTokenAlreadyAdded`: `token` is already configured.
    ///
    /// # Events
    /// - `xr_added`.
    pub fn add_reward_token(
        env: Env,
        admin: Address,
        token: Address,
        rate_bps: u32,
    ) -> Result<(), VaultFeature6Error> {
        admin::require_admin_as(&env, &admin)?;
        if rate_bps == 0 || rate_bps > balance::MAX_RATE_BPS {
            return Err(VaultFeature6Error::InvalidRate);
        }
        let mut tokens = load(&env);
        if find(&tokens, &token).is_some() {
            return Err(VaultFeature6Error::RewardTokenAlreadyAdded);
        }
        if tokens.len() >= MAX_EXTRA_REWARD_TOKENS {
            return Err(VaultFeature6Error::RewardTokenLimitReached);
        }
        tokens.push_back(ExtraRewardToken {
            token: token.clone(),
            rate_bps,
            acc_per_share: 0,
            last_update: env.ledger().sequence(),
            pool: 0,
        });
        save(&env, &tokens);
        env.events()
            .publish((symbol_short!("xr_added"), token), rate_bps);
        Ok(())
    }

    /// Funds an extra reward token's pool with `amount`, taken from `funder`.
    /// Anyone can fund (for example the partner offering the incentive).
    ///
    /// # Authorization
    /// `funder` must sign.
    ///
    /// # Errors
    /// - `ZeroAmount`: `amount <= 0`.
    /// - `RewardTokenNotFound`: `token` is not a configured extra reward token.
    ///
    /// # Events
    /// - `xr_fund`.
    pub fn fund_reward_token(
        env: Env,
        funder: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), VaultFeature6Error> {
        funder.require_auth();
        if amount <= 0 {
            return Err(VaultFeature6Error::ZeroAmount);
        }
        let mut tokens = load(&env);
        let index = find(&tokens, &token).ok_or(VaultFeature6Error::RewardTokenNotFound)?;
        let mut t = tokens.get(index).unwrap();
        t.pool = t
            .pool
            .checked_add(amount)
            .ok_or(VaultFeature6Error::ArithmeticError)?;
        tokens.set(index, t);
        save(&env, &tokens);
        token::Client::new(&env, &token).transfer(
            &funder,
            &env.current_contract_address(),
            &amount,
        );
        env.events()
            .publish((symbol_short!("xr_fund"), token, funder), amount);
        Ok(())
    }

    /// What `user` has pending on each extra reward token, as
    /// `(token, amount)` pairs. The primary reward is not included.
    pub fn get_pending_rewards(env: Env, user: Address) -> Vec<(Address, i128)> {
        let shares = balance::get_shares(&env, &user);
        let mut result = Vec::new(&env);
        for t in load(&env).iter() {
            let mut projected = t.clone();
            advance(&env, &mut projected);
            let pending = stored_pending(&env, &t.token, &user)
                .saturating_add(earned_since_checkpoint(&env, &projected, &user, shares));
            result.push_back((t.token, pending));
        }
        result
    }

    /// Claims `user`'s pending amount of one extra reward token, up to what
    /// the token's pool holds; the rest stays pending. Returns the amount paid.
    ///
    /// # Authorization
    /// `user` must sign.
    ///
    /// # Errors
    /// - `RewardTokenNotFound`: `token` is not a configured extra reward token.
    ///
    /// # Events
    /// - `xr_claim` when a non-zero amount is paid.
    pub fn claim_token(
        env: Env,
        user: Address,
        token: Address,
    ) -> Result<i128, VaultFeature6Error> {
        user.require_auth();
        settle_user(&env, &user);
        let mut tokens = load(&env);
        let index = find(&tokens, &token).ok_or(VaultFeature6Error::RewardTokenNotFound)?;
        let paid = take_payable(&env, &mut tokens, index, &user);
        save(&env, &tokens);
        pay_out(&env, &token, &user, paid);
        Ok(paid)
    }

    /// Claims `user`'s pending amount of every extra reward token, each up to
    /// what its pool holds. A token with an empty pool pays `0` and does not
    /// block the others. Returns `(token, amount paid)` for every token.
    ///
    /// # Authorization
    /// `user` must sign.
    ///
    /// # Events
    /// - `xr_claim` for each token with a non-zero payout.
    pub fn claim_all(env: Env, user: Address) -> Vec<(Address, i128)> {
        user.require_auth();
        settle_user(&env, &user);
        let mut tokens = load(&env);
        let mut result = Vec::new(&env);
        for i in 0..tokens.len() {
            let paid = take_payable(&env, &mut tokens, i, &user);
            result.push_back((tokens.get(i).unwrap().token, paid));
        }
        save(&env, &tokens);
        for (token_addr, paid) in result.iter() {
            pay_out(&env, &token_addr, &user, paid);
        }
        result
    }
}
