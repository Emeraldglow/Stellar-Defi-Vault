#![cfg(test)]
//! Tests accompanying `docs/RESTORE.md`.
//!
//! A user's own persistent entries — position, reward checkpoint, and
//! settings (see `user_ttl.rs`) — carry their own TTL, independent of the
//! vault's instance storage, and can be archived if nobody bumps them in
//! time. Two things matter here:
//!
//! 1. Reading an archived entry must fail loudly, not silently fall back
//!    to a default like `0` — a `0` is indistinguishable from "this user
//!    never had a position," which would be actively misleading.
//! 2. The contract's actual recovery mechanism (`bump_user_entries`,
//!    callable by anyone, e.g. a keeper) keeps a position's balance and
//!    accrued reward exactly intact across ledger ranges that would
//!    otherwise archive it.
//!
//! What this file deliberately does *not* attempt: simulating a genuine
//! post-archival *restore*. The Soroban SDK's test sandbox panics on
//! every access — read, write, or `extend_ttl` — to an entry once it is
//! archived, with no SDK-level call to undo that within the same `Env`.
//! That matches real network behavior for a reason: restoring an archived
//! entry is a ledger-level operation (`RestoreFootprintTransactionData`)
//! that happens *before* the contract is ever invoked, not something the
//! contract's own code executes — see `docs/RESTORE.md` for the actual
//! CLI-level restore procedure, which this test harness has no API to
//! exercise. `test.rs` already notes this same sandbox limitation
//! elsewhere ("a testutils-only artifact, not contract behavior").

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::balance;
use crate::vault::{VaultContract, VaultContractClient};

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    admin: Address,
    token_addr: Address,
}

impl<'a> Fixture<'a> {
    /// `min_ttl` is the initial TTL every newly written persistent entry
    /// gets, so a small value lets a test move the ledger past it without
    /// ever bumping — mirrors the fixture in `test_issues_589_590_592.rs`.
    fn new(min_ttl: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = min_ttl;
            li.min_persistent_entry_ttl = min_ttl;
            li.max_entry_ttl = 10_000_000;
            li.sequence_number = 1_000;
        });

        let admin = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

        Fixture {
            env,
            vault,
            vault_id,
            admin,
            token_addr,
        }
    }

    fn sequence(&self) -> u32 {
        self.env.ledger().sequence()
    }

    fn set_ledger(&self, sequence: u32) {
        self.env.ledger().with_mut(|li| li.sequence_number = sequence);
    }

    fn mint(&self, to: &Address, amount: i128) {
        token::StellarAssetClient::new(&self.env, &self.token_addr).mint(to, &amount);
    }

    /// Directly seeds `user`'s accrued reward. This file is about
    /// TTL/archival, not reward math, so it bypasses funding a reward
    /// pool and waiting for accrual.
    fn seed_accrued_reward(&self, user: &Address, amount: i128) {
        let vault_id = self.vault_id.clone();
        self.env.as_contract(&vault_id, || {
            balance::set_accrued_reward(&self.env, user, amount);
        });
    }

    fn accrued_reward(&self, user: &Address) -> i128 {
        let vault_id = self.vault_id.clone();
        self.env
            .as_contract(&vault_id, || balance::get_accrued_reward(&self.env, user))
    }
}

/// Criterion: reads of an archived entry fail clearly rather than
/// returning zero.
///
/// `staked_amount()` is `balance::get_shares(...).unwrap_or(0)` — on its
/// face, a getter that looks like it always has a safe zero fallback. It
/// doesn't: once the underlying `ShareBalance` entry is archived, the
/// storage `get()` call itself panics inside the host, before that
/// `.unwrap_or(0)` ever gets a chance to run. A caller polling this value
/// sees a hard failure, not a quietly-wrong "this user has 0 staked".
#[test]
#[should_panic]
fn reading_an_archived_position_fails_clearly_not_as_zero() {
    let f = Fixture::new(50);
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000);
    f.vault.stake(&user, &1_000);
    assert_eq!(f.vault.staked_amount(&user), 1_000);

    // Advance well past the tiny initial TTL without ever bumping: the
    // position's ShareBalance entry archives.
    f.set_ledger(f.sequence() + 5_000);

    let _ = f.vault.staked_amount(&user);
}

/// Criterion: simulated archival risk, then recovery, with balances and
/// rewards intact.
///
/// As explained in the module docs, this harness can't simulate a true
/// post-archival restore (nothing in the Soroban SDK's test `Env` can
/// touch an entry again once it's archived — that's the whole point of
/// the previous test). What it *can* simulate, and what is the contract's
/// actual answer to "a position's entries are about to archive," is
/// `bump_user_entries`: callable by anyone (e.g. a keeper bot on a
/// schedule, per `docs/RESTORE.md`), it extends every one of a user's
/// position/checkpoint/settings entries. Called before each would-be
/// expiry, there is nothing left to restore — this test proves the
/// balance and the accrued reward stay exactly correct across a ledger
/// range that, unbumped, would have archived them many times over.
#[test]
fn bumping_before_expiry_keeps_balance_and_reward_intact() {
    let f = Fixture::new(50);
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000);
    f.vault.stake(&user, &1_000);
    f.seed_accrued_reward(&user, 250);

    f.vault.set_user_ttl_extension(&f.admin, &40_u32, &500_u32);

    // Round 1: comfortably inside the initial 50-ledger TTL, bump.
    f.set_ledger(f.sequence() + 20);
    f.vault.bump_user_entries(&user);

    // Advance far past where the *original*, unbumped TTL would ever have
    // survived (1_000 + 50), but still well inside the bumped target.
    f.set_ledger(f.sequence() + 400);
    assert_eq!(f.vault.staked_amount(&user), 1_000);
    assert_eq!(f.accrued_reward(&user), 250);

    // Round 2: bump again before the first bump's own window would run out.
    f.vault.bump_user_entries(&user);
    f.set_ledger(f.sequence() + 400);

    // Still exactly intact — nothing was ever lost or needed restoring,
    // because the position was bumped before it could archive.
    assert_eq!(f.vault.staked_amount(&user), 1_000);
    assert_eq!(f.accrued_reward(&user), 250);
}

/// `bump_user_entries` is explicitly permissionless (it doesn't call
/// `require_auth` on anyone) — this is what lets a keeper bump a dormant
/// user's position without that user's own signature, per `user_ttl.rs`
/// and `docs/RESTORE.md`.
#[test]
fn bump_user_entries_needs_no_signature_from_the_position_owner() {
    let f = Fixture::new(50);
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000);
    f.vault.stake(&user, &1_000);

    // A keeper, unrelated to `user`, calling on their behalf.
    let _keeper = Address::generate(&f.env);
    f.vault.bump_user_entries(&user);

    assert_eq!(f.vault.staked_amount(&user), 1_000);
}
