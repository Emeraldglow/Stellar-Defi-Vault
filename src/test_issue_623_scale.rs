#![cfg(test)]
//! Issue #623: performance/scale coverage at a realistic depositor count.
//!
//! This benchmarks core operations at N = 100 / 500 / 1000 depositors (the
//! issue's own suggested sizes) using `env.budget()`'s real CPU-instruction
//! accounting, and empirically confirms which functions are safe at any pool
//! size and which ones are not — the "measured companion" to the design
//! review in issue #609's `STORAGE.md`.
//!
//! ## Findings (see `STORAGE.md` → "Known scaling risks" for the write-up)
//!
//! - `balance::get_all_stakers` / `DataKey::AllStakers` is a single
//!   instance-storage key holding **one `Vec<Address>` of every depositor who
//!   has ever staked**, appended to (never pruned) by
//!   `balance::register_staker`. Every read of it costs proportionally more
//!   as the pool grows, and it is read in full (not paginated) by several
//!   functions.
//! - `VaultContract::stake` calls `balance::register_staker` exactly once per
//!   address, the first time it stakes (`current_shares == 0`).
//!   `register_staker` reads the *entire* `AllStakers` vec, does a linear
//!   `.iter().any()` de-dup scan over it, and — for a genuinely new address —
//!   writes the whole (now one-longer) vec back. So **every new depositor's
//!   first stake is O(n) in the total depositor count**, not O(1); a
//!   *returning* depositor's top-up stake skips this path entirely and stays
//!   cheap regardless of pool size (`repeat_stake_cost_does_not_scale_with_pool_size`
//!   below).
//! - `VaultContract::get_top_depositors` is bounded: `rank_top_depositors`
//!   scans at most `MAX_DEPOSITOR_SCAN` (200) registration-order entries no
//!   matter how large the pool is (`top_depositors_cost_is_bounded_by_max_scan`
//!   below), matching its own doc comment.
//! - `VaultContract::stake_weighted_average_duration` (public, no auth) reads
//!   the full `AllStakers` vec and loops over every entry with no cap — an
//!   unbounded O(n) read anyone can call for free
//!   (`stake_weighted_average_duration_cost_scales_with_pool_size` below).
//!   `export_state`, `get_reward_gini_coefficient` (bounded, reverts above
//!   `MAX_GINI_STAKERS`), and `view_all_positions` (paginated iteration, but
//!   still loads the full vec to paginate over it) have the same
//!   full-vec-read shape and should be budgeted the same way if a pool ever
//!   approaches this scale.
//! - `VaultContract::deposit`'s own `is_first_deposit` check
//!   (`balance::get_all_stakers(&env).iter().any(...)`) has the identical
//!   unbounded shape, but `deposit()` currently fails to build on `main` for
//!   an unrelated, pre-existing reason (its call to `Self::stake` is missing
//!   the `min_shares_out` argument added after it was written — see
//!   `broken_baseline_build` project notes) and so cannot be exercised here.
//!   Once that's fixed, this same O(n) shape applies to it too.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::vault::{VaultContract, VaultContractClient, MAX_DEPOSITOR_SCAN};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    token_admin: token::StellarAssetClient<'a>,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 1000;
        });

        let admin = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);

        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        Fixture {
            env,
            vault,
            token_admin,
        }
    }

    /// Mints `amount` to a fresh address and stakes it, growing the
    /// depositor registry by one. Budget-unlimited: used only for setup, not
    /// for measurement.
    fn seed_new_depositor(&self, amount: i128) -> Address {
        let user = Address::generate(&self.env);
        self.token_admin.mint(&user, &amount);
        self.env.budget().reset_unlimited();
        self.vault.stake(&user, &amount, &0);
        user
    }

    /// Seeds `n` fresh depositors of 1000 units each.
    fn seed_n_depositors(&self, n: u32) {
        for _ in 0..n {
            self.seed_new_depositor(1_000);
        }
    }

    /// Runs `f` under a freshly reset budget and returns the CPU-instruction
    /// cost of exactly that call, isolated from setup cost.
    fn measure(&self, f: impl FnOnce()) -> u64 {
        self.env.budget().reset_unlimited();
        f();
        self.env.budget().cpu_instruction_cost()
    }
}

/// A brand-new depositor's first stake registers them in `AllStakers`
/// (`balance::register_staker`), which reads, linearly scans, and rewrites
/// the *entire* registry — so its cost grows with how many depositors
/// already exist, not just with the new depositor's own data.
#[test]
fn new_depositor_stake_cost_scales_with_pool_size() {
    let f = Fixture::new();

    let cost_at_100 = {
        f.seed_n_depositors(100);
        let newcomer = Address::generate(&f.env);
        f.token_admin.mint(&newcomer, &1_000);
        f.measure(|| {
            f.vault.stake(&newcomer, &1_000, &0);
        })
    };

    let cost_at_500 = {
        f.seed_n_depositors(400); // 100 -> 500
        let newcomer = Address::generate(&f.env);
        f.token_admin.mint(&newcomer, &1_000);
        f.measure(|| {
            f.vault.stake(&newcomer, &1_000, &0);
        })
    };

    let cost_at_1000 = {
        f.seed_n_depositors(500); // 500 -> 1000
        let newcomer = Address::generate(&f.env);
        f.token_admin.mint(&newcomer, &1_000);
        f.measure(|| {
            f.vault.stake(&newcomer, &1_000, &0);
        })
    };

    std::println!(
        "new-depositor stake cost (CPU insns): n=100 -> {}, n=500 -> {}, n=1000 -> {}",
        cost_at_100,
        cost_at_500,
        cost_at_1000
    );

    // This is the empirical finding, not a tuned threshold: the cost must
    // rise as the registry grows, since `register_staker` re-reads and
    // rewrites the whole `AllStakers` vec on every new address.
    assert!(
        cost_at_500 > cost_at_100,
        "expected new-depositor stake cost to grow between 100 and 500 existing depositors"
    );
    assert!(
        cost_at_1000 > cost_at_500,
        "expected new-depositor stake cost to grow between 500 and 1000 existing depositors"
    );
}

/// A *returning* depositor's top-up stake (`current_shares > 0`) skips
/// `register_staker` entirely, so its cost should stay flat regardless of how
/// many other depositors exist.
#[test]
fn repeat_stake_cost_does_not_scale_with_pool_size() {
    let f = Fixture::new();
    let regular = f.seed_new_depositor(1_000);

    let cost_at_100 = {
        f.seed_n_depositors(99); // regular + 99 others = 100
        f.token_admin.mint(&regular, &1_000);
        f.measure(|| {
            f.vault.stake(&regular, &1_000, &0);
        })
    };

    let cost_at_1000 = {
        f.seed_n_depositors(900); // -> 1000 others
        f.token_admin.mint(&regular, &1_000);
        f.measure(|| {
            f.vault.stake(&regular, &1_000, &0);
        })
    };

    std::println!(
        "repeat-stake cost (CPU insns): n=100 -> {}, n=1000 -> {}",
        cost_at_100,
        cost_at_1000
    );

    // Allow generous headroom (2x) for incidental cost drift (e.g. larger
    // ledger/TTL bookkeeping numbers), but a repeat stake must not scale
    // anywhere near linearly with pool size the way a new one does above.
    assert!(
        cost_at_1000 < cost_at_100 * 2,
        "repeat-stake cost should stay roughly constant regardless of pool size, \
         got {} at n=100 vs {} at n=1000",
        cost_at_100,
        cost_at_1000
    );
}

/// `get_top_depositors` caps its scan at `MAX_DEPOSITOR_SCAN` registrations,
/// so its cost should plateau well before N reaches the hundreds/thousands
/// this issue asks to test at.
#[test]
fn top_depositors_cost_is_bounded_by_max_scan() {
    let f = Fixture::new();
    assert!(
        MAX_DEPOSITOR_SCAN < 500,
        "test assumes the scan cap is well under 500"
    );

    f.seed_n_depositors(500);
    let cost_at_500 = f.measure(|| {
        f.vault.get_top_depositors(&50);
    });

    f.seed_n_depositors(500); // -> 1000
    let cost_at_1000 = f.measure(|| {
        f.vault.get_top_depositors(&50);
    });

    std::println!(
        "get_top_depositors cost (CPU insns): n=500 -> {}, n=1000 -> {}",
        cost_at_500,
        cost_at_1000
    );

    // Both scans are clamped to MAX_DEPOSITOR_SCAN entries, so cost at 1000
    // depositors must not meaningfully exceed cost at 500 (generous 30%
    // tolerance for the vec being physically longer even though the scan
    // itself stops early).
    assert!(
        cost_at_1000 < cost_at_500 + (cost_at_500 / 3),
        "get_top_depositors cost must stay bounded regardless of pool size, \
         got {} at n=500 vs {} at n=1000",
        cost_at_500,
        cost_at_1000
    );
}

/// `stake_weighted_average_duration` is public, requires no auth, and loops
/// over every entry in `AllStakers` with no cap — flagged in the module doc
/// comment above as an unbounded O(n) read.
#[test]
fn stake_weighted_average_duration_cost_scales_with_pool_size() {
    let f = Fixture::new();

    f.seed_n_depositors(100);
    let cost_at_100 = f.measure(|| {
        f.vault.stake_weighted_average_duration();
    });

    f.seed_n_depositors(900); // -> 1000
    let cost_at_1000 = f.measure(|| {
        f.vault.stake_weighted_average_duration();
    });

    std::println!(
        "stake_weighted_average_duration cost (CPU insns): n=100 -> {}, n=1000 -> {}",
        cost_at_100,
        cost_at_1000
    );

    assert!(
        cost_at_1000 > cost_at_100,
        "expected stake_weighted_average_duration cost to grow with pool size \
         (it scans the full AllStakers registry with no cap)"
    );
}

/// At the issue's upper bound (1000 depositors), a *returning* depositor's
/// stake and the bounded leaderboard query must both still fit comfortably
/// within the real default Soroban resource budget (not `reset_unlimited`),
/// i.e. they would not fail on a live network at this scale.
#[test]
fn bounded_operations_fit_default_budget_at_1000_depositors() {
    let f = Fixture::new();
    f.seed_n_depositors(1000);
    let regular = f.seed_new_depositor(1_000);

    f.env.budget().reset_default();
    f.token_admin.mint(&regular, &1_000);
    f.vault.stake(&regular, &1_000, &0);
    f.vault.get_top_depositors(&50);
    // No panic above means both calls stayed within the real, non-unlimited
    // network resource budget at 1000 depositors.
}
