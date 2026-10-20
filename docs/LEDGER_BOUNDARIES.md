# Ledger boundary semantics

Ledger deadlines are inclusive throughout the vault: a deadline or unlock at
ledger `N` becomes effective at exactly `N`. Ledger `N - 1` is still before the
boundary, and `N + 1` remains after it.

| Feature | One before | Exact boundary | One after |
| --- | --- | --- | --- |
| Position lock | locked | unlocked | unlocked |
| Withdrawal cooldown | waiting | executable | executable |
| Reward vesting cliff | unvested | vested | vested |
| Reward-rate ramp | interpolating | target rate | target rate |

These comparisons share `ledger_boundary` helpers, whose tests cover all
three positions around both absolute deadlines and elapsed durations.
