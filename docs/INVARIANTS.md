# Vault Invariants

This document is the single source of truth for the vault's core share-price and
accounting invariants (issue #665). Tests, property-based fuzzing, and any
future formal verification should assert exactly what is written here; if an
invariant changes, this file changes first and the corresponding test changes
with it.

It covers the share accounting model only. Feature-specific invariants (governance
snapshots, reward boost tiers, insurance, referral payouts, etc.) are out of
scope; they are documented and tested alongside their feature modules.

## 1. Notation and state

All quantities are `i128`. Every balance is persisted through `src/balance.rs`,
whose setters reject negative writes with `VaultInvariantError::NegativeBalance`
(`assert_non_negative`, `src/balance.rs:22`).

| Symbol | Meaning | Storage |
|---|---|---|
| `S` | `total_shares` | `DataKey::TotalShares` |
| `D` | `total_deposited` | `DataKey::TotalDeposited` |
| `P` | reward pool balance | `DataKey::RewardPoolBalance` |
| `s_u` | shares held by user `u` | `DataKey::ShareBalance(u)` |
| `U` | set of registered stakers | `DataKey::AllStakers` |
| `B` | vault's live balance of the **stake token** | token contract |
| `T` | stake-token address | `DataKey::Token` |
| `R` | reward-token address (optional) | `symbol_short!("rwd_tok")` |
| `d` | stake-token decimals (queried at init) | `symbol_short!("stk_dec")` |

Two pure conversion functions drive every mint and burn
(`src/balance.rs:574`–`628`). Write `scale(a) = a · 10^(7 − d)` when `d < 7`,
`a ÷ 10^(d − 7)` when `d > 7`, and `a` when `d = 7`:

```
amount_to_shares(a) =  a                       if S = 0 or D = 0   (scale: first deposit)
                       floor(a · S / D)        otherwise

shares_to_amount(x) =  x · D / S   (floor)     if S > 0
                       0                       if S = 0
```

`preview_redeem(x) = shares_to_amount(x)` and `preview_deposit(a)` use the same
formulas read-only, so a preview and the executed path agree.

## 2. Invariants

### INV-1 — Share conservation

```
S = Σ_{u ∈ U} s_u
```

Every share in existence is attributed to a registered staker, and the registry
is exactly the set of addresses with a minted position (`register_staker` is
called on every mint). There are no unowned or phantom shares.

- Enforced by construction in `stake` / `do_unstake` (paired updates of
  `ShareBalance` and `TotalShares`).
- Checked on-chain by `check_invariants` (`src/invariants.rs:32`–`44`),
  exposed as the read-only `assert_invariants()` entrypoint.
- Tests: `randomized_share_invariants_hold` (`src/test.rs:217`, proptest over
  1–80 deposit/withdraw/claim operations asserts `total_shares == Σ shares`),
  `test_two_depositors_get_proportional_shares`, `test_full_withdraw_clears_shares`.

### INV-2 — Non-negativity

```
S ≥ 0,  D ≥ 0,  P ≥ 0,  s_u ≥ 0,  accrued_reward(u) ≥ 0,  reward_remainder(u) ≥ 0
```

`i128` is signed, so the type system does not provide this; the balance setters
enforce it as a last line of defence and panic with
`VaultInvariantError::NegativeBalance` on a negative write (issue #621).

- Checked by `check_invariants` for `S`, `D`, `P` and every `s_u`
  (`src/invariants.rs:28`–`41`).
- Tests: `set_shares_rejects_negative_value`,
  `set_total_shares_rejects_negative_value`,
  `set_total_deposited_rejects_negative_value`,
  `set_reward_pool_balance_rejects_negative_value`,
  `set_accrued_reward_rejects_negative_value`,
  `set_reward_remainder_rejects_negative_value`,
  `set_total_rewards_paid_rejects_negative_value`,
  `set_insurance_fund_balance_rejects_negative_value`
  (`src/test_issue_621_negative_balance.rs`); `randomized_share_invariants_hold`
  also asserts `S ≥ 0` and `D ≥ 0` after every operation.

### INV-3 — Solvency (token backing)

```
B ≥ D + (R = T ? P : 0)
```

The vault holds at least enough stake tokens to return every share's principal,
plus the reward pool when rewards are paid in the stake token. Donations,
protocol/unstake fees and unclaimed dust only increase `B` relative to `D`, so
this is a one-sided bound.

- Checked on-chain by `check_invariants` (`src/invariants.rs:46`–`60`), which
  reads the live token balance and reverts with `Undercollateralized`.
- Tests: `test_contract_balance_equals_staked_amount`,
  `test_withdraw_returns_correct_amount`. `randomized_share_invariants_hold`
  asserts the weaker "full pool is redeemable" form
  (`preview_redeem(S) = D` while `S > 0`).

### INV-4 — Full-pool redeemability

```
S > 0  ⟹  shares_to_amount(S) = D            S = 0  ⟹  D = 0
```

Redeeming the whole share supply returns exactly the tracked principal. For
`S > 0`, `shares_to_amount(S) = floor(S · D / S) = D`. When all shares are
burned, no principal may remain tracked.

- Tests: `randomized_share_invariants_hold` asserts both branches after every
  operation; `test_full_withdraw_clears_shares`;
  `test_preview_redeem_matches_actual_withdraw`.

### INV-5 — Price monotonicity under yield

```
add_yield(Δ):  D' = D + Δ,  S' = S          (Δ > 0)
⟹  D'/S' ≥ D/S   and   ∀x ≥ 0: shares_to_amount'(x) ≥ shares_to_amount(x)
```

Yield is added to the pool without minting shares, so the value per share
(`D/S`) is non-decreasing; no holder is diluted by a reward contribution.

- Enforced by `add_yield` (increases `D`, leaves `S` unchanged).
- Test: `test_add_yield_increases_share_price` (single worked example:
  500k shares, +100k yield ⇒ `preview_redeem(250k)` goes 250k → 300k).

### INV-6 — No value creation over a deposit/withdraw cycle

```
a > 0,  S > 0,  D > 0:
  m = amount_to_shares(a) = floor(a · S / D)
  r = shares_to_amount(m) = floor(m · D / S)
  ⟹  r ≤ a
```

A user cannot extract more value than they put in by cycling
deposit → withdraw: integer truncation can only lose value, never create it.
(Equality holds when `a · S` is divisible by `D`.)

- Covered in spirit by `randomized_share_invariants_hold` (random
  deposit/withdraw/claim streams) and `test_preview_redeem_matches_actual_withdraw`;
  see the gaps table for the missing dedicated property test.

### INV-7 — Donation isolation

```
share ratios depend only on (S, D), never on B
```

`amount_to_shares` and `shares_to_amount` read the tracked counters, not the
vault's raw token balance. Transferring stake tokens directly to the vault
inflates `B` (helping INV-3) but does not change the mint ratio, so the
first-depositor / donation inflation attack cannot dilute a later depositor.

- Test: `test_first_depositor_donation_does_not_inflate_share_price` (stake 1,
  donate 10M, next depositor of 1M still receives exactly 1M shares).

### INV-8 — First-deposit decimal scaling

```
S = 0  or  D = 0  ⟹  amount_to_shares(a) = scale(a, d → 7)
```

The first deposit establishes the share precision (7 decimals) independently of
the underlying token's precision, so a non-default-decimals token mints the
same number of shares as a 7-decimals token of equal economic value.

- Test: `share_price_uses_queried_non_default_token_decimals`
  (`src/test_share_decimals.rs`), `test_first_deposit_mints_1to1_shares`.

### INV-9 — Mint/burn accounting

```
stake(a):    S' = S + m,              D' = D + a
unstake(x):  S' = S − x,              D' = D − gross
             user receives gross − fee,  fee ≥ 0 stays in the pool
```

Every mint raises `S` by exactly the shares credited to the user and `D` by
exactly the tokens pulled in; every burn lowers `S` by exactly the shares burned
and `D` by the *gross* value of those shares (so any fee remains as backing for
the remaining holders). `emergency_withdraw` performs the same burn with no fee.

- Enforced in `do_unstake` (`src/vault.rs`), whose fee-bearing path decrements
  `total_deposited` by `amount_returned + unstake_fee` while paying the user only
  `amount_returned`.
- Tests: `test_first_deposit_mints_1to1_shares`,
  `test_withdraw_returns_correct_amount`, `test_full_withdraw_clears_shares`.

## 3. Test / invariant map

| Invariant | Mapped tests | In-contract check |
|---|---|---|
| INV-1 share conservation | `randomized_share_invariants_hold`, `test_two_depositors_get_proportional_shares`, `test_full_withdraw_clears_shares` | `check_invariants` (step 2) |
| INV-2 non-negativity | `test_issue_621_negative_balance.rs::*`, `randomized_share_invariants_hold` | `check_invariants` (step 1) + setters |
| INV-3 solvency | `test_contract_balance_equals_staked_amount`, `test_withdraw_returns_correct_amount` | `check_invariants` (step 3) |
| INV-4 full-pool redeemability | `randomized_share_invariants_hold`, `test_full_withdraw_clears_shares`, `test_preview_redeem_matches_actual_withdraw` | — |
| INV-5 price monotonicity | `test_add_yield_increases_share_price` | — |
| INV-6 no value creation over cycle | `randomized_share_invariants_hold`, `test_preview_redeem_matches_actual_withdraw` | — |
| INV-7 donation isolation | `test_first_depositor_donation_does_not_inflate_share_price` | — |
| INV-8 first-deposit scaling | `share_price_uses_queried_non_default_token_decimals`, `test_first_deposit_mints_1to1_shares` | — |
| INV-9 mint/burn accounting | `test_first_deposit_mints_1to1_shares`, `test_withdraw_returns_correct_amount`, `test_full_withdraw_clears_shares` | — |

## 4. Gaps

These invariants are stated and implemented but not yet covered as precisely as
this spec requires. They are the backlog for the property-based/fuzz suite and
for #662 (mutation testing), which is scoped to the accounting modules.

| # | Gap | Why it matters | Suggested test |
|---|---|---|---|
| G1 | **`assert_invariants()` is never called by a test.** The on-chain checker in `src/invariants.rs` is exercised by no test, so a regression in the checker itself would go unnoticed. | The checker is the published health check integrators rely on. | A test that drives stake/withdraw/add_yield then calls `assert_invariants()`, plus negative tests that corrupt storage directly and assert each of `NegativeAccounting` / `SharesMismatch` / `Undercollateralized`. |
| G2 | **INV-5 has only a single worked example, no property test.** | Monotonicity must hold for all `(S, D, Δ, x)`, not one triple. | proptest: after `add_yield`, `preview_redeem(x)` is non-decreasing for sampled `x`, and `D/S` is non-decreasing. |
| G3 | **INV-6 (round-trip loss bound) is not asserted directly.** | "No value creation over a cycle" is the core economic safety property. | proptest: for `S, D, a > 0`, `shares_to_amount(amount_to_shares(a)) ≤ a`. |
| G4 | **INV-8 is only checked for one non-default decimal case.** | Decimal scaling is easy to get wrong at boundaries. | proptest over `d ∈ 0..=18` asserting `scale` round-trips within truncation, and that first-deposit share values are monotone in `a`. |
| G5 | **INV-3 is not asserted after fee/reward paths.** Solvency is checked by the fresh-balance tests only, not after unstake fees, referral bonuses, or insurance debits. | Those paths move tokens out of `B`; a mistake there is exactly an undercollateralization bug. | Property test asserting `B ≥ D` after randomized mixes that include fee-bearing unstakes and reward payouts. |
| G6 | **No cross-invariant fuzz target combining all nine.** `randomized_share_invariants_hold` covers INV-1/2/4 but not INV-5/6/7/8. | A single assertion harness should evaluate every invariant per step. | One proptest that calls `check_invariants`' logic (via `assert_invariants`) each step and additionally asserts INV-5–8. |

## 5. Optional follow-up: formal verification tooling

The spec above is written to be a target for lightweight formal methods. This is
a survey, not a commitment (the deliverable for #665 is the spec):

- **Kani** (`cargo-kani`) — bounded model checking for Rust; can prove the
  arithmetic conversions (`amount_to_shares` / `shares_to_amount`) total and
  bounded for symbolic `i128` inputs with `checked_*` reasoning. Best fit for
  INV-4 and INV-6 as pure functions.
- **Creusot / Prusti** — deductive verification of Rust functions. Higher setup
  cost for a `soroban-sdk` crate (storage/token calls must be modelled as
  uninterpreted), so practical scope is again the pure conversion functions.
- **Soroban's own test host + proptest** — already in use
  (`randomized_share_invariants_hold`); closing gaps G1–G6 here is cheaper than
  any external tool and gives the most value per unit effort.

Recommendation: close G1–G6 with proptest first, then evaluate Kani on the two
conversion functions once the property suite is stable.
