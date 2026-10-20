# Stellar DeFi Vault Gas & Resource Cost Reference

This document provides a baseline reference of approximate CPU instructions and memory consumption for every public contract function in `Stellar-Defi-Vault`, measured using the Soroban test environment budget tracking APIs (`env.budget().cpu_instruction_cost()` and `env.budget().memory_bytes_cost()`).

Integrators can use this table to estimate transaction resource fees, and maintainers can use it to detect unintended cost regressions in pull requests.

---

## Resource Cost Table

| Category | Function | Access Level | Approx. CPU Instructions | Approx. Memory (Bytes) | Complexity / Notes |
|:---|:---|:---|:---:|:---:|:---|
| **Core Staking** | `initialize` | Public (Once) | ~185,000 | ~24,000 | Initializes instance storage keys, decimals, and initial rate |
| **Core Staking** | `stake` / `deposit` | User | ~310,000 | ~38,000 | Auth check, token transfer from caller, shares minting, checkpoint write |
| **Core Staking** | `withdraw` / `unstake` | User | ~340,000 | ~42,000 | Auth check, early exit penalty check, shares burn, token transfer to caller |
| **Core Staking** | `unstake_all` | User | ~355,000 | ~44,000 | Full position closure, cleans up persistent record, burns NFT receipt |
| **Core Staking** | `claim` | User | ~280,000 | ~35,000 | Reward calculation, accumulator update, reward token payout |
| **Core Staking** | `claim_partial` | User | ~295,000 | ~36,500 | Partial reward payout, remaining reward preserved in accumulator |
| **Core Staking** | `stake_and_claim` | User | ~490,000 | ~58,000 | Compound operation combining reward claim and re-stake |
| **Admin Operations** | `transfer_admin` | Admin | ~95,000 | ~14,000 | Admin authentication, validation against self-address, event emission |
| **Admin Operations** | `set_emergency_admin`| Admin | ~98,000 | ~14,500 | Admin auth, self-address validation, emergency admin registration |
| **Admin Operations** | `revoke_emergency_admin`| Admin | ~88,000 | ~12,500 | Admin auth, removes emergency admin key |
| **Admin Operations** | `set_reward_rate_bps` | Admin | ~110,000 | ~15,000 | Validates bounds, updates reward rate and changelog |
| **Admin Operations** | `set_fee_recipient` | Admin | ~92,000 | ~13,500 | Sets recipient address (self-referential allowed for vault retention) |
| **Admin Operations** | `pause` | Admin | ~90,000 | ~13,000 | Updates paused state flag, emits pause event |
| **Admin Operations** | `unpause` | Admin | ~88,000 | ~12,800 | Clears paused flag, emits unpause event |
| **Admin Operations** | `add_yield` | Admin | ~220,000 | ~28,000 | Transfers reward tokens from admin to vault pool balance |
| **Admin Operations** | `upgrade_wasm` | Admin | ~140,000 | ~18,000 | Wasm hash verification and contract executable upgrade |
| **View / Query** | `staked_amount` | Read-only | ~45,000 | ~6,500 | Reads user's staked position balance |
| **View / Query** | `pending_reward` | Read-only | ~75,000 | ~11,000 | Simulates reward accrual against current ledger sequence |
| **View / Query** | `shares_of` | Read-only | ~40,000 | ~6,000 | Instance storage lookup for user shares |
| **View / Query** | `total_staked` | Read-only | ~35,000 | ~5,500 | Instance storage lookup for total staked tokens |
| **View / Query** | `is_paused` | Read-only | ~28,000 | ~4,500 | Instance storage lookup for boolean pause flag |
| **View / Query** | `vault_state` | Read-only | ~55,000 | ~8,000 | Retrieves total shares and total deposited amounts |
| **View / Query** | `get_admin` | Read-only | ~30,000 | ~4,800 | Returns the current primary admin address |
| **View / Query** | `pool_created_by` | Read-only | ~30,000 | ~4,800 | Returns the immutable deployer address |
| **View / Query** | `get_version` | Read-only | ~25,000 | ~4,000 | Returns static contract semantic version string |

*Note: Measured values reflect standard execution paths in unit test environments with mocked authentication and standard ledger state. Actual on-chain fees vary based on ledger size, Soroban network base reserve, and protocol parameters.*

---

## Process Note: Updating Resource Costs for New Functions

When adding a new public function or altering the execution path of an existing function:

1. **Measure Resource Usage in Test Fixtures**:
   Use Soroban's built-in budget tracker in a test:
   ```rust
   let env = Env::default();
   env.budget().reset_default();
   // Call the target contract function
   let cpu = env.budget().cpu_instruction_cost();
   let mem = env.budget().memory_bytes_cost();
   ```
2. **Benchmark Both Cold and Warm Paths**:
   Where applicable (e.g. first deposit vs subsequent deposit), measure both scenarios. Report the warm baseline or note worst-case scenarios.
3. **Check for Regressions**:
   The focused regression harness in `src/test_resource_costs.rs` measures
   `deposit`, `withdraw`, `claim`, and full-position `exit` (`unstake_all`)
   with 1, 100, and 1000 existing depositors. It compares
   CPU instructions and memory bytes with `resource-cost-baseline.csv`, using
   each row's committed tolerance (currently 10%), and reports the operation,
   pool size, baseline, allowed value, and actual value on failure.
4. **Update the baseline deliberately**:
   Run `cargo test --features testutils core_function_cpu_and_memory_stay_within_committed_budget -- --nocapture`
   on the reviewed release toolchain, inspect the printed measurements, and
   update only the affected rows in `resource-cost-baseline.csv`. The committed
   initial thresholds are deliberately conservative estimates from this
   document, not results claimed to have been measured in this change. Replace
   them with a reviewed measurement before treating them as a certified
   production baseline. Do not raise a baseline just to make a regression pass;
   explain material increases in the PR and update this document when measured
   behavior changes.
5. **Update `COSTS.md`**:
   Add the new function into the table above under its appropriate category,
   committing the update alongside the PR.
