# Contributing to Stellar DeFi Vault

Thank you for your interest in contributing! This project is part of the **Stellar Wave Program** on Drips. All contributions that resolve tagged issues during an active Wave are eligible for rewards.

## Full Contributor Guide

For a detailed walkthrough — including environment setup, architecture overview, a worked example PR, and common mistakes — see **[docs/CONTRIBUTING_GUIDE.md](docs/CONTRIBUTING_GUIDE.md)**.

## Getting Started

### Prerequisites

- Rust (stable) — install via [rustup](https://rustup.rs/)
- `wasm32-unknown-unknown` target:
  ```bash
  rustup target add wasm32-unknown-unknown
  ```
- Stellar CLI (optional, for local deployment):
  ```bash
  cargo install --locked stellar-cli
  ```

### Setup

```bash
git clone https://github.com/YOUR_ORG/stellar-defi-vault.git
cd stellar-defi-vault
cargo build
```

### Running Tests

```bash
cargo test --features testutils
```

### Building the WASM Contract

```bash
cargo build --target wasm32-unknown-unknown --release
```

The compiled `.wasm` file will be at `target/wasm32-unknown-unknown/release/stellar_defi_vault.wasm`.

---

## How to Contribute (Wave Program)

1. Browse open issues tagged **`Stellar Wave`** at [drips.network/wave/stellar](https://www.drips.network/wave/stellar/issues).
2. Apply for an issue through the Drips Wave app.
3. Wait to be **assigned** before starting work — do not open a PR before assignment.
4. Fork the repo, create a branch named `fix/<issue-number>-short-description` or `feat/<issue-number>-short-description`.
5. Open a PR with `Closes #<issue-number>` in the description so the Drips bot can track it.
6. Respond promptly to review feedback — the Wave window is short.

## Code Style

- Run `cargo fmt` before committing.
- Ensure `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` pass.
- Pull requests must keep the formatting and clippy CI checks green; these checks
  run automatically on every pull request.
- All new functionality must include unit tests.
- Public functions must have doc comments (`///`).
- Keep the README's **Trust Assumptions and Admin Powers** section and
  `docs/SECURITY.md` synchronized whenever an admin-gated function, role, or
  admin-controlled parameter is added or changed.
- When introducing or altering public contract functions, add the signature to `VaultTrait` (`src/interface.rs`) first before implementing it on `VaultContract`.
- Update `COSTS.md` with benchmarked CPU instructions and memory consumption for any new public functions.
- Keep `CONTRACT_VERSION` in sync with `Cargo.toml` when updating the contract version.

## PR Checklist

Opening a pull request pre-fills
[`.github/pull_request_template.md`](.github/pull_request_template.md), which
covers the full checklist (tests, events, docs, lint), a storage-layout
declaration, and a section for any deferred work. The essentials:

- [ ] Tests pass (`cargo test --features testutils`)
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes with no warnings
- [ ] New logic is covered by tests
- [ ] PR description references the issue (`Closes #N`)

## Questions?

Open a [GitHub Discussion](../../discussions) or ask in the [Drips Discord](https://discord.gg/BakDKKDpHF).
