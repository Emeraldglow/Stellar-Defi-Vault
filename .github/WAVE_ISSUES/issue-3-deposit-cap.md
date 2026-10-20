# [WAVE] Add per-user deposit cap

**Labels:** `Stellar Wave`, `enhancement`, `good first issue`
**Suggested Points:** 150

## Summary

Add an admin-configurable maximum deposit limit per user address. This is a common risk control in early-stage DeFi vaults.

## Acceptance Criteria

- [ ] New `set_deposit_cap(admin, cap)` function; `cap = 0` means no limit
- [ ] `get_deposit_cap()` read-only query
- [ ] `deposit` rejects if `user's total deposited value + new amount > cap` (when cap > 0)
- [ ] New `DepositCapExceeded` error variant in `VaultError`
- [ ] Track per-user deposited amount separately from shares (or derive it correctly from shares × share price)
- [ ] Tests: deposit within cap succeeds, deposit exceeding cap fails, cap of 0 disables limit, admin can raise/lower cap

## Notes

- Cap is denominated in token units, not shares.
- Think carefully about how accumulated yield affects a user's "deposited value" — document your approach in the PR.
