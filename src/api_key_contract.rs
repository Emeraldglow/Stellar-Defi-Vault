use soroban_sdk::{contract, contractimpl, Address, Bytes, Env};

use crate::admin::{require_admin, require_admin_or_emergency_admin};
use crate::api_key::{APIKey, APIKeyManager};
use crate::errors::VaultError;

#[contract]
pub struct APIKeyContract;

#[contractimpl]
impl APIKeyContract {
    /// Issue a new API key for a partner contract.
    /// 
    /// # Arguments
    /// * `admin` - The admin address (must authenticate)
    /// * `owner` - The address of the partner contract that will use this key
    /// * `valid_for_ledgers` - Number of ledgers the key should be valid for (0 = never expires)
    /// * `max_calls` - Maximum number of calls allowed (0 = unlimited)
    /// 
    /// # Returns
    /// The generated API key bytes (should be stored securely by the partner)
    pub fn issue_api_key(
        env: Env,
        admin: Address,
        owner: Address,
        valid_for_ledgers: u32,
        max_calls: u64,
    ) -> Result<Bytes, VaultError> {
        // Require admin authentication
        admin.require_auth();
        require_admin(&env)?;
        
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, valid_for_ledgers, max_calls)?;
        
        // Emit api_key_issued event
        let topics = (soroban_sdk::symbol_short!("api_key_iss"), admin.clone());
        env.events().publish(
            topics,
            (
                owner.clone(),
                key_bytes.clone(),
                valid_for_ledgers,
                max_calls,
                env.ledger().sequence(),
            ),
        );
        
        Ok(key_bytes)
    }

    /// Revoke an existing API key.
    /// 
    /// # Arguments
    /// * `admin` - The admin address (must authenticate)
    /// * `key_hash` - Hash of the API key to revoke
    pub fn revoke_api_key(env: Env, admin: Address, key_hash: Bytes) -> Result<(), VaultError> {
        // Require admin authentication
        admin.require_auth();
        require_admin(&env)?;
        
        APIKeyManager::revoke_api_key(&env, &key_hash)?;
        
        // Emit api_key_revoked event
        let topics = (soroban_sdk::symbol_short!("api_key_rvk"), admin.clone());
        env.events().publish(
            topics,
            (key_hash.clone(), env.ledger().sequence()),
        );
        
        Ok(())
    }

    /// Verify an API key and increment its call count.
    /// 
    /// # Arguments
    /// * `key_bytes` - The API key bytes to verify
    /// 
    /// # Returns
    /// `true` if the key is valid, `false` otherwise
    pub fn verify_api_key(env: Env, key_bytes: Bytes) -> bool {
        let is_valid = APIKeyManager::verify_api_key(&env, &key_bytes);
        
        if is_valid {
            // Get key hash for event emission
            let key_hash = env.crypto().sha256(&key_bytes);
            
            // Check if max calls reached after this verification
            if let Some(api_key) = APIKeyManager::get_api_key_stats(&env, &key_hash) {
                // Emit api_key_exhausted event if max calls reached
                if api_key.max_calls > 0 && api_key.call_count >= api_key.max_calls {
                    let topics = (soroban_sdk::symbol_short!("api_key_exh"), api_key.owner);
                    env.events().publish(
                        topics,
                        (key_hash.clone(), env.ledger().sequence()),
                    );
                }
            }
        }
        
        is_valid
    }

    /// Get statistics for an API key.
    /// 
    /// # Arguments
    /// * `key_hash` - Hash of the API key
    /// 
    /// # Returns
    /// The API key data if found, `None` otherwise
    pub fn get_api_key_stats(env: Env, key_hash: Bytes) -> Option<APIKey> {
        APIKeyManager::get_api_key_stats(&env, &key_hash)
    }

    /// Validate an API key without incrementing call count.
    /// 
    /// # Arguments
    /// * `key_bytes` - The API key bytes to validate
    /// 
    /// # Returns
    /// `true` if the key is valid, `false` otherwise
    pub fn validate_api_key(env: Env, key_bytes: Bytes) -> bool {
        APIKeyManager::validate_api_key(&env, &key_bytes)
    }

    /// Get the owner of an API key.
    /// 
    /// # Arguments
    /// * `key_bytes` - The API key bytes
    /// 
    /// # Returns
    /// The owner address if the key is valid, `None` otherwise
    pub fn get_key_owner(env: Env, key_bytes: Bytes) -> Option<Address> {
        APIKeyManager::get_key_owner(&env, &key_bytes)
    }

    /// Issue a new API key (emergency admin version).
    /// 
    /// Can be called by either primary admin or emergency admin.
    /// 
    /// # Arguments
    /// * `caller` - The caller address (must authenticate as admin or emergency admin)
    /// * `owner` - The address of the partner contract that will use this key
    /// * `valid_for_ledgers` - Number of ledgers the key should be valid for (0 = never expires)
    /// * `max_calls` - Maximum number of calls allowed (0 = unlimited)
    /// 
    /// # Returns
    /// The generated API key bytes (should be stored securely by the partner)
    pub fn issue_api_key_emergency(
        env: Env,
        caller: Address,
        owner: Address,
        valid_for_ledgers: u32,
        max_calls: u64,
    ) -> Result<Bytes, VaultError> {
        // Require admin or emergency admin authentication
        caller.require_auth();
        require_admin_or_emergency_admin(&env, &caller)?;
        
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, valid_for_ledgers, max_calls)?;
        
        // Emit api_key_issued event with caller as admin
        let topics = (soroban_sdk::symbol_short!("api_key_iss"), caller.clone());
        env.events().publish(
            topics,
            (
                owner.clone(),
                key_bytes.clone(),
                valid_for_ledgers,
                max_calls,
                env.ledger().sequence(),
            ),
        );
        
        Ok(key_bytes)
    }

    /// Revoke an existing API key (emergency admin version).
    /// 
    /// Can be called by either primary admin or emergency admin.
    /// 
    /// # Arguments
    /// * `caller` - The caller address (must authenticate as admin or emergency admin)
    /// * `key_hash` - Hash of the API key to revoke
    pub fn revoke_api_key_emergency(
        env: Env,
        caller: Address,
        key_hash: Bytes,
    ) -> Result<(), VaultError> {
        // Require admin or emergency admin authentication
        caller.require_auth();
        require_admin_or_emergency_admin(&env, &caller)?;
        
        APIKeyManager::revoke_api_key(&env, &key_hash)?;
        
        // Emit api_key_revoked event with caller as admin
        let topics = (soroban_sdk::symbol_short!("api_key_rvk"), caller.clone());
        env.events().publish(
            topics,
            (key_hash.clone(), env.ledger().sequence()),
        );
        
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use soroban_sdk::{testutils::Ledger, Address, Bytes, Env};

    use super::*;
    use crate::admin::{set_admin, set_emergency_admin};

    #[test]
    fn test_issue_api_key_as_admin() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        // Set admin
        set_admin(&env, &admin);
        env.ledger().set_sequence(1000);
        
        // Admin auth
        env.mock_auths(&[(
            admin.clone(),
            &env,
            &[],
        )]);
        
        // Issue API key
        let key_bytes = APIKeyContract::issue_api_key(
            env.clone(),
            admin.clone(),
            owner.clone(),
            1000,
            10,
        ).unwrap();
        
        // Verify the key works
        assert!(APIKeyContract::verify_api_key(env.clone(), key_bytes.clone()));
        
        // Get stats
        let key_hash = env.crypto().sha256(&key_bytes);
        let stats = APIKeyContract::get_api_key_stats(env.clone(), key_hash).unwrap();
        assert_eq!(stats.owner, owner);
        assert_eq!(stats.max_calls, 10);
    }

    #[test]
    fn test_revoke_api_key_as_admin() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        // Set admin
        set_admin(&env, &admin);
        env.ledger().set_sequence(1000);
        
        // Admin auth for issue
        env.mock_auths(&[(
            admin.clone(),
            &env,
            &[],
        )]);
        
        // Issue API key
        let key_bytes = APIKeyContract::issue_api_key(
            env.clone(),
            admin.clone(),
            owner.clone(),
            10000,
            100,
        ).unwrap();
        let key_hash = env.crypto().sha256(&key_bytes);
        
        // Key should work initially
        assert!(APIKeyContract::verify_api_key(env.clone(), key_bytes.clone()));
        
        // Admin auth for revoke
        env.mock_auths(&[(
            admin.clone(),
            &env,
            &[],
        )]);
        
        // Revoke key
        APIKeyContract::revoke_api_key(env.clone(), admin.clone(), key_hash.clone()).unwrap();
        
        // Key should no longer work
        assert!(!APIKeyContract::verify_api_key(env.clone(), key_bytes));
    }

    #[test]
    fn test_issue_api_key_emergency_admin() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let emergency_admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        // Set admin and emergency admin
        set_admin(&env, &admin);
        set_emergency_admin(&env, &emergency_admin).unwrap();
        env.ledger().set_sequence(1000);
        
        // Emergency admin auth
        env.mock_auths(&[(
            emergency_admin.clone(),
            &env,
            &[],
        )]);
        
        // Issue API key using emergency admin
        let key_bytes = APIKeyContract::issue_api_key_emergency(
            env.clone(),
            emergency_admin.clone(),
            owner.clone(),
            1000,
            10,
        ).unwrap();
        
        // Verify the key works
        assert!(APIKeyContract::verify_api_key(env.clone(), key_bytes.clone()));
        
        // Get stats
        let key_hash = env.crypto().sha256(&key_bytes);
        let stats = APIKeyContract::get_api_key_stats(env.clone(), key_hash).unwrap();
        assert_eq!(stats.owner, owner);
    }

    #[test]
    fn test_revoke_api_key_emergency_admin() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let emergency_admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        // Set admin and emergency admin
        set_admin(&env, &admin);
        set_emergency_admin(&env, &emergency_admin).unwrap();
        env.ledger().set_sequence(1000);
        
        // Admin auth for issue
        env.mock_auths(&[(
            admin.clone(),
            &env,
            &[],
        )]);
        
        // Issue API key as regular admin
        let key_bytes = APIKeyContract::issue_api_key(
            env.clone(),
            admin.clone(),
            owner.clone(),
            10000,
            100,
        ).unwrap();
        let key_hash = env.crypto().sha256(&key_bytes);
        
        // Emergency admin auth for revoke
        env.mock_auths(&[(
            emergency_admin.clone(),
            &env,
            &[],
        )]);
        
        // Revoke key using emergency admin
        APIKeyContract::revoke_api_key_emergency(
            env.clone(),
            emergency_admin.clone(),
            key_hash.clone(),
        ).unwrap();
        
        // Key should no longer work
        assert!(!APIKeyContract::verify_api_key(env.clone(), key_bytes));
    }

    #[test]
    fn test_api_key_exhausted_event() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        // Set admin
        set_admin(&env, &admin);
        env.ledger().set_sequence(1000);
        
        // Admin auth
        env.mock_auths(&[(
            admin.clone(),
            &env,
            &[],
        )]);
        
        // Issue API key with max 2 calls
        let key_bytes = APIKeyContract::issue_api_key(
            env.clone(),
            admin.clone(),
            owner.clone(),
            10000,
            2,
        ).unwrap();
        let key_hash = env.crypto().sha256(&key_bytes);
        
        // Use key twice (should trigger exhausted event on second verification)
        assert!(APIKeyContract::verify_api_key(env.clone(), key_bytes.clone()));
        assert!(APIKeyContract::verify_api_key(env.clone(), key_bytes.clone()));
        
        // Third call should fail
        assert!(!APIKeyContract::verify_api_key(env.clone(), key_bytes));
    }

    #[test]
    fn test_validate_without_increment() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        // Set admin
        set_admin(&env, &admin);
        env.ledger().set_sequence(1000);
        
        // Admin auth
        env.mock_auths(&[(
            admin.clone(),
            &env,
            &[],
        )]);
        
        // Issue API key with max 2 calls
        let key_bytes = APIKeyContract::issue_api_key(
            env.clone(),
            admin.clone(),
            owner.clone(),
            10000,
            2,
        ).unwrap();
        
        // Validate without incrementing
        assert!(APIKeyContract::validate_api_key(env.clone(), key_bytes.clone()));
        
        // Call count should still be 0
        let key_hash = env.crypto().sha256(&key_bytes);
        let stats = APIKeyContract::get_api_key_stats(env.clone(), key_hash.clone()).unwrap();
        assert_eq!(stats.call_count, 0);
        
        // Now verify (increments)
        assert!(APIKeyContract::verify_api_key(env.clone(), key_bytes.clone()));
        
        // Call count should be 1
        let stats = APIKeyContract::get_api_key_stats(env.clone(), key_hash).unwrap();
        assert_eq!(stats.call_count, 1);
    }

    #[test]
    fn test_get_key_owner() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        // Set admin
        set_admin(&env, &admin);
        env.ledger().set_sequence(1000);
        
        // Admin auth
        env.mock_auths(&[(
            admin.clone(),
            &env,
            &[],
        )]);
        
        // Issue API key
        let key_bytes = APIKeyContract::issue_api_key(
            env.clone(),
            admin.clone(),
            owner.clone(),
            10000,
            100,
        ).unwrap();
        
        // Should be able to get owner
        let retrieved_owner = APIKeyContract::get_key_owner(env.clone(), key_bytes.clone()).unwrap();
        assert_eq!(retrieved_owner, owner);
        
        // Invalid key should return None
        let invalid_key = Bytes::from_array(&env, &[0u8; 32]);
        assert!(APIKeyContract::get_key_owner(env.clone(), invalid_key).is_none());
    }
}