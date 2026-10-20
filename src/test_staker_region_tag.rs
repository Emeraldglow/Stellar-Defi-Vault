#![cfg(test)]

use soroban_sdk::{testutils::Address as _, testutils::Events, token, Address, Env, String, Symbol, TryFromVal};

use crate::errors::VaultCampaignError;
use crate::staker_region_tag::{RegionDistribution, MAX_REGION_CODE_LEN};
use crate::vault::{VaultContract, VaultContractClient};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    admin: Address,
    token_admin: token::StellarAssetClient<'a>,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_admin = token::StellarAssetClient::new(&env, &token_addr);
        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &0_u32, &None, &None);

        Self {
            env,
            vault,
            admin,
            token_admin,
        }
    }

    /// A new address holding an active position of `amount`.
    fn staker(&self, amount: i128) -> Address {
        let user = Address::generate(&self.env);
        self.token_admin.mint(&user, &amount);
        self.vault.deposit(&user, &amount, &0);
        user
    }

    fn code(&self, s: &str) -> String {
        String::from_str(&self.env, s)
    }
}

#[test]
fn valid_code_is_stored_upper_cased() {
    let f = Fixture::new();
    let alice = f.staker(1_000);

    f.vault.set_region_tag(&alice, &f.code("ng"));
    assert_eq!(f.vault.get_region_tag(&alice), Some(f.code("NG")));

    // Replacing an existing tag overwrites it.
    f.vault.set_region_tag(&alice, &f.code("EU"));
    assert_eq!(f.vault.get_region_tag(&alice), Some(f.code("EU")));
}

#[test]
fn set_region_tag_emits_event() {
    let f = Fixture::new();
    let alice = f.staker(1_000);
    f.vault.set_region_tag(&alice, &f.code("US"));

    let (_, topics, data) = f.env.events().all().last().unwrap();
    let topic = Symbol::try_from_val(&f.env, &topics.get(0).unwrap()).unwrap();
    assert_eq!(topic, Symbol::new(&f.env, "region_tag_set"));
    let (user, code, _ledger) = <(Address, String, u32)>::try_from_val(&f.env, &data).unwrap();
    assert_eq!(user, alice);
    assert_eq!(code, f.code("US"));
}

#[test]
fn invalid_characters_are_rejected() {
    let f = Fixture::new();
    let alice = f.staker(1_000);
    for bad in ["U-S", "N G", "EU!", "ÜS", ""] {
        assert_eq!(
            f.vault.try_set_region_tag(&alice, &f.code(bad)),
            Err(Ok(VaultCampaignError::InvalidRegionCode)),
            "{bad:?} should be rejected"
        );
    }
    assert_eq!(f.vault.get_region_tag(&alice), None);
}

#[test]
fn too_long_code_is_rejected() {
    let f = Fixture::new();
    let alice = f.staker(1_000);

    // Exactly the maximum length is accepted.
    let max = "A".repeat(MAX_REGION_CODE_LEN as usize);
    f.vault.set_region_tag(&alice, &f.code(&max));

    let too_long = "A".repeat(MAX_REGION_CODE_LEN as usize + 1);
    assert_eq!(
        f.vault.try_set_region_tag(&alice, &f.code(&too_long)),
        Err(Ok(VaultCampaignError::RegionCodeTooLong))
    );
}

#[test]
fn requires_active_position() {
    let f = Fixture::new();
    let stranger = Address::generate(&f.env);
    assert_eq!(
        f.vault.try_set_region_tag(&stranger, &f.code("US")),
        Err(Ok(VaultCampaignError::PositionNotFound))
    );
}

#[test]
fn clear_removes_tag_and_is_idempotent() {
    let f = Fixture::new();
    let alice = f.staker(1_000);
    f.vault.set_region_tag(&alice, &f.code("US"));

    f.vault.clear_region_tag(&alice);
    assert_eq!(f.vault.get_region_tag(&alice), None);
    assert_eq!(f.vault.get_region_distribution(&f.admin).len(), 0);

    // Clearing again is a harmless no-op.
    f.vault.clear_region_tag(&alice);
}

#[test]
fn distribution_aggregates_tagged_stakers_only() {
    let f = Fixture::new();
    let a = f.staker(1_000);
    let b = f.staker(2_000);
    let c = f.staker(500);
    let _untagged = f.staker(9_999);

    f.vault.set_region_tag(&a, &f.code("NG"));
    f.vault.set_region_tag(&b, &f.code("ng"));
    f.vault.set_region_tag(&c, &f.code("US"));

    let dist = f.vault.get_region_distribution(&f.admin);
    assert_eq!(dist.len(), 2);
    assert_eq!(
        dist.get(0).unwrap(),
        RegionDistribution {
            region_code: f.code("NG"),
            staker_count: 2,
            total_staked: 3_000,
        }
    );
    assert_eq!(
        dist.get(1).unwrap(),
        RegionDistribution {
            region_code: f.code("US"),
            staker_count: 1,
            total_staked: 500,
        }
    );
}

#[test]
fn distribution_is_admin_only() {
    let f = Fixture::new();
    let stranger = Address::generate(&f.env);
    assert_eq!(
        f.vault.try_get_region_distribution(&stranger),
        Err(Ok(VaultCampaignError::Unauthorized))
    );
}
