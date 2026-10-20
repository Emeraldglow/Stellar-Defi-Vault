#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::{
    vault::{VaultContract, VaultContractClient},
};

const DEPOSITOR_COUNTS: [u32; 3] = [1, 100, 1000];
const BASELINE: &str = include_str!("../resource-cost-baseline.csv");

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    token_admin: token::StellarAssetClient<'a>,
    users: std::vec::Vec<Address>,
}

#[derive(Debug)]
struct Measurement {
    operation: &'static str,
    depositors: u32,
    cpu_instructions: u64,
    memory_bytes: u64,
}

impl<'a> Fixture<'a> {
    fn with_depositors(count: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();
        env.ledger().with_mut(|ledger| {
            ledger.min_temp_entry_ttl = 10_000_000;
            ledger.min_persistent_entry_ttl = 10_000_000;
            ledger.max_entry_ttl = 10_000_000;
            ledger.sequence_number = 1_000;
        });

        let admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract(admin.clone());
        let token_admin = token::StellarAssetClient::new(&env, &token_id);
        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_id, &0_u32, &None, &None);

        let mut users = std::vec::Vec::with_capacity(count as usize);
        for _ in 0..count {
            let user = Address::generate(&env);
            token_admin.mint(&user, &1_000);
            env.budget().reset_unlimited();
            vault.stake(&user, &1_000);
            users.push(user);
        }

        Self {
            env,
            vault,
            token_admin,
            users,
        }
    }

    fn measure(&self, operation: &'static str, depositors: u32, call: impl FnOnce()) -> Measurement {
        self.env.budget().reset_unlimited();
        call();
        Measurement {
            operation,
            depositors,
            cpu_instructions: self.env.budget().cpu_instruction_cost(),
            memory_bytes: self.env.budget().memory_bytes_cost(),
        }
    }

    fn measure_core_operations(&self, depositors: u32) -> std::vec::Vec<Measurement> {
        let depositor = Address::generate(&self.env);
        self.token_admin.mint(&depositor, &1_000);
        let deposit = self.measure("deposit", depositors, || {
            self.vault.deposit(&depositor, &1_000, &None);
        });

        let user = &self.users[0];
        let withdraw = self.measure("withdraw", depositors, || {
            self.vault.withdraw(user, &1);
        });
        let claim = self.measure("claim", depositors, || {
            self.vault.claim(user);
        });

        let exit = self.measure("exit", depositors, || {
            self.vault.unstake_all(user);
        });

        std::vec![deposit, withdraw, claim, exit]
    }
}

fn baseline_for(operation: &str, depositors: u32) -> (u64, u64, u64) {
    for line in BASELINE.lines().filter(|line| !line.starts_with('#')) {
        let mut columns = line.split(',');
        let Some(name) = columns.next() else { continue };
        let Some(count) = columns.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let Some(cpu) = columns.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let Some(memory) = columns.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let Some(tolerance) = columns.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        if name == operation && count == depositors {
            return (cpu, memory, tolerance);
        }
    }
    panic!("missing baseline for {operation} at {depositors} depositors");
}

#[test]
fn core_function_cpu_and_memory_stay_within_committed_budget() {
    for depositors in DEPOSITOR_COUNTS {
        let fixture = Fixture::with_depositors(depositors);
        for measurement in fixture.measure_core_operations(depositors) {
            let (baseline_cpu, baseline_memory, tolerance) =
                baseline_for(measurement.operation, measurement.depositors);
            let allowed_cpu = baseline_cpu + baseline_cpu * tolerance / 100;
            let allowed_memory = baseline_memory + baseline_memory * tolerance / 100;

            assert!(
                measurement.cpu_instructions <= allowed_cpu,
                "CPU regression for {} at {} depositors: baseline={} instructions, tolerance={}%, allowed={}, actual={}",
                measurement.operation,
                measurement.depositors,
                baseline_cpu,
                tolerance,
                allowed_cpu,
                measurement.cpu_instructions
            );
            assert!(
                measurement.memory_bytes <= allowed_memory,
                "memory regression for {} at {} depositors: baseline={} bytes, tolerance={}%, allowed={}, actual={}",
                measurement.operation,
                measurement.depositors,
                baseline_memory,
                tolerance,
                allowed_memory,
                measurement.memory_bytes
            );
            std::println!(
                "{} at {} depositors: cpu={} (limit {}), memory={} (limit {})",
                measurement.operation,
                measurement.depositors,
                measurement.cpu_instructions,
                allowed_cpu,
                measurement.memory_bytes,
                allowed_memory
            );
        }
    }
}
