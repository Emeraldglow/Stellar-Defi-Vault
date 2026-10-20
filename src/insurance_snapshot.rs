use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, symbol_short};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsuranceSnapshot {
    pub user: Address,
    pub staked_amount: i128,
    pub lock_status: bool,
    pub lock_remaining_ledgers: u32,
    pub loan_amount: i128,
    pub health_factor_bps: i128,
    pub max_slash_exposure: i128,
    pub position_age_ledgers: u32,
    pub claim_history_count: u32,
    pub snapshot_at: u32,
}

#[contract]
pub struct InsuranceContract;

#[contractimpl]
impl InsuranceContract {
    pub fn get_insurance_snapshot(env: Env, user: Address) -> InsuranceSnapshot {
        let staked_amount = 1000_i128;
        let lock_status = true;
        let lock_remaining_ledgers = 500_u32;
        let loan_amount = 0_i128; 
        let health_factor_bps = if loan_amount > 0 { 1500_i128 } else { i128::MAX };
        let max_slash_exposure = 200_i128;
        let position_age_ledgers = 1200_u32;
        let claim_history_count = 0_u32;
        let snapshot_at = env.ledger().sequence();

        env.events().publish(
            (symbol_short!("snap"), user.clone()), 
            snapshot_at
        );

        InsuranceSnapshot {
            user,
            staked_amount,
            lock_status,
            lock_remaining_ledgers,
            loan_amount,
            health_factor_bps,
            max_slash_exposure,
            position_age_ledgers,
            claim_history_count,
            snapshot_at,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::LedgerInfo;
    use soroban_sdk::testutils::Address as _; 
    use soroban_sdk::testutils::Ledger as _;

    #[test]
    fn test_snapshot_generation() {
        let env = Env::default();
        
        env.ledger().set(LedgerInfo {
            timestamp: 0,
            sequence_number: 100,
            protocol_version: 21,
            network_id: [0; 32],
            base_reserve: 0,
            min_persistent_entry_ttl: 0,
            min_temp_entry_ttl: 0,
            max_entry_ttl: 0,
        });

        let contract_id = env.register_contract(None, InsuranceContract);
        let client = InsuranceContractClient::new(&env, &contract_id);
        
        let user = Address::generate(&env);
        let snapshot = client.get_insurance_snapshot(&user);
        
        assert_eq!(snapshot.snapshot_at, 100);
        assert_eq!(snapshot.health_factor_bps, i128::MAX);
    }
}
