# [WAVE] Implement configurable deposit & withdrawal fee (basis points)

**Labels:** `Stellar Wave`, `enhancement`
**Suggested Points:** 250

## Summary

Add an optional fee in basis points (bps) that is deducted from deposits and/or withdrawals. Fees collected stay in the vault, raising the share price for all existing holders.

## Acceptance Criteria

- [ ] New `set_fee(admin, deposit_fee_bps, withdraw_fee_bps)` admin function
- [ ] Fees stored in contract storage (default 0 bps — no fee)
- [ ] `deposit` deducts fee before computing shares: `effective_amount = amount - fee`
- [ ] `withdraw` deducts fee from the amount returned: `effective_return = amount - fee`
- [ ] Fee stays in vault (increases `total_deposited`, not transferred out)
- [ ] `get_fee()` read-only function returns current fee config
- [ ] Max fee capped at 1000 bps (10%) — reject higher values with a new `FeeTooHigh` error
- [ ] Full test coverage: zero fee (default behaviour unchanged), deposit fee applied correctly, withdraw fee applied correctly, fee-too-high rejected

## Example

```rust
vault.set_fee(&admin, &50, &0); // 0.5% deposit fee, no withdraw fee
let shares = vault.deposit(&alice, &10_000);
// effective deposit = 9_950; shares minted based on 9_950
```

## Notes

- 1 bps = 0.01%. Use integer arithmetic only; no floating point.
- This change must not break existing tests (zero-fee path).
