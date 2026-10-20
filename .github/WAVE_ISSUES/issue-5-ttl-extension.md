# [WAVE] Implement storage TTL extension (persistent entry bump)

**Labels:** `Stellar Wave`, `enhancement`
**Suggested Points:** 200

## Summary

Soroban persistent storage entries expire after a set number of ledgers. The vault must bump TTL on all relevant keys to ensure user share balances and vault state are never lost.

## Acceptance Criteria

- [ ] A `bump_ttl(user)` public function that extends TTL on the user's `ShareBalance` persistent entry
- [ ] A `bump_vault_ttl()` admin function that extends TTL on all instance storage keys
- [ ] TTL thresholds configurable via constants in `storage.rs` (e.g. `LEDGER_THRESHOLD`, `LEDGER_BUMP`)
- [ ] `deposit` automatically bumps the depositor's share entry TTL after minting
- [ ] `withdraw` bumps the user's entry if shares remain > 0 after withdrawal
- [ ] Tests that call `bump_ttl` and verify no panics; comment explaining the TTL values chosen

## Reference

- [Soroban State Archival Docs](https://developers.stellar.org/docs/learn/encyclopedia/storage/state-archival)
- `env.storage().persistent().extend_ttl(key, threshold, extend_to)`

## Notes

- Typical values: threshold = 100,000 ledgers, extend_to = 500,000 ledgers (~100 days).
- This is important for production readiness; reviewers will check the numbers are sensible.
