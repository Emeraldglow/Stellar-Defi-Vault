# Code Coverage

Line coverage is measured with [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov)
and published on every pull request (issue #663). Reviewers use it to see which
new code is untested; the CI job fails if total line coverage drops below the
recorded threshold.

## Running locally

```bash
rustup component add llvm-tools-preview   # required by cargo-llvm-cov
cargo install cargo-llvm-cov --locked

# Tests only build with the `testutils` feature — always pass it.
cargo llvm-cov --features testutils                       # HTML report
cargo llvm-cov --features testutils --summary-only        # console summary
cargo llvm-cov --features testutils --lcov --output-path lcov.info
cargo llvm-cov --features testutils --html                # open target/llvm-cov/html
```

`make coverage` runs the summary; `make coverage-lcov` writes `lcov.info` for
editors (VS Code Coverage Gutters, etc.).

### Why the feature flag is required

The Soroban test host lives behind the crate's `testutils` feature
(`[features] testutils = ["soroban-sdk/testutils"]`). Without it the test
binaries do not compile, so `cargo llvm-cov` would report zero coverage. CI and
the Makefile always pass `--features testutils`.

## CI

`.github/workflows/ci.yml` job `coverage`:

1. installs `llvm-tools-preview` and `cargo-llvm-cov`;
2. runs the suite under instrumentation with `--features testutils`;
3. writes `lcov.info` and appends the summary to the workflow's step summary so
   it is visible directly on the pull request;
4. enforces the threshold with `cargo llvm-cov report --fail-under-lines`;
5. uploads `lcov.info` as the `coverage-lcov` artifact for inspection.

## Threshold and baseline

The no-regression threshold is the integer in `.github/coverage-threshold`
(currently `60`, percent of lines). The policy:

- **No pull request may lower total line coverage below the threshold.** The
  `coverage` job fails otherwise.
- **The baseline is raised, never silently lowered.** After a run that lifts
  coverage, update `.github/coverage-threshold` to the new measured total in the
  same pull request, so the ratchet moves forward.
- `60` is the agreed minimum floor. The first CI run prints the measured total
  in the step summary; pin the threshold to that value (rounded down) once it is
  known so the floor reflects reality rather than the provisional start.

To read the current measured value locally:

```bash
cargo llvm-cov --features testutils --summary-only | tail -n 1
```

## README badge

The `coverage` badge in `README.md` reflects the enforced floor and links to the
CI workflow. It is a floor badge, not a live number: it stays green as long as CI
passes. The percentage in the badge mirrors `.github/coverage-threshold` — update
both together when the threshold changes. If the project later adopts a coverage
service (Codecov, Coveralls), swap the badge for the service's dynamic badge and
keep the threshold check in CI as the source of truth.
