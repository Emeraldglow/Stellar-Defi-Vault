# Vault Wasm size

Build the deployable vault artifact with:

```sh
cargo build --target wasm32-unknown-unknown --release --features vault-wasm
wc -c target/wasm32-unknown-unknown/release/stellar_defi_vault.wasm
```

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

The resulting byte count must be filled from the command above once the
pre-existing upstream compile failures are repaired; CI runs that exact build
so future size measurements remain reproducible.
