<!--
Thanks for contributing! Fill in each section, tick what applies, and mark
anything that doesn't apply as "N/A" with a short reason. Unticked items
without an explanation will be asked about in review.
-->

## Linked issue

Closes #<!-- issue number — required so the Drips bot can track the PR -->

## Summary

<!-- What does this PR change, and why? Note any design decisions reviewers should know about. -->

## Changes

<!-- Bullet list of the main changes (files/modules touched, new public functions, etc.). -->

-

## Checklist

### Tests
- [ ] Tests added or updated for the new/changed behaviour
- [ ] `cargo test --features testutils` passes locally

### Events
- [ ] State-changing functions emit an event (see `src/events.rs`)
- [ ] `docs/EVENTS.md` updated for any new or changed event

### Docs
- [ ] Public functions have `///` doc comments
- [ ] New/changed public functions are declared in `VaultTrait` (`src/interface.rs`) and `docs/INTERFACE.md` is updated
- [ ] `COSTS.md` updated with CPU/memory benchmarks for new public functions
- [ ] README **Trust Assumptions and Admin Powers** and `docs/SECURITY.md` updated if an admin-gated function, role or parameter changed
- [ ] `CONTRACT_VERSION` kept in sync with `Cargo.toml` (if the version changed)

### Lint
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes

## Storage layout

<!-- Required. Changing a storage key, a DataKey variant, or a stored type can break existing deployments. -->

- [ ] **No** storage layout changes
- [ ] **Yes**, storage layout changed. Details below:

<!--
If yes: which keys/types (`src/storage.rs`, `DataKey` variants, `#[contracttype]` structs)
were added, removed, renamed or re-typed? Is a migration needed (see
`src/migration_path.rs`)? Is `docs/STORAGE.md` updated? Any TTL implications
(`src/ttl_management.rs`)?
-->

## Deferred work

<!--
List anything intentionally left for a follow-up (e.g. tests, docs, benchmarks,
edge cases), with a reason and, ideally, a linked follow-up issue.
Deferring is fine; it just needs to be an explicit, visible choice.
Write "None" if nothing is deferred.
-->

- [ ] Nothing deferred
- [ ] Deferred items:
  -

## Notes for reviewers

<!-- Anything else: testing steps, screenshots/logs, known limitations, questions. -->
