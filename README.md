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
make coverage              # line coverage summary (cargo-llvm-cov)   — docs/COVERAGE.md
make scout                 # Scout static analysis (Soroban)          — docs/STATIC_ANALYSIS.md
make mutants                # mutation testing of the accounting core  — docs/MUTATION_TESTING.md
make interface-spec-check  # fail if the public interface drifted     — docs/INTERFACE_SPEC.md
make interface-spec        # regenerate the committed snapshot        — docs/INTERFACE_SPEC.md
```

- [docs/COVERAGE.md](docs/COVERAGE.md) — coverage on every PR, the no-regression
  threshold in `.github/coverage-threshold`, and how to reproduce it locally.
- [docs/STATIC_ANALYSIS.md](docs/STATIC_ANALYSIS.md) — Scout in CI, how to run it
  locally, and the finding-triage allowlist.
- [docs/MUTATION_TESTING.md](docs/MUTATION_TESTING.md) — `cargo-mutants` config,
  the baseline, and how survivors are triaged.
- [docs/INVARIANTS.md](docs/INVARIANTS.md) — the precise accounting and
  share-price invariants that these tools protect, mapped to tests.
- [docs/INTERFACE_SPEC.md](docs/INTERFACE_SPEC.md) — the committed public-interface
  snapshot (`interface-spec/VaultContract.json`), generated from the real release
  build so any change to an exported function's name, parameters, or types shows
  up as a CI-failing diff instead of shipping silently; see
  [CHANGELOG.md](CHANGELOG.md) for the entry an intentional change needs
  alongside the refreshed snapshot.

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

## Reward Pool Runway Simulation

[`scripts/runway-sim.ts`](./scripts/runway-sim.ts) projects when the reward
pool would run dry under three TVL growth scenarios (low / base / high), so
an operator can size `fund_reward_pool` ahead of time instead of guessing.
It uses the exact same pool-wide APR math as `runway_guard.rs`'s
`projected_runway_ledgers` (`BOOST_BPS_BASE`, `STELLAR_LEDGERS_PER_YEAR`, and
`LEDGERS_PER_DAY`, mirrored as plain constants at the top of the script —
see its own header comment if those ever change on-chain).

It's a dependency-free TypeScript file — no `package.json` or `npm install`
needed — runnable directly with [`tsx`](https://github.com/privatenumber/tsx)
or `ts-node`:

```bash
npx tsx scripts/runway-sim.ts --tvl 1000000 --rate-bps 800 --reward-pool 15000
```

Pool state can instead be read live from a deployed contract via the Stellar
CLI, using the same `CONTRACT_ID` / `IDENTITY` / `NETWORK` convention as
`scripts/pool.sh`. `IDENTITY` is required here too — the CLI needs a source
account even for a read-only simulated call:

```bash
CONTRACT_ID=CB...YOUR_CONTRACT_ID IDENTITY=alice npx tsx scripts/runway-sim.ts --network testnet
```

Model a prospective top-up with `--fund`, and widen or narrow the projection
with `--months` and the `--low-growth`/`--base-growth`/`--high-growth`
annual rates (e.g. `--base-growth 0.5` for +50%/year):

```text
$ npx tsx scripts/runway-sim.ts --tvl 1000000 --rate-bps 500 --reward-pool 50000 --months 12
Pool state: TVL=1,000,000 rate_bps=500 reward_pool=50,000

Month  low (-25%/yr)  base (50%/yr)  high (200%/yr)
-----  -------------  -------------  --------------
    0       50000.00       50000.00        50000.00
    1       45939.09       45819.80        45693.41
    2       41974.37       41495.93        40973.94
    3       38103.57       37023.48        35802.00
    4       34324.46       32397.32        30134.21
    5       30634.88       27612.18        23923.04
    6       27032.69       22662.60        17116.38
    7       23515.84       17542.91         9657.15
    8       20082.29       12247.29         1482.79
    9       16730.08        6769.67            0.00
   10       13457.28        1103.81            0.00
   11       10262.00           0.00            0.00
   12        7142.42           0.00            0.00

low   (-25%/yr TVL growth): survives the full 12-month horizon
base  (50%/yr TVL growth): exhausted around month 11
high  (200%/yr TVL growth): exhausted around month 9
```

Run `npx tsx scripts/runway-sim.ts --help` for the full option list. The
output above is from an actual run of the script, not fabricated — try the
same command to reproduce it.

## Seeding Demo Accounts

[`scripts/seed-testnet.sh`](./scripts/seed-testnet.sh) creates and
friendbot-funds `N` throwaway test-network accounts, mints each one a test
token, and deposits it into the deployed vault — useful for demos and
manual QA when you need several positions in the pool without hand-running
the same CLI calls over and over.

```bash
scripts/seed-testnet.sh --accounts 5 --amount 250000000 --contract-id "$CONTRACT_ID"
```

By default it deploys a fresh, freely-mintable test token (a Stellar Asset
Contract wrapping a classic asset it issues itself) and mints `--amount`
(raw base units, 7 decimals, same convention as `scripts/stake.sh`) of it to
each new account before depositing that same amount into the vault. Pass
`--token-id` + `--token-issuer` to mint from an already-deployed test token
instead. `--accounts`, `--amount`, and `--contract-id` (or `$CONTRACT_ID`)
are the only required inputs; run with `--help` for the rest (`--prefix`,
`--asset-code`, `--network`).

It **never runs against mainnet**: the network (`--network`, or `$NETWORK`,
default `testnet`) must be one of `testnet` / `futurenet` / `local` /
`standalone`, checked both by name and, as a second, independent check,
against the network's own passphrase — and there is no flag or environment
variable to bypass either check.

Each account is seeded independently: one failing (e.g. a flaky friendbot
request) doesn't stop the rest, and the final summary table marks exactly
which step failed for it:

```text
Summary
Name          Address                                                   Minted          Shares          Status
------------  --------------------------------------------------------  --------------  --------------  ------
seed-1        GA2SAV4ISWSF42FWFFMA5C54EMHFPZWG6S2JRPLW53HXK3GDAMYMJW54  250000000       2.5000000       ok
seed-2        GC3QSFHWDI7G3MFNCNTHAS3FCJ5KAJMJSAPLPGOMTAERFKNM77GSO737  250000000       2.5000000       ok

2/2 accounts fully seeded (created, funded, minted, deposited).
```

(Account creation, funding, and minting above are from an actual testnet
run; the deposit/shares columns are illustrative, since reproducing them
needs a deployed vault contract ID.)

## Pool Health Dashboard Endpoint

[`scripts/pool-status.ts`](./scripts/pool-status.ts) polls the vault's
read-only functions (TVL, reward rate, reward pool balance, projected
runway, pause state) and prints a single JSON document to stdout —
suitable for a status page, or as a monitoring check in its own right:
it exits non-zero when a configured alert threshold is breached (low
runway, or the pool being paused).

```bash
CONTRACT_ID=C... IDENTITY=alice npx tsx scripts/pool-status.ts
```

```json
{
  "timestamp": "2026-10-20T00:00:00.000Z",
  "network": "testnet",
  "contractId": "C...",
  "decimals": 7,
  "tvl": 1000000,
  "tvlRaw": "10000000000000",
  "rateBps": 500,
  "rewardPoolBalance": 50000,
  "rewardPoolBalanceRaw": "500000000000",
  "projectedRunwayLedgers": 980000,
  "paused": false,
  "alerts": [],
  "ok": true
}
```

(Shape and field names above are illustrative — reproducing real values
needs an actual deployed vault contract. The argument parsing, help
text, and error/exit-code paths were verified against the real Stellar
CLI on testnet while writing this script; a successful fetch against a
live vault was not, since that needs a deployed contract.)

Mainnet has no default RPC endpoint in the Stellar CLI ("Bring Your
Own"), so point at your own provider explicitly:

```bash
CONTRACT_ID=C... IDENTITY=ops npx tsx scripts/pool-status.ts \
  --network mainnet \
  --rpc-url https://your-mainnet-rpc-provider.example \
  --network-passphrase "Public Global Stellar Network ; September 2015"
```

Run `npx tsx scripts/pool-status.ts --help` for every option, including
`--min-runway-ledgers` and `--no-alert-on-pause`.

### Cron usage

```cron
# Every 15 minutes, append a status line and alert (via the shell's own
# mailing, or swap the `||` branch for your paging tool) on exit code 1
# or 2.
*/15 * * * * CONTRACT_ID=C... IDENTITY=alice npx tsx /path/to/scripts/pool-status.ts >> /var/log/pool-status.jsonl || echo "pool-status alert: see /var/log/pool-status.jsonl" | mail -s "Vault alert" ops@example.com
```

### CI usage (GitHub Actions)

```yaml
name: Pool health check
on:
  schedule:
    - cron: "*/15 * * * *"
jobs:
  pool-status:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
      - name: Install Stellar CLI
        run: cargo install --locked stellar-cli
      - name: Poll pool status
        env:
          CONTRACT_ID: ${{ secrets.VAULT_CONTRACT_ID }}
          IDENTITY: ${{ secrets.VAULT_MONITOR_IDENTITY }}
          NETWORK: mainnet
          RPC_URL: ${{ secrets.MAINNET_RPC_URL }}
          NETWORK_PASSPHRASE: "Public Global Stellar Network ; September 2015"
        run: npx tsx scripts/pool-status.ts | tee pool-status.json
      # A non-zero exit above (alert breached, or fetch failure) fails
      # this step and the workflow run, which is itself the alert signal
      # most CI-based paging integrations watch for.
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: pool-status
          path: pool-status.json
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

A persistent entry — the vault's own instance storage, or a specific
user's position/checkpoint/settings entries — can be archived if its TTL
runs out before anyone extends it. See
[docs/RESTORE.md](./docs/RESTORE.md) for how to recognize that happened
(reads fail clearly rather than returning a default) and the CLI steps to
restore it.

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
- Vulnerability disclosure policy: private reporting channel, response SLA,
  scope, severity levels, and (currently unfunded) bounty terms

This contract is unaudited. Do not use in production without an independent security audit. If you find a vulnerability, please report it privately via a [GitHub Security Advisory](../../security/advisories/new) rather than a public issue — see [docs/SECURITY.md](./docs/SECURITY.md#vulnerability-disclosure-policy) for the full policy, or [.github/SECURITY.md](./.github/SECURITY.md) for GitHub's own security tab.

See [docs/AUDIT_READINESS.md](./docs/AUDIT_READINESS.md) for the checklist this project works through before engaging an external auditor, and for the defined commit/tag an audit is actually scoped against once one is underway.

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
