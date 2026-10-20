# Referrals, Vote-Escrow Boost, Gauge Voting, and Vote Receipts

Architecture and usage guide for features added in issues #757, #758, #759, and #760.

## 1. Referral Statistics & Claimable Balances (Issue #757)

### Motivation
Previously, referral bonuses were transferred immediately upon deposit, and referrers had no consolidated view of their referred network performance or claimable earnings.

### Architecture
- **Consolidated Struct**: `ReferralStats { referee_count: u32, total_referred_deposits: i128, earned: i128, claimed: i128 }`
- **Claimable Accrual**: On referee deposit, referral bonus amounts accrue to the referrer's claimable balance (`earned`) and are deducted from the reward pool into contract reserve.
- **Claim**: Referrers authenticate and call `claim_referral_earnings(referrer)` to withdraw accumulated rewards to their wallet.
- **Inspection**: `get_referral_stats(referrer)` returns real-time referee count, total deposits brought in, total earned, and total claimed.

---

## 2. Vote-Escrow Reward Boost (Issue #758)

### Motivation
Encourages long-term protocol alignment by enabling users to lock reward tokens for a chosen duration to receive a linear-decaying reward boost on their staked positions, modeled after vote-escrow (`ve`) mechanics.

### Mechanics
- **Lock**: `lock_for_boost(user, amount, duration_ledgers)` transfers reward tokens into the contract and records an unlock ledger timestamp.
- **Linear Decay**: Boost multiplier in basis points is calculated dynamically:
  $$\text{boost\_bps} = \min\left(\text{max\_boost\_bps}, \frac{\text{amount} \times \text{remaining\_ledgers} \times \text{max\_boost\_bps}}{\text{amount\_scale} \times \text{max\_duration}}\right)$$
- **Configurability**: Admin can configure maximum duration, maximum boost basis points, and amount scale via `set_ve_boost_config()`.
- **Unlock**: Tokens remain locked until `current_ledger >= unlock_ledger`. Early unlocks revert with `LockNotExpired`.

---

## 3. Gauge Voting for Emissions (Issue #759)

### Motivation
Decentralizes reward emissions across sub-pools by letting stakers vote on emission allocations instead of relying solely on static administrative parameterization.

### Mechanics
- **Voting**: Stakers call `vote_gauge(user, allocations: Vec<(u32 sub_pool_id, u32 bps)>)` where total allocated basis points must sum to 10,000 (100%).
- **Weight Derivation**: Weights are computed from staker voting power (active share balances).
- **Re-voting**: Calling `vote_gauge` again replaces prior allocations and updates the staker's directional preference.
- **Guardrails**:
  - `set_gauge_bounds(admin, sub_pool_id, min_bps, max_bps)` clamps resulting pool emissions within protocol limits.
  - `set_gauge_override(admin, overrides)` provides an administrative safety override in emergencies.

---

## 4. Vote Receipt Queries (Issue #760)

### Motivation
Provides transparent, read-only queries for frontends and stakers to inspect individual governance vote receipts without requiring transaction simulation or log parsing.

### Methods
- `get_vote(proposal_id, voter) -> Option<VoteReceipt { support: bool, weight: i128, ledger: u32 }>`
- `has_voted(proposal_id, voter) -> bool`
- `change_vote(user, proposal_id, support)` allows stakers to switch their vote on active proposals before the voting deadline, recalculating tallies and updating receipts automatically.
