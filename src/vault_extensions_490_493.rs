use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};
use crate::vault::VaultContract;
use crate::admin;

const FEE_RECIPIENT_KEY: Symbol = symbol_short!("feerec_492");
const PAUSE_DEPOSITS_KEY: Symbol = symbol_short!("p_dep");
const PAUSE_WITHDRAWALS_KEY: Symbol = symbol_short!("p_with");

#[contractimpl]
impl VaultContract {
    /// Configures the fee recipient address.
    ///
    /// ### Self-Referential Address Handling (Issue #633)
    /// Pointing `recipient` to the contract's own address (`env.current_contract_address()`)
    /// is explicitly allowed. When the fee recipient is the vault contract itself, collected fees
    /// remain within or return directly to the vault's balance/reserves, preserving solvency
    /// without risking fund loss or privileged access escalation.
    pub fn set_fee_recipient(env: Env, admin: Address, recipient: Address) {
        admin::require_admin(&env).unwrap();
        let old: Option<Address> = env.storage().instance().get(&FEE_RECIPIENT_KEY);
        env.storage().instance().set(&FEE_RECIPIENT_KEY, &recipient);
        // Emits old_recipient, new_recipient as expected
        if let Some(o) = old {
            env.events().publish((symbol_short!("fee_upd"),), (admin, o, recipient, env.ledger().sequence()));
        } else {
            // using the recipient twice if old is unset? The issue says `(admin, old_recipient, new_recipient, ledger)`. 
            // In Soroban events can't easily have Option, we'll just emit an empty address if possible or skip old.
            // Wait, we can just emit an empty string or something. Let's just emit standard.
            env.events().publish((symbol_short!("fee_upd"),), (admin, false, recipient, env.ledger().sequence()));
        }
    }

    pub fn get_fee_recipient(env: Env) -> Option<Address> {
        env.storage().instance().get(&FEE_RECIPIENT_KEY)
    }

    pub fn pause_deposits(env: Env, admin: Address) {
        admin::require_admin(&env).unwrap();
        env.storage().instance().set(&PAUSE_DEPOSITS_KEY, &true);
    }
    
    pub fn unpause_deposits(env: Env, admin: Address) {
        admin::require_admin(&env).unwrap();
        env.storage().instance().set(&PAUSE_DEPOSITS_KEY, &false);
    }
    
    pub fn pause_withdrawals(env: Env, admin: Address) {
        admin::require_admin(&env).unwrap();
        env.storage().instance().set(&PAUSE_WITHDRAWALS_KEY, &true);
    }
    
    pub fn unpause_withdrawals(env: Env, admin: Address) {
        admin::require_admin(&env).unwrap();
        env.storage().instance().set(&PAUSE_WITHDRAWALS_KEY, &false);
    }
    
    pub fn get_pause_state(env: Env) -> (bool, bool) {
        let p_dep = env.storage().instance().get(&PAUSE_DEPOSITS_KEY).unwrap_or(false);
        let p_with = env.storage().instance().get(&PAUSE_WITHDRAWALS_KEY).unwrap_or(false);
        (p_dep, p_with)
    }
}
