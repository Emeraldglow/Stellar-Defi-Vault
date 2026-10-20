# Consumer Contract Example

A minimal Soroban contract that integrates with the Stellar DeFi Vault,
demonstrating the three core cross-contract patterns described in
[`docs/INTEGRATION.md`](../../docs/INTEGRATION.md).

## What it shows

| Function | Pattern |
|---|---|
| `read_position(vault, user)` | Reading positions (no auth) |
| `pending_reward(vault, user)` | Reading pending reward (no auth) |
| `simulate_deposit(vault, amount)` | Previewing a deposit (no auth) |
| `simulate_redeem(vault, shares)` | Previewing a redemption (no auth) |
| `deposit_for_user(vault, user, amount)` | Depositing on behalf of a user |
| `deposit_with_min_out(vault, user, amount, min)` | Deposit with slippage protection |

## Running the tests

From the workspace root:

```bash
cargo test --manifest-path examples/consumer-contract/Cargo.toml --features testutils
```

Or from this directory:

```bash
cargo test --features testutils
```

## Auth model

`deposit_for_user` and `deposit_with_min_out` call `vault.stake(user, …)`.
The vault requires `user.require_auth()`. In Soroban the user's top-level
transaction signature propagates down through sub-invocations automatically,
so your contract just needs to declare `user.require_auth()` at its own
level — see the inline comments in `src/lib.rs` for details.
