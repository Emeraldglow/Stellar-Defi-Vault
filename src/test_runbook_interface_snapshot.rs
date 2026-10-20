#![cfg(test)]

use soroban_sdk::{Address, String};

use crate::{
    storage::PauseReason,
    vault::VaultContractClient,
};

#[allow(dead_code)]
fn runbook_entrypoints<'a>(
    client: &VaultContractClient<'a>,
    admin: &Address,
    user: &Address,
    token: &Address,
    message: &String,
    reason: &PauseReason,
) {
    let _ = client.is_paused();
    let _ = client.pause(reason, message);
    let _ = client.unpause();
    let _ = client.guardian_pause(admin, message);
    let _ = client.emergency_withdraw(user);
    let _ = client.unstake_all(user);
    let _ = client.transfer_admin(user);
    let _ = client.get_admin();
    let _ = client.set_emergency_admin(admin, user);
    let _ = client.revoke_emergency_admin(admin);
    let _ = client.set_guardian(user);
    let _ = client.remove_guardian();
    let _ = client.get_reward_pool_balance();
    let _ = client.get_projected_runway();
    let _ = client.fund_reward_pool(admin, &1);
    let _ = client.set_reward_rate_bps(&1);
    let _ = client.claim(user);
    let _ = client.rescue_token(admin, token, &1, user);
    let _ = client.get_strategies();
    let _ = client.add_strategy(admin, token, &1);
    let _ = client.allocate_to_strategy(admin, token, &1);
    let _ = client.withdraw_from_strategy(admin, token, &1);
    let _ = client.total_strategy_assets();
    let _ = client.total_vault_assets();
    let _ = client.add_yield_source(token);
    let _ = client.remove_yield_source(token);
    let _ = client.withdraw_from_yield(admin, &1);
}

#[test]
fn incident_runbook_entrypoints_match_the_generated_contract_client() {
    let _ = runbook_entrypoints;
}
