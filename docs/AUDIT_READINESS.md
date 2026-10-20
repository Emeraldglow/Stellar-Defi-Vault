# Audit Readiness Checklist

This is the gate before engaging an external auditor: everything below
should be checked off against the **frozen commit/tag** defined in
["Commit/tag to be audited"](#committag-to-be-audited), not against a
moving `main`. An auditor bills by the hour against a fixed scope — code
that changes under them during the engagement invalidates findings and
wastes their time and the project's budget.

Each item links to the document, file, or CI job that is its source of
truth. An item with no link yet is not done — add the link when it is.

## Checklist

### Scope is frozen

- [ ] A specific commit has been tagged for audit (see
      ["Commit/tag to be audited"](#committag-to-be-audited)) and
      communicated to the auditor in writing.
- [ ] No merges land on the tagged commit until the audit concludes, or
      any that must land are tracked and re-reviewed with the auditor
      explicitly (don't silently rebase the scope out from under them).
- [ ] The contract's public entrypoint surface at that commit matches
      [docs/INTERFACE.md](./INTERFACE.md) — regenerate and diff it if the
      two have drifted.

### Build and CI are green on the frozen commit

- [ ] `cargo test --features testutils` passes (CI job **Build & Test**,
      [.github/workflows/ci.yml](../.github/workflows/ci.yml)).
- [ ] The optimized release WASM builds for both
      `--features vault-wasm` and `--no-default-features`, and the
      no-default-features build carries no test-only symbols (CI job
      **WASM Build**).
- [ ] `cargo deny check advisories` passes, or every exception is
      documented with an owner and review date (CI job
      **Dependency Audit**; see
      [docs/SECURITY.md § Dependency Advisory Triage](./SECURITY.md#dependency-advisory-triage)).
- [ ] `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`
      pass (CI jobs **Rustfmt**, **Clippy**).
- [ ] `cargo doc --no-deps --lib` builds with no new undocumented public
      items (CI job **Rustdoc**).
- [ ] Coverage meets the project's no-regression threshold (CI job
      **Coverage**; see [docs/COVERAGE.md](./COVERAGE.md)).
- [ ] No duplicate or orphaned module wiring: every `mod`/`pub mod`
      declaration resolves to exactly the file it's meant to, and no two
      items share an inherent-method name on `VaultContract`. This is
      cheap to eyeball-miss in a large, many-contributor `src/vault.rs`
      and expensive for an auditor to discover instead — a clean
      `cargo build`/`cargo test` run *on the exact frozen commit*,
      checked directly (not inferred from an older green CI run), is the
      actual evidence here.

### Documented invariants

- [ ] [docs/INVARIANTS.md](./INVARIANTS.md) — the accounting and
      share-price invariants — is current against the frozen commit.
- [ ] [docs/STORAGE.md](./STORAGE.md) and
      [docs/STORAGE_LAYOUT.md](./STORAGE_LAYOUT.md) — which Soroban
      storage type each piece of state uses, and the upgrade-safety
      snapshot — are current.
- [ ] [docs/LEDGER_BOUNDARIES.md](./LEDGER_BOUNDARIES.md) — assumptions
      about ledger sequence/timestamp monotonicity and boundaries — is
      current.
- [ ] [docs/RESOURCE_LIMITS.md](./RESOURCE_LIMITS.md) — the resource
      ceilings on batch/loop entrypoints — is current.

### Threat model and security posture

- [ ] [docs/SECURITY.md](./SECURITY.md) — admin powers, what the admin
      explicitly cannot do, failure scenarios, and key rotation — is
      current against the frozen commit.
- [ ] [docs/SECURITY.md § Vulnerability Disclosure Policy](./SECURITY.md#vulnerability-disclosure-policy)
      and [.github/SECURITY.md](../.github/SECURITY.md) are published, so
      the auditor (and anyone else) has a private channel for findings
      before and after the engagement.
- [ ] [docs/RESTORE.md](./RESTORE.md) — recovery procedure for archived
      storage entries — is current.
- [ ] [docs/PUBLIC_API_ERRORS.md](./PUBLIC_API_ERRORS.md) — the
      public-facing error surface — is current, and
      [docs/EVENTS.md](./EVENTS.md) — the event/topic conventions — is
      current.

### Known issues list compiled

- [ ] The ["Known Issues"](#known-issues) section below reflects the
      frozen commit, with every accepted risk's rationale either inline
      or linked.
- [ ] [README.md § Known Limitations & Remediations](../README.md#known-limitations--remediations)
      is current (issues that *were* known and have since been resolved
      belong there, not in Known Issues below).

### Supporting analysis available

- [ ] [docs/STATIC_ANALYSIS.md](./STATIC_ANALYSIS.md) — Scout findings and
      triage — reflects a run against the frozen commit.
- [ ] [docs/MUTATION_TESTING.md](./MUTATION_TESTING.md) — `cargo-mutants`
      baseline on the accounting core — reflects a run against the frozen
      commit.
- [ ] [docs/WASM_SIZE.md](./WASM_SIZE.md) — compiled size budget — is
      current.
- [ ] Fuzz targets (`fuzz/`, [README § Fuzzing](../README.md#fuzzing))
      have run for a meaningful duration with no un-triaged crashes since
      the last dependency or math-path change.
- [ ] [README § Deterministic Build & Bytecode Verification](../README.md#deterministic-build--bytecode-verification)
      has been re-run against the frozen commit, so the auditor (and
      anyone verifying their report later) can confirm the WASM they're
      given actually corresponds to this source.

### Audit scope packaged

- [ ] The package described in
      ["Packaging the audit scope"](#packaging-the-audit-scope) below has
      been generated from the frozen commit and handed to the auditor.

## Commit/tag to be audited

No audit has been scoped yet as of this writing, so no tag exists. When
one is:

1. Confirm every box above is checked against the commit you're about to
   freeze.
2. Create an annotated tag, not a lightweight one, so the tag itself
   carries who froze it and why:

   ```bash
   git tag -a audit-candidate-YYYY-MM-DD -m "Audit scope: <auditor name/firm>, frozen <date>" <commit-sha>
   git push origin audit-candidate-YYYY-MM-DD
   ```

3. Record the tag name, the commit SHA it points at, and the auditor's
   name in this section, replacing the placeholder below, and keep that
   record even after the tag is deleted or the branch moves on — it's the
   permanent answer to "what exactly did they look at."

> **Illustrative example only — not a live audit record.** At the time
> this checklist was written, `HEAD` was `9bc3de7456c7990e562ef66a55c31a0715f6b9e7`
> on the `published` branch. No tag has been cut from it, no auditor has
> been engaged against it, and most boxes above are unchecked — this
> number exists only to show the format the real record should take.

## Known Issues

Design decisions and trust assumptions the project has already accepted,
each with its rationale and where it's documented in full. These are
*out of scope* for an auditor to re-litigate as novel findings — they're
listed here precisely so an auditor's time goes to what's actually
unreviewed instead.

- **Single-admin trust model.** One admin key (optionally a secondary
  emergency admin) controls pausing, rate/fee/cap configuration, and
  token rescue, with no on-chain timelock on most of those actions. The
  admin cannot access user principal, mint shares to itself, change the
  vault token, or re-initialize the contract — see
  [docs/SECURITY.md § What Admin Cannot Do](./SECURITY.md#what-admin-cannot-do).
  Accepted as the deployment model unless/until a funded audit and
  governance process justifies a multisig-by-default design; mitigated
  by the key-rotation procedure in the same document.
- **Admin key rotation has no recovery path.** `transfer_admin` is a
  single irreversible step; an unreachable new admin address has no
  on-chain fallback. Accepted because adding one (e.g. a time-locked
  two-step handoff) is a deliberate design decision with its own
  trade-offs, not a bug — see
  [docs/SECURITY.md § Admin Key Rotation](./SECURITY.md#admin-key-rotation).
- **No funded bug bounty.** This project is unaudited and
  community-maintained with no treasury for rewards; severity levels are
  defined for when a program is funded, but no monetary reward is
  currently guaranteed — see
  [docs/SECURITY.md § Rewards](./SECURITY.md#rewards). An external audit
  is itself the mitigation this checklist exists to drive toward.
- **Pinned dependency-advisory exceptions.** `adler`, `paste` (unmaintained)
  and `spin 0.9.8` (yanked) are accepted exceptions in `deny.toml`,
  pulled in transitively through the Soroban SDK with no reported safe
  upgrade at the time they were pinned, with an explicit reassessment
  date — see
  [docs/SECURITY.md § Dependency Advisory Triage](./SECURITY.md#dependency-advisory-triage).
  Confirm that date hasn't passed before freezing the audit commit; if it
  has, reassess first rather than carrying a stale exception into the
  audit.
- **Commit authorship hygiene needs an explicit policy.** Contributor
  commits in this repository's history use a range of author names and
  emails with no enforced signing or identity verification. For a
  contract handling real value, an auditor (and any later incident
  response) benefits from being able to trust *who* actually wrote a
  given change. Accepted as a gap to close before freezing the audit
  commit, not a design trade-off: at minimum, confirm the commits on the
  frozen commit's history are ones the project recognizes as legitimate,
  and consider requiring signed commits going forward.
- **Reward math has resolved rounding history.** Earlier rounding-dust
  loss in reward accrual was resolved by the reward-per-share accumulator
  pattern — this is closed, not open, but is listed here as the kind of
  finding this checklist expects an auditor to re-verify rather than
  take on faith. See
  [README § Reward Rounding Dust Loss](../README.md#reward-rounding-dust-loss-resolved-in-issue-650).

## Packaging the audit scope

Generate all of the following from a clean checkout of the frozen
tag/commit (`git checkout audit-candidate-YYYY-MM-DD`), not from a
working tree that might carry local changes.

### Files and lines of code

```bash
# Every Rust source file, excluding build output.
find src -name '*.rs' | sort > audit-file-list.txt

# Split test from non-test for an honest "surface under audit" number —
# test files matter for understanding intent but aren't themselves the
# deployed contract.
find src -name '*.rs' ! -name 'test*.rs' | xargs wc -l | tail -1   # non-test LOC
find src -name 'test*.rs' | xargs wc -l | tail -1                  # test LOC

# Or, for a tool-generated breakdown by file and language:
tokei src/
```

As of this writing (the illustrative commit above, not a frozen scope):
172 non-test `.rs` files and 71 test `.rs` files under `src/`, totaling
roughly 78,800 lines across both. Regenerate these numbers for the actual
frozen commit — don't quote this document's numbers to an auditor.

### Dependencies

```bash
# Direct dependencies, from the manifest.
sed -n '/^\[dependencies\]/,/^\[/p' Cargo.toml

# The full resolved dependency graph the audited WASM is actually built
# from, including transitive dependencies.
cargo tree --target wasm32-unknown-unknown --features vault-wasm > audit-dependency-tree.txt

# Every advisory-checked crate name/version in the lockfile, for a quick
# count or diff against a previous audit's dependency snapshot.
grep -c '^name = ' Cargo.lock
```

Call out explicitly to the auditor: this crate has exactly one direct
runtime dependency (`soroban-sdk`, pinned to an exact version in
`Cargo.toml`), with the rest of the resolved graph being its own
transitive dependencies and this project's `dev-dependencies`
(`proptest`, and `soroban-sdk`'s `testutils` feature) — the
`dev-dependencies` never ship in the deployed WASM and are out of scope
for a review of what's actually on-chain, but are in scope for reviewing
the project's own test rigor.

### What to actually hand the auditor

- A full clone (or `git archive` of the frozen tag) — auditors generally
  want history and commit messages for context, not just a source
  tarball of one commit.
- The outputs of the three commands above (file list, dependency tree,
  lockfile), generated fresh against the frozen commit.
- Direct links to every document referenced in the ["Checklist"](#checklist)
  section above, plus this file itself.
- The compiled, optimized release WASM from CI's **WASM Build** job for
  the frozen commit, alongside the
  [reproducible-build verification steps](../README.md#deterministic-build--bytecode-verification)
  so the auditor can confirm their own rebuild matches it byte-for-byte.
- Explicitly exclude `target/` and any local `.env`/identity files —
  build artifacts and secrets have no place in an audit package.
