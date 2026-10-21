# Changelog

Format loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Entries are grouped under **Unreleased** until they are cut into a release.
See [docs/RELEASING.md](./docs/RELEASING.md) for the release process
(tagging, the CI build, and its signed build-provenance attestation) — see
[docs/AUDIT_READINESS.md](./docs/AUDIT_READINESS.md) instead for the
separate commit/tag-freeze process ahead of an external audit.

When `Unreleased` entries are cut into a release, they move under a new
`## vX.Y.Z — YYYY-MM-DD` heading, which must record the released Wasm's
SHA-256 (see docs/RELEASING.md § Cutting a release):

```markdown
## vX.Y.Z — YYYY-MM-DD
- Wasm SHA-256: `<sha256 of the release build>`

### <function or area name>
...
```

## Why this file exists

Most changes to this contract don't need an entry here — commit messages
and PR descriptions cover routine work. An entry **is** required when a
pull request changes the contract's public interface (adds, removes, or
renames a function or parameter, or changes a type) — i.e. whenever
[`make interface-spec`](./docs/INTERFACE_SPEC.md) had to be re-run and its
output committed. The interface-spec diff shows *what* changed; the entry
here records *why*, for anyone (an auditor, an integrator, a future
maintainer) who needs the human reasoning without re-deriving it from the
diff alone.

Format for an entry:

```markdown
### <function or area name>
- **What:** one line, matching the shape of the interface-spec diff
  (e.g. "removed `set_old_rate_bps`, added `set_reward_rate_bps` with an
  extra `ramp_ledgers` parameter").
- **Why:** the reason, in enough detail that someone unfamiliar with the
  change understands the motivation without reading the PR.
- **Breaking:** yes/no, and what callers need to do differently if yes.
- **PR:** link or number.
```

## Unreleased

No public-interface changes recorded yet. The first entry here should
land alongside the first commit of
`interface-spec/VaultContract.json` (see
[docs/INTERFACE_SPEC.md § One-time setup](./docs/INTERFACE_SPEC.md#one-time-setup) —
that initial snapshot establishes the baseline rather than describing a
change, so it does not itself need an entry, but the first *change*
against it does).
