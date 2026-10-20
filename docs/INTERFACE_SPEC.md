# Public Interface Snapshot

The contract's exported function names, parameters, and types are part of
its public ABI: every off-chain caller — the Stellar CLI, generated SDK
bindings, another contract's cross-contract client, an indexer — depends
on them staying exactly what they say they are. This snapshot makes a
change to that surface a visible, reviewable diff in a pull request,
rather than something that ships silently because nobody happened to
notice a renamed parameter or a changed return type.

## What's committed

[`interface-spec/VaultContract.json`](../interface-spec/VaultContract.json)
is the output of:

```bash
stellar contract info interface --wasm <release wasm> --output json-formatted
```

run against the **same release build CI actually ships** — `cargo build
--target wasm32-unknown-unknown --release --features vault-wasm`, matching
the "WASM Build" job in
[.github/workflows/ci.yml](../.github/workflows/ci.yml). It is not
hand-written and should never be hand-edited: it is read directly out of
the compiled WASM's embedded `contractspecv0` section by the Stellar CLI,
the same mechanism SDK binding generators use, so it reflects exactly
what a caller would actually see.

## How CI enforces it

The **Interface Spec** job runs
[`scripts/interface_spec.sh`](../scripts/interface_spec.sh) on every pull
request. It rebuilds the release WASM, extracts its interface the same
way, and diffs it against the committed file. A match passes silently; a
difference fails the job and prints the diff directly in the CI log, with
instructions for what to do next.

This means **any** change to the public interface — adding, removing, or
renaming a function or parameter, or changing a type — fails CI until a
human explicitly accepts it (see below). It does not distinguish
"breaking" from "additive" changes for you; a reviewer still has to read
the diff and judge that, the same as any other code review.

## Refreshing the snapshot for an intentional change

1. Make your contract change as normal.
2. Run:

   ```bash
   make interface-spec
   # equivalently: scripts/interface_spec.sh --update
   ```

   This rebuilds the release WASM and overwrites
   `interface-spec/VaultContract.json` with its current interface.
3. **Read the diff** (`git diff interface-spec/VaultContract.json`) before
   committing it. This is the actual review step — confirm it contains
   only the change you intended, and nothing from an unrelated
   in-progress edit elsewhere in the tree.
4. Add a [`CHANGELOG.md`](../CHANGELOG.md) entry under **Unreleased**
   describing the change and why it's intentional — see that file for the
   exact convention. CI does not currently enforce this step
   automatically; it's a review-checklist item (see
   [docs/AUDIT_READINESS.md](./AUDIT_READINESS.md) and
   [.github/pull_request_template.md](../.github/pull_request_template.md))
   until/unless it's worth automating as its own CI check.
5. Commit the updated spec file, the changelog entry, and the code change
   together in the same pull request, so a reviewer sees all three at once.

## One-time setup

This mechanism is new: `interface-spec/VaultContract.json` has not been
generated yet, because doing so requires an actual release build, which
this change was written without performing. Before the **Interface
Spec** CI job can pass, a maintainer with a working build environment
must run `make interface-spec` once against a known-good commit, review
the output, and commit it — exactly the same step described above for
any later intentional change. Until that's done,
`scripts/interface_spec.sh` (and the CI job that runs it) fails clearly
with instructions to do exactly this, rather than silently passing
against a snapshot that was never real.
