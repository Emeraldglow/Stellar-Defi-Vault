# Scout Static Analysis

[Scout](https://github.com/CoinFabrik/scout-audit) (by CoinFabrik) is an
open-source static analyser for Soroban contracts. It flags common
vulnerability patterns — authorization mistakes, denial-of-service shapes,
unsafe arithmetic, best-practice deviations — automatically on every pull
request (issue #664).

## Running locally

Scout is distributed on crates.io as `cargo-scout-audit`. It relies on
rustup-managed toolchain components (its detectors are built with a pinned
nightly), so install Rust with rustup first.

```bash
# Install the CLI (pin a version in automation; see the CI workflow).
cargo install cargo-scout-audit --locked --version 0.3.17

# Analyse the project. Run from the directory containing Cargo.toml.
cargo scout-audit

# Useful variants
cargo scout-audit --list-detectors            # what can run against this project
cargo scout-audit --output-format json        # machine-readable findings
cargo scout-audit --output-format sarif       # what CI consumes
cargo scout-audit --output-format md          # markdown report for review
```

`make scout` runs the default analysis. `make scout-sarif` writes
`scout.sarif` for the triage script.

### Offline / reproducible runs

Scout sets the Soroban build target for you. To silence its telemetry and update
check, set `SCOUT_OFFLINE=1`. A fully network-isolated run additionally needs
cached Cargo dependencies and local detector sources:

```bash
SCOUT_OFFLINE=1 cargo scout-audit \
  --local-detectors /path/to/scout-audit/nightly \
  --scout-source /path/to/scout-audit
```

The published Docker image (`coinfabrik/scout:0.3.17`) bundles the pinned
toolchain and detectors if you prefer a container:

```bash
docker run --rm --platform linux/amd64 \
  -e INPUT_TARGET=/scoutme -e INPUT_SCOUT_ARGS="--output-format sarif" \
  -v "$PWD:/scoutme" coinfabrik/scout:0.3.17
```

## CI

`.github/workflows/ci.yml` job `scout`:

1. installs the pinned `cargo-scout-audit`;
2. runs `cargo scout-audit --output-format sarif` (and uploads the SARIF and a
   Markdown report as artifacts);
3. runs `.github/scripts/scout_triage.py`, which prints a severity summary to
   the step summary and **fails the job if any high-severity finding is not
   triaged.**

CI fails on new high-severity findings by construction: the triage script exits
non-zero for every high-severity result that the allowlist does not match.

## Triage

Findings are handled one of two ways:

1. **Fix it.** Preferred. Change the code so the detector no longer fires.
2. **Suppress with justification.** Add an entry to the tracked allowlist
   `.github/scout-triage.json`. Suppression is only valid for a high-severity
   finding that is a confirmed false positive or an accepted, documented risk.

An allowlist entry looks like:

```json
{
  "ruleId": "soroban/some-detector",
  "match": "substring of the finding message (case-insensitive, optional)",
  "justification": "Why this finding is acceptable, with a tracking issue if applicable."
}
```

Rules for entries:

- `justification` must be non-empty; an entry with an empty justification does
  **not** suppress anything.
- Keep `match` as specific as possible so the entry does not hide a *new*
  finding from the same detector.
- Never suppress a finding to make CI green without understanding it.

### Initial triage status

The allowlist starts empty. Scout cannot be run in this repository without the
detector toolchain, so the first CI run (or a local `make scout`) is what
enumerates the current findings; each one is then fixed or added to the
allowlist with a justification. This is intentional: the dependency versions and
detector set drift, so the baseline is re-established by the tool rather than
asserted by hand.

## Version pinning

The CI workflow pins `cargo-scout-audit` to a specific version. Scout's detector
toolchain is a pinned nightly and its findings can change between releases, so
bump the pin deliberately and re-triage the allowlist when you do.
