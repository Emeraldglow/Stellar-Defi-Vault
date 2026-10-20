//! Snapshot-based airdrop distribution (issue #527).
//!
//! Uses the existing `vote_weight_at` snapshot mechanism to let the admin
//! distribute a one-off external token airdrop proportionally to stakers'
//! historical positions at a chosen ledger.
//!
//! # Storage
//!
//! Raw `Symbol`-keyed persistent storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, token, Address, Env, Symbol};

use crate::admin;
use crate::errors::VaultFeature2Error;
use crate::vault::VaultContractClient;
use crate::VaultContract;

const AIRDROP_REGISTRY_KEY: Symbol = symbol_short!("air_rgst");
const AIRDROP_CLAIMED_KEY: Symbol = symbol_short!("air_clmd");
const AIRDROP_COUNTER_KEY: Symbol = symbol_short!("air_cnt");

/// Registry of all created airdrops.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AirdropRecord {
    pub id: u32,
    pub token: Address,
    pub total_amount: i128,
    pub snapshot_ledger: u32,
    pub total_weight_at_snapshot: i128,
    pub created_at: u32,
    pub claimed_count: u32,
}

/// Get the next airdrop id counter.
fn next_airdrop_id(env: &Env) -> u32 {
    let current: u32 = env.storage().instance().get(&AIRDROP_COUNTER_KEY).unwrap_or(0);
    let next = current + 1;
    env.storage().instance().set(&AIRDROP_COUNTER_KEY, &next);
    next
}

/// Read an airdrop record by id.
pub fn get_airdrop(env: &Env, id: u32) -> Option<AirdropRecord> {
    env.storage()
        .persistent()
        .get(&(AIRDROP_REGISTRY_KEY, id))
}

/// Store an airdrop record.
fn set_airdrop(env: &Env, record: &AirdropRecord) {
    env.storage()
        .persistent()
        .set(&(AIRDROP_REGISTRY_KEY, record.id), record);
}

/// Check if a user has already claimed a specific airdrop.
fn has_claimed(env: &Env, user: &Address, airdrop_id: u32) -> bool {
    env.storage()
        .persistent()
        .get(&(AIRDROP_CLAIMED_KEY, user.clone(), airdrop_id))
        .unwrap_or(false)
}

/// Mark a user as having claimed a specific airdrop.
fn mark_claimed(env: &Env, user: &Address, airdrop_id: u32) {
    env.storage().persistent().set(
        &(AIRDROP_CLAIMED_KEY, user.clone(), airdrop_id),
        &true,
    );
}

#[contractimpl]
impl VaultContract {
    /// Admin creates a new airdrop keyed to a past snapshot ledger.
    /// The snapshot_ledger must be in the past.
    /// Returns the airdrop id.
    pub fn create_airdrop(
        env: Env,
        token: Address,
        total_amount: i128,
        snapshot_ledger: u32,
    ) -> Result<u32, VaultFeature2Error> {
        admin::require_admin(&env)?;

        if total_amount <= 0 {
            return Err(VaultFeature2Error::ZeroAmount);
        }

        let current_ledger = env.ledger().sequence();
        if snapshot_ledger >= current_ledger {
            return Err(VaultFeature2Error::InvalidRecoveryConfig);
        }

        // Snapshot the total weight at the given ledger.
        // We use a sentinel "total_staked" address to retrieve the snapshot.
        let total_weight = env
            .storage()
            .persistent()
            .get(&(symbol_short!("snap_w"), snapshot_ledger))
            .unwrap_or(0i128);

        let id = next_airdrop_id(&env);
        let record = AirdropRecord {
            id,
            token: token.clone(),
            total_amount,
            snapshot_ledger,
            total_weight_at_snapshot: total_weight,
            created_at: current_ledger,
            claimed_count: 0,
        };
        set_airdrop(&env, &record);

        env.events().publish(
            (symbol_short!("air_crt"),),
            (id, token, total_amount, snapshot_ledger, current_ledger),
        );
        Ok(id)
    }

    /// User claims their proportional share of an airdrop.
    /// Payout = total_amount * user_weight_at_snapshot / total_weight_at_snapshot.
    /// Reverts on double-claim.
    pub fn claim_airdrop(
        env: Env,
        user: Address,
        airdrop_id: u32,
    ) -> Result<i128, VaultFeature2Error> {
        let record = get_airdrop(&env, airdrop_id)
            .ok_or(VaultFeature2Error::PositionNotFound)?;

        if has_claimed(&env, &user, airdrop_id) {
            return Err(VaultFeature2Error::AlreadyClaimed);
        }

        if record.total_weight_at_snapshot <= 0 {
            return Err(VaultFeature2Error::ZeroAmount);
        }

        // Look up user's weight at the snapshot ledger.
        let user_weight: i128 = env
            .storage()
            .persistent()
            .get(&(symbol_short!("snap_u"), user.clone(), record.snapshot_ledger))
            .unwrap_or(0);

        if user_weight <= 0 {
            return Err(VaultFeature2Error::InsufficientStake);
        }

        // Calculate proportional payout using checked arithmetic.
        let payout = user_weight
            .checked_mul(record.total_amount)
            .ok_or(VaultFeature2Error::ArithmeticError)?
            .checked_div(record.total_weight_at_snapshot)
            .ok_or(VaultFeature2Error::ArithmeticError)?;

        if payout <= 0 {
            return Err(VaultFeature2Error::ZeroAmount);
        }

        mark_claimed(&env, &user, airdrop_id);

        // Update claimed count on the record.
        let mut updated = record.clone();
        updated.claimed_count += 1;
        set_airdrop(&env, &updated);

        // Transfer tokens from the contract to the user.
        // The admin must have funded the contract with the airdrop token beforehand.
        let token_client = token::Client::new(&env, &record.token);
        token_client.transfer(&env.current_contract_address(), &user, &payout);

        env.events().publish(
            (symbol_short!("air_clm"),),
            (user, airdrop_id, payout, env.ledger().sequence()),
        );
        Ok(payout)
    }
}
