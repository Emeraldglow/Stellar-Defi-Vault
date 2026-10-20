# IStakingPool Interface Documentation

The `IStakingPool` interface is a standard composable trait designed to enable cross-contract interactions with the Staking Vault. Any external contract can query a pool implementing this interface to check staked balances, pending rewards, total staking volume, or pause status.

## Public Trait Declaration

```rust
use soroban_sdk::{contractclient, Address, Env};

#[contractclient(name = "IStakingPoolClient")]
pub trait IStakingPool {
    /// Query the user's active staked amount (in underlying token units).
    fn staked_amount(env: Env, user: Address) -> i128;

    /// Query the user's pending reward balance (accrued but not yet claimed/withdrawn).
    fn pending_reward(env: Env, user: Address) -> i128;

    /// Query the total amount of shares or assets currently staked in the pool.
    fn total_staked(env: Env) -> i128;

    /// Query whether the staking pool operations are currently paused.
    fn is_paused(env: Env) -> bool;
}
```

## ABI Specifications

| Function Name | Parameters | Return Type | Description |
|---|---|---|---|
| `staked_amount` | `user: Address` | `i128` | Calculates and returns the user's staked token equivalent. |
| `pending_reward` | `user: Address` | `i128` | Returns the total unclaimed accrued reward in reward token decimals. |
| `total_staked` | *None* | `i128` | Returns the total staked shares or tokens in the pool. |
| `is_paused` | *None* | `bool` | Returns `true` if the contract is paused, `false` otherwise. |

## Cross-Contract Usage Example

Below is a short example showing how an external contract can use `IStakingPoolClient` to query a target pool:

```rust
use soroban_sdk::{contract, contractimpl, Address, Env};
use stellar_defi_vault::interface::IStakingPoolClient;

#[contract]
pub struct ConsumerContract;

#[contractimpl]
impl ConsumerContract {
    /// Retrieve information about a user's position in a target staking pool.
    pub fn get_pool_info(env: Env, pool: Address, user: Address) -> (i128, i128) {
        let client = IStakingPoolClient::new(&env, &pool);
        let staked = client.staked_amount(&user);
        let pending = client.pending_reward(&user);
        (staked, pending)
    }
}
```
