# Integration Guide: Contracts Calling the Vault

This guide is for Soroban contract developers (lending markets, aggregators,
routers, yield strategies) that need to call the Stellar DeFi Vault from
another contract.

A working example consumer contract lives in
[`examples/consumer-contract/`](../examples/consumer-contract/) with tests
you can run with:

```bash
cargo test --features testutils
```

---

## Contents

1. [Adding the dependency](#1-adding-the-dependency)
2. [Reading positions](#2-reading-positions)
3. [Previewing deposits and redemptions](#3-previewing-deposits-and-redemptions)
4. [Depositing on behalf of a user](#4-depositing-on-behalf-of-a-user)
5. [Auth requirements for every call](#5-auth-requirements-for-every-call)
6. [Error handling](#6-error-handling)
7. [Security considerations](#7-security-considerations)

---

## 1. Adding the dependency

The vault exposes a generated cross-contract client via the `IStakingPoolClient`
and `VaultTraitClient` types in `src/interface.rs`. In a real project you
would reference the vault as a workspace crate or add it as a path/git
dependency. For testing purposes the vault is registered in the same test
environment using `env.register_contract`.

```toml
# Cargo.toml of your consumer contract
[dependencies]
soroban-sdk = { version = "=29.0.1" }

# For integration tests against the real vault contract:
[dev-dependencies]
stellar-defi-vault = { path = "../Stellar-Defi-Vault", features = ["testutils"] }
```

Build the cross-contract client from the vault's interface:

```rust
use soroban_sdk::contractclient;

// The generated client is already available if you import from the vault crate:
use stellar_defi_vault::interface::IStakingPoolClient;
// or use the full VaultTraitClient for the extended API:
// use stellar_defi_vault::interface::VaultTraitClient;
```

---

## 2. Reading positions

All read-only calls require **no authorization**. Call them freely from any
contract with only the vault contract's address.

### Shares held by a user

```rust
let client = IStakingPoolClient::new(&env, &vault_address);
let shares: i128 = client.staked_amount(&user); // alias: shares_of
```

### Pending (unclaimed) reward

```rust
let pending: i128 = client.calc_pending_reward(&user);
```

### Total staked in the pool

```rust
let total: i128 = client.total_staked();
```

### Vault paused?

```rust
let paused: bool = client.is_paused();
```

### Share price (compute from vault state)

The vault does not expose a single `share_price` function. Derive it from
`vault_state()`:

```rust
// vault_state returns (total_shares, total_deposited)
let (total_shares, total_deposited) = vault_client.vault_state();
// share_price_numerator/denominator; price = total_deposited / total_shares
// Use preview_redeem for concrete amounts:
let tokens_for_100_shares: i128 = vault_client.preview_redeem(&100_i128);
```

---

## 3. Previewing deposits and redemptions

Both preview functions are **read-only** and require no auth. Use them to
simulate the outcome of a deposit or withdrawal before committing.

### Preview a deposit (how many shares would be minted?)

```rust
let shares_out: i128 = vault_client.preview_deposit(&deposit_amount);
```

### Preview a redemption (how many tokens would a share burn return?)

```rust
let tokens_out: i128 = vault_client.preview_redeem(&share_amount);
```

These match the formulas in [docs/ECONOMICS.md](ECONOMICS.md) exactly:

```
preview_deposit(a)  = floor(a × total_shares / total_deposited)   [if pool non-empty]
                    = a × 10^(7 − token_decimals)                  [first deposit]
preview_redeem(x)   = floor(x × total_deposited / total_shares)
```

---

## 4. Depositing on behalf of a user

This is the most common cross-contract flow: your contract collects tokens
from a user and routes them into the vault.

```rust
pub fn deposit_for_user(
    env: Env,
    vault: Address,
    user: Address,
    amount: i128,
    min_shares_out: i128,
) -> i128 {
    let vault_client = VaultContractClient::new(&env, &vault);

    // The vault's `stake` function requires `user.require_auth()`.
    // Your contract must obtain that auth before calling the vault.
    // The standard Soroban pattern: the user authorises YOUR contract,
    // and your contract's sub-invocation of vault.stake() is covered
    // by the same auth tree.
    user.require_auth();

    let shares = vault_client.stake(&user, &amount, &min_shares_out)
        .expect("vault stake failed");

    shares
}
```

The call graph is:

```
User signs tx
  └── YourContract::deposit_for_user(user, amount, …)
        └── VaultContract::stake(user, amount, …)  ← user auth propagated
              └── token::transfer(user → vault, amount)
```

### Slippage protection

Always pass a non-zero `min_shares_out`. The vault reverts with
`VaultError::InsufficientShares` (code 5) if the actual shares minted fall
below this floor, protecting your users from front-running.

---

## 5. Auth requirements for every call

| Function | Who must `require_auth` | Notes |
|---|---|---|
| `stake(user, amount, min_shares_out)` | `user` | Token is pulled from `user` |
| `deposit(depositor, amount, min_shares_out)` | `depositor` | Alias of `stake` |
| `withdraw(withdrawer, shares)` | `withdrawer` | Burns shares, returns tokens to `withdrawer` |
| `unstake(staker, shares)` | `staker` | Same as withdraw |
| `unstake_all(user)` | `user` | Unstakes all shares |
| `claim(staker)` | `staker` | Reward payout to `staker` |
| `claim_partial(staker, amount)` | `staker` | Partial reward payout |
| `stake_and_claim(user, amount)` | `user` | Combines stake + claim |
| `initialize(admin, …)` | `admin` | One-time setup |
| `transfer_admin(new_admin)` | current admin | Admin rotation |
| `set_reward_rate_bps(rate)` | admin | Rate change |
| `pause(…)` / `unpause()` | admin | Emergency controls |
| `add_yield(admin, amount)` | `admin` | Yield injection |
| `fund_reward_pool(funder, amount)` | `funder` | Reward pool top-up |
| — **Read-only (no auth)** — | — | — |
| `shares_of(user)` | none | Balance query |
| `staked_amount(user)` | none | Same as `shares_of` |
| `pending_reward(user)` | none | Pending reward query — use `calc_pending_reward(user)` on the vault |
| `preview_deposit(amount)` | none | Mint simulation |
| `preview_redeem(shares)` | none | Burn simulation |
| `vault_state()` | none | `(total_shares, total_deposited)` |
| `total_staked()` | none | Pool total |
| `is_paused()` | none | Pause state |
| `get_reward_rate_bps()` | none | Current APR |
| `get_version()` | none | Contract version |

---

## 6. Error handling

The vault uses typed `#[contracterror]` enums. Catch them at the call site:

```rust
use stellar_defi_vault::errors::VaultError;

match vault_client.try_stake(&user, &amount, &0_i128) {
    Ok(Ok(shares)) => { /* success */ }
    Ok(Err(VaultError::VaultPaused)) => { /* vault is paused */ }
    Ok(Err(VaultError::PoolCapReached)) => { /* pool is at capacity */ }
    Ok(Err(VaultError::BelowMinimumStake)) => { /* amount too small */ }
    Ok(Err(e)) => { /* other vault error */ }
    Err(_) => { /* host/VM error */ }
}
```

Common errors a consumer contract should handle:

| Error | Meaning |
|---|---|
| `VaultPaused` | Vault is paused; retry later |
| `PoolCapReached` | Pool at max capacity; cannot deposit |
| `BelowMinimumStake` | Amount below the configured minimum |
| `InsufficientShares` | Withdraw/unstake amount exceeds balance, or below `min_shares_out` |
| `InsufficientRewardPool` | Reward pool dry; claim blocked |
| `NotInitialized` | Vault has no admin set |
| `ArithmeticError` | Share math overflow (pathological input) |

---

## 7. Security considerations

- **Never forward user funds without slippage protection.** Always pass a
  meaningful `min_shares_out` to `stake`/`deposit`.
- **Check `is_paused()` before depositing.** A paused vault reverts the
  transaction, but checking first lets you surface a better error.
- **Do not rely on `pending_reward` for settlement.** The value can change
  between your read and the `claim` call. Use `claim` and check its return
  value.
- **Whitelist/KYC.** If the vault has `WhitelistEnabled` or `KycRequired`
  set, your contract must ensure users are approved before routing their
  deposits in. Check for `NotWhitelisted` (code 19) and `KycNotApproved`
  (code 29) in error handling.
- **Contract-address deposits.** The vault's `stake`/`deposit` functions
  require `user.require_auth()`, where `user` is a user account, not a
  contract address. If your contract is the nominal staker, it must be
  registered as a `ContractDelegate` (`register_contract_delegate`) first.
