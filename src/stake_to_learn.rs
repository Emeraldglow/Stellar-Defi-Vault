//! Stake-to-Learn: on-chain quiz system for higher reward tier unlocks (issue #391).
//!
//! The contract entrypoints (`add_quiz`, `submit_quiz_answer`,
//! `get_completed_quizzes`, `get_unlocked_tier`) live in `vault.rs` alongside
//! the rest of the vault's `impl VaultContract` block.
//!
//! This module exposes the quiz storage helpers that are shared with tests
//! and other modules.
//!
//! # Storage keys (all Symbol/tuple-keyed, DataKey is at the 50-variant cap)
//!
//! | Key | Kind | Meaning |
//! |-----|------|---------|
//! | `("quiz", quiz_id)` | persistent | `Quiz` struct for a given quiz |
//! | `("quiz_attempts", user, quiz_id)` | persistent | attempts remaining |
//! | `("completed_quizzes", user)` | persistent | `Vec<u32>` of completed IDs |
//! | `("user_quiz_tier", user)` | persistent | highest tier unlocked |
//! | `Symbol::new(env, "quiz_count")` | instance | total quiz count |
//!
//! See [`crate::balance`] for the low-level storage getter/setter functions.
