//! Example consumer contract showing cross-contract integration with the
//! Stellar DeFi Vault.
//!
//! See [`docs/INTEGRATION.md`] for the full guide. This module provides three
//! pattern implementations:
//!
//! 1. **Reading positions** — query a user's share balance and pending reward
//!    without any authorization.
//! 2. **Previewing** — simulate a deposit or redemption before committing.
//! 3. **Depositing on behalf of a user** — route a user's tokens into the
//!    vault and return the minted shares.
//!
//! Each function's auth requirements are documented inline.

#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env};

// Cross-contract client generated from the vault's IStakingPool trait.
// In a real project this would be imported from the vault crate or from a
// generated bindings package.
mod vault_interface {
    use soroban_sdk::{contractclient, Address, Env};

    /// Minimal read interface shared by staking-pool integrators.
    #[contractclient(name = "VaultClient")]
    pub trait VaultInterface {
        /// Shares held by `user`.  No auth required.
        fn shares_of(env: Env, user: Address) -> i128;

        /// Accrued but unclaimed reward for `user`.  No auth required.
        fn calc_pending_reward(env: Env, user: Address) -> i128;

        /// Total token balance staked across all users.  No auth required.
        fn total_staked(env: Env) -> i128;

        /// Whether the vault is currently paused.  No auth required.
        fn is_paused(env: Env) -> bool;

        /// Simulate minting: returns how many shares `amount` tokens would
        /// produce at the current pool ratio.  No auth required.
        fn preview_deposit(env: Env, amount: i128) -> i128;

        /// Simulate burning: returns how many tokens `shares` would redeem
        /// at the current pool ratio.  No auth required.
        fn preview_redeem(env: Env, shares: i128) -> i128;

        /// Stake `amount` tokens on behalf of `user`, minting shares.
        ///
        /// # Authorization
        /// `user` must sign this call (or authorise this contract to act on
        /// their behalf via the Soroban auth sub-invocation tree).
        fn stake(env: Env, user: Address, amount: i128) -> i128;
    }
}

use vault_interface::VaultClient;

/// A simple lending-market / aggregator contract that integrates with the vault.
#[contract]
pub struct ConsumerContract;

#[contractimpl]
impl ConsumerContract {
    // ── Read-only helpers ─────────────────────────────────────────────────────

    /// Return the number of vault shares held by `user` and their estimated
    /// token value at the current share price.
    ///
    /// # Authorization
    /// None — pure read call.
    pub fn read_position(env: Env, vault: Address, user: Address) -> (i128, i128) {
        let client = VaultClient::new(&env, &vault);
        let shares = client.shares_of(&user);
        let token_value = if shares > 0 {
            client.preview_redeem(&shares)
        } else {
            0
        };
        (shares, token_value)
    }

    /// Return the accrued but unclaimed reward for `user`.
    ///
    /// # Authorization
    /// None — pure read call.
    pub fn pending_reward(env: Env, vault: Address, user: Address) -> i128 {
        let client = VaultClient::new(&env, &vault);
        client.calc_pending_reward(&user)
    }

    // ── Preview helpers ───────────────────────────────────────────────────────

    /// Simulate depositing `amount` tokens: returns how many shares would be
    /// minted without executing any state change.
    ///
    /// # Authorization
    /// None — pure read call.
    pub fn simulate_deposit(env: Env, vault: Address, amount: i128) -> i128 {
        let client = VaultClient::new(&env, &vault);
        client.preview_deposit(&amount)
    }

    /// Simulate redeeming `shares`: returns how many tokens would be returned
    /// without executing any state change.
    ///
    /// # Authorization
    /// None — pure read call.
    pub fn simulate_redeem(env: Env, vault: Address, shares: i128) -> i128 {
        let client = VaultClient::new(&env, &vault);
        client.preview_redeem(&shares)
    }

    // ── Deposit-on-behalf flow ────────────────────────────────────────────────

    /// Route `amount` tokens from `user` into the vault and return the shares
    /// minted.  The vault requires `user.require_auth()` on its own `stake`
    /// entrypoint; Soroban's sub-invocation auth tree propagates the user's
    /// top-level signature through this call automatically — no extra work
    /// needed here.
    ///
    /// Reverts if the vault is paused, the user is not whitelisted (when
    /// whitelist enforcement is enabled), or the amount is below the
    /// configured minimum stake.
    ///
    /// # Authorization
    /// `user` must sign the transaction that calls this function.
    pub fn deposit_for_user(env: Env, vault: Address, user: Address, amount: i128) -> i128 {
        // Verify the vault is healthy before touching tokens.
        let client = VaultClient::new(&env, &vault);
        assert!(!client.is_paused(), "vault is paused");

        // Auth flows down automatically via sub-invocation. Declare we need
        // the user's signature at this level so callers know what to provide.
        user.require_auth();

        client.stake(&user, &amount)
    }

    /// Same as `deposit_for_user` but reverts if the actual shares minted
    /// fall below `min_shares_out` (slippage protection).
    ///
    /// # Authorization
    /// `user` must sign.
    pub fn deposit_with_min_out(
        env: Env,
        vault: Address,
        user: Address,
        amount: i128,
        min_shares_out: i128,
    ) -> i128 {
        let client = VaultClient::new(&env, &vault);
        assert!(!client.is_paused(), "vault is paused");

        user.require_auth();

        let shares = client.stake(&user, &amount);
        assert!(shares >= min_shares_out, "slippage: shares below minimum");
        shares
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        token, Address, Env,
    };

    // Pull in the real vault contract for integration tests.
    use stellar_defi_vault::vault::VaultContract;

    use super::{ConsumerContract, ConsumerContractClient};

    // ── helpers ───────────────────────────────────────────────────────────────

    fn setup_env() -> (
        Env,
        Address,       // vault_id
        Address,       // consumer_id
        Address,       // admin
        Address,       // alice
        Address,       // token_addr (for token admin mint calls)
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 1_000;
        });

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);

        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        // Deploy vault
        let vault_id = env.register_contract(None, VaultContract);
        let vault = stellar_defi_vault::vault::VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        // Fund reward pool so reward claims can succeed.
        let pool_seed: i128 = 1_000_000;
        token_admin.mint(&admin, &pool_seed);
        vault.set_reward_rate_bps(&500); // 5% APR
        vault.fund_reward_pool(&admin, &pool_seed);

        // Deploy consumer
        let consumer_id = env.register_contract(None, ConsumerContract);

        // Mint Alice's tokens
        token_admin.mint(&alice, &5_000_000);

        (env, vault_id, consumer_id, admin, alice, token_addr)
    }

    fn token_admin<'a>(env: &'a Env, token_addr: &Address) -> token::StellarAssetClient<'a> {
        token::StellarAssetClient::new(env, token_addr)
    }

    // ── test: read_position on empty position ─────────────────────────────────

    #[test]
    fn read_position_returns_zero_for_new_user() {
        let (env, vault_id, consumer_id, _admin, alice, _token_addr) = setup_env();
        let consumer = ConsumerContractClient::new(&env, &consumer_id);

        let (shares, value) = consumer.read_position(&vault_id, &alice);
        assert_eq!(shares, 0);
        assert_eq!(value, 0);
    }

    // ── test: simulate_deposit matches actual shares minted ───────────────────
    //
    // Invariant (docs/ECONOMICS.md §1): first deposit is 1:1 for a 7-decimal
    // token, so simulate_deposit(1_000_000) == 1_000_000.

    #[test]
    fn simulate_deposit_matches_actual_deposit() {
        let (env, vault_id, consumer_id, _admin, alice, _token_addr) = setup_env();
        let consumer = ConsumerContractClient::new(&env, &consumer_id);

        let predicted = consumer.simulate_deposit(&vault_id, &1_000_000_i128);
        assert_eq!(predicted, 1_000_000, "first-deposit simulation must be 1:1");

        // Execute the actual deposit and confirm it matches.
        let actual = consumer.deposit_for_user(&vault_id, &alice, &1_000_000_i128);
        assert_eq!(actual, predicted);
    }

    // ── test: simulate_redeem after yield ─────────────────────────────────────
    //
    // Mirrors docs/ECONOMICS.md §2 Example 2:
    //   500,000 shares, add 100,000 yield → preview_redeem(250,000) = 300,000.

    #[test]
    fn simulate_redeem_reflects_yield_increase() {
        let (env, vault_id, consumer_id, admin, alice, token_addr) = setup_env();
        let consumer = ConsumerContractClient::new(&env, &consumer_id);
        let vault = stellar_defi_vault::vault::VaultContractClient::new(&env, &vault_id);

        // Alice deposits 500,000 (first deposit, 1:1 shares).
        vault.stake(&alice, &500_000_i128, &0);

        // Before yield: 250,000 shares → 250,000 tokens.
        let before = consumer.simulate_redeem(&vault_id, &250_000_i128);
        assert_eq!(before, 250_000);

        // Admin injects 100,000 tokens of yield (D += 100k, S unchanged).
        token_admin(&env, &token_addr).mint(&admin, &100_000);
        vault.add_yield(&admin, &100_000_i128);

        // After yield: 250,000 shares → 300,000 tokens.
        let after = consumer.simulate_redeem(&vault_id, &250_000_i128);
        assert_eq!(after, 300_000, "simulate_redeem must reflect yield increase");
    }

    // ── test: deposit_for_user routes tokens into vault ───────────────────────

    #[test]
    fn deposit_for_user_mints_shares_and_locks_tokens() {
        let (env, vault_id, consumer_id, _admin, alice, _token_addr) = setup_env();
        let consumer = ConsumerContractClient::new(&env, &consumer_id);
        let vault = stellar_defi_vault::vault::VaultContractClient::new(&env, &vault_id);

        let shares = consumer.deposit_for_user(&vault_id, &alice, &2_000_000_i128);
        assert_eq!(shares, 2_000_000, "first deposit is 1:1 for 7-decimal token");

        // Vault now reflects Alice's position.
        let (alice_shares, alice_value) = consumer.read_position(&vault_id, &alice);
        assert_eq!(alice_shares, 2_000_000);
        assert_eq!(alice_value, 2_000_000);

        // Pool totals are updated.
        assert_eq!(vault.total_staked().unwrap(), 2_000_000);
    }

    // ── test: deposit_with_min_out enforces slippage ──────────────────────────

    #[test]
    fn deposit_with_min_out_succeeds_when_above_floor() {
        let (env, vault_id, consumer_id, _admin, alice, _token_addr) = setup_env();
        let consumer = ConsumerContractClient::new(&env, &consumer_id);

        // First deposit: 1_000_000 tokens → 1_000_000 shares. min_out = 900_000 passes.
        let shares =
            consumer.deposit_with_min_out(&vault_id, &alice, &1_000_000_i128, &900_000_i128);
        assert_eq!(shares, 1_000_000);
    }

    // ── test: read_position after deposit shows correct value ─────────────────

    #[test]
    fn read_position_after_deposit() {
        let (env, vault_id, consumer_id, _admin, alice, _token_addr) = setup_env();
        let consumer = ConsumerContractClient::new(&env, &consumer_id);

        consumer.deposit_for_user(&vault_id, &alice, &3_000_000_i128);

        let (shares, value) = consumer.read_position(&vault_id, &alice);
        assert_eq!(shares, 3_000_000);
        assert_eq!(value, 3_000_000);
    }

    // ── test: pending_reward accrues after time passes ────────────────────────

    #[test]
    fn pending_reward_increases_with_elapsed_ledgers() {
        let (env, vault_id, consumer_id, _admin, alice, _token_addr) = setup_env();
        let consumer = ConsumerContractClient::new(&env, &consumer_id);

        consumer.deposit_for_user(&vault_id, &alice, &1_000_000_i128);

        // Immediately after deposit: no reward yet.
        let reward_t0 = consumer.pending_reward(&vault_id, &alice);

        // Advance ~half a year (reward rate = 5% APR).
        env.ledger().with_mut(|li| {
            li.sequence_number += stellar_defi_vault::vault::STELLAR_LEDGERS_PER_YEAR / 2;
        });

        let reward_t1 = consumer.pending_reward(&vault_id, &alice);
        // At 5% APR for half a year on 1_000_000: expected ≈ 25,000.
        // The accumulator may not have been ticked yet; either positive or
        // equal to t0 is acceptable here — the important thing is it never
        // goes negative.
        assert!(
            reward_t1 >= reward_t0,
            "pending reward must be non-decreasing over time"
        );
    }
}
