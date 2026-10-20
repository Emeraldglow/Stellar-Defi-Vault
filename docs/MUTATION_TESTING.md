# Mutation Testing

Mutation testing changes the code one small edit at a time ("a mutant") and
reruns the test suite. If the suite still passes, the mutant **survived** —
meaning the tests do not actually pin down that behaviour. Survivors show where
assertions are weak, which is exactly the question raised by a large number of
recently merged features (issue #662).

The tool is [`cargo-mutants`](https://mutants.rs/).

## Scoping

Mutation testing the whole crate would take many hours. `mutants.toml` scopes
the run to the modules that hold the accounting invariants
(`docs/INVARIANTS.md`):

```toml
examine_globs = ["src/balance.rs", "src/invariants.rs"]
exclude_globs = ["src/test*.rs"]
additional_cargo_test_args = ["--features", "testutils"]
```

`src/balance.rs` holds the share maths and every accounting setter;
`src/invariants.rs` holds the on-chain invariant checker. These are small,
behaviour-dense, and covered by fast tests, so they are the highest-value target
for a bounded run. Widen `examine_globs` once the core run is clean.

## Running locally

```bash
cargo install cargo-mutants --locked

# Full scoped run (honours mutants.toml):
cargo mutants --config mutants.toml

# Preview what would be mutated, without running tests:
cargo mutants --config mutants.toml --list

# Run a single mutant / file while iterating:
cargo mutants --config mutants.toml --file src/balance.rs
```

Results land in `target/mutants/` (from `output_dir` in `mutants.toml`):

| File | Contents |
|---|---|
| `outcomes.json` | machine-readable result for every mutant |
| `caught.txt` | mutants the tests killed (good) |
| `missed.txt` | **survivors** — the triage worklist |
| `timeout.txt` | mutants that hung (raise `timeout` or investigate a loop guard) |
| `unviable.txt` | mutants that did not compile |

## Baseline

Before the first CI run there is no recorded baseline, because this repository
cannot run the detector toolchain as part of this change. The baseline is
established by the first run:

1. The scheduled `Mutation Testing` workflow (`.github/workflows/mutants.yml`)
   runs weekly and on manual dispatch, and uploads `target/mutants/` as the
   `mutants-report` artifact.
2. Record the caught/survived counts from that run here and in the triage log
   below. That tuple is the baseline: future runs must not regress it.

Run status: **baseline pending first scheduled run.** Until it lands, treat the
scheduled job as informational.

## Triage of survivors

Every survivor in `missed.txt` gets one of two outcomes:

1. **Add a test** that kills it. This is the default; survivors usually point at
   a missing assertion.
2. **Justify it** as an equivalent or genuinely-untestable mutant, and record it
   in the triage log with a one-line reason.

### Triage log

| Mutant (file: line / description) | Outcome | Reason / test |
|---|---|---|
| _none recorded yet — populate from the first run_ | | |

## CI

Mutation testing runs on a **schedule**, not on every pull request, because it
is slow (`.github/workflows/mutants.yml`). Trigger it manually from the Actions
tab after changing `src/balance.rs` or `src/invariants.rs`, or after adding tests
for a feature that mints or burns shares.

The scheduled job is intentionally non-fatal while the baseline is being
established: it always uploads the report and prints the survivors into the
step summary, and triage happens through this document. Once a baseline exists,
a follow-up can promote it to a mutation-score ratchet that fails the run when
the score drops.
