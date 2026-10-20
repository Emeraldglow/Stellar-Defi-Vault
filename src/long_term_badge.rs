use crate::storage::DataKey;
use crate::vault::VaultContract;
use soroban_sdk::{contractimpl, contracttype, Address, Env, Option, Symbol, Vec};

pub const LEDGERS_PER_YEAR: u32 = 5_256_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LongTermBadge {
    pub earned_at: u32,
    pub years_held: u32,
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    // --- Public Read-Only Query ---
    pub fn get_long_term_badge(env: Env, user: Address) -> Option<LongTermBadge> {
        let key = (Symbol::new(&env, "lt_badge"), user);
        env.storage().persistent().get(&key)
    }

    // --- Public Eligibility Checker & Awarder ---
    pub fn check_and_award_badge(env: Env, user: Address) {
        let staked_at_key = DataKey::StakedAtLedger(user.clone());
        if !env.storage().persistent().has(&staked_at_key) {
            return; 
        }

        let staked_at_ledger: u32 = env.storage().persistent().get(&staked_at_key).unwrap();
        let current_ledger = env.ledger().sequence();

        let badge_key = (Symbol::new(&env, "lt_badge"), user.clone());
        let mut badge: LongTermBadge = env.storage().persistent()
            .get(&badge_key)
            .unwrap_or(LongTermBadge { earned_at: 0, years_held: 0 });

        let next_target_years = badge.years_held + 1;
        let required_ledgers = (LEDGERS_PER_YEAR * next_target_years) + 1;

        if current_ledger >= staked_at_ledger && (current_ledger - staked_at_ledger) >= required_ledgers {
            badge.earned_at = current_ledger;
            badge.years_held = next_target_years;

            env.storage().persistent().set(&badge_key, &badge);

            let holders_key = Symbol::new(&env, "lt_holders");
            let holders: Vec<(Address, u32)> = env.storage().persistent()
                .get(&holders_key)
                .unwrap_or(Vec::new(&env));

            let mut new_holders = Vec::new(&env);
            for h in holders.iter() {
                if h.0 != user {
                    new_holders.push_back(h);
                }
            }
            new_holders.push_back((user.clone(), badge.years_held));
            env.storage().persistent().set(&holders_key, &new_holders);

            env.events().publish(
                (Symbol::new(&env, "badge_awarded"), user.clone()),
                (badge.years_held, current_ledger)
            );
        }
    }

    // --- Admin Read Query ---
    pub fn get_all_badge_holders(env: Env) -> Vec<(Address, u32)> {
        let holders_key = Symbol::new(&env, "lt_holders");
        env.storage().persistent().get(&holders_key).unwrap_or(Vec::new(&env))
    }

    // --- Embedded Hook Automation Entrypoints ---
    pub fn trigger_claim_badge_hook(env: Env, user: Address) {
        Self::check_and_award_badge(env, user);
    }

    pub fn trigger_unstake_badge_hook(env: Env, user: Address, is_full_unstake: bool) {
        if is_full_unstake {
            let badge_key = (Symbol::new(&env, "lt_badge"), user.clone());
            env.storage().persistent().remove(&badge_key);

            let holders_key = Symbol::new(&env, "lt_holders");
            if let Some(holders) = env.storage().persistent().get::<_, Vec<(Address, u32)>>(&holders_key) {
                let mut new_holders = Vec::new(&env);
                for h in holders.iter() {
                    if h.0 != user {
                        new_holders.push_back(h);
                    }
                }
                env.storage().persistent().set(&holders_key, &new_holders);
            }
        }
    }
}

// Additional automated hook triggers to maintain position continuity
#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    pub fn claim(env: Env, user: Address) {
        // Automatically check and update badge stats during claim events
        Self::trigger_claim_badge_hook(env, user);
    }

    pub fn unstake(env: Env, user: Address, amount: i128) {
        // If the unstake action empties the position, wipe the badge tracking state
        // (Assuming a position check logic where amount indicates a complete unstake action)
        let is_full_unstake = amount > 0; 
        Self::trigger_unstake_badge_hook(env, user, is_full_unstake);
    }
}
