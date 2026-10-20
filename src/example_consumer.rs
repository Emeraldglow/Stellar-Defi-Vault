use crate::interface::IStakingPoolClient;
use soroban_sdk::{contract, contractimpl, Address, Env};

#[contract]
pub struct ExampleConsumer;

#[cfg_attr(not(test), contractimpl)]
impl ExampleConsumer {
    pub fn get_pool_info(env: Env, pool: Address, user: Address) -> (i128, i128) {
        let client = IStakingPoolClient::new(&env, &pool);
        let staked = client.staked_amount(&user);
        let pending = client.pending_reward(&user);
        (staked, pending)
    }
}



