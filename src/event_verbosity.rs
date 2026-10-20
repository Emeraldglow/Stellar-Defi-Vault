use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Vec,
};

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EventCategory {
    CoreOps = 0,
    Rewards = 1,
    Governance = 2,
    Analytics = 3,
    Admin = 4,
    Integration = 5,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Verbosity(u32),
    Admin,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    Unauthorized = 1,
    CoreOpsCannotBeDisabled = 2,
    AlreadyInitialized = 3,
}

#[contract]
pub struct EventVerbosityContract;

#[contractimpl]
impl EventVerbosityContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    pub fn set_event_verbosity(
        env: Env,
        admin: Address,
        category: u32,
        enabled: bool,
    ) -> Result<(), Error> {
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        stored_admin.require_auth();

        if category == 0 {
            // CoreOps
            if !enabled {
                return Err(Error::CoreOpsCannotBeDisabled);
            }
        }

        env.storage()
            .instance()
            .set(&DataKey::Verbosity(category), &enabled);

        let ledger = env.ledger().sequence();
        env.events().publish(
            (symbol_short!("updated"), stored_admin, category),
            (enabled, ledger),
        );

        Ok(())
    }

    // Returning primitive integers ensures the Windows test environment doesn't suffer stack misalignments
    pub fn get_event_verbosity(env: Env) -> Vec<(u32, bool)> {
        let mut config = Vec::new(&env);
        for cat in 0..6 {
            let is_enabled = Self::is_category_enabled_internal(&env, cat);
            config.push_back((cat, is_enabled));
        }
        config
    }

    pub fn emit_test_event(env: Env, category: u32) -> bool {
        if Self::is_category_enabled_internal(&env, category) {
            env.events().publish(
                (symbol_short!("test_ev"), env.current_contract_address()),
                (category, 1_u32),
            );
            true
        } else {
            false
        }
    }
}

impl EventVerbosityContract {
    fn is_category_enabled_internal(env: &Env, category: u32) -> bool {
        if category == 0 {
            // CoreOps
            return true;
        }
        if let Some(enabled) = env.storage().instance().get(&DataKey::Verbosity(category)) {
            enabled
        } else {
            match category {
                1 | 2 | 4 => true, // Rewards, Governance, Admin = on by default
                3 | 5 => false,    // Analytics, Integration = off by default
                _ => false,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Events, Ledger, LedgerInfo},
        Symbol, TryFromVal,
    };

    fn setup_test(env: &Env) -> (Address, EventVerbosityContractClient) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract_id = env.register_contract(None, EventVerbosityContract);
        let client = EventVerbosityContractClient::new(env, &contract_id);
        client.initialize(&admin);
        (admin, client)
    }

    #[test]
    fn test_default_verbosity_configuration() {
        let env = Env::default();
        let (_, client) = setup_test(&env);

        let verbosity = client.get_event_verbosity();

        assert_eq!(verbosity.get(0).unwrap(), (0, true)); // CoreOps
        assert_eq!(verbosity.get(1).unwrap(), (1, true)); // Rewards
        assert_eq!(verbosity.get(2).unwrap(), (2, true)); // Governance
        assert_eq!(verbosity.get(3).unwrap(), (3, false)); // Analytics
        assert_eq!(verbosity.get(4).unwrap(), (4, true)); // Admin
        assert_eq!(verbosity.get(5).unwrap(), (5, false)); // Integration
    }

    #[test]
    fn test_coreops_cannot_be_disabled() {
        let env = Env::default();
        let (admin, client) = setup_test(&env);

        let result = client.try_set_event_verbosity(&admin, &0, &false);
        assert!(result.is_err());
    }

    #[test]
    fn test_initialize_is_one_time() {
        let env = Env::default();
        let (admin, client) = setup_test(&env);

        assert_eq!(
            client.try_initialize(&admin),
            Err(Ok(Error::AlreadyInitialized))
        );
    }

    #[test]
    fn test_initialize_rejects_missing_admin_auth() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, EventVerbosityContract);
        let client = EventVerbosityContractClient::new(&env, &contract_id);

        assert!(client.try_initialize(&admin).is_err());
    }

    #[test]
    fn test_set_verbosity_rejects_a_different_admin_address() {
        let env = Env::default();
        let (admin, client) = setup_test(&env);
        let other = Address::generate(&env);

        assert_eq!(
            client.try_set_event_verbosity(&other, &3, &true),
            Err(Ok(Error::Unauthorized))
        );
        assert_eq!(client.get_event_verbosity().get(3).unwrap(), (3, false));
        assert_ne!(admin, other);
    }

    #[test]
    fn test_updated_event_indexes_admin_address() {
        let env = Env::default();
        let (admin, client) = setup_test(&env);
        client.set_event_verbosity(&admin, &3, &true);

        let update = env
            .events()
            .all()
            .into_iter()
            .find(|(_, topics, _)| {
                Symbol::try_from_val(&env, &topics.get(0).unwrap())
                    .map(|symbol| symbol == Symbol::new(&env, "updated"))
                    .unwrap_or(false)
            })
            .unwrap();
        assert_eq!(update.1.len(), 3);
        assert_eq!(
            Address::try_from_val(&env, &update.1.get(1).unwrap()).unwrap(),
            admin
        );
    }

    #[test]
    fn test_toggle_and_event_suppression() {
        let env = Env::default();

        env.ledger().set(LedgerInfo {
            timestamp: 0,
            sequence_number: 555,
            protocol_version: 21,
            network_id: [0; 32],
            base_reserve: 0,
            min_persistent_entry_ttl: 0,
            min_temp_entry_ttl: 0,
            max_entry_ttl: 0,
        });

        let (admin, client) = setup_test(&env);

        let emitted_before = client.emit_test_event(&3); // Analytics
        assert!(!emitted_before);

        let _ = client.set_event_verbosity(&admin, &3, &true);

        let emitted_after = client.emit_test_event(&3);
        assert!(emitted_after);
    }
}
