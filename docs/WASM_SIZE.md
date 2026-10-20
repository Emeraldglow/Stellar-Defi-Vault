# Vault Wasm size

Build the deployable vault artifact with:

```sh
cargo build --target wasm32-unknown-unknown --release --features vault-wasm
wc -c target/wasm32-unknown-unknown/release/stellar_defi_vault.wasm
```

## Size budget gate

CI fails when the optimized binary exceeds the budget stored in
`.github/wasm-size-budget`. The budget is a plain integer (bytes) on a
single line, committed alongside the workflow so it is reviewed in PRs like
any other change.

The current budget is **200,000 bytes (200 KB)**, which sits comfortably
below Soroban's deployment limit while leaving headroom for near-term
feature work.

### What the CI step reports

The "Check Wasm size budget" step in the `wasm` job writes a summary table
to the GitHub Actions job summary (visible on every PR run):

| Metric | Meaning |
|--------|---------|
| Size | Measured byte count of the release artifact |
| Budget | Value from `.github/wasm-size-budget` |
| Delta vs budget | `size − budget` (negative = within budget) |

### How to raise the budget deliberately

1. Measure the actual binary after your changes:
   ```sh
   cargo build --target wasm32-unknown-unknown --release --features vault-wasm
   wc -c target/wasm32-unknown-unknown/release/stellar_defi_vault.wasm
   ```
2. Edit `.github/wasm-size-budget` to the new value (round up to the
   nearest 10 KB to avoid churn).
3. Open a **dedicated PR** that contains only the budget bump — separate
   from the feature work that caused the increase.
4. In the PR description explain:
   - What feature added the bytes.
   - Why the increase is justified.
   - Whether any offsetting size work was attempted (dead-code removal,
     feature gating, etc.).

A reviewer must approve the budget bump before merge. The intent is to make
size growth a deliberate, visible decision rather than silent creep.

## Baseline

At upstream commit `315b0c5`, no baseline binary could be measured: the crate
linked `VaultContract`, `StakeReceiptNFT`, and the example consumer into one
Wasm artifact, producing duplicate exported contract functions. The same
revision also contains pre-existing compile failures tracked separately from
issue #605.

## Size pass

The `vault-wasm` feature now excludes the two independent contracts from the
vault artifact. The release profile was audited against Soroban's standard
size-oriented settings and already has the appropriate values:

- `opt-level = "z"`
- `lto = true`
- `codegen-units = 1`
- `strip = "symbols"`, `debug = 0`, and `panic = "abort"`

The dependency audit found one runtime dependency, `soroban-sdk`, which is
required. Two dead `DataKey` variants were removed; one was unused and the
other represented a reentrancy guard that had no implementation. The guard
now uses a compact raw `Symbol` key, keeping `DataKey` within Soroban's
50-variant contract-type limit.

The resulting byte count must be measured from the command above once the
pre-existing upstream compile failures are repaired; CI runs that exact build
so future size measurements remain reproducible.
