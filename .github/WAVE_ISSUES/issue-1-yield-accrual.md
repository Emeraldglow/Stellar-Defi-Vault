# [WAVE] Add yield accrual mechanism to vault

**Labels:** `Stellar Wave`, `enhancement`, `good first issue`
**Suggested Points:** 300

## Summary

The vault currently tracks deposits and shares but has no way for the admin to add yield to the pool. When yield is added, the share price should automatically increase — meaning existing shareholders receive more tokens when they withdraw.

## Motivation

This is the core value-add of a yield vault. Without it, shares are always 1:1 with deposits and there's no incentive to use the vault over holding tokens directly.

## Acceptance Criteria

- [ ] New `add_yield(admin, amount)` function on the contract
- [ ] `add_yield` requires admin auth
- [ ] `add_yield` transfers `amount` tokens from the admin into the vault
- [ ] `add_yield` increases `total_deposited` without minting new shares (this raises share price)
- [ ] `add_yield` emits a `yield_added` event with `(admin, amount)`
- [ ] Unit tests covering: normal yield add, share price increase verified via `preview_redeem`, unauthorized caller fails, paused vault blocks yield add
- [ ] Doc comment on the new function

## Example

```rust
// Before yield: 1000 shares = 1000 tokens (1:1)
vault.add_yield(&admin, &200);
// After yield: 1000 shares = 1200 tokens (1.2:1)
assert_eq!(vault.preview_redeem(&1000), 1200);
```

## Notes

- The yield source in this PR can be the admin manually funding it; auto-yield strategies are a separate issue.
- Make sure `shares_to_amount` math still holds after yield is added.
