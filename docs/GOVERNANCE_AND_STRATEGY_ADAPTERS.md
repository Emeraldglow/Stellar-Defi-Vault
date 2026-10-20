# Governance Enhancements and Strategy Adapter Architecture

This document specifies the architecture and implementation details for proposal categories, expedited emergency governance, vote delegation expiry, and the external strategy adapter system (Issues #761, #762, #763, #764).

---

## 1. Proposal Categories & Per-Category Quorum (Issue #761)

Governance proposals have distinct risk profiles. Critical parameter adjustments, treasury expenditures, and smart contract upgrades require different levels of community participation and consensus:

- **Categories**:
  - `Parameter` (Risk Tier 1): Standard parameter adjustments (unstake fees, reward rate, caps). Default: 10% quorum, 50% majority threshold, 1,000 ledgers duration.
  - `Treasury` (Risk Tier 2): Capital allocation and treasury spending. Default: 20% quorum, 60% approval threshold, 2,000 ledgers duration.
  - `Upgrade` (Risk Tier 3): Contract upgrades and structural migrations. Default: 30% quorum, 66.67% supermajority threshold, 5,000 ledgers duration.
  - `Other` (General): Community proposals and miscellaneous updates. Default: 10% quorum, 50% threshold, 1,000 ledgers duration.

- **Immutability & Snapshotting**:
  When a proposal is submitted, the active parameters (`quorum_bps`, `threshold_bps`, and `period_ledgers`) are snapshotted into the proposal storage record. Subsequent admin updates to category parameters never alter the rules of in-flight proposals.

---

## 2. Expedited Emergency Governance (Issue #762)

Emergencies (such as severe market dislocations or vulnerability mitigations) require rapid action without waiting out standard voting windows, balanced by strict security guardrails:

- **Economic Bond**: Proposers must lock a bond of at least `100_000_000` units (`MIN_EXPEDITED_BOND`) to deter spam and frivolous proposals.
- **Shortened Timeline**: The voting duration is compressed to `200` ledgers, followed by an execution timelock delay of `50` ledgers.
- **Elevated Consensus Bar**: Requires a super-quorum of `4,000` bps (40%) and an approval threshold of `7,500` bps (75%).
- **Guardian Veto**: Guardians retain the ability to veto expedited proposals during either the voting window or execution delay.
- **Bond Slashing**:
  - If an expedited proposal fails to achieve quorum, the bond is slashed and credited to the reward pool as compensation for emergency network bandwidth.
  - If quorum is met and the proposal passes (or is legitimately voted down by the community), the bond is fully refunded.
- **Concurrency Limit**: At most 3 expedited proposals can be open concurrently (`MAX_OPEN_EXPEDITED_PROPOSALS = 3`).

---

## 3. Vote Delegation with Expiry (Issue #763)

Permanent vote delegation carries the risk of stale power accumulation when delegators become inactive or forget their delegation choices:

- **Optional Expiration**: Delegators can specify an explicit ledger expiry `expires_at: Option<u32>` via `delegate_vote_weight_with_expiry(user, delegate, expires_at)`.
- **Automatic Expiration**: When `current_ledger >= expires_at`, the delegation automatically ceases to count toward the delegate without requiring an on-chain transaction.
- **Weight Reversion**: Staker voting power instantly reverts to the delegator once the expiry is reached.
- **Historical Snapshot Integrity**: Snapshots (`effective_vote_weight_at_ledger`) evaluate delegation expiry as of the specified snapshot ledger, preserving reproducible voting weight calculations.

---

## 4. StrategyAdapter Architecture for Idle Yield (Issue #764)

The vault can deploy idle capital into external Soroban yield protocols (e.g., lending markets, liquidity pools) through a standardized adapter interface:

```rust
pub trait StrategyAdapter {
    fn deposit(env: &Env, adapter: &Address, amount: i128) -> Result<(), GovernanceStrategyError>;
    fn withdraw(env: &Env, adapter: &Address, amount: i128) -> Result<i128, GovernanceStrategyError>;
    fn total_assets(env: &Env, adapter: &Address) -> Result<i128, GovernanceStrategyError>;
}
```

### Safety and Accounting Constraints
1. **Strategy Limit**: Up to 3 active strategies (`MAX_STRATEGIES = 3`).
2. **Allocation Caps**: Each strategy has an individual allocation cap in basis points (`cap_bps <= 10_000`), and the aggregate sum of all caps cannot exceed 100%.
3. **Share Price Transparency**: `total_vault_assets()` computes `idle_tokens + sum(strategy.total_assets())`, ensuring that the vault share price (`get_vault_share_price`) reflects external yields and deployed principal in real time.
