#![cfg(test)]

use crate::{vault::VaultContract, vault_extensions_463_466::*};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn test_clawback_window() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token = Address::generate(&env);
    
    // Initialize vault
    let _ = VaultContract::initialize(env.clone(), admin.clone(), token.clone(), 1000, None, None);
    
    // Test setting clawback window
    let result = VaultContract::set_clawback_window_ledgers(env.clone(), admin.clone(), 50_000);
    assert!(result.is_ok());
    
    let window = get_clawback_window(&env);
    assert_eq!(window, 50_000);
}

#[test]
fn test_nft_yield_boost() {
    let env = Env::default();
    let user = Address::generate(&env);
    
    // Without NFT, should return base multiplier
    let boost = get_nft_yield_boost(&env, &user);
    assert_eq!(boost, 10_000);
    
    // Mark user as NFT holder
    set_nft_holder(&env, &user, true);
    
    // With NFT, should return boosted multiplier
    let boost = get_nft_yield_boost(&env, &user);
    assert_eq!(boost, 12_000); // 20% boost
}

#[test]
fn test_milestone_progress() {
    let env = Env::default();
    env.mock_all_auths();
    
    let user = Address::generate(&env);
    
    // Test milestone progress
    let progress = get_user_milestone_progress(&env, &user, 1);
    assert_eq!(progress.milestone_id, 1);
    assert_eq!(progress.progress_pct, 0); // No stake yet
}

#[test]
fn test_parameter_change_log() {
    let env = Env::default();
    let admin = Address::generate(&env);
    
    // Log a parameter change
    log_parameter_change(&env, &admin, "reward_rate", 1000, 2000);
    
    let log = get_parameter_change_log(&env);
    assert_eq!(log.len(), 1);
    
    let entry = log.get(0).unwrap();
    assert_eq!(entry.old_value, 1000);
    assert_eq!(entry.new_value, 2000);
    assert_eq!(entry.changed_by, admin);
}
