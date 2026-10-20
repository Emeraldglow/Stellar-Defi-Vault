# Stellar DeFi Vault

[![CI](https://github.com/YOUR_ORG/stellar-defi-vault/actions/workflows/ci.yml/badge.svg)](https://github.com/YOUR_ORG/stellar-defi-vault/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-%E2%89%A560%25-brightgreen)](https://github.com/YOUR_ORG/stellar-defi-vault/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
[![Stellar Wave](https://img.shields.io/badge/Stellar-Wave%20Program-blue)](https://www.drips.network/wave/stellar)

A non-custodial, share-based DeFi yield vault built on **Stellar** using **Soroban** smart contracts (Rust). Users deposit a Stellar token and receive proportional vault shares in return. Shares accrue value as yield is added to the vault, and can be redeemed at any time for the underlying token. The contract also exposes staking-oriented helpers for governance vote snapshots, minimum stake enforcement, reward claims, and time-based reward boosts.

## Architecture

```
VaultContract
├── initialize(admin, token, stake_decimals?, reward_decimals?) — one-time setup (stake precision is queried from token)
├── deposit(depositor, amount) — mint shares proportional to pool
├── stake(staker, amount)      — staking-friendly alias for deposit
├── withdraw(user, shares)     — burn shares, return tokens
├── unstake(staker, shares)    — staking-friendly alias for withdraw
├── claim(staker)              — claim accrued reward tokens
├── calc_pending_reward(user)  — read-only pending rewards
├── vote_weight_at(user, lgr)  — historical governance weight
├── current_vote_weight(user)  — current governance weight
├── total_vote_weight()        — pool-wide governance weight
├── preview_redeem(shares)     — read-only: how much would I get?
├── vault_state()              — total shares & total deposited
├── set_min_stake(amount)      — admin dust-position control
├── set_boost_schedule(tiers)  — admin reward multiplier tiers
├── pause() / unpause()        — admin circuit breaker
└── transfer_admin(new_admin)  — rotate admin key
```

### Share Price Formula

```
shares_minted = amount × (total_shares / total_deposited)   # existing pool
shares_minted = amount scaled from token_decimals to 7     # first deposit

amount_returned = shares × (total_deposited / total_shares)
```

This is the same ratio model used by ERC-4626 vaults, adapted for Soroban.
Token amounts are stored in the underlying token's base units. Shares always
use seven decimal places internally, so the vault queries the token's actual
`decimals()` value during initialization rather than assuming every token uses
Stellar's common seven-decimal precision. The legacy `stake_decimals` argument
is retained for ABI compatibility but does not override the token query.

## Trust Assumptions and Admin Powers

Depositors must trust the current admin key, and any configured Treasurer,
Pauser, or Rater role holder. The following admin-controlled actions are
available. Unless marked **delayed**, they take effect in the transaction that
calls them:

- **Vault operation:** `pause`, `unpause`, `pause_until`,
  `set_max_pause_duration`, `initiate_sunset`, `emergency_stop`, and
  `start_graceful_shutdown` can stop deposits, stop withdrawals, or permanently
  close new deposits. `pause_until` is lazy and auto-unpauses at its target
  ledger; `unpause` always clears that schedule. A configured pause-duration
  limit lets anyone force an overdue pause open. `enter_withdraw_only_mode`
  blocks all inflows (deposits, staking, compounding, locking) while keeping
  withdrawals and claims open and fee-free; `exit_withdraw_only_mode` reverses
  it.
- **Economics:** `set_reward_rate_bps`, `set_referral_bonus_bps`,
  `set_unstake_fee_bps`, `set_fee_recipients`, `set_treasury_split`,
  `set_boost_schedule`, `set_lock_boost_schedule`, `set_rounding_policy`,
  `set_pool_cap`, `set_min_stake`, withdrawal limits, claim fees, insurance
  rates, reward smoothing, halving, and other rate/fee/cap setters can change
  future returns, fees, eligibility, or the rate at which rewards are paid.
- **Assets and treasury:** `set_reward_token`, `add_yield`, reward funding,
  treasury withdrawals, fee routing, buyback configuration, output-token and
  bridge configuration, and `rescue_token` can change reward liquidity or move
  treasury and non-vault assets. `rescue_token` cannot move the configured stake
  or reward token. `add_yield` moves tokens into the vault; it does not withdraw
  user principal.
- **Access and administration:** `transfer_admin`, `set_emergency_admin`,
  `revoke_emergency_admin`, `set_roles`, `grant_role`, `revoke_role`,
  `initialize_multisig`, `set_timelock_delay`, and admin recovery change who
  can exercise these powers. `upgrade_wasm` can replace the contract code.
- **User-state and migration:** `slash`, `schedule_vesting`, KYC/allowlist
  batches, `export_state`, and `import_state` can affect individual positions,
  eligibility, or migration. `slash` can reduce a user's position; it does not
  require that user's signature. `set_dust_threshold`, `set_dust_keeper_tip_bps`
  and `set_dust_treasury` let any keeper close a position worth less than the
  threshold without the owner's signature; the value is paid to the owner (or
  to the dust treasury if the admin sets one), less the keeper tip of at most
  10%.
- **Delayed controls:** `queue_action` followed by `execute_action` is delayed
  by the configured ledger timelock for supported rate and pause actions.
  Multisig proposals require their approval threshold but are not an
  additional time delay. Admin recovery has its own delay. The timelock can be
  set to zero, so delayed actions are not an absolute guarantee.

The admin cannot withdraw another user's shares or principal through the normal
withdrawal path: user withdrawals require the user's own authorization. Read
the full function-by-function model, shutdown behavior, and failure scenarios
in [`docs/SECURITY.md`](docs/SECURITY.md). This section and that document must
be updated whenever an admin-gated function, role, or admin-controlled
parameter is added or changed.

## Getting Started

### Dev container (recommended)

The repo ships a [dev container](https://containers.dev/) in `.devcontainer/` with
everything preinstalled: the Rust toolchain pinned in `rust-toolchain.toml`,
the `wasm32-unknown-unknown` target, `rustfmt`/`clippy`/`rust-analyzer`, and the
Stellar CLI.

- **VS Code:** install the *Dev Containers* extension, open the repo, then run
  **Dev Containers: Reopen in Container**.
- **GitHub Codespaces:** **Code → Codespaces → Create codespace on main**.
- **CLI:** `devcontainer up --workspace-folder .` then
  `devcontainer exec --workspace-folder . make test`.

Inside the container, `make build` and `make test` work with no further setup.
When `rust-toolchain.toml` changes, update `RUST_TOOLCHAIN` in
`.devcontainer/devcontainer.json` (and the Dockerfile default) to match.

### Prerequisites (without the dev container)

```bash
rustup target add wasm32-unknown-unknown
# Stellar CLI, needed for deploy/stake/claim scripts:
# https://developers.stellar.org/docs/tools/cli/install-cli
```

### Build

```bash
cargo build --target wasm32-unknown-unknown --release
```

### Test

```bash
cargo test --features testutils
```

### Lint

```bash
cargo fmt --check
cargo clippy --features testutils -- -D warnings
```

### Coverage, static analysis, and mutation testing

Quality tooling complements the test suite; each has a doc and a `make` target:

```bash
make coverage   # line coverage summary (cargo-llvm-cov)   — docs/COVERAGE.md
make scout      # Scout static analysis (Soroban)          — docs/STATIC_ANALYSIS.md
make mutants    # mutation testing of the accounting core  — docs/MUTATION_TESTING.md
```

- [docs/COVERAGE.md](docs/COVERAGE.md) — coverage on every PR, the no-regression
  threshold in `.github/coverage-threshold`, and how to reproduce it locally.
- [docs/STATIC_ANALYSIS.md](docs/STATIC_ANALYSIS.md) — Scout in CI, how to run it
  locally, and the finding-triage allowlist.
- [docs/MUTATION_TESTING.md](docs/MUTATION_TESTING.md) — `cargo-mutants` config,
  the baseline, and how survivors are triaged.
- [docs/INVARIANTS.md](docs/INVARIANTS.md) — the precise accounting and
  share-price invariants that these tools protect, mapped to tests.

### Deploying to Testnet

To deploy the staking vault to Stellar Testnet and initialize it:

1. **Deploy and Initialize**:
   Run the deployment script. By default, it will generate a new deployment identity (`deployer`), fund it via Friendbot, build the optimized contract WASM, deploy the contract, deploy the native XLM wrapper contract (or resolve its existing ID), and initialize the vault.

   ```bash
   make deploy-testnet
   ```

   _Alternatively, you can customize the identity or network via environment variables:_

   ```bash
   IDENTITY=my-identity NETWORK=testnet make deploy-testnet
   ```

2. **Configure your Environment**:
   The script will print configuration variables. Save them to a `.env` file in the project root:

   ```env
   CONTRACT_ID=CB...
   TOKEN_ID=CD...
   IDENTITY=my-identity
   NETWORK=testnet
   ```

3. **Staking via CLI**:
   Stake tokens into the vault (amount in raw units, e.g., 10 XLM = `100000000` stroops):

   ```bash
   make stake AMOUNT=100000000
   ```

4. **Claiming Rewards**:
   Claim accrued rewards from the vault:
   ```bash
   make claim
   ```

## Contract Interface

| Function                                                      | Auth Required | Description                                              |
| ------------------------------------------------------------- | ------------- | -------------------------------------------------------- |
| `initialize(admin, token, stake_decimals?, reward_decimals?)` | —             | One-time init; stake decimals are queried from the token |
| `deposit(depositor, amount)`                                  | depositor     | Deposit tokens, receive shares                           |
| `stake(staker, amount)`                                       | staker        | Alias for `deposit`                                      |
| `withdraw(user, shares)`                                      | user          | Burn shares, receive tokens                              |
| `unstake(staker, shares)`                                     | staker        | Alias for `withdraw`                                     |
| `claim(staker)`                                               | staker        | Claim accrued rewards from the reward pool               |
| `calc_pending_reward(user)`                                   | —             | Pending reward query                                     |
| `shares_of(user)`                                             | —             | Query share balance                                      |
| `current_vote_weight(user)`                                   | —             | Current governance vote weight                           |
| `vote_weight_at(user, ledger)`                                | —             | Historical governance vote weight                        |
| `total_vote_weight()`                                         | —             | Pool-wide governance vote weight                         |
| `preview_redeem(shares)`                                      | —             | Preview token return                                     |
| `vault_state()`                                               | —             | Query pool totals                                        |
| `set_min_stake(amount)`                                       | admin         | Configure minimum stake; `0` disables it                 |
| `get_min_stake()`                                             | —             | Read current minimum stake                               |
| `set_unstake_fee_bps(admin, bps)`                             | admin         | Configure unstake fee (max 500 bps); `0` disables it     |
| `get_unstake_fee_bps()`                                       | —             | Read current unstake fee in bps                          |
| `set_reward_rate_bps(rate_bps)`                               | admin         | Configure base reward APR                                |
| `fund_reward_pool(admin_addr, amount)`                        | admin         | Deposit claimable rewards                                |
| `set_boost_schedule(tiers)`                                   | admin         | Configure up to 5 reward-boost tiers                     |
| `get_boost_multiplier(user)`                                  | —             | Current reward multiplier for a user                     |
| `pause()`                                                     | admin         | Emergency pause                                          |
| `unpause()`                                                   | admin         | Resume operations                                        |
| `add_yield(admin_addr, amount)`                               | admin         | Inject yield; raises share price                         |
| `transfer_admin(new_admin)`                                   | admin         | Rotate admin key                                         |

## Using the CLI Helper

The repo includes [`scripts/pool.sh`](./scripts/pool.sh), an interactive helper for the most common pool operations:

- `stake`
- `unstake`
- `claim`
- `position`
- `pending`
- `pool-info`

The script reads `CONTRACT_ID` and `IDENTITY` from your shell environment or a local `.env` file:

```bash
CONTRACT_ID=CB...YOUR_CONTRACT_ID
IDENTITY=alice
NETWORK=testnet
```

You can run it interactively:

```bash
scripts/pool.sh
```

Or invoke a specific action directly:

```bash
scripts/pool.sh stake 25000000 GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF
scripts/pool.sh --dry-run pending GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF
```

Example output:

```text
$ scripts/pool.sh position GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF
Address: GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF
Staked shares: 2.5000000 (25000000 raw)
Pending reward: 0.1375000 (1375000 raw)
Boost multiplier: 11000 bps
```

## Events

Every emitted event indexes an event-type symbol in topic 0 and its primary
affected address in topic 1. Pool-wide events use the vault contract address;
admin audit events additionally index the action type in topic 2.

See [docs/EVENTS.md](./docs/EVENTS.md) for the topic conventions and event
reference used by off-chain indexers.

## Roadmap / Open Issues

The following features are planned and tracked as open issues — great targets for Wave contributors:

- [ ] Yield accrual mechanism (admin deposits yield into the pool)
- [ ] Deposit/withdraw fee with configurable basis points
- [ ] Maximum deposit cap per user
- [ ] Multi-token support
- [ ] Testnet deployment script
- [ ] Integration tests against Stellar testnet

See [Issues](../../issues) for the full list, including those tagged **`Stellar Wave`**.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for setup instructions and the Wave contribution workflow.

## Storage

See [docs/STORAGE.md](./docs/STORAGE.md) for the full audit of which Soroban
storage type (instance, persistent, or temporary) each piece of contract
state uses and why, including known scaling risks around the depositor
registry.

## Fuzzing

Coverage-guided fuzz targets for share conversion and reward math live in
[`fuzz/`](./fuzz/) and complement the `proptest` property tests. They need a
nightly toolchain and [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz):

```bash
cargo install cargo-fuzz
cargo +nightly fuzz run share_conversion                  # until it finds a crash
cargo +nightly fuzz run reward_calculation -- -max_total_time=120
```

`make fuzz-check` compiles the targets without nightly, and
`.github/workflows/fuzz.yml` runs a bounded pass daily. See
[`fuzz/README.md`](./fuzz/README.md) for the invariants each target asserts.

## Security

See [docs/SECURITY.md](./docs/SECURITY.md) for the full security model, including:

- Complete list of admin-only functions and their effects
- What the admin can and cannot do (the admin **cannot** access user principal)
- Failure scenarios: paused vault, halted yield, key compromise
- Admin key rotation procedure via `transfer_admin`

This contract is unaudited. Do not use in production without an independent security audit. If you find a vulnerability, please open a private [GitHub Security Advisory](../../security/advisories/new) rather than a public issue.

## Known Limitations & Remediations

### Reward Rounding Dust Loss (Resolved in Issue #650)

In fixed-point math, calculating reward using standard integer division previously led to rounding dust truncation where small stakes over short periods truncated to 0.

#### Resolution: High-Precision Reward-Per-Share Accumulator
Reward accrual now utilizes the standard accumulator pattern: a global `reward_per_share` index scaled by `ACCUMULATOR_SCALE = 1_000_000_000_000` (1e12), with each position storing a checkpoint:
- **O(1) Accrual**: Updated on every stake, unstake, claim, and rate change without loop iteration.
- **Dust Elimination**: Scaled index tracks sub-stroop accrual with 12 decimal places of sub-unit precision, preventing loss of fractional rewards.

#### Scaling Constant & Overflow Headroom Analysis
- **Scale Factor**: `10^12`
- **Integer Storage**: Signed 128-bit integer (`i128::MAX` ≈ `1.7014 * 10^38`)
- **Maximum Stellar Total Supply / TVL**: `10^11` XLM = `10^18` stroops
- **Maximum Configurable APR**: `50,000` bps = 500% APR
- **Accumulator Growth**: Over 100 continuous years at 500% APR, the scaled index increases by:
  `100 * 5.0 * 10^12 = 5 * 10^14`
- **Multiplication Headroom**:
  When calculating a staker's pending reward, the intermediate product is:
  `user_shares * delta_index <= 10^18 * (5 * 10^14) = 5 * 10^32`
- **Headroom Margin**:
  `i128::MAX / (5 * 10^32) ≈ 340,282` (> 340,000x safety headroom)
  The accumulator is mathematically incapable of overflowing under any realistic protocol lifetime and parameters.

## License

[MIT](./LICENSE)

# Stellar-Defi-Vault

## Gas & Resource Costs

Approximate CPU instruction and RAM byte costs for each public function are tracked in **[COSTS.md](./COSTS.md)** based on Soroban's test environment budget reporter. Integrators can consult this table when calculating transaction fee buffers.

## Deterministic Build & Bytecode Verification

To support trust minimization and independent verification, the contract Wasm binary is byte-reproducible across different host machines:

- **Path Normalization**: Host file system paths are remapped using `--remap-path-prefix` in `.cargo/config.toml`.
- **Deterministic Codegen**: Codegen units are pinned to `codegen-units = 1` with LTO enabled.

### Independent Verification Procedure

Any third party can verify that an on-chain deployed contract bytecode matches this source tree:

1. Clone the repository and checkout the target release commit or tag:
   ```bash
   git clone https://github.com/Rickyy1017/Stellar-Defi-Vault.git
   cd Stellar-Defi-Vault
   git checkout <release-tag-or-commit>
   ```
2. Verify with the pinned Rust toolchain (1.81.0) and WASM target:
   ```bash
   ./scripts/verify-build-reproducibility.sh
   ```
3. Generate the SHA-256 digest:
   ```bash
   sha256sum target/wasm32-unknown-unknown/release/stellar_defi_vault.wasm
   ```
4. Compare this digest with the contract code hash published on the Stellar ledger explorer.

## Testnet Integration Tests

You can run the integration test suite against the Stellar Testnet by executing:

```bash
./scripts/integration-test.sh
```

Note: Ensure you have the Stellar CLI configured with testnet credentials before running this script.
