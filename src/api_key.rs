use soroban_sdk::{contracttype, Address, Bytes, Env};

use crate::errors::VaultError;

/// API Key struct for whitelabel pool integrations.
/// 
/// Partner applications register their contract address and receive an access token
/// that must be included in read-only calls to premium data endpoints, enabling
/// rate limiting and access control for high-frequency integrations.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct APIKey {
    /// Hash of the API key bytes (for verification)
    pub key_hash: Bytes,
    /// Owner address (the contract that can use this key)
    pub owner: Address,
    /// Ledger sequence when the key was issued
    pub issued_at: u32,
    /// Ledger sequence when the key expires (0 = never expires)
    pub expires_at: u32,
    /// Number of times this key has been used for verification
    pub call_count: u64,
    /// Maximum number of calls allowed (0 = unlimited)
    pub max_calls: u64,
    /// Whether the key has been revoked
    pub revoked: bool,
}

/// Storage keys for API keys.
/// Uses Symbol keys since DataKey enum is at its 50-variant limit.
pub struct APIKeyStorage;

impl APIKeyStorage {
    /// Get an API key by its hash.
    pub fn get(env: &Env, key_hash: &Bytes) -> Option<APIKey> {
        let key = (soroban_sdk::symbol_short!("api_key"), key_hash.clone());
        env.storage().persistent().get(&key)
    }

    /// Set an API key.
    pub fn set(env: &Env, key_hash: &Bytes, api_key: &APIKey) {
        let key = (soroban_sdk::symbol_short!("api_key"), key_hash.clone());
        env.storage().persistent().set(&key, api_key);
    }

    /// Remove an API key.
    pub fn remove(env: &Env, key_hash: &Bytes) {
        let key = (soroban_sdk::symbol_short!("api_key"), key_hash.clone());
        env.storage().persistent().remove(&key);
    }

    /// Check if an API key exists.
    pub fn has(env: &Env, key_hash: &Bytes) -> bool {
        let key = (soroban_sdk::symbol_short!("api_key"), key_hash.clone());
        env.storage().persistent().has(&key)
    }
}

/// API key management and verification functions.
pub struct APIKeyManager;

impl APIKeyManager {
    /// Issue a new API key for a partner contract.
    /// 
    /// # Arguments
    /// * `env` - The contract environment
    /// * `owner` - The address of the partner contract that will use this key
    /// * `valid_for_ledgers` - Number of ledgers the key should be valid for (0 = never expires)
    /// * `max_calls` - Maximum number of calls allowed (0 = unlimited)
    /// 
    /// # Returns
    /// The generated API key bytes (should be stored securely by the partner)
    pub fn issue_api_key(
        env: &Env,
        owner: &Address,
        valid_for_ledgers: u32,
        max_calls: u64,
    ) -> Result<Bytes, VaultError> {
        // Generate random key bytes
        let mut random_bytes = [0u8; 32];
        env.prng().fill(&mut random_bytes);
        let key_bytes = Bytes::from_array(env, &random_bytes);
        
        // Hash the key for storage (we store hash, not plain key)
        let key_hash = env.crypto().sha256(&key_bytes);
        
        let current_ledger = env.ledger().sequence();
        let expires_at = if valid_for_ledgers > 0 {
            current_ledger.checked_add(valid_for_ledgers)
                .ok_or(VaultError::ArithmeticError)?
        } else {
            0 // 0 means never expires
        };
        
        let api_key = APIKey {
            key_hash: key_hash.clone(),
            owner: owner.clone(),
            issued_at: current_ledger,
            expires_at,
            call_count: 0,
            max_calls,
            revoked: false,
        };
        
        // Store the API key
        APIKeyStorage::set(env, &key_hash, &api_key);
        
        Ok(key_bytes)
    }

    /// Revoke an existing API key.
    /// 
    /// # Arguments
    /// * `env` - The contract environment
    /// * `key_hash` - Hash of the API key to revoke
    pub fn revoke_api_key(env: &Env, key_hash: &Bytes) -> Result<(), VaultError> {
        let mut api_key = APIKeyStorage::get(env, key_hash)
            .ok_or(VaultError::PositionNotFound)?; // Using PositionNotFound as "key not found"
        
        api_key.revoked = true;
        APIKeyStorage::set(env, key_hash, &api_key);
        
        Ok(())
    }

    /// Verify an API key and increment its call count.
    /// 
    /// # Arguments
    /// * `env` - The contract environment
    /// * `key_bytes` - The API key bytes to verify
    /// 
    /// # Returns
    /// `true` if the key is valid, `false` otherwise
    pub fn verify_api_key(env: &Env, key_bytes: &Bytes) -> bool {
        let key_hash = env.crypto().sha256(key_bytes);
        
        let mut api_key = match APIKeyStorage::get(env, &key_hash) {
            Some(key) => key,
            None => return false,
        };
        
        // Check if key is revoked
        if api_key.revoked {
            return false;
        }
        
        // Check if key has expired
        let current_ledger = env.ledger().sequence();
        if api_key.expires_at > 0 && current_ledger > api_key.expires_at {
            return false;
        }
        
        // Check if max calls reached
        if api_key.max_calls > 0 && api_key.call_count >= api_key.max_calls {
            return false;
        }
        
        // Increment call count
        api_key.call_count = api_key.call_count.checked_add(1)
            .unwrap_or(api_key.call_count); // On overflow, keep max value
        
        APIKeyStorage::set(env, &key_hash, &api_key);
        
        true
    }

    /// Get statistics for an API key.
    /// 
    /// # Arguments
    /// * `env` - The contract environment
    /// * `key_hash` - Hash of the API key
    /// 
    /// # Returns
    /// The API key data if found, `None` otherwise
    pub fn get_api_key_stats(env: &Env, key_hash: &Bytes) -> Option<APIKey> {
        APIKeyStorage::get(env, key_hash)
    }

    /// Validate an API key without incrementing call count.
    /// 
    /// # Arguments
    /// * `env` - The contract environment
    /// * `key_bytes` - The API key bytes to validate
    /// 
    /// # Returns
    /// `true` if the key is valid, `false` otherwise
    pub fn validate_api_key(env: &Env, key_bytes: &Bytes) -> bool {
        let key_hash = env.crypto().sha256(key_bytes);
        
        let api_key = match APIKeyStorage::get(env, &key_hash) {
            Some(key) => key,
            None => return false,
        };
        
        // Check if key is revoked
        if api_key.revoked {
            return false;
        }
        
        // Check if key has expired
        let current_ledger = env.ledger().sequence();
        if api_key.expires_at > 0 && current_ledger > api_key.expires_at {
            return false;
        }
        
        // Check if max calls reached
        if api_key.max_calls > 0 && api_key.call_count >= api_key.max_calls {
            return false;
        }
        
        true
    }

    /// Get the owner of an API key.
    /// 
    /// # Arguments
    /// * `env` - The contract environment
    /// * `key_bytes` - The API key bytes
    /// 
    /// # Returns
    /// The owner address if the key is valid, `None` otherwise
    pub fn get_key_owner(env: &Env, key_bytes: &Bytes) -> Option<Address> {
        let key_hash = env.crypto().sha256(key_bytes);
        
        APIKeyStorage::get(env, &key_hash)
            .map(|api_key| api_key.owner)
    }
}

#[cfg(test)]
mod test {
    use soroban_sdk::{testutils::Ledger, Bytes, Env};

    use super::*;
    use crate::errors::VaultError;

    #[test]
    fn test_issue_and_verify_api_key() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        // Set up ledger
        env.ledger().set_sequence(1000);
        
        // Issue API key valid for 1000 ledgers with max 10 calls
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, 1000, 10)
            .unwrap();
        
        // Verify the key
        assert!(APIKeyManager::verify_api_key(&env, &key_bytes));
        
        // Check stats
        let key_hash = env.crypto().sha256(&key_bytes);
        let stats = APIKeyManager::get_api_key_stats(&env, &key_hash).unwrap();
        assert_eq!(stats.owner, owner);
        assert_eq!(stats.issued_at, 1000);
        assert_eq!(stats.expires_at, 2000); // 1000 + 1000
        assert_eq!(stats.call_count, 1);
        assert_eq!(stats.max_calls, 10);
        assert!(!stats.revoked);
    }

    #[test]
    fn test_api_key_expiration() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        // Set up ledger
        env.ledger().set_sequence(1000);
        
        // Issue API key valid for only 10 ledgers
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, 10, 100)
            .unwrap();
        
        // Verify key works initially
        assert!(APIKeyManager::verify_api_key(&env, &key_bytes));
        
        // Advance ledger past expiration
        env.ledger().set_sequence(1011);
        
        // Key should now be invalid
        assert!(!APIKeyManager::verify_api_key(&env, &key_bytes));
    }

    #[test]
    fn test_api_key_max_calls() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        env.ledger().set_sequence(1000);
        
        // Issue API key with max 3 calls
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, 10000, 3)
            .unwrap();
        
        // Use key 3 times
        for i in 0..3 {
            assert!(APIKeyManager::verify_api_key(&env, &key_bytes));
            
            let key_hash = env.crypto().sha256(&key_bytes);
            let stats = APIKeyManager::get_api_key_stats(&env, &key_hash).unwrap();
            assert_eq!(stats.call_count, (i + 1) as u64);
        }
        
        // Fourth call should fail
        assert!(!APIKeyManager::verify_api_key(&env, &key_bytes));
    }

    #[test]
    fn test_revoke_api_key() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        env.ledger().set_sequence(1000);
        
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, 10000, 100)
            .unwrap();
        let key_hash = env.crypto().sha256(&key_bytes);
        
        // Key should work initially
        assert!(APIKeyManager::verify_api_key(&env, &key_bytes));
        
        // Revoke the key
        APIKeyManager::revoke_api_key(&env, &key_hash).unwrap();
        
        // Key should no longer work
        assert!(!APIKeyManager::verify_api_key(&env, &key_bytes));
        
        // Stats should show revoked
        let stats = APIKeyManager::get_api_key_stats(&env, &key_hash).unwrap();
        assert!(stats.revoked);
    }

    #[test]
    fn test_get_key_owner() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        env.ledger().set_sequence(1000);
        
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, 10000, 100)
            .unwrap();
        
        // Should be able to get owner
        let retrieved_owner = APIKeyManager::get_key_owner(&env, &key_bytes).unwrap();
        assert_eq!(retrieved_owner, owner);
        
        // Invalid key should return None
        let invalid_key = Bytes::from_array(&env, &[0u8; 32]);
        assert!(APIKeyManager::get_key_owner(&env, &invalid_key).is_none());
    }

    #[test]
    fn test_validate_without_increment() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        env.ledger().set_sequence(1000);
        
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, 10000, 3)
            .unwrap();
        
        // Validate without incrementing
        assert!(APIKeyManager::validate_api_key(&env, &key_bytes));
        
        // Call count should still be 0
        let key_hash = env.crypto().sha256(&key_bytes);
        let stats = APIKeyManager::get_api_key_stats(&env, &key_hash).unwrap();
        assert_eq!(stats.call_count, 0);
        
        // Now verify (increments)
        assert!(APIKeyManager::verify_api_key(&env, &key_bytes));
        
        // Call count should be 1
        let stats = APIKeyManager::get_api_key_stats(&env, &key_hash).unwrap();
        assert_eq!(stats.call_count, 1);
    }

    #[test]
    fn test_never_expires_key() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        env.ledger().set_sequence(1000);
        
        // Issue key that never expires (valid_for_ledgers = 0)
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, 0, 100)
            .unwrap();
        
        let key_hash = env.crypto().sha256(&key_bytes);
        let stats = APIKeyManager::get_api_key_stats(&env, &key_hash).unwrap();
        assert_eq!(stats.expires_at, 0); // 0 means never expires
        
        // Advance ledger far into the future
        env.ledger().set_sequence(1000000);
        
        // Key should still work
        assert!(APIKeyManager::verify_api_key(&env, &key_bytes));
    }

    #[test]
    fn test_unlimited_calls_key() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        
        env.ledger().set_sequence(1000);
        
        // Issue key with unlimited calls (max_calls = 0)
        let key_bytes = APIKeyManager::issue_api_key(&env, &owner, 10000, 0)
            .unwrap();
        
        // Use key many times
        for _ in 0..100 {
            assert!(APIKeyManager::verify_api_key(&env, &key_bytes));
        }
        
        // Should still work
        assert!(APIKeyManager::verify_api_key(&env, &key_bytes));
    }
}