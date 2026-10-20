# Security Model

This document describes the trust model of the Stellar DeFi Vault contract: what the admin account can and cannot do, what happens in failure scenarios, and how to rotate the admin key. All claims are verified against the deployed source code in `src/vault.rs`.

---

## Admin-Only Functions

The following functions gate on `admin::require_admin`, which calls `admin.require_auth()` using the Soroban auth framework. Any call that does not carry a valid authorization signature from the current admin address is rejected by the host before the function body executes.

### `pause()`

Flips the `Paused` flag in instance storage to `true`.

**Effect:** Subsequent calls to `deposit` and `withdraw` return `VaultError::VaultPaused` (error code 6) and no state changes occur. No funds are moved. User share balances and the underlying token holdings of the contract are unaffected.

**Emits:** `paused` event with the admin address.

### `unpause()`

Flips the `Paused` flag back to `false`.

**Effect:** Deposits and withdrawals resume normally. No funds are moved during the call itself.

**Emits:** `unpaused` event with the admin address.

### `enter_withdraw_only_mode(admin)` / `exit_withdraw_only_mode(admin)`

Switches the pool into, and back out of, withdraw-only mode.

**Effect:** While active, deposits, staking, compounding, locking and reward auto-restake are rejected, while withdrawals and claims stay open and their fees are waived. Unlike `pause()`, user funds are never frozen. Unlike `initiate_sunset()`, the mode is immediate, has no deadline and can be reversed by the admin. `get_pool_mode()` reports `Normal`, `Paused` or `WithdrawOnly`.

**Emits:** `wd_only` event with the admin address, the new state and the ledger sequence.

### `set_dust_threshold(admin, amount)` / `set_dust_keeper_tip_bps(admin, bps)` / `set_dust_treasury(admin, treasury)`

Configure dust cleanup. A keeper may call `close_dust_position(keeper, user)` for a position worth less than the threshold, without the owner's signature.

**Effect:** The position's whole value is paid to the owner (or to the dust treasury if one is set), less a keeper tip capped at 10%. Locked positions cannot be closed, and accrued rewards stay claimable by the owner.

**Emits:** `dust_clos` event with the amount paid out, the keeper tip and the ledger sequence.

### `add_yield(admin_addr, amount)`

Transfers `amount` tokens **from the admin's own wallet** into the vault contract, then increments `total_deposited` by the same amount without minting new shares.

**Effect:** The share price rises for all existing holders. No user share balances change. The admin must hold sufficient token balance and approve the transfer. This function requires the vault to be unpaused.

**Emits:** `yield_add` event with the admin address and the amount added.

**Key constraint:** The admin is sending their own tokens _into_ the vault, not extracting anything from it. This function cannot be used to remove user principal.

### `rescue_token(admin_addr, token, amount, recipient)`

Transfers a stuck non-stake, non-reward token from the vault to the specified `recipient`. This function is admin-only and rejects if the token matches either the configured stake token or the configured reward token.

**Effect:** Only third-party tokens accidentally sent to the contract can be rescued. The admin cannot use this to move user principal or the registered reward token balance.

**Emits:** `tk_rescue` event with the rescued token, amount, recipient, and ledger sequence.

### `transfer_admin(new_admin)`

Replaces the stored admin address with `new_admin` in a single atomic step.

**Effect:** The calling address (current admin) loses admin privileges immediately. The new address gains them immediately. There is no two-phase handoff — the current admin must trust the new address before calling.

**Does not emit an event.** Monitor on-chain storage changes or set up an indexer alert on this function call if admin rotation observability matters.

---

## What Admin Cannot Do

The following operations are **not possible** for the admin, confirmed by code review:

### Admin cannot access user principal

There is no function in the contract that allows the admin to withdraw, transfer, or redirect tokens that belong to depositors. The only path by which tokens leave the vault is `withdraw()`, which requires:

```rust
withdrawer.require_auth();
```

This means the `withdrawer` address must sign the transaction. The admin key alone cannot satisfy this requirement for another user's address. The admin and a user are distinct addresses — even if the same entity controlled both, they are separate authorization contexts enforced by the Soroban host.

User share balances are stored in persistent storage under `DataKey::ShareBalance(user_address)`. The contract exposes no setter for this key that is gated on admin auth alone.

### Admin cannot mint shares to themselves

`add_yield` increases `total_deposited` but explicitly does **not** call `balance::set_shares` or `balance::set_total_shares`. New shares are only minted inside `deposit()`, which requires the depositor's own auth.

### Admin cannot change the vault token

The `Token` key is written once during `initialize` and there is no setter for it exposed through any function.

### Admin cannot re-initialize the contract

`initialize` checks `env.storage().instance().has(&DataKey::Admin)` and returns `VaultError::AlreadyInitialized` (error code 2) if the admin key is already set.

---

## Shutdown Modes

The vault has three distinct shutdown mechanisms. They differ in reversibility, scope, and intended use case.

### `pause()` / `unpause()` — reversible, blocks all activity

**Trigger:** Admin calls `pause()`.

**Effect:** Both `stake` and `unstake` revert with `VaultError::VaultPaused` (error code 6). `claim` and `withdraw_vested` are also blocked. No funds move.

**Reversible:** Yes. The admin can call `unpause()` at any time to restore full operation.

**Storage flag:** `DataKey::Paused` in instance storage.

**Intended use:** Routine maintenance, emergency hotfix window, or any situation where the admin needs a brief freeze with the intent to resume.

---

### `start_graceful_shutdown()` — irreversible, blocks new stakes only

**Trigger:** Admin calls `start_graceful_shutdown()`.

**Effect:** Any new call to `stake` or `stake_with_referral` reverts with `VaultError::PoolShuttingDown` (error code 45). Existing stakers are unaffected: `unstake`, `claim`, and `withdraw_vested` all continue to work normally. The pool winds down naturally as current positions exit.

**Reversible:** No. The `ShuttingDown` flag is set to `true` and there is no function to clear it.

**Coexistence:** Graceful shutdown and pause are independent flags. Both can be active simultaneously without conflict.

**Storage flag:** `DataKey::ShuttingDown` in instance storage.

**Event emitted:** `shutdown_started` — contains the admin address and the ledger sequence at which shutdown began.

**Intended use:** Planned end-of-life for the pool. Lets existing participants exit at their own pace without forcing anything. No funds are lost; every staker can retrieve their principal and accrued rewards.

**Query:** `is_shutting_down(env) -> bool` — read-only, no auth required.

---

### `emergency_stop()` — irreversible, blocks stake permanently

**Trigger:** Admin calls `emergency_stop()`.

**Effect:** `stake` and `stake_with_referral` revert with `VaultError::ContractStopped` (error code 9). Unlike graceful shutdown, `pause`/`unpause` also revert with `ContractStopped` once this flag is set — the contract cannot be paused or unpaused after an emergency stop. `unstake` and `claim` continue to work.

**Reversible:** No.

**Storage flag:** `DataKey::Stopped` in instance storage.

**Event emitted:** `stopped` — contains the admin address.

**Intended use:** Critical security incident where the admin needs to immediately and permanently prevent any new capital from entering the pool. Existing stakers can still exit.

**Query:** `is_stopped(env) -> bool` — read-only, no auth required.

---

### Comparison table

| Mode                      | Blocks stake | Blocks unstake/claim | Reversible | Event              |
| ------------------------- | ------------ | -------------------- | ---------- | ------------------ |
| `pause`                   | Yes          | Yes                  | Yes        | `paused`           |
| `start_graceful_shutdown` | Yes          | No                   | No         | `shutdown_started` |
| `emergency_stop`          | Yes          | No                   | No         | `stopped`          |

---

## Failure Scenarios

### Vault is paused

Both `deposit` and `withdraw` call `require_not_paused` before any state changes:

```rust
Self::require_not_paused(&env)?;
```

If paused:

- Deposits return `VaultError::VaultPaused`. No tokens are moved.
- Withdrawals return `VaultError::VaultPaused`. User principal stays in the vault contract.
- User share balances are unchanged — they continue to represent the same ownership fraction.
- `add_yield` also checks `require_not_paused`, so the share price cannot change while paused.

User funds are frozen, not lost. As soon as `unpause()` is called, full functionality resumes and users can withdraw their proportional share at the current (unchanged) price.

### Yield injection exhausted (admin has no tokens to add)

This vault uses a share-price appreciation model rather than a streaming reward token. Yield is delivered only when the admin calls `add_yield`. If the admin stops calling `add_yield` (whether due to insufficient balance, operational decision, or key compromise):

- Existing depositors retain their shares at the last recorded share price.
- No yield accrues passively — share price is constant until the next `add_yield` call.
- Withdrawals continue to work normally; users receive `shares × (total_deposited / total_shares)` tokens, which reflects all previously added yield.

There is no separate reward token pool that can be "emptied." The vault holds only the single token specified at `initialize`. Yield additions and user principal are fungible in the contract balance; the accounting invariant is:

```
contract token balance ≥ total_deposited
```

This invariant holds as long as no external mechanism drains the contract (no such mechanism exists in this contract).

### Arithmetic overflow or zero-division

Share minting and redemption use `checked_mul` / `checked_div`. On failure these return `None`, which is mapped to `VaultError::ArithmeticError` (error code 8). The transaction reverts with no state changes.

### Admin key compromise

If the admin key is compromised, an attacker can:

- Pause the vault (freezing user withdrawals).
- Call `add_yield` with a zero-value amount (no effect due to `amount <= 0` guard).
- Transfer admin to another address, locking out the legitimate admin.

An attacker with the admin key **cannot** drain user funds (see above). The highest-impact action is a sustained pause that prevents users from withdrawing. This is mitigated by rotating the admin key promptly (see below).

---

## Admin Key Rotation

To rotate the admin key:

1. Generate or designate a new Stellar keypair (or a multisig policy address).
2. Call `transfer_admin(new_admin)` signed by the **current** admin key.
3. Verify on-chain that `DataKey::Admin` now holds the new address.
4. Revoke or destroy the old private key.

This is a single-step, irreversible operation. The current admin loses authority the moment the transaction is confirmed. There is no recovery path if the new address is inaccessible, so verify the new address is under your control before calling.

**Recommended practice:** Use a hardware wallet or threshold-signature scheme for the admin address in any deployment holding significant value.

---

## Audit Status

This contract is unaudited. Do not use in production without an independent security audit. If you discover a vulnerability, please open a private [GitHub Security Advisory](../../security/advisories/new) rather than a public issue.

## Dependency Advisory Triage

CI runs `cargo deny check advisories` against `Cargo.lock` on every pull request. Resolve reported advisories by updating the affected dependency and reviewing the resulting lockfile changes. If an advisory is accepted temporarily, add its RustSec ID to `deny.toml` under `[advisories].ignore` as an object with a `reason`. The accompanying pull request must explain why the affected code is not exploitable here or why immediate remediation is impractical, identify the planned remediation and owner, and set a review date. Reassess every exception when dependencies change and remove it as soon as the risk is resolved; do not add blanket ignores.

Current exceptions are limited to the `adler` and `paste` unmaintained advisories and the yanked `spin 0.9.8`, all pulled in through Soroban SDK 21.7.7 host dependencies. The advisory database reports no safe upgrades for the two unmaintained crates. The vault maintainers must reassess these entries by 2026-12-31, including when the Soroban SDK dependency is upgraded.
