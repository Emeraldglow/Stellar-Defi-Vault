#![cfg(test)]
//! Admin handover vs. pending timelocked actions: an admin change must
//! invalidate everything the previous admin queued, so a stale action can't
//! execute under the new admin and the new admin inherits nothing.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Bytes, Env, String, Symbol, TryFromVal,
};

use crate::{
    admin,
    admin_recovery::ADMIN_RECOVERY_DELAY_LEDGERS,
    errors::{VaultExtError, VaultFeature5Error, VaultQuizError},
    storage::AdminAction,
    time_locked_admin_proposal::AdminProposal,
    vault::{VaultContract, VaultContractClient},
};

const TIMELOCK_DELAY: u32 = 100;

fn fixture() -> (Env, VaultContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault_id = env.register_contract(None, VaultContract);
    env.as_contract(&vault_id, || admin::set_admin(&env, &admin));
    let client = VaultContractClient::new(&env, &vault_id);
    client.set_timelock_delay(&TIMELOCK_DELAY);
    (env, client, admin)
}

fn advance(env: &Env, ledgers: u32) {
    env.ledger().with_mut(|l| l.sequence_number += ledgers);
}

/// Starts a recovery proposal and advances to just before it is executable,
/// so actions queued next are still mid-delay when the admin changes.
fn start_recovery(env: &Env, client: &VaultContractClient, new_admin: &Address) {
    client.propose_admin_recovery(&Address::generate(env), new_admin);
    advance(env, ADMIN_RECOVERY_DELAY_LEDGERS - 1);
}

/// Completes the recovery (well inside the timelock delay), then advances
/// past the delay so any surviving action would otherwise be executable.
fn finish_recovery(env: &Env, client: &VaultContractClient) {
    advance(env, 1);
    client.execute_admin_recovery();
    advance(env, TIMELOCK_DELAY);
}

#[test]
fn admin_change_drops_actions_queued_via_queue_action() {
    let (env, client, _old_admin) = fixture();
    let new_admin = Address::generate(&env);
    start_recovery(&env, &client, &new_admin);

    let id = client.queue_action(&AdminAction::Pause, &Bytes::new(&env));
    finish_recovery(&env, &client);

    assert_eq!(client.get_admin(), new_admin);
    assert!(client.get_pending_actions().is_empty());
    assert_eq!(
        client.try_execute_action(&id),
        Err(Ok(VaultExtError::ActionNotFound))
    );
    assert!(!client.is_paused());
}

#[test]
fn admin_change_drops_actions_queued_via_queue_admin_action() {
    let (env, client, old_admin) = fixture();
    let new_admin = Address::generate(&env);
    start_recovery(&env, &client, &new_admin);

    let id = client.queue_admin_action(&old_admin, &AdminAction::Pause);
    finish_recovery(&env, &client);

    assert_eq!(
        client.try_execute_admin_action(&new_admin, &id),
        Err(Ok(VaultFeature5Error::ActionNotFound))
    );
}

// #455's `#[contractimpl]` is compiled out under `testutils`, so these
// entrypoints are called directly inside the contract context.
fn announce(env: &Env, client: &VaultContractClient, admin_addr: &Address, value: i128, delay: u32) -> u32 {
    env.as_contract(&client.address, || {
        VaultContract::announce_config_change(
            env.clone(),
            admin_addr.clone(),
            String::from_str(env, "fee_bps"),
            value,
            delay,
        )
    })
    .unwrap()
}

fn execute_announced(
    env: &Env,
    client: &VaultContractClient,
    admin_addr: &Address,
    id: u32,
) -> Result<(), VaultQuizError> {
    env.as_contract(&client.address, || {
        VaultContract::execute_config_change(env.clone(), admin_addr.clone(), id)
    })
}

fn announced(env: &Env, client: &VaultContractClient, id: u32) -> AdminProposal {
    env.as_contract(&client.address, || VaultContract::get_admin_proposal(env.clone(), id))
        .unwrap()
}

#[test]
fn admin_change_cancels_open_config_change_announcements() {
    let (env, client, old_admin) = fixture();
    let new_admin = Address::generate(&env);

    let executed = announce(&env, &client, &old_admin, 10, 0);
    execute_announced(&env, &client, &old_admin, executed).unwrap();

    start_recovery(&env, &client, &new_admin);
    let open = announce(&env, &client, &old_admin, 50, TIMELOCK_DELAY);
    finish_recovery(&env, &client);

    // History is preserved: the executed one stays executed, the open one
    // is marked cancelled rather than deleted.
    let executed = announced(&env, &client, executed);
    assert!(executed.executed && !executed.cancelled);
    assert!(announced(&env, &client, open).cancelled);
    assert_eq!(
        execute_announced(&env, &client, &new_admin, open),
        Err(VaultQuizError::AdminProposalAlreadyCancelled)
    );
}

#[test]
fn admin_change_emits_invalidation_event_with_counts() {
    let (env, client, old_admin) = fixture();
    let new_admin = Address::generate(&env);
    start_recovery(&env, &client, &new_admin);

    client.queue_action(&AdminAction::Pause, &Bytes::new(&env));
    client.queue_action(&AdminAction::Unpause, &Bytes::new(&env));
    client.queue_admin_action(&old_admin, &AdminAction::Pause);
    announce(&env, &client, &old_admin, 1, TIMELOCK_DELAY);

    advance(&env, 1);
    client.execute_admin_recovery();

    let event = env
        .events()
        .all()
        .iter()
        .find(|(_, topics, _)| {
            topics.get(0).and_then(|t| Symbol::try_from_val(&env, &t).ok())
                == Some(Symbol::new(&env, "adm_inval"))
        })
        .expect("adm_inval event should be emitted on admin change");
    let (_, topics, data) = event;
    assert_eq!(Address::try_from_val(&env, &topics.get(1).unwrap()).unwrap(), old_admin);
    let (to, queued, timelocked, announced): (Address, u32, u32, u32) =
        TryFromVal::try_from_val(&env, &data).unwrap();
    assert_eq!((to, queued, timelocked, announced), (new_admin, 2, 1, 1));
}

#[test]
fn new_admin_can_requeue_and_execute_after_a_fresh_delay() {
    let (env, client, _old_admin) = fixture();
    let new_admin = Address::generate(&env);
    start_recovery(&env, &client, &new_admin);
    client.queue_action(&AdminAction::Pause, &Bytes::new(&env));
    finish_recovery(&env, &client);

    let id = client.queue_action(&AdminAction::Pause, &Bytes::new(&env));
    assert_eq!(
        client.try_execute_action(&id),
        Err(Ok(VaultExtError::ActionNotYetExecutable))
    );
    advance(&env, TIMELOCK_DELAY);
    client.execute_action(&id);
    assert!(client.is_paused());
}

#[test]
fn rewriting_the_same_admin_keeps_pending_actions() {
    let (env, client, admin_addr) = fixture();
    let id = client.queue_action(&AdminAction::Pause, &Bytes::new(&env));

    env.as_contract(&client.address, || admin::set_admin(&env, &admin_addr));

    assert_eq!(client.get_pending_actions().get(0).unwrap().id, id);
}
