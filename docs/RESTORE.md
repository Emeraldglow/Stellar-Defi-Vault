# Restoring Archived Position Entries

Soroban archives a persistent ledger entry once its time-to-live (TTL)
reaches zero. For this vault, that can happen to:

- **The contract's own instance storage** (admin, token, totals, config) —
  guarded by `extend_contract_ttl` / `ttl_management.rs`.
- **A single user's own entries** — their position (`ShareBalance`,
  `StakeHistory`, `StakedAtLedger`, `UnbondingPosition`, `FrozenAt`,
  `FirstStakedAt`, `Restaked`, `VestingEntries`, `LastUnstakeLedger`),
  reward checkpoint (`RewardCheckpointLedger`, `LastClaimLedger`,
  `AccruedReward`, `RewardRemainder`), and settings
  (`Delegate`, `UserClaimWindow`, `Whitelisted`, `KycApproved`) — guarded
  by `bump_user_entries` / `user_ttl.rs`.

This document covers the second case: a staker who deposited once and
never interacted again, and whose own entries nobody ever bumped before
their TTL ran out.

## Preventing it in the first place

Every deposit, withdrawal, and claim already calls `user_ttl::bump_entries`
as a side effect, extending that user's own entries. The failure mode this
document is about only happens to a position that someone — the user
themselves, or, since `bump_user_entries` requires no signature from the
position's owner, anyone running a keeper bot — never touched for long
enough. Run `bump_user_entries(user)` on a schedule (e.g. sweeping
`get_all_stakers()`) and no position should ever reach this state. See
`user_ttl.rs` for the admin-configurable threshold/target
(`set_user_ttl_extension`, `get_user_ttl_extension_threshold`,
`get_user_ttl_extension_target`).

## Recognizing it happened

A read of an archived entry does **not** return a default value like `0`
or `None` — it fails. Concretely: `staked_amount(user)` is implemented as
`balance::get_shares(...).unwrap_or(0)`, which looks like it always has a
safe fallback. It doesn't, once the entry is archived: the storage read
itself fails before that `.unwrap_or(0)` ever runs, so the call errors out
instead of quietly reporting "this user has 0 staked." The same is true of
every other getter that touches an archived key — `get_vote_record`,
`calc_pending_reward`, `get_vote_change_count`, and so on.

If a call that used to work for a specific, long-dormant user suddenly
starts failing outright (rather than returning zero or `None`), suspect
archival before anything else. `src/test_restore_archived_entries.rs`
exercises exactly this and documents why: a silent zero would be
indistinguishable from "this user never had a position," which is exactly
the ambiguity this behavior avoids.

## Restoring it

Archival is undone by a **restore footprint**, a ledger-level operation
that runs before the contract is invoked at all — the contract's own code
never executes a "restore" step; by the time it runs, the entry is either
already live again or the transaction never reached the contract. There
are two ways to trigger it:

### 1. Just retry the call (works in most cases)

A current `stellar` CLI (and most RPC client SDKs) simulate a transaction
before sending it. If that simulation reports the call would touch an
archived entry, the CLI automatically includes the restore operation in
the same submitted transaction — so for a normal state-changing call (a
deposit, a claim, `bump_user_entries`), simply re-running it is often
enough:

```bash
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --source "$IDENTITY" \
  --network "$NETWORK" \
  -- \
  bump_user_entries \
  --user "$USER_ADDRESS"
```

If this still fails with an archival-related error, the client you're
using doesn't auto-restore, or the restore needs explicit resource fees
you want to control yourself — use the explicit path below.

### 2. Restore explicitly

The `stellar` CLI has a dedicated command for this:

```bash
stellar contract restore \
  --id "$CONTRACT_ID" \
  --source "$IDENTITY" \
  --network "$NETWORK" \
  --durability persistent \
  --key-xdr "$KEY_XDR"
```

`--durability persistent` is required — every per-user entry this vault
writes is persistent storage, never temporary.

**For the contract's own instance entries** (admin, totals, config), omit
`--key`/`--key-xdr` entirely — `stellar contract restore --id "$CONTRACT_ID" --source "$IDENTITY" --network "$NETWORK"`
restores the contract instance itself.

**For a specific user's entries**, you need the exact storage key, and
`stellar contract restore`'s `--key` flag only accepts a bare `Symbol` —
every key this vault uses for a user's own entries is a **composite** key
(e.g. `DataKey::ShareBalance(user)`, or a raw `(Symbol, Address)` tuple
like `(symbol_short!("vc_rec"), proposal_id, user)`), so you must use
`--key-xdr` with the base64-encoded XDR of that exact
`LedgerKey::ContractData` entry instead of `--key`. Soroban encodes a Rust
tuple key as an `ScVal::Vec` of its elements in that same order, so for
example `DataKey::ShareBalance(user)` (an enum variant carrying one
`Address`) becomes an `ScVal::Vec` of `[Symbol("ShareBalance"),
Address(user)]` wrapped as the `LedgerKey::ContractData`'s `key` field.
Build and base64-encode that XDR with the official Stellar SDK for
whichever language you're scripting in (e.g. the JS SDK's
`xdr.LedgerKey.contractData(...)`/`nativeToScVal`, or the Python SDK's
equivalents) rather than hand-assembling it — getting a single byte wrong
produces a key that silently points at a different (and almost certainly
nonexistent) entry rather than an obvious error.

You can sanity-check a key before or after restoring it with the matching
read command:

```bash
stellar contract read --id "$CONTRACT_ID" --key-xdr "$KEY_XDR" --durability persistent --output json
```

### Confirming the restore worked

Re-run the call that was failing. For a user's position specifically,
`staked_amount`/`get_vote_record`/`calc_pending_reward` etc. should go
back to returning their normal values — the balances and accrued rewards
the user had before archival, never zeroed or reset, since restoring an
entry recovers its exact prior value rather than recreating it from
scratch.

## See also

- `user_ttl.rs` — the prevention mechanism (`bump_user_entries`) and its
  admin-configurable threshold/target.
- `ttl_management.rs` — the equivalent mechanism for the contract's own
  instance storage.
- `src/test_restore_archived_entries.rs` — tests that an archived read
  fails clearly rather than returning a default, and that bumping before
  expiry keeps a position's balance and accrued reward exactly intact.
