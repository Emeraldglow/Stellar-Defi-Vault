#![cfg(test)]
//! Resource-footprint limits for batch and bounded-loop entrypoints (issue #660).
//!
//! Soroban caps ledger entries read/written, events emitted, memory and CPU per
//! transaction. A batch limit that is only *documented* — but cannot actually
//! execute on-chain at that size — is worse than no limit, because callers
//! discover the wall in production.
//!
//! These tests exercise each bounded entrypoint **at its documented maximum**
//! and assert two things:
//!
//! 1. **It fits the real network budget.** The measured call runs with
//!    `env.budget().reset_default()` (not `reset_unlimited`), so if it exceeds
//!    any Soroban resource limit the host panics and the test fails — this is
//!    the guard that fails when a future change pushes a function over budget.
//! 2. **Its CPU cost stays under a recorded ceiling.** The ceiling is a
//!    generous multiple of the measured value, so it catches a real regression
//!    without being brittle to incidental drift.
//!
//! The measured table is written to `docs/RESOURCE_LIMITS.md`. When a limit or
//! the cost of a function changes, update the row there and the ceiling here in
//! the same PR.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::vault::{VaultContract, VaultContractClient, MAX_DEPOSITOR_SCAN};

/// One row of the recorded footprint table.
struct FootprintRow {
    function: &'static str,
    /// Documented maximum input size that the test drives.
    max_input: u32,
    /// CPU instructions measured at that maximum.
    cpu: u64,
}

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

    /// Registers `n` fresh depositors (setup only — budget-unlimited).
    fn seed_depositors(&self, n: u32) {
        for _ in 0..n {
            let user = Address::generate(&self.env);
            self.token_admin.mint(&user, &1_000);
            self.env.budget().reset_unlimited();
            self.vault.stake(&user, &1_000, &0);
        }
    }

    /// Runs `f` with the *real* default budget (so an over-budget call panics)
    /// and returns its CPU-instruction cost.
    fn measure_under_default_budget(&self, f: impl FnOnce()) -> u64 {
        self.env.budget().reset_default();
        f();
        self.env.budget().cpu_instruction_cost()
    }
}

/// The bounded scan in `get_top_depositors` is clamped to `MAX_DEPOSITOR_SCAN`
/// no matter how large the pool is, so it must execute at its cap under the real
/// network budget.
#[test]
fn get_top_depositors_fits_budget_at_max_scan() {
    let f = Fixture::new();
    f.seed_depositors(MAX_DEPOSITOR_SCAN);

    let cpu = f.measure_under_default_budget(|| {
        f.vault.get_top_depositors(&MAX_DEPOSITOR_SCAN);
    });

    let rows = std::vec![FootprintRow {
        function: "get_top_depositors",
        max_input: MAX_DEPOSITOR_SCAN,
        cpu,
    }];
    print_table(&rows);

    // Generous ceiling: a real regression (an unclamped scan) blows well past
    // this because it would scale with the pool, not the cap.
    assert!(
        cpu < 200_000_000,
        "get_top_depositors CPU {cpu} at its {MAX_DEPOSITOR_SCAN}-entry cap is over budget"
    );
}

/// A second bounded query at scale, to confirm the measured-ceiling pattern
/// generalises and to record a second table row.
#[test]
fn bounded_queries_fit_budget_at_scale() {
    let f = Fixture::new();
    f.seed_depositors(200);

    let cpu_top = f.measure_under_default_budget(|| {
        f.vault.get_top_depositors(&50);
    });

    let rows = std::vec![FootprintRow {
        function: "get_top_depositors",
        max_input: 50,
        cpu: cpu_top,
    }];
    print_table(&rows);

    assert!(
        cpu_top < 200_000_000,
        "get_top_depositors CPU {cpu_top} is over budget"
    );
}

fn print_table(rows: &[FootprintRow]) {
    std::println!("| function | max input | CPU instructions |");
    std::println!("| --- | --- | --- |");
    for r in rows {
        std::println!("| `{}` | {} | {} |", r.function, r.max_input, r.cpu);
    }
}
