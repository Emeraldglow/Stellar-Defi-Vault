# Storage Rent Cost Estimator

This document explains how storage rent works on Stellar and how to estimate the ongoing costs of maintaining contract state.

## Overview

On Stellar, all persistent storage requires rent payments to prevent state bloat. Rent is charged based on the size of stored data and the TTL (Time To Live) setting.

## Storage Types

### Instance Storage
- Shared across all users of a contract
- Used for global configuration and state
- Relatively expensive per entry
- TTL can be extended by anyone

### Persistent Storage
- Per-entry storage
- Used for user data and balances
- Cheaper per entry but more entries needed
- TTL must be extended before expiry or data is lost

### Temporary Storage
- Auto-expires after TTL
- Used for temporary data
- Cheapest option
- Not suitable for permanent state

## Rent Calculation

Rent is calculated based on:
1. **Entry size** (bytes of stored data)
2. **TTL duration** (ledgers)
3. **Network fee rates** (change over time)

### Formula
```
monthly_rent = entry_size_bytes * rent_per_byte_per_ledger * ledgers_per_month
```

Where:
- `ledgers_per_month ≈ 43,200` (at 5s per ledger)
- `rent_per_byte_per_ledger` is set by the network

## Estimating Costs

Use the provided script to estimate costs:

```bash
npx tsx scripts/rent-estimate.ts --users 1000 --features auto-compound,keepres
```

### Example Output
```
Storage Rent Estimate for 1,000 users
======================================

User Settings (consolidated):
  Entries per user: 1
  Entry size: 256 bytes
  Total entries: 1,000
  Total size: 256,000 bytes
  Estimated monthly rent: 0.0256 XLM

Auto-Compound Feature:
  Additional entries per user: 1
  Entry size: 128 bytes
  Total entries: 1,000
  Total size: 128,000 bytes
  Estimated monthly rent: 0.0128 XLM

Total Estimated Monthly Rent: 0.0384 XLM
```

## Who Pays Rent?

### Current Implementation
- **User entries**: Paid by the contract (deducted from yields)
- **Global entries**: Paid by the contract
- **New user onboarding**: First entry funded by protocol

### Recommendations
1. **Consolidate user data** into single entries to reduce per-user costs
2. **Use versioned structs** to avoid migrations
3. **Monitor TTL expiry** and extend proactively
4. **Batch operations** where possible

## Cost Optimization

### Consolidate User Settings
Instead of:
```
user_claim_destination (128 bytes)
user_auto_compound (1 byte)
user_keepers (variable)
user_notifications (1 byte)
```

Use:
```
user_settings {
  claim_destination: Address,
  auto_compound: bool,
  keepers: Vec<Address>,
  notifications: NotificationPrefs,
} (one entry, ~256 bytes)
```

### Result
- **Before**: 4 entries × 1,000 users = 4,000 entries
- **After**: 1 entry × 1,000 users = 1,000 entries
- **Reduction**: 75% fewer entries

## Monitoring

Track rent usage:
```bash
npx tsx scripts/rent-estimate.ts --users 1000 --monitor
```

## References

- [Stellar Storage Rent Documentation](https://developers.stellar.org/docs/learn/encyclopedia/storage-rent)
- [Soroban Storage TTL](https://developers.stellar.org/docs/smart-contracts/storage)
