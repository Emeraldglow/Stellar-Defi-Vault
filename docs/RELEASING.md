# Releasing

How a release of this contract's Wasm is cut, built, signed, and verified.
See [docs/AUDIT_READINESS.md](./AUDIT_READINESS.md) for the separate
audit-freeze tag convention (`audit-candidate-YYYY-MM-DD`) — that process
freezes a commit for an external auditor and is unrelated to the versioned
releases described here.

## Why build provenance matters

Anyone downloading a release Wasm from this repo's Releases page should be
able to confirm it was actually compiled by this repo's own CI, from the
exact commit the release tag points at — not hand-built and uploaded by
whoever cut the release. GitHub's
[build provenance attestations](https://docs.github.com/en/actions/security-guides/using-artifact-attestations-to-establish-provenance-for-builds)
do this: the `release.yml` workflow signs the built Wasm over OIDC
(Sigstore) and records a verifiable attestation tying the artifact's hash to
the workflow run, the commit SHA, and this repository.

## Cutting a release

1. Pick the version (this project does not yet enforce strict semver —
   use judgement; a breaking public-interface change, per
   [CHANGELOG.md](../CHANGELOG.md), should bump the minor version at least).
2. Add a `## vX.Y.Z` section to `CHANGELOG.md` above `## Unreleased`,
   moving the relevant entries out of `Unreleased` into it. Leave the Wasm
   SHA-256 line as a placeholder for now — you don't have it until CI
   builds the release:

   ```markdown
   ## vX.Y.Z — YYYY-MM-DD
   - Wasm SHA-256: `<filled in after the release build completes>`

   ### <function or area name>
   - **What:** ...
   - **Why:** ...
   - **Breaking:** ...
   - **PR:** ...
   ```

3. Commit that changelog update on `main` (regular PR, reviewed as usual).
4. Tag the commit and push the tag:

   ```bash
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin vX.Y.Z
   ```

   Pushing a `v*` tag triggers `.github/workflows/release.yml`.
5. The workflow builds the release Wasm (the same `--no-default-features`
   build CI's "WASM Build" job uses, with no test-only symbols — see issue
   #658), computes its SHA-256, generates a signed build-provenance
   attestation for it, and publishes a GitHub Release with three things
   attached:
   - `stellar_defi_vault.wasm` — the release artifact.
   - `stellar_defi_vault.wasm.sha256` — its SHA-256, in `sha256sum` format.
   - A link to the attestation and the exact verification command, in the
     release notes.
6. Go back and fill in the Wasm SHA-256 placeholder in the `CHANGELOG.md`
   entry from step 2 with the real hash from the release, in a small
   follow-up commit. The changelog entry for a release is not considered
   complete until this hash is recorded.

## Verifying a release

Given a downloaded `stellar_defi_vault.wasm` from a release:

```bash
# Confirm the file matches the published hash.
sha256sum stellar_defi_vault.wasm
# Compare against stellar_defi_vault.wasm.sha256 from the same release.

# Confirm it was built by this repo's CI from the tagged commit, using the
# GitHub CLI (requires `gh auth login` once):
gh attestation verify stellar_defi_vault.wasm --repo <org>/stellar-defi-vault
```

A successful `gh attestation verify` confirms:
- The Wasm's digest matches a signed attestation.
- That attestation was produced by a GitHub Actions run in this repository
  (not a fork, not a local build).
- The attestation names the `release.yml` workflow and the commit SHA that
  triggered it, which you can cross-reference against the release tag.

If verification fails, treat the artifact as untrusted — do not deploy it.

## Refreshing release fixtures used by tests

The upgrade test in `tests/upgrade/` (see that directory's README) uses the
Wasm from the *previous* release as its starting point. After cutting a new
release, follow that README to refresh the stored fixture so the next
release's upgrade test exercises the latest-released-to-current path.
