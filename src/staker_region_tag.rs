//! Voluntary staker region tags (issue #430).
//!
//! Stakers may self-report a short ISO 3166 country / region code (e.g. "US",
//! "NG", "EU") so pool operators can see stake distribution by region for
//! regulatory reporting, liquidity planning, and community building.
//!
//! - Tags are **voluntary and self-reported**: nothing is verified against IP,
//!   KYC, or any other data source. Treat the distribution as indicative.
//! - Codes are 1–10 ASCII alphanumeric characters, normalised to upper case so
//!   "us" and "US" aggregate together.
//! - **Untagged stakers are not included** in `get_region_distribution()`, and
//!   neither are tagged addresses that no longer hold any stake (the tag is
//!   kept, so it applies again if they re-stake, until they clear it).
//! - At most `MAX_REGION_TAGGED_STAKERS` addresses can hold a tag at once so the
//!   distribution report stays within a single invocation's storage footprint.
//!
//! # Storage (`DataKey` is at Soroban's 50-variant cap — raw `Symbol` keys)
//!
//! - Per-user tag: `(Symbol::new(env, "rgn_tag"), user)` -> `String`
//! - Tagged-user index: `symbol_short!("rgn_idx")` -> `Vec<Address>`

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::errors::VaultCampaignError;
use crate::vault::{VaultContract, VaultContractClient};

const INDEX_KEY: Symbol = symbol_short!("rgn_idx");

/// Maximum region code length, in characters.
pub const MAX_REGION_CODE_LEN: u32 = 10;
/// Maximum number of addresses that may hold a region tag at once.
pub const MAX_REGION_TAGGED_STAKERS: u32 = 100;

/// Aggregated stake for one region in `get_region_distribution()`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RegionDistribution {
    pub region_code: String,
    pub staker_count: u32,
    pub total_staked: i128,
}

fn tag_key(env: &Env, user: &Address) -> (Symbol, Address) {
    (Symbol::new(env, "rgn_tag"), user.clone())
}

fn get_index(env: &Env) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&INDEX_KEY)
        .unwrap_or(Vec::new(env))
}

fn set_index(env: &Env, index: &Vec<Address>) {
    env.storage().persistent().set(&INDEX_KEY, index);
}

/// Validate `region_code` and return it upper-cased.
fn normalize_region_code(env: &Env, region_code: &String) -> Result<String, VaultCampaignError> {
    let len = region_code.len();
    if len > MAX_REGION_CODE_LEN {
        return Err(VaultCampaignError::RegionCodeTooLong);
    }
    if len == 0 {
        return Err(VaultCampaignError::InvalidRegionCode);
    }
    let mut buf = [0u8; MAX_REGION_CODE_LEN as usize];
    let bytes = &mut buf[..len as usize];
    region_code.copy_into_slice(bytes);
    for b in bytes.iter_mut() {
        if !b.is_ascii_alphanumeric() {
            return Err(VaultCampaignError::InvalidRegionCode);
        }
        b.make_ascii_uppercase();
    }
    Ok(String::from_bytes(env, bytes))
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #430: set the caller's voluntary region tag (1–10 alphanumeric
    /// characters, e.g. an ISO 3166 code). Requires an active position.
    /// Replaces any existing tag.
    pub fn set_region_tag(
        env: Env,
        user: Address,
        region_code: String,
    ) -> Result<(), VaultCampaignError> {
        user.require_auth();
        if balance::get_shares(&env, &user) <= 0 {
            return Err(VaultCampaignError::PositionNotFound);
        }
        let code = normalize_region_code(&env, &region_code)?;

        let key = tag_key(&env, &user);
        if !env.storage().persistent().has(&key) {
            let mut index = get_index(&env);
            if index.len() >= MAX_REGION_TAGGED_STAKERS {
                return Err(VaultCampaignError::TooManyRegionTags);
            }
            index.push_back(user.clone());
            set_index(&env, &index);
        }
        env.storage().persistent().set(&key, &code);

        env.events().publish(
            (Symbol::new(&env, "region_tag_set"),),
            (user, code, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Issue #430: the region tag for `user`, if any.
    pub fn get_region_tag(env: Env, user: Address) -> Option<String> {
        env.storage().persistent().get(&tag_key(&env, &user))
    }

    /// Issue #430: remove the caller's region tag. No-op if none is set.
    pub fn clear_region_tag(env: Env, user: Address) {
        user.require_auth();
        let key = tag_key(&env, &user);
        if !env.storage().persistent().has(&key) {
            return;
        }
        env.storage().persistent().remove(&key);

        let mut index = get_index(&env);
        if let Some(pos) = index.first_index_of(&user) {
            index.remove(pos);
            set_index(&env, &index);
        }

        env.events().publish(
            (Symbol::new(&env, "region_tag_cleared"),),
            (user, env.ledger().sequence()),
        );
    }

    /// Issue #430: admin-only stake distribution by self-reported region.
    ///
    /// Aggregates every tagged address that currently holds stake; untagged
    /// stakers and tagged addresses with no stake are excluded. Regions are
    /// listed in the order they were first seen.
    pub fn get_region_distribution(
        env: Env,
        admin_addr: Address,
    ) -> Result<Vec<RegionDistribution>, VaultCampaignError> {
        admin_addr.require_auth();
        if admin_addr != admin::get_admin(&env)? {
            return Err(VaultCampaignError::Unauthorized);
        }

        let mut out: Vec<RegionDistribution> = Vec::new(&env);
        for user in get_index(&env).iter() {
            let staked = balance::get_shares(&env, &user);
            if staked <= 0 {
                continue;
            }
            let code: String = match env.storage().persistent().get(&tag_key(&env, &user)) {
                Some(code) => code,
                None => continue,
            };

            let mut found = false;
            for i in 0..out.len() {
                let mut entry = out.get_unchecked(i);
                if entry.region_code == code {
                    entry.staker_count += 1;
                    entry.total_staked = entry
                        .total_staked
                        .checked_add(staked)
                        .ok_or(VaultCampaignError::ArithmeticError)?;
                    out.set(i, entry);
                    found = true;
                    break;
                }
            }
            if !found {
                out.push_back(RegionDistribution {
                    region_code: code,
                    staker_count: 1,
                    total_staked: staked,
                });
            }
        }
        Ok(out)
    }
}
