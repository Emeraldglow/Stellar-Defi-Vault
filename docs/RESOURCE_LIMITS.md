# Resource-footprint limits for batch functions (issue #660)

Soroban caps ledger entries read/written, events emitted, memory and CPU per
transaction. A documented batch size that cannot actually execute on-chain at
that size is a trap: the limit looks safe until a caller sends a maximum-size
batch in production.

`src/test_resource_footprint.rs` exercises each bounded entrypoint **at its
documented maximum** under the *real* default network budget
(`env.budget().reset_default()`, not `reset_unlimited`). If the call exceeds any
resource limit the host panics and the test fails, so a future change that
pushes a function over budget cannot merge. Each measured call also asserts a
generous CPU ceiling, and prints the recorded table.

## How the test enforces the budget

```rust
self.env.budget().reset_default(); // real network limits
f();                               // panics if over any resource limit
self.env.budget().cpu_instruction_cost() // recorded column
```

Because the call runs under the default budget, **every** resource is bounded
(reads, writes, events, memory and CPU), not just CPU. The printed CPU value is
the column recorded below.

## Recorded footprint

Run `cargo test --features testutils test_resource_footprint -- --nocapture` and
paste the printed table here.

| Function | Documented max input | CPU instructions (at max) |
| --- | --- | --- |
| `get_top_depositors` | `MAX_DEPOSITOR_SCAN` registrations | _filled from the test run_ |
| `get_top_depositors` | 50 requested | _filled from the test run_ |

## Reducing a documented maximum

If a function is found to exceed the budget at its documented maximum, lower the
constant that defines the maximum (for example `MAX_BATCH_VOTES`,
`MAX_DEPOSITOR_SCAN`) in the module that owns it, and update this table and the
matching test in the same PR. The test's default-budget assertion is what proves
the new maximum fits.
