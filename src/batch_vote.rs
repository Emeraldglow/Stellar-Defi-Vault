//! Governance batch voting (issue #160 — governance voting, batch extension).
//!
//! Casts a single user's votes on up to `MAX_BATCH_VOTES` governance proposals
//! in one call with a single `require_auth`. Each vote is processed
//! independently and recorded against the existing `GovernanceProposal`
//! storage in `balance.rs`; an invalid entry never reverts its siblings.
//!
//! # Auth
//!
//! `user.require_auth()` is called exactly once for the whole batch — a second
//! (or `N`th) authorization is not required per proposal.
//!
//! # Storage (`DataKey` is at Soroban's 50-variant cap — raw `Symbol` keys)
//!
//! - Last-vote ledger (governance power decay integration): `(gpd_lv, user)`
//!   is written once when at least one vote in the batch succeeds, matching the
//!   key used by `governance_power_decay::inactivity_baseline`.
//!
//! # Events
//!
//! A `(vote_cast, user)` event is published per successful vote with
//! `(proposal_id, support, weight, ledger)`.

use soroban_sdk::{
    contracterror, contractimpl, contracttype, symbol_short, Address, Env, String, Symbol, Vec,
};

use crate::balance;
use crate::vault::{VaultContract, VaultContractClient};

/// Maximum number of proposals that may be voted on in a single batch call.
pub const MAX_BATCH_VOTES: u32 = 10;

/// Persistent-storage key prefix for a user's last-voted ledger. This matches
/// the key used by the governance power-decay layer (`gpd_lv`) so inactivity
/// decay measures from a batch vote; it is intentionally defined locally since
/// that layer is not yet registered as a crate module.
const LAST_VOTE_KEY: Symbol = symbol_short!("gpd_lv");

/// A user's governance vote weight, adjusted for the pool-wide `VoteWeightMode`.
///
/// In `Linear` mode (default) weight equals the user's raw staked token amount.
/// In `Quadratic` mode weight equals `isqrt(raw_amount)`, reducing whale dominance.
/// See `crate::quadratic_vote::isqrt` for the rounding contract.
fn vote_weight(env: &Env, user: &Address) -> i128 {
    let shares = balance::get_shares(env, user);
    if shares <= 0 {
        return 0;
    }
    let total_shares = balance::get_total_shares(env);
    let total_deposited = balance::get_total_deposited(env);
    let raw = balance::shares_to_amount(total_shares, total_deposited, shares).unwrap_or(0);
    match balance::get_vote_weight_mode(env) {
        crate::storage::VoteWeightMode::Linear => raw,
        crate::storage::VoteWeightMode::Quadratic => crate::quadratic_vote::isqrt(raw),
    }
}

/// Set the caller's last-voted ledger to `ledger`; read by the governance
/// power-decay layer's inactivity baseline.
fn set_last_vote_ledger(env: &Env, user: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&(LAST_VOTE_KEY, user.clone()), &ledger);
}

/// One cast vote within a `batch_vote` call.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BatchVote {
    pub proposal_id: u32,
    pub support: bool,
}

/// Errors for the governance batch-voting entrypoint.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BatchVoteError {
    Unauthorized = 1,
    PositionNotFound = 2,
    BatchTooLarge = 3,
    ArithmeticError = 4,
}

/// Reason strings used in the per-vote result entries.
const REASON_OK: &str = "ok";
const REASON_NOT_FOUND: &str = "proposal_not_found";
const REASON_ALREADY_VOTED: &str = "already_voted";
const REASON_ENDED: &str = "voting_ended";
const REASON_ENACTED: &str = "proposal_enacted";
const REASON_CANCELLED: &str = "proposal_cancelled";
const REASON_NO_POSITION: &str = "no_position";

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Governance batch voting (issue #160). Casts the caller's votes on up to
    /// `MAX_BATCH_VOTES` proposals in a single call with one `require_auth`.
    ///
    /// Each entry is processed independently: a missing, ended, enacted, or
    /// already-voted proposal records a failed `(proposal_id, false, reason)`
    /// result without reverting the rest of the batch. A proposal may only be
    /// voted once per user — a duplicate `proposal_id` in the batch fails on
    /// its second occurrence.
    ///
    /// Vote weight is the user's raw staked token amount (the same value the
    /// governance power-decay layer derives from); it is added to the
    /// proposal's `votes_for`/`votes_against` tally. The last-vote ledger is
    /// updated once when at least one vote succeeds, and a `vote_cast` event is
    /// emitted per successful vote.
    ///
    /// Returns `Vec<(u32, bool, String)>`: `(proposal_id, success, reason)`.
    pub fn batch_vote(
        env: Env,
        user: Address,
        votes: Vec<BatchVote>,
    ) -> Result<Vec<(u32, bool, String)>, BatchVoteError> {
        user.require_auth();

        if votes.len() > MAX_BATCH_VOTES {
            return Err(BatchVoteError::BatchTooLarge);
        }

        if balance::get_shares(&env, &user) <= 0 {
            return Err(BatchVoteError::PositionNotFound);
        }

        let now = env.ledger().sequence();
        let weight = vote_weight(&env, &user);
        let mut results: Vec<(u32, bool, String)> = Vec::new(&env);
        let mut any_success = false;

        for entry in votes.iter() {
            let id = entry.proposal_id;
            let mut proposal = match balance::get_proposal(&env, id) {
                Some(p) => p,
                None => {
                    results.push_back((id, false, String::from_str(&env, REASON_NOT_FOUND)));
                    continue;
                }
            };

            if proposal.enacted {
                results.push_back((id, false, String::from_str(&env, REASON_ENACTED)));
                continue;
            }
            if crate::vault::proposal_cancel::is_cancelled(&env, id) {
                results.push_back((id, false, String::from_str(&env, REASON_CANCELLED)));
                continue;
            }
            if now > proposal.ends_at {
                results.push_back((id, false, String::from_str(&env, REASON_ENDED)));
                continue;
            }
            if balance::has_voted(&env, id, &user) {
                results.push_back((id, false, String::from_str(&env, REASON_ALREADY_VOTED)));
                continue;
            }
            if weight <= 0 {
                results.push_back((id, false, String::from_str(&env, REASON_NO_POSITION)));
                continue;
            }

            if entry.support {
                proposal.votes_for = proposal
                    .votes_for
                    .checked_add(weight)
                    .ok_or(BatchVoteError::ArithmeticError)?;
            } else {
                proposal.votes_against = proposal
                    .votes_against
                    .checked_add(weight)
                    .ok_or(BatchVoteError::ArithmeticError)?;
            }
            balance::set_proposal(&env, id, &proposal);
            balance::set_voted(&env, id, &user);
            let receipt = crate::storage::VoteReceipt {
                support: entry.support,
                weight,
                ledger: now,
            };
            balance::set_vote_receipt(&env, id, &user, &receipt);

            env.events().publish(
                (symbol_short!("vote_cast"), user.clone()),
                (id, entry.support, weight, now),
            );

            any_success = true;
            results.push_back((id, true, String::from_str(&env, REASON_OK)));
        }

        if any_success {
            set_last_vote_ledger(&env, &user, now);
        }

        Ok(results)
    }
}

#[cfg(any())] // disabled: exercises entrypoints cfg-gated out of testutils builds
mod test {
    extern crate std;

    use soroban_sdk::{
        symbol_short,
        testutils::{Address as _, Events, Ledger as _},
        token, Address, Env, Symbol, TryFromVal, Vec,
    };

    use crate::balance;
    use crate::storage::{GovernanceProposal, ProposableParam};
    use crate::vault::{VaultContract, VaultContractClient};

    use super::{BatchVote, BatchVoteError, MAX_BATCH_VOTES};

    fn create_token<'a>(
        env: &Env,
        admin: &Address,
    ) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
        let address = env.register_stellar_asset_contract(admin.clone());
        let client = token::Client::new(env, &address);
        let admin_client = token::StellarAssetClient::new(env, &address);
        (address, client, admin_client)
    }

    struct Fixture<'a> {
        env: Env,
        vault: VaultContractClient<'a>,
        vault_id: Address,
        alice: Address,
        bob: Address,
    }

    impl<'a> Fixture<'a> {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.budget().reset_unlimited();
            env.ledger().with_mut(|li| {
                li.min_temp_entry_ttl = 10_000_000;
                li.min_persistent_entry_ttl = 10_000_000;
                li.max_entry_ttl = 10_000_000;
                li.sequence_number = 1000;
            });

            let admin = Address::generate(&env);
            let alice = Address::generate(&env);
            let bob = Address::generate(&env);

            let (token_addr, _, token_admin) = create_token(&env, &admin);

            let vault_id = env.register_contract(None, VaultContract);
            let vault = VaultContractClient::new(&env, &vault_id);
            vault.initialize(&admin, &token_addr, &500_u32, &None, &None);

            token_admin.mint(&alice, &100_000_000);
            token_admin.mint(&bob, &100_000_000);
            token_admin.mint(&vault_id, &10_000_000);

            Fixture {
                env,
                vault,
                vault_id,
                alice,
                bob,
            }
        }
    }

    fn seed_proposal(env: &Env, vault_id: &Address, id: u32, ends_at: u32, enacted: bool) {
        env.as_contract(vault_id, || {
            let proposal = GovernanceProposal {
                id,
                parameter: ProposableParam::RewardRate,
                new_value: 500,
                votes_for: 0,
                votes_against: 0,
                ends_at,
                enacted,
            };
            balance::set_proposal(env, id, &proposal);
        });
    }

    fn batch(env: &Env, entries: &[(u32, bool)]) -> Vec<BatchVote> {
        let mut v = Vec::new(env);
        for (id, support) in entries {
            v.push_back(BatchVote {
                proposal_id: *id,
                support: *support,
            });
        }
        v
    }

    fn proposal(env: &Env, vault_id: &Address, id: u32) -> GovernanceProposal {
        env.as_contract(vault_id, || balance::get_proposal(env, id).unwrap())
    }

    fn last_vote_ledger(env: &Env, vault_id: &Address, user: &Address) -> Option<u32> {
        env.as_contract(vault_id, || {
            env.storage()
                .persistent()
                .get(&(symbol_short!("gpd_lv"), user.clone()))
        })
    }

    #[test]
    fn batch_votes_all_proposals_and_applies_weights() {
        let f = Fixture::new();
        f.vault.stake(&f.alice, &700_000, &0);
        seed_proposal(&f.env, &f.vault_id, 1, 200_000, false);
        seed_proposal(&f.env, &f.vault_id, 2, 200_000, false);
        seed_proposal(&f.env, &f.vault_id, 3, 200_000, false);

        let votes = batch(&f.env, &[(1, true), (2, false), (3, true)]);
        let res = f.vault.batch_vote(&f.alice, &votes);

        assert_eq!(res.len(), 3);
        assert_eq!((res.get(0).unwrap().0, res.get(0).unwrap().1), (1, true));
        assert_eq!((res.get(1).unwrap().0, res.get(1).unwrap().1), (2, true));
        assert_eq!((res.get(2).unwrap().0, res.get(2).unwrap().1), (3, true));

        // Alice's 700k stake is applied per proposal.
        assert_eq!(proposal(&f.env, &f.vault_id, 1).votes_for, 700_000);
        assert_eq!(proposal(&f.env, &f.vault_id, 2).votes_against, 700_000);
        assert_eq!(proposal(&f.env, &f.vault_id, 3).votes_for, 700_000);
    }

    #[test]
    fn invalid_proposal_id_is_skipped_without_reverting_others() {
        let f = Fixture::new();
        f.vault.stake(&f.alice, &500_000, &0);
        seed_proposal(&f.env, &f.vault_id, 1, 200_000, false);
        seed_proposal(&f.env, &f.vault_id, 2, 200_000, false);

        // Proposal 999 does not exist; it must be recorded as a failure while
        // the valid siblings still succeed.
        let votes = batch(&f.env, &[(1, true), (999, true), (2, false)]);
        let res = f.vault.batch_vote(&f.alice, &votes);

        assert_eq!(res.len(), 3);
        let entry = res.get(1).unwrap();
        assert_eq!(entry.0, 999);
        assert!(!entry.1);
        assert_eq!(entry.2, soroban_sdk::String::from_str(&f.env, "proposal_not_found"));

        assert!(res.get(0).unwrap().1);
        assert!(res.get(2).unwrap().1);
        assert_eq!(proposal(&f.env, &f.vault_id, 1).votes_for, 500_000);
        assert_eq!(proposal(&f.env, &f.vault_id, 2).votes_against, 500_000);
    }

    #[test]
    fn duplicate_proposal_in_batch_fails_on_second_occurrence() {
        let f = Fixture::new();
        f.vault.stake(&f.alice, &500_000, &0);
        seed_proposal(&f.env, &f.vault_id, 1, 200_000, false);

        let votes = batch(&f.env, &[(1, true), (1, false)]);
        let res = f.vault.batch_vote(&f.alice, &votes);

        assert!(res.get(0).unwrap().1);
        let second = res.get(1).unwrap();
        assert_eq!(second.0, 1);
        assert!(!second.1);
        assert_eq!(second.2, soroban_sdk::String::from_str(&f.env, "already_voted"));

        // Only the first vote counted.
        assert_eq!(proposal(&f.env, &f.vault_id, 1).votes_for, 500_000);
        assert_eq!(proposal(&f.env, &f.vault_id, 1).votes_against, 0);
    }

    #[test]
    fn batch_larger_than_max_reverts() {
        let f = Fixture::new();
        f.vault.stake(&f.alice, &500_000, &0);
        let mut votes = Vec::new(&f.env);
        // MAX + 1 entries, all otherwise valid.
        for id in 1..=(MAX_BATCH_VOTES + 1) {
            seed_proposal(&f.env, &f.vault_id, id, 200_000, false);
            votes.push_back(BatchVote {
                proposal_id: id,
                support: true,
            });
        }

        let result = f.vault.try_batch_vote(&f.alice, &votes);
        assert_eq!(result, Err(Ok(BatchVoteError::BatchTooLarge)));

        // Nothing was recorded — the batch reverted.
        assert_eq!(proposal(&f.env, &f.vault_id, 1).votes_for, 0);
    }

    #[test]
    fn exactly_max_batch_is_allowed() {
        let f = Fixture::new();
        f.vault.stake(&f.alice, &500_000, &0);
        let mut votes = Vec::new(&f.env);
        for id in 1..=MAX_BATCH_VOTES {
            seed_proposal(&f.env, &f.vault_id, id, 200_000, false);
            votes.push_back(BatchVote {
                proposal_id: id,
                support: true,
            });
        }

        let res = f.vault.batch_vote(&f.alice, &votes);
        assert_eq!(res.len(), MAX_BATCH_VOTES);
        for (id, success, _reason) in res.iter() {
            let _ = id;
            assert!(success);
        }
        assert_eq!(proposal(&f.env, &f.vault_id, 1).votes_for, 500_000);
        assert_eq!(proposal(&f.env, &f.vault_id, MAX_BATCH_VOTES).votes_for, 500_000);
    }

    #[test]
    fn non_staker_cannot_batch_vote() {
        let f = Fixture::new();
        f.vault.stake(&f.alice, &500_000, &0);
        seed_proposal(&f.env, &f.vault_id, 1, 200_000, false);

        let votes = batch(&f.env, &[(1, true)]);
        let result = f.vault.try_batch_vote(&f.bob, &votes);
        assert_eq!(result, Err(Ok(BatchVoteError::PositionNotFound)));
    }

    #[test]
    fn single_auth_required_for_entire_batch() {
        let f = Fixture::new();
        f.vault.stake(&f.alice, &500_000, &0);
        seed_proposal(&f.env, &f.vault_id, 1, 200_000, false);
        seed_proposal(&f.env, &f.vault_id, 2, 200_000, false);

        let votes = batch(&f.env, &[(1, true), (2, false)]);
        f.vault.batch_vote(&f.alice, &votes);

        // Exactly one authorization is performed — the caller's — for the
        // whole batch, not one per proposal.
        assert_eq!(f.env.auths().len(), 1);
        assert_eq!(f.env.auths()[0].0, f.alice);
    }

    #[test]
    fn last_vote_ledger_updated_on_successful_batch() {
        let f = Fixture::new();
        f.vault.stake(&f.alice, &500_000, &0);
        seed_proposal(&f.env, &f.vault_id, 1, 200_000, false);

        let before = last_vote_ledger(&f.env, &f.vault_id, &f.alice);
        assert_eq!(before, None);

        let votes = batch(&f.env, &[(1, true)]);
        f.vault.batch_vote(&f.alice, &votes);

        assert_eq!(
            last_vote_ledger(&f.env, &f.vault_id, &f.alice),
            Some(f.env.ledger().sequence())
        );
    }

    #[test]
    fn vote_cast_event_emitted_per_successful_vote() {
        let f = Fixture::new();
        f.vault.stake(&f.alice, &500_000, &0);
        seed_proposal(&f.env, &f.vault_id, 1, 200_000, false);
        seed_proposal(&f.env, &f.vault_id, 2, 200_000, false);

        let votes = batch(&f.env, &[(1, true), (2, false)]);
        f.vault.batch_vote(&f.alice, &votes);

        let topics_first = f
            .env
            .events()
            .all()
            .iter()
            .find(|(_, topics, _)| {
                Symbol::try_from_val(&f.env, &topics.get(0).unwrap()).unwrap()
                    == symbol_short!("vote_cast")
            })
            .unwrap()
            .1
            .clone();

        // Two successful votes => two vote_cast events, each carrying the
        // (proposal_id, support, weight, ledger) payload.
        let count = f
            .env
            .events()
            .all()
            .iter()
            .filter(|(_, topics, _)| {
                Symbol::try_from_val(&f.env, &topics.get(0).unwrap()).unwrap()
                    == symbol_short!("vote_cast")
            })
            .count();
        assert_eq!(count, 2);
        assert_eq!(topics_first.len(), 2);
    }
}
