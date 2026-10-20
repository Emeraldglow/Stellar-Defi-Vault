# Fuzz targets (issue #661)

Coverage-guided fuzzing for the vault's core arithmetic and state transitions,
complementing the `proptest` property tests. Fuzzing explores the input space
with feedback, so it reaches crashes and invariant violations that a fixed
sample of hand-written or generated cases misses.

## Targets

| Target | Drives | Invariants asserted |
| --- | --- | --- |
| `share_conversion` | `balance::amount_to_shares_with_decimals`, `balance::shares_to_amount_with_decimals` | no panic; no negative balances for non-negative inputs; redeeming all shares returns exactly `total_deposited` |
| `reward_calculation` | `reward_math::reward_for_ledgers` | no panic (overflow ⇒ `ArithmeticError`); zero amount / ledgers / rate ⇒ 0; non-negative reward; monotonic in elapsed ledgers |

The targets reach the code through `stellar_defi_vault::fuzz_support`, a module
gated behind `#[cfg(any(test, feature = "testutils"))]` so none of it lands in
the deployed WASM (issue #658).

## Running locally

`cargo-fuzz` needs a nightly toolchain and `libfuzzer`:

```bash
rustup toolchain install nightly
cargo install cargo-fuzz

# Run one target until it finds a crash (Ctrl-C to stop):
cargo +nightly fuzz run share_conversion

# Time-boxed run, as CI does:
cargo +nightly fuzz run reward_calculation -- -max_total_time=120
```

To confirm the targets still compile without a nightly toolchain / libFuzzer,
which is what the `fuzz-check` Make target and the fast CI job do:

```bash
cargo check --manifest-path fuzz/Cargo.toml
```

## CI

`.github/workflows/fuzz.yml` runs each target for a bounded time on a schedule
(and on demand via `workflow_dispatch`) so a regression is caught within a day
without blocking every PR. Crashes are uploaded as artifacts.

## Invariants

The asserted invariants are the ones written down in
[`docs/INVARIANTS.md`](../docs/INVARIANTS.md); if an invariant changes there,
update the matching assertion here in the same PR.
