use soroban_sdk::{contractclient, Address, BytesN, Env, String, Vec};
use crate::errors::VaultError;
use crate::vault::{MigrationExport, PauseReason, RateChange, SharePriceSnapshot};

#[contractclient(name = "IStakingPoolClient")]
pub trait IStakingPool {
    fn staked_amount(env: Env, user: Address) -> i128;
    fn pending_reward(env: Env, user: Address) -> i128;
    fn total_staked(env: Env) -> i128;
    fn is_paused(env: Env) -> bool;
}

/// Canonical contract interface defining the public API surface of the Stellar DeFi Vault.
///
/// External consumers, cross-contract integrators, and client generators can reference this trait
/// as a single, compiler-checked source of truth for the core vault functionality.
#[contractclient(name = "VaultTraitClient")]
pub trait VaultTrait {
    /// Initialize the vault with an admin and deposit token.
    fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        reward_rate_bps: u32,
        stake_decimals: Option<u32>,
        reward_decimals: Option<u32>,
    ) -> Result<(), VaultError>;

    /// Rotate the primary admin key to a new address.
    fn transfer_admin(env: Env, new_admin: Address) -> Result<(), VaultError>;

    /// Stake tokens on behalf of the caller.
    fn stake(
        env: Env,
        user: Address,
        amount: i128,
        min_shares_out: i128,
    ) -> Result<i128, VaultError>;

    /// Deposit tokens into the vault.
    fn deposit(
        env: Env,
        depositor: Address,
        amount: i128,
        min_shares_out: i128,
    ) -> Result<i128, VaultError>;

    /// Register a referrer for the caller.
    fn register_referrer(env: Env, user: Address) -> Result<(), VaultError>;

    /// Configure the referral bonus in basis points.
    fn set_referral_bonus_bps(
        env: Env,
        admin_addr: Address,
        bonus_bps: u32,
    ) -> Result<(), VaultError>;

    /// Upgrade the contract's Wasm bytecode to a new hash.
    fn upgrade_wasm(
        env: Env,
        admin: Address,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), VaultError>;

    /// Configure the APR reward rate in basis points.
    fn set_reward_rate_bps(env: Env, rate_bps: u32) -> Result<(), VaultError>;

    /// Query the current reward rate in basis points.
    fn get_reward_rate_bps(env: Env) -> u32;

    /// Retrieve the rate change history log.
    fn get_rate_history(env: Env) -> Vec<RateChange>;

    /// Designate a secondary crisis/emergency admin.
    fn set_emergency_admin(
        env: Env,
        admin_addr: Address,
        new_emergency_admin: Address,
    ) -> Result<(), VaultError>;

    /// Export the contract state for migration.
    fn export_state(env: Env, admin_addr: Address) -> Result<MigrationExport, VaultError>;

    /// Revoke the active emergency admin.
    fn revoke_emergency_admin(env: Env, admin_addr: Address) -> Result<(), VaultError>;

    /// Burn shares and withdraw underlying deposited tokens.
    fn withdraw(env: Env, withdrawer: Address, shares: i128) -> Result<i128, VaultError>;

    /// Emergency withdrawal of principal bypassing penalties when available.
    fn emergency_withdraw(env: Env, user: Address) -> Result<i128, VaultError>;

    /// Unstake a specific number of shares.
    fn unstake(env: Env, staker: Address, shares: i128) -> Result<i128, VaultError>;

    /// Unstake all shares for the user.
    fn unstake_all(env: Env, user: Address) -> Result<i128, VaultError>;

    /// Claim all accrued rewards for the staker.
    fn claim(env: Env, staker: Address) -> Result<i128, VaultError>;

    /// Claim a partial amount of accrued rewards.
    fn claim_partial(env: Env, staker: Address, amount: i128) -> Result<i128, VaultError>;

    /// Stake additional tokens and claim accrued rewards in a single transaction.
    fn stake_and_claim(env: Env, user: Address, amount: i128) -> Result<i128, VaultError>;

    /// Query shares held by an address.
    fn shares_of(env: Env, user: Address) -> i128;

    /// Take a snapshot of the current share price.
    fn take_share_price_snapshot(env: Env) -> bool;

    /// Query the history of share price snapshots.
    fn get_share_price_history(env: Env) -> Vec<SharePriceSnapshot>;

    /// Query total staked tokens for a user.
    fn staked_amount(env: Env, user: Address) -> i128;

    /// Query the primary contract admin.
    fn get_admin(env: Env) -> Result<Address, VaultError>;

    /// Query the immutable deployer address.
    fn pool_created_by(env: Env) -> Result<Address, VaultError>;

    /// Query the contract version string.
    fn get_version(env: Env) -> String;

    /// Query the total staked token balance in the pool.
    fn total_staked(env: Env) -> Result<i128, VaultError>;

    /// Check if the vault is currently paused.
    fn is_paused(env: Env) -> bool;

    /// Query total shares and total deposited amount.
    fn vault_state(env: Env) -> Result<(i128, i128), VaultError>;

    /// Pause contract operations with a reason and message.
    fn pause(
        env: Env,
        reason: PauseReason,
        message: String,
    ) -> Result<(), VaultError>;

    /// Unpause contract operations.
    fn unpause(env: Env) -> Result<(), VaultError>;

    /// Admin: deposit yield tokens into the reward pool.
    fn add_yield(env: Env, admin_addr: Address, amount: i128) -> Result<(), VaultError>;
}
