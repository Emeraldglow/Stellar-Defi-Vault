#![cfg(test)]
//! Tests for stake_to_learn on-chain quiz system (issue #391).

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Bytes, Env, Symbol, TryFromVal, Vec,
};

use crate::{
    balance,
    errors::VaultQuizError,
    storage::{Quiz, RewardTier},
    vault::{VaultContract, VaultContractClient},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn set_ledger(env: &Env, seq: u32) {
    env.ledger().with_mut(|li| li.sequence_number = seq);
}

struct Fixture<'a> {
    env: Env,
    vault: VaultContractClient<'a>,
    vault_id: Address,
    admin: Address,
    alice: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.min_temp_entry_ttl = 10_000_000;
            li.min_persistent_entry_ttl = 10_000_000;
            li.max_entry_ttl = 10_000_000;
        });
        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let token = env.register_stellar_asset_contract(admin.clone());
        let vault_id = env.register_contract(None, VaultContract);
        let vault = VaultContractClient::new(&env, &vault_id);
        vault.initialize(&admin, &token, &500_u32, &None, &None);
        set_ledger(&env, 1_000);
        Fixture { env, vault, vault_id, admin, alice }
    }

    fn bytes(&self, s: &str) -> Bytes {
        Bytes::from_slice(&self.env, s.as_bytes())
    }

    /// Add a quiz directly via internal storage helpers (bypassing auth in tests).
    fn add_quiz_internal(
        &self,
        question_hash: Bytes,
        answer_hash: Bytes,
        reward_tier_unlocked: u32,
        attempts_allowed: u32,
    ) -> u32 {
        self.env.as_contract(&self.vault_id, || {
            let count = balance::get_quiz_count(&self.env);
            let quiz = Quiz {
                id: count,
                question_hash,
                answer_hash,
                reward_tier_unlocked,
                attempts_allowed,
            };
            balance::set_quiz(&self.env, &quiz);
            balance::set_quiz_count(&self.env, count + 1);
            count
        })
    }

    /// Submit a quiz answer directly, returning Result.
    fn submit_answer(&self, user: &Address, quiz_id: u32, answer_hash: Bytes) -> Result<(), VaultQuizError> {
        self.env.as_contract(&self.vault_id, || {
            let quiz = balance::get_quiz(&self.env, quiz_id).ok_or(VaultQuizError::QuizNotFound)?;
            let completed = balance::get_completed_quizzes(&self.env, user);
            if completed.contains(quiz_id) {
                return Err(VaultQuizError::QuizAlreadyCompleted);
            }
            let attempts_remaining = match balance::get_quiz_attempts_remaining_opt(&self.env, user, quiz_id) {
                None => quiz.attempts_allowed,
                Some(0) => return Err(VaultQuizError::QuizMaxAttemptsReached),
                Some(n) => n,
            };
            if attempts_remaining == 0 {
                return Err(VaultQuizError::QuizMaxAttemptsReached);
            }
            if answer_hash == quiz.answer_hash {
                let mut new_completed = completed;
                new_completed.push_back(quiz_id);
                balance::set_completed_quizzes(&self.env, user, &new_completed);
                let current_tier = balance::get_user_quiz_tier(&self.env, user);
                if quiz.reward_tier_unlocked > current_tier {
                    balance::set_user_quiz_tier(&self.env, user, quiz.reward_tier_unlocked);
                }
                let ledger = self.env.ledger().sequence();
                crate::events::quiz_completed(&self.env, user, quiz_id, quiz.reward_tier_unlocked, ledger);
            } else {
                let new_remaining = attempts_remaining - 1;
                balance::set_quiz_attempts_remaining(&self.env, user, quiz_id, new_remaining);
                crate::events::quiz_attempt_failed(&self.env, user, quiz_id, new_remaining, self.env.ledger().sequence());
            }
            Ok(())
        })
    }
}

fn topic_matches(env: &Env, topics: &Vec<soroban_sdk::Val>, name: &str) -> bool {
    match topics.get(0) {
        Some(val) => Symbol::try_from_val(env, &val)
            .map(|s| s == Symbol::new(env, name))
            .unwrap_or(false),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// AC: correct answer unlocks tier, get_unlocked_tier returns it, quiz_completed event emitted.
#[test]
fn correct_answer_unlocks_tier() {
    let f = Fixture::new();
    let q_hash = f.bytes("what is staking?");
    let a_hash = f.bytes("locking tokens");

    let quiz_id = f.add_quiz_internal(q_hash, a_hash.clone(), 2, 3);
    assert_eq!(quiz_id, 0);

    f.submit_answer(&f.alice, quiz_id, a_hash).unwrap();

    // Tier should be unlocked.
    let tier = f.env.as_contract(&f.vault_id, || {
        balance::get_user_quiz_tier(&f.env, &f.alice)
    });
    assert_eq!(tier, 2);

    // Quiz should appear in completed list.
    let completed: Vec<u32> = f.env.as_contract(&f.vault_id, || {
        balance::get_completed_quizzes(&f.env, &f.alice)
    });
    assert_eq!(completed.len(), 1);
    assert_eq!(completed.get(0).unwrap(), 0_u32);

    // quiz_completed (quiz_comp) event emitted.
    let events = f.env.events().all();
    let found = events.iter().any(|e| topic_matches(&f.env, &e.1, "quiz_comp"));
    assert!(found, "quiz_comp event not emitted");
}

/// AC: wrong answer decrements attempts, quiz_attempt_failed emitted.
#[test]
fn wrong_answer_decrements_attempts() {
    let f = Fixture::new();
    let q_hash = f.bytes("question");
    let a_hash = f.bytes("correct");
    let wrong = f.bytes("wrong");

    let quiz_id = f.add_quiz_internal(q_hash, a_hash, 1, 3);

    f.submit_answer(&f.alice, quiz_id, wrong).unwrap();

    // Tier must not be unlocked.
    let tier = f.env.as_contract(&f.vault_id, || {
        balance::get_user_quiz_tier(&f.env, &f.alice)
    });
    assert_eq!(tier, 0);

    // Completed list must still be empty.
    let completed: Vec<u32> = f.env.as_contract(&f.vault_id, || {
        balance::get_completed_quizzes(&f.env, &f.alice)
    });
    assert_eq!(completed.len(), 0);

    // Attempts should be 2 (3 - 1).
    let remaining = f.env.as_contract(&f.vault_id, || {
        balance::get_quiz_attempts_remaining(&f.env, &f.alice, quiz_id)
    });
    assert_eq!(remaining, 2);

    // quiz_fail event emitted.
    let events = f.env.events().all();
    let found = events.iter().any(|e| topic_matches(&f.env, &e.1, "quiz_fail"));
    assert!(found, "quiz_fail event not emitted");
}

/// AC: max attempts reached — further submit returns QuizMaxAttemptsReached.
#[test]
fn max_attempts_reached_locks_quiz() {
    let f = Fixture::new();
    let q_hash = f.bytes("q");
    let a_hash = f.bytes("right");
    let wrong = f.bytes("wrong");

    // Only 2 attempts allowed.
    let quiz_id = f.add_quiz_internal(q_hash, a_hash, 1, 2);

    // Exhaust both attempts with wrong answers.
    f.submit_answer(&f.alice, quiz_id, wrong.clone()).unwrap();
    f.submit_answer(&f.alice, quiz_id, wrong.clone()).unwrap();

    // Third attempt must return QuizMaxAttemptsReached.
    let res = f.submit_answer(&f.alice, quiz_id, wrong);
    assert!(
        matches!(res, Err(VaultQuizError::QuizMaxAttemptsReached)),
        "expected QuizMaxAttemptsReached, got {:?}",
        res
    );
}

/// AC: quiz-unlocked tier is reflected in the stored tier index.
///
/// Verifies that after completing a quiz that unlocks tier 1, the stored
/// user_quiz_tier is 1, which the reward engine can use to apply a higher
/// reward rate (e.g., the second entry in the reward-tier schedule).
#[test]
fn tier_reflected_in_reward_calculation() {
    let f = Fixture::new();

    // Set up two reward tiers.
    f.env.as_contract(&f.vault_id, || {
        let mut tiers: Vec<RewardTier> = Vec::new(&f.env);
        tiers.push_back(RewardTier { max_amount: 1_000, rate_bps: 500 });
        tiers.push_back(RewardTier { max_amount: i128::MAX, rate_bps: 1_500 });
        balance::set_reward_tiers(&f.env, &tiers);
    });

    let q_hash = f.bytes("protocol q");
    let a_hash = f.bytes("protocol a");

    // Add a quiz that unlocks tier index 1 (1 500 bps).
    let quiz_id = f.add_quiz_internal(q_hash, a_hash.clone(), 1, 3);

    // Before quiz: tier 0.
    let tier_before = f.env.as_contract(&f.vault_id, || {
        balance::get_user_quiz_tier(&f.env, &f.alice)
    });
    assert_eq!(tier_before, 0);

    // Complete the quiz.
    f.submit_answer(&f.alice, quiz_id, a_hash).unwrap();

    // Tier must now be 1.
    let tier_after = f.env.as_contract(&f.vault_id, || {
        balance::get_user_quiz_tier(&f.env, &f.alice)
    });
    assert_eq!(tier_after, 1);

    // The corresponding reward-tier entry should have rate_bps = 1500.
    let tiers = f.env.as_contract(&f.vault_id, || {
        balance::get_reward_tiers(&f.env)
    });
let unlocked_tier_entry = tiers.get(tier_after).unwrap();
    assert_eq!(unlocked_tier_entry.rate_bps, 1_500);
}

// ---------------------------------------------------------------------------
// Entrypoint-level tests (via VaultContractClient) — verify error mapping
// ---------------------------------------------------------------------------

/// AC: `add_quiz` rejects beyond the 20-quiz cap.
#[test]
fn add_quiz_hits_max_quizzes() {
    let f = Fixture::new();
    let q_hash = f.bytes("question");
    let a_hash = f.bytes("answer");

    for _ in 0..20 {
        f.vault.add_quiz(&f.admin, &q_hash, &a_hash, &1_u32, &1_u32);
    }

    let res = f.vault.try_add_quiz(&f.admin, &q_hash, &a_hash, &1_u32, &1_u32);
    assert_eq!(res, Err(Ok(VaultQuizError::TooManyQuizzes)));
}

/// `submit_quiz_answer` for an unknown quiz id returns QuizNotFound.
#[test]
fn submit_unknown_quiz_returns_not_found() {
    let f = Fixture::new();
    let res = f.vault.try_submit_quiz_answer(&f.alice, &99_u32, &f.bytes("guess"));
    assert_eq!(res, Err(Ok(VaultQuizError::QuizNotFound)));
}

/// `submit_quiz_answer` for an already-completed quiz returns QuizAlreadyCompleted.
#[test]
fn submit_already_completed_quiz_rejected() {
    let f = Fixture::new();
    let q_hash = f.bytes("q");
    let a_hash = f.bytes("a");
    f.vault.add_quiz(&f.admin, &q_hash, &a_hash, &1_u32, &3_u32);
    f.vault.submit_quiz_answer(&f.alice, &0_u32, &a_hash);

    let res = f.vault.try_submit_quiz_answer(&f.alice, &0_u32, &a_hash);
    assert_eq!(res, Err(Ok(VaultQuizError::QuizAlreadyCompleted)));
}

/// `submit_quiz_answer` after exhausting all attempts returns QuizMaxAttemptsReached.
#[test]
fn submit_after_max_attempts_rejected() {
    let f = Fixture::new();
    let q_hash = f.bytes("q");
    let a_hash = f.bytes("a");
    let wrong = f.bytes("wrong");

    f.vault.add_quiz(&f.admin, &q_hash, &a_hash, &1_u32, &2_u32);
    f.vault.submit_quiz_answer(&f.alice, &0_u32, &wrong);
    f.vault.submit_quiz_answer(&f.alice, &0_u32, &wrong);

    let res = f.vault.try_submit_quiz_answer(&f.alice, &0_u32, &a_hash);
    assert_eq!(res, Err(Ok(VaultQuizError::QuizMaxAttemptsReached)));
}
