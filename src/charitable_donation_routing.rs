//! Opt-in charitable giving: users route a percentage of claimed rewards to
//! registered charity addresses (issue #419).
//!
//! # Storage
//!
//! `DataKey` is at Soroban's 50-variant cap, so this uses raw `Symbol`-keyed
//! storage, matching `balance.rs`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

use crate::admin;
use crate::errors::VaultError;
use crate::vault::VaultContract;

const CHARITY_LIST_KEY: Symbol = symbol_short!("ch_lst");
const DONATION_CFG_KEY: Symbol = symbol_short!("ch_dcfg");
const DONATED_KEY: Symbol = symbol_short!("ch_don");

const MAX_CHARITIES: u32 = 10;
const MAX_DONATION_BPS: u32 = 5000;

/// A registered charity: address + display name.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CharityEntry {
    pub address: Address,
    pub name: String,
}

/// A user's donation configuration: which charity and what percentage.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DonationConfig {
    pub charity: Address,
    pub donation_bps: u32,
}

fn get_charities(env: &Env) -> Vec<CharityEntry> {
    env.storage()
        .instance()
        .get(&CHARITY_LIST_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_charities(env: &Env, charities: &Vec<CharityEntry>) {
    env.storage().instance().set(&CHARITY_LIST_KEY, charities);
}

fn get_donation_config(env: &Env, user: &Address) -> Option<DonationConfig> {
    env.storage()
        .persistent()
        .get(&(DONATION_CFG_KEY, user.clone()))
}

fn set_donation_config(env: &Env, user: &Address, config: &DonationConfig) {
    env.storage()
        .persistent()
        .set(&(DONATION_CFG_KEY, user.clone()), config);
}

fn get_total_donated(env: &Env, charity: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&(DONATED_KEY, charity.clone()))
        .unwrap_or(0)
}

fn set_total_donated(env: &Env, charity: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&(DONATED_KEY, charity.clone()), &amount);
}

/// Returns the donation routing amount for `user`: `(charity_address, donation_amount)`
/// or `None` if the user has no active donation config or donation_bps is 0.
pub fn compute_donation(env: &Env, user: &Address, reward: i128) -> Option<(Address, i128)> {
    let config = get_donation_config(env, user)?;
    if config.donation_bps == 0 || reward <= 0 {
        return None;
    }
    let donation = reward.saturating_mul(config.donation_bps as i128) / 10_000;
    if donation <= 0 {
        return None;
    }
    Some((config.charity, donation))
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Register a charity address and display name. Admin only. Max 10.
    pub fn add_charity(
        env: Env,
        admin: Address,
        address: Address,
        name: String,
    ) -> Result<(), VaultError> {
        admin.require_auth();
        admin::get_admin(&env)?; // ensure caller is the stored admin

        let mut charities = get_charities(&env);

        // Check max
        if charities.len() >= MAX_CHARITIES {
            return Err(VaultError::MaxPositionsReached);
        }

        // Check no duplicate address
        let n = charities.len();
        let mut i = 0u32;
        while i < n {
            let entry = charities.get(i).unwrap();
            if entry.address == address {
                return Err(VaultError::InvalidAddress);
            }
            i += 1;
        }

        charities.push_back(CharityEntry { address, name });
        set_charities(&env, &charities);
        Ok(())
    }

    /// Remove a charity from the registry. Admin only.
    pub fn remove_charity(env: Env, admin: Address, address: Address) -> Result<(), VaultError> {
        admin.require_auth();
        admin::get_admin(&env)?;

        let mut charities = get_charities(&env);
        let n = charities.len();
        let mut found = false;
        let mut i = 0u32;
        while i < n {
            let entry = charities.get(i).unwrap();
            if entry.address == address {
                charities.remove(i);
                found = true;
                break;
            }
            i += 1;
        }
        if !found {
            return Err(VaultError::InvalidAddress);
        }
        set_charities(&env, &charities);
        Ok(())
    }

    /// Read-only query: all registered charities.
    pub fn get_charities(env: Env) -> Vec<CharityEntry> {
        get_charities(&env)
    }

    /// Set or update the caller's donation configuration. `donation_bps` max
    /// 5000 (50%). The charity must be registered. Set `donation_bps` to 0
    /// to disable donations.
    pub fn set_donation_config(
        env: Env,
        user: Address,
        charity: Address,
        donation_bps: u32,
    ) -> Result<(), VaultError> {
        user.require_auth();

        if donation_bps > MAX_DONATION_BPS {
            return Err(VaultError::InvalidRate);
        }

        // Verify charity is registered
        let charities = get_charities(&env);
        let n = charities.len();
        let mut charity_found = false;
        let mut i = 0u32;
        while i < n {
            let entry = charities.get(i).unwrap();
            if entry.address == charity {
                charity_found = true;
                break;
            }
            i += 1;
        }
        if !charity_found {
            return Err(VaultError::InvalidAddress);
        }

        set_donation_config(
            &env,
            &user,
            &DonationConfig {
                charity,
                donation_bps,
            },
        );
        Ok(())
    }

    /// Read-only query: the caller's donation config, if any.
    pub fn get_donation_config(env: Env, user: Address) -> Option<DonationConfig> {
        get_donation_config(&env, &user)
    }

    /// Lifetime total donated to a charity address.
    pub fn get_total_donated_to_charity(env: Env, charity: Address) -> i128 {
        get_total_donated(&env, &charity)
    }
}
