# Test Snapshot Hygiene

## Decision: gitignore `test_snapshots/`

Soroban's test harness writes JSON snapshot files to `test_snapshots/` when
tests run. These files record the XDR state of every contract invocation and
are regenerated on every test run, so committing them would:

- Produce noisy, meaningless diffs in every PR.
- Cause merge conflicts when two branches run tests.
- Inflate the repository with multi-hundred-file blobs that carry no
  reviewed information.

**`test_snapshots/` is listed in `.gitignore` and must never be committed.**

## CI enforcement

The `snapshot-hygiene` job in `.github/workflows/ci.yml` runs
`cargo test --features testutils`, then asserts that no files under
`test_snapshots/` are tracked by git:

```sh
git ls-files test_snapshots/ | grep . && exit 1 || exit 0
```

If any snapshot file slips through the `.gitignore` and is staged or
committed, CI fails immediately.

## Local clean-up

If you accidentally staged snapshot files, remove them from the index:

```sh
git rm -r --cached test_snapshots/
```

Then re-confirm `.gitignore` covers the path:

```sh
git check-ignore -v test_snapshots/
```

You can safely delete the local directory at any time — tests regenerate it:

```sh
rm -rf test_snapshots/
```

## Why not commit them?

An alternative policy would be to commit snapshots as a regression suite —
failing the test when any snapshot changes. This was evaluated and rejected
for this project because:

1. The snapshots encode XDR-encoded VM invocations that are an
   implementation detail of the soroban-sdk test harness, not an auditable
   API contract.
2. They change whenever unrelated internal state changes (TTL bumps, ledger
   metadata, etc.), requiring constant manual regeneration.
3. The `test-data/storage_layout.snapshot` file in `test-data/` already
   provides a compact, human-readable encoding stability check
   (`test_storage_layout`). Adding hundreds of raw XDR blobs on top of that
   is redundant.

If future work adds snapshot-based regression value that cannot be achieved
with `storage_layout.snapshot`, this decision should be revisited with a
concrete proposal.
