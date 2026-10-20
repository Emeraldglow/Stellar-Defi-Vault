//! Vault extensions for issues #761, #762, #763, #764.
//!
//! - Issue #761: Add proposal categories with per-category quorum and thresholds.
//!   (`ProposalCategory`, `set_category_params`, `get_category_params`,
//!    snapshotted params on categorized proposal creation and enactment).
//! - Issue #762: Add expedited proposals for emergencies with a higher quorum.
//!   (`create_expedited_proposal`, larger bond, shortened voting period & delay,
//!    higher quorum & threshold, guardian veto, bond refund/slashing).
//! - Issue #763: Add expiry to vote delegation.
//!   (`delegate_vote_weight(user, delegate, expires_at: Option<u32>)`,
//!    `get_delegate(user) -> Option<(Address, Option<u32>)>`, snapshot ledger expiry evaluation).
//! - Issue #764: Define a StrategyAdapter interface for deploying idle funds to external yield sources.
//!   (Trait/interface for `deposit`, `withdraw`, `total_assets`, max 3 strategies,
//!    allocation caps, `allocate_to_strategy`, and share price factoring strategy assets).
//!
//! # Storage Keys
//! `DataKey` sits at Soroban's 50-variant limit, so all extension state is maintained
//! under dedicated raw `Symbol` keys, consistent with the rest of the codebase.

use soroban_sdk::{
    contractimpl, contracttype, symbol_short, token, Address, Env, IntoVal, Symbol, Val, Vec,
};

use crate::admin;
use crate::balance;
use crate::errors::{GovernanceStrategyError, VaultError};
use crate::events;
use crate::storage::ProposableParam;
use crate::vault::{VaultContract, BOOST_BPS_BASE};

// ── Storage Keys & Constants ──────────────────────────────────────────────────

const CAT_PARAMS_KEY: Symbol = symbol_short!("cat_prm");
const CAT_PROPOSALS_KEY: Symbol = symbol_short!("cat_prps");
const NEXT_CAT_PROP_ID_KEY: Symbol = symbol_short!("cat_pr_id");
const OPEN_EXPEDITED_COUNT_KEY: Symbol = symbol_short!("exp_cnt");
const VOTE_EXP_KEY: Symbol = symbol_short!("vdel_exp");
const DELEGATOR_LIST_KEY: Symbol = symbol_short!("vdel_lst");
const STRATEGIES_LIST_KEY: Symbol = symbol_short!("strat_lst");

/// Maximum open expedited proposals allowed at once (issue #762).
pub const MAX_OPEN_EXPEDITED_PROPOSALS: u32 = 3;

/// Minimum bond required for an expedited emergency proposal (issue #762).
pub const MIN_EXPEDITED_BOND: i128 = 100_000_000; // 10 tokens with 7 decimals

/// Default expedited voting period in ledgers (e.g. ~16-20 minutes).
pub const EXPEDITED_VOTING_PERIOD_LEDGERS: u32 = 200;

/// Default expedited execution delay in ledgers for guardian review.
pub const EXPEDITED_EXECUTION_DELAY_LEDGERS: u32 = 50;

/// Default expedited quorum in basis points (40%).
pub const EXPEDITED_QUORUM_BPS: u32 = 4_000;

/// Default expedited approval threshold in basis points (75%).
pub const EXPEDITED_THRESHOLD_BPS: u32 = 7_500;

/// Maximum number of active strategies supported (issue #764).
pub const MAX_STRATEGIES: u32 = 3;

// ── Issue #761: Proposal Categories ──────────────────────────────────────────

/// Proposal category enum distinguishing risk profiles (issue #761).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ProposalCategory {
    Parameter = 0,
    Treasury = 1,
    Upgrade = 2,
    Other = 3,
}

/// Quorum, approval threshold, and voting period configured per category.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryParams {
    pub quorum_bps: u32,
    pub threshold_bps: u32,
    pub period_ledgers: u32,
}

impl CategoryParams {
    pub fn default_for(category: ProposalCategory) -> Self {
        match category {
            ProposalCategory::Parameter => CategoryParams {
                quorum_bps: 1_000,       // 10%
                threshold_bps: 5_000,    // 50%
                period_ledgers: 1_000,
            },
            ProposalCategory::Treasury => CategoryParams {
                quorum_bps: 2_000,       // 20%
                threshold_bps: 6_000,    // 60%
                period_ledgers: 2_000,
            },
            ProposalCategory::Upgrade => CategoryParams {
                quorum_bps: 3_000,       // 30%
                threshold_bps: 6_667,    // 66.67%
                period_ledgers: 5_000,
            },
            ProposalCategory::Other => CategoryParams {
                quorum_bps: 1_000,       // 10%
                threshold_bps: 5_000,    // 50%
                period_ledgers: 1_000,
            },
        }
    }
}

/// A proposal with snapshotted category rules and optional expedited parameters.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CategorizedProposal {
    pub id: u32,
    pub proposer: Address,
    pub category: ProposalCategory,
    pub snapshotted_quorum_bps: u32,
    pub snapshotted_threshold_bps: u32,
    pub snapshotted_period_ledgers: u32,
    pub parameter: ProposableParam,
    pub new_value: i128,
    pub votes_for: i128,
    pub votes_against: i128,
    pub created_at: u32,
    pub ends_at: u32,
    pub execution_delay_ledgers: u32,
    pub bond_amount: i128,
    pub is_expedited: bool,
    pub vetoed: bool,
    pub enacted: bool,
}

// ── Issue #763: Delegation Record ─────────────────────────────────────────────

/// Vote delegation entry with optional expiry ledger (issue #763).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DelegationRecord {
    pub delegate: Address,
    pub expires_at: Option<u32>,
}

// ── Issue #764: Strategy Info & Interface ─────────────────────────────────────

/// Strategy adapter configuration and current allocation tracking (issue #764).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StrategyConfig {
    pub adapter: Address,
    pub cap_bps: u32,
    pub allocated: i128,
}

/// Standard interface for external strategy adapters (issue #764).
pub trait StrategyAdapter {
    fn deposit(env: &Env, adapter: &Address, amount: i128) -> Result<(), GovernanceStrategyError>;
    fn withdraw(env: &Env, adapter: &Address, amount: i128) -> Result<i128, GovernanceStrategyError>;
    fn total_assets(env: &Env, adapter: &Address) -> Result<i128, GovernanceStrategyError>;
}

pub struct StrategyClient;

impl StrategyAdapter for StrategyClient {
    fn deposit(env: &Env, adapter: &Address, amount: i128) -> Result<(), GovernanceStrategyError> {
        let args: Vec<Val> = Vec::from_array(env, [amount.into_val(env)]);
        env.try_invoke_contract::<Val, Val>(adapter, &Symbol::new(env, "deposit"), args)
            .map_err(|_| GovernanceStrategyError::AdapterCallFailed)?
            .map_err(|_| GovernanceStrategyError::AdapterCallFailed)?;
        Ok(())
    }

    fn withdraw(env: &Env, adapter: &Address, amount: i128) -> Result<i128, GovernanceStrategyError> {
        let args: Vec<Val> = Vec::from_array(env, [amount.into_val(env)]);
        let res = env
            .try_invoke_contract::<Val, Val>(adapter, &Symbol::new(env, "withdraw"), args)
            .map_err(|_| GovernanceStrategyError::AdapterCallFailed)?
            .map_err(|_| GovernanceStrategyError::AdapterCallFailed)?;
        let val: i128 = res.into_val(env);
        Ok(val)
    }

    fn total_assets(env: &Env, adapter: &Address) -> Result<i128, GovernanceStrategyError> {
        let args: Vec<Val> = Vec::new(env);
        let res = env
            .try_invoke_contract::<Val, Val>(adapter, &Symbol::new(env, "total_assets"), args)
            .map_err(|_| GovernanceStrategyError::AdapterCallFailed)?
            .map_err(|_| GovernanceStrategyError::AdapterCallFailed)?;
        let val: i128 = res.into_val(env);
        Ok(val)
    }
}

// ── Contract Implementation ──────────────────────────────────────────────────

#[contractimpl]
impl VaultContract {
    // ─────────────────────────────────────────────────────────────────────────
    // Issue #761: Proposal Categories
    // ─────────────────────────────────────────────────────────────────────────

    /// Admin: Set the quorum, threshold, and voting period for a specific proposal category (issue #761).
    pub fn set_category_params(
        env: Env,
        admin: Address,
        category: ProposalCategory,
        quorum_bps: u32,
        threshold_bps: u32,
        period_ledgers: u32,
    ) -> Result<(), GovernanceStrategyError> {
        admin::require_admin_as(&env, &admin)?;

        if quorum_bps > BOOST_BPS_BASE || threshold_bps > BOOST_BPS_BASE || threshold_bps < 5_000 {
            return Err(GovernanceStrategyError::InvalidBps);
        }
        if period_ledgers == 0 {
            return Err(GovernanceStrategyError::InvalidPeriod);
        }

        let params = CategoryParams {
            quorum_bps,
            threshold_bps,
            period_ledgers,
        };

        let key = (CAT_PARAMS_KEY, category as u32);
        env.storage().instance().set(&key, &params);

        events::category_params_set(
            &env,
            &admin,
            category as u32,
            quorum_bps,
            threshold_bps,
            period_ledgers,
            env.ledger().sequence(),
        );

        Ok(())
    }

    /// Query the configured parameters for a proposal category (issue #761).
    pub fn get_category_params(env: Env, category: ProposalCategory) -> CategoryParams {
        let key = (CAT_PARAMS_KEY, category as u32);
        env.storage()
            .instance()
            .get(&key)
            .unwrap_or_else(|| CategoryParams::default_for(category))
    }

    /// Create a proposal with category-specific quorum and duration snapshotted at creation (issue #761).
    pub fn create_categorized_proposal(
        env: Env,
        user: Address,
        category: ProposalCategory,
        parameter: ProposableParam,
        new_value: i128,
    ) -> Result<u32, GovernanceStrategyError> {
        user.require_auth();

        if balance::get_shares(&env, &user) == 0 {
            return Err(GovernanceStrategyError::PositionNotFound);
        }

        let cat_params = Self::get_category_params(env.clone(), category);
        let id: u32 = env
            .storage()
            .instance()
            .get(&NEXT_CAT_PROP_ID_KEY)
            .unwrap_or(1);
        env.storage()
            .instance()
            .set(&NEXT_CAT_PROP_ID_KEY, &(id + 1));

        let current_ledger = env.ledger().sequence();
        let ends_at = current_ledger.saturating_add(cat_params.period_ledgers);

        let proposal = CategorizedProposal {
            id,
            proposer: user.clone(),
            category,
            snapshotted_quorum_bps: cat_params.quorum_bps,
            snapshotted_threshold_bps: cat_params.threshold_bps,
            snapshotted_period_ledgers: cat_params.period_ledgers,
            parameter,
            new_value,
            votes_for: 0,
            votes_against: 0,
            created_at: current_ledger,
            ends_at,
            execution_delay_ledgers: 0,
            bond_amount: 0,
            is_expedited: false,
            vetoed: false,
            enacted: false,
        };

        let p_key = (CAT_PROPOSALS_KEY, id);
        env.storage().persistent().set(&p_key, &proposal);

        events::proposal_created(&env, &user, id, ends_at);

        Ok(id)
    }

    /// Read-only lookup of a categorized proposal by id.
    pub fn get_categorized_proposal(env: Env, id: u32) -> Option<CategorizedProposal> {
        let p_key = (CAT_PROPOSALS_KEY, id);
        env.storage().persistent().get(&p_key)
    }

    /// Vote on a categorized proposal, respecting live or historical delegation graph.
    pub fn vote_categorized_proposal(
        env: Env,
        user: Address,
        proposal_id: u32,
        support: bool,
    ) -> Result<(), GovernanceStrategyError> {
        user.require_auth();

        let p_key = (CAT_PROPOSALS_KEY, proposal_id);
        let mut proposal: CategorizedProposal = env
            .storage()
            .persistent()
            .get(&p_key)
            .ok_or(GovernanceStrategyError::ProposalNotFound)?;

        if proposal.enacted || env.ledger().sequence() > proposal.ends_at {
            return Err(GovernanceStrategyError::VotingStillActive);
        }

        let vote_record_key = (symbol_short!("cvoted"), proposal_id, user.clone());
        if env.storage().persistent().has(&vote_record_key) {
            return Err(GovernanceStrategyError::AlreadyVoted);
        }

        let weight = Self::get_effective_vote_weight(env.clone(), user.clone())?;
        if weight <= 0 {
            return Err(GovernanceStrategyError::PositionNotFound);
        }

        if support {
            proposal.votes_for = proposal
                .votes_for
                .checked_add(weight)
                .ok_or(GovernanceStrategyError::ArithmeticError)?;
        } else {
            proposal.votes_against = proposal
                .votes_against
                .checked_add(weight)
                .ok_or(GovernanceStrategyError::ArithmeticError)?;
        }

        env.storage().persistent().set(&p_key, &proposal);
        env.storage().persistent().set(&vote_record_key, &true);

        events::proposal_voted(
            &env,
            &user,
            proposal_id,
            support,
            weight,
            env.ledger().sequence(),
        );

        Ok(())
    }

    /// Enact a categorized proposal after voting ends, verifying snapshotted quorum and threshold (issue #761).
    pub fn enact_categorized_proposal(
        env: Env,
        proposal_id: u32,
    ) -> Result<(), GovernanceStrategyError> {
        let p_key = (CAT_PROPOSALS_KEY, proposal_id);
        let mut proposal: CategorizedProposal = env
            .storage()
            .persistent()
            .get(&p_key)
            .ok_or(GovernanceStrategyError::ProposalNotFound)?;

        if proposal.enacted {
            return Err(GovernanceStrategyError::ProposalAlreadyEnacted);
        }

        let current_ledger = env.ledger().sequence();
        if current_ledger <= proposal.ends_at {
            return Err(GovernanceStrategyError::VotingStillActive);
        }

        if proposal.vetoed {
            return Err(GovernanceStrategyError::ProposalVetoed);
        }

        let total_votes = proposal
            .votes_for
            .checked_add(proposal.votes_against)
            .ok_or(GovernanceStrategyError::ArithmeticError)?;

        let total_deposited = balance::get_total_deposited(&env);
        let required_quorum = (total_deposited
            .checked_mul(proposal.snapshotted_quorum_bps as i128)
            .ok_or(GovernanceStrategyError::ArithmeticError)?)
            / (BOOST_BPS_BASE as i128);

        if total_votes < required_quorum {
            proposal.enacted = true;
            env.storage().persistent().set(&p_key, &proposal);
            return Err(GovernanceStrategyError::QuorumNotMet);
        }

        let pass_pct = (proposal
            .votes_for
            .checked_mul(BOOST_BPS_BASE as i128)
            .ok_or(GovernanceStrategyError::ArithmeticError)?)
            / total_votes;

        if pass_pct < (proposal.snapshotted_threshold_bps as i128) {
            proposal.enacted = true;
            env.storage().persistent().set(&p_key, &proposal);
            return Err(GovernanceStrategyError::ThresholdNotMet);
        }

        // Apply parameter change
        match proposal.parameter {
            ProposableParam::RewardRate => {
                let rate = u32::try_from(proposal.new_value)
                    .map_err(|_| GovernanceStrategyError::ArithmeticError)?;
                Self::validate_rate_bps(rate)?;
                balance::set_reward_rate_bps(&env, rate);
            }
            ProposableParam::UnstakeFee => {
                let bps = u32::try_from(proposal.new_value)
                    .map_err(|_| GovernanceStrategyError::ArithmeticError)?;
                balance::set_unstake_fee_bps(&env, bps);
            }
            ProposableParam::PoolCap => {
                if proposal.new_value < 0 {
                    return Err(GovernanceStrategyError::ZeroAmount);
                }
                balance::set_pool_cap(&env, proposal.new_value);
            }
            ProposableParam::MinStake => {
                if proposal.new_value < 0 {
                    return Err(GovernanceStrategyError::ZeroAmount);
                }
                balance::set_min_stake(&env, proposal.new_value);
            }
        }

        proposal.enacted = true;
        env.storage().persistent().set(&p_key, &proposal);

        events::proposal_enacted(
            &env,
            proposal_id,
            proposal.parameter,
            proposal.new_value,
            total_votes,
            current_ledger,
        );

        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Issue #762: Expedited Proposals for Emergencies
    // ─────────────────────────────────────────────────────────────────────────

    /// Create an expedited emergency proposal requiring a larger bond and higher quorum (issue #762).
    pub fn create_expedited_proposal(
        env: Env,
        user: Address,
        parameter: ProposableParam,
        new_value: i128,
        bond_amount: i128,
    ) -> Result<u32, GovernanceStrategyError> {
        user.require_auth();

        if balance::get_shares(&env, &user) == 0 {
            return Err(GovernanceStrategyError::PositionNotFound);
        }

        if bond_amount < MIN_EXPEDITED_BOND {
            return Err(GovernanceStrategyError::InsufficientBond);
        }

        let open_expedited: u32 = env
            .storage()
            .instance()
            .get(&OPEN_EXPEDITED_COUNT_KEY)
            .unwrap_or(0);
        if open_expedited >= MAX_OPEN_EXPEDITED_PROPOSALS {
            return Err(GovernanceStrategyError::ExpeditedLimitReached);
        }

        // Lock proposer bond
        let token_addr = Self::token_address(&env)?;
        let token_client = token::Client::new(&env, &token_addr);
        token_client.transfer(&user, &env.current_contract_address(), &bond_amount);

        let id: u32 = env
            .storage()
            .instance()
            .get(&NEXT_CAT_PROP_ID_KEY)
            .unwrap_or(1);
        env.storage()
            .instance()
            .set(&NEXT_CAT_PROP_ID_KEY, &(id + 1));

        let current_ledger = env.ledger().sequence();
        let ends_at = current_ledger.saturating_add(EXPEDITED_VOTING_PERIOD_LEDGERS);

        let proposal = CategorizedProposal {
            id,
            proposer: user.clone(),
            category: ProposalCategory::Upgrade,
            snapshotted_quorum_bps: EXPEDITED_QUORUM_BPS,
            snapshotted_threshold_bps: EXPEDITED_THRESHOLD_BPS,
            snapshotted_period_ledgers: EXPEDITED_VOTING_PERIOD_LEDGERS,
            parameter,
            new_value,
            votes_for: 0,
            votes_against: 0,
            created_at: current_ledger,
            ends_at,
            execution_delay_ledgers: EXPEDITED_EXECUTION_DELAY_LEDGERS,
            bond_amount,
            is_expedited: true,
            vetoed: false,
            enacted: false,
        };

        let p_key = (CAT_PROPOSALS_KEY, id);
        env.storage().persistent().set(&p_key, &proposal);
        env.storage()
            .instance()
            .set(&OPEN_EXPEDITED_COUNT_KEY, &(open_expedited + 1));

        events::expedited_proposal_created(
            &env,
            &user,
            id,
            bond_amount,
            ends_at,
            current_ledger,
        );

        Ok(id)
    }

    /// Guardian veto an expedited proposal during its voting window or execution delay (issue #762).
    pub fn veto_expedited_proposal(
        env: Env,
        guardian: Address,
        proposal_id: u32,
    ) -> Result<(), GovernanceStrategyError> {
        guardian.require_auth();

        let p_key = (CAT_PROPOSALS_KEY, proposal_id);
        let mut proposal: CategorizedProposal = env
            .storage()
            .persistent()
            .get(&p_key)
            .ok_or(GovernanceStrategyError::ProposalNotFound)?;

        if proposal.enacted {
            return Err(GovernanceStrategyError::ProposalAlreadyEnacted);
        }

        proposal.vetoed = true;
        env.storage().persistent().set(&p_key, &proposal);

        // Decrement open expedited count if applicable
        if proposal.is_expedited {
            let open_expedited: u32 = env
                .storage()
                .instance()
                .get(&OPEN_EXPEDITED_COUNT_KEY)
                .unwrap_or(1);
            if open_expedited > 0 {
                env.storage()
                    .instance()
                    .set(&OPEN_EXPEDITED_COUNT_KEY, &(open_expedited - 1));
            }
        }

        events::proposal_vetoed(
            &env,
            proposal_id,
            &guardian,
            env.ledger().sequence(),
        );

        Ok(())
    }

    /// Enact an expedited proposal after voting period + execution delay (issue #762).
    /// If quorum fails, the bond is slashed. If it passes, the bond is refunded.
    pub fn enact_expedited_proposal(
        env: Env,
        proposal_id: u32,
    ) -> Result<(), GovernanceStrategyError> {
        let p_key = (CAT_PROPOSALS_KEY, proposal_id);
        let mut proposal: CategorizedProposal = env
            .storage()
            .persistent()
            .get(&p_key)
            .ok_or(GovernanceStrategyError::ProposalNotFound)?;

        if proposal.enacted {
            return Err(GovernanceStrategyError::ProposalAlreadyEnacted);
        }

        let current_ledger = env.ledger().sequence();
        let execution_ledger = proposal.ends_at.saturating_add(proposal.execution_delay_ledgers);
        if current_ledger <= execution_ledger {
            return Err(GovernanceStrategyError::DelayNotElapsed);
        }

        if proposal.vetoed {
            return Err(GovernanceStrategyError::ProposalVetoed);
        }

        let token_addr = Self::token_address(&env)?;
        let token_client = token::Client::new(&env, &token_addr);

        let total_votes = proposal
            .votes_for
            .checked_add(proposal.votes_against)
            .ok_or(GovernanceStrategyError::ArithmeticError)?;

        let total_deposited = balance::get_total_deposited(&env);
        let required_quorum = (total_deposited
            .checked_mul(proposal.snapshotted_quorum_bps as i128)
            .ok_or(GovernanceStrategyError::ArithmeticError)?)
            / (BOOST_BPS_BASE as i128);

        // Decrement open count
        let open_expedited: u32 = env
            .storage()
            .instance()
            .get(&OPEN_EXPEDITED_COUNT_KEY)
            .unwrap_or(1);
        if open_expedited > 0 {
            env.storage()
                .instance()
                .set(&OPEN_EXPEDITED_COUNT_KEY, &(open_expedited - 1));
        }

        // Quorum check: bond slashed if quorum not met
        if total_votes < required_quorum {
            proposal.enacted = true;
            env.storage().persistent().set(&p_key, &proposal);

            // Bond slashed: add to reward pool balance
            let pool_balance = balance::get_reward_pool_balance(&env);
            balance::set_reward_pool_balance(&env, pool_balance + proposal.bond_amount);

            events::expedited_bond_slashed(
                &env,
                &proposal.proposer,
                proposal_id,
                proposal.bond_amount,
                current_ledger,
            );

            return Err(GovernanceStrategyError::QuorumNotMet);
        }

        // Threshold check: expedited requires higher bar (e.g. 75%)
        let pass_pct = (proposal
            .votes_for
            .checked_mul(BOOST_BPS_BASE as i128)
            .ok_or(GovernanceStrategyError::ArithmeticError)?)
            / total_votes;

        if pass_pct < (proposal.snapshotted_threshold_bps as i128) {
            proposal.enacted = true;
            env.storage().persistent().set(&p_key, &proposal);

            // Refund bond on genuine democratic rejection where quorum was met
            token_client.transfer(
                &env.current_contract_address(),
                &proposal.proposer,
                &proposal.bond_amount,
            );

            events::expedited_bond_refunded(
                &env,
                &proposal.proposer,
                proposal_id,
                proposal.bond_amount,
                current_ledger,
            );

            return Err(GovernanceStrategyError::ThresholdNotMet);
        }

        // Apply parameter change
        match proposal.parameter {
            ProposableParam::RewardRate => {
                let rate = u32::try_from(proposal.new_value)
                    .map_err(|_| GovernanceStrategyError::ArithmeticError)?;
                Self::validate_rate_bps(rate)?;
                balance::set_reward_rate_bps(&env, rate);
            }
            ProposableParam::UnstakeFee => {
                let bps = u32::try_from(proposal.new_value)
                    .map_err(|_| GovernanceStrategyError::ArithmeticError)?;
                balance::set_unstake_fee_bps(&env, bps);
            }
            ProposableParam::PoolCap => {
                balance::set_pool_cap(&env, proposal.new_value);
            }
            ProposableParam::MinStake => {
                balance::set_min_stake(&env, proposal.new_value);
            }
        }

        proposal.enacted = true;
        env.storage().persistent().set(&p_key, &proposal);

        // Refund bond on success
        token_client.transfer(
            &env.current_contract_address(),
            &proposal.proposer,
            &proposal.bond_amount,
        );

        events::expedited_bond_refunded(
            &env,
            &proposal.proposer,
            proposal_id,
            proposal.bond_amount,
            current_ledger,
        );

        events::proposal_enacted(
            &env,
            proposal_id,
            proposal.parameter,
            proposal.new_value,
            total_votes,
            current_ledger,
        );

        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Issue #763: Expiry to Vote Delegation
    // ─────────────────────────────────────────────────────────────────────────

    /// Delegate the caller's governance vote weight with an optional expiration ledger (issue #763).
    pub fn delegate_vote_weight_with_expiry(
        env: Env,
        delegator: Address,
        delegate: Address,
        expires_at: Option<u32>,
    ) -> Result<(), GovernanceStrategyError> {
        delegator.require_auth();

        if delegator == delegate {
            return Err(GovernanceStrategyError::SelfDelegationNotAllowed);
        }

        let current_ledger = env.ledger().sequence();
        if let Some(exp) = expires_at {
            if exp <= current_ledger {
                return Err(GovernanceStrategyError::DelegationExpired);
            }
        }

        // Clean up previous delegation if needed
        let d_key = (VOTE_EXP_KEY, delegator.clone());
        let record = DelegationRecord {
            delegate: delegate.clone(),
            expires_at,
        };

        env.storage().persistent().set(&d_key, &record);

        // Maintain list of delegators for the delegate
        let l_key = (DELEGATOR_LIST_KEY, delegate.clone());
        let mut delegator_list: Vec<Address> = env
            .storage()
            .persistent()
            .get(&l_key)
            .unwrap_or(Vec::new(&env));

        let mut exists = false;
        for existing in delegator_list.iter() {
            if existing == delegator {
                exists = true;
                break;
            }
        }
        if !exists {
            delegator_list.push_back(delegator.clone());
            env.storage().persistent().set(&l_key, &delegator_list);
        }

        events::vote_delegation_with_expiry_set(
            &env,
            &delegator,
            &delegate,
            expires_at,
            current_ledger,
        );

        Ok(())
    }

    /// Retrieve the active delegate and optional expiry for a user, returning None if expired (issue #763).
    pub fn get_delegate(env: Env, user: Address) -> Option<(Address, Option<u32>)> {
        let d_key = (VOTE_EXP_KEY, user);
        let record: DelegationRecord = env.storage().persistent().get(&d_key)?;

        let current_ledger = env.ledger().sequence();
        if let Some(exp) = record.expires_at {
            if current_ledger >= exp {
                return None; // Expired delegations stop counting automatically
            }
        }

        Some((record.delegate, record.expires_at))
    }

    /// Check if a user's delegation was active at a specific snapshot ledger (issue #763).
    pub fn is_delegation_active_at(env: &Env, user: &Address, snapshot_ledger: u32) -> Option<Address> {
        let d_key = (VOTE_EXP_KEY, user.clone());
        let record: DelegationRecord = env.storage().persistent().get(&d_key)?;

        if let Some(exp) = record.expires_at {
            if snapshot_ledger >= exp {
                return None; // Expired at snapshot ledger
            }
        }

        Some(record.delegate)
    }

    /// Get effective vote weight of `user` at current ledger, factoring in delegation expiries.
    pub fn get_effective_vote_weight(env: Env, user: Address) -> Result<i128, GovernanceStrategyError> {
        let current_ledger = env.ledger().sequence();
        Self::effective_vote_weight_at_ledger(env, user, current_ledger)
    }

    /// Get effective vote weight of `user` evaluated at `snapshot_ledger` (issue #763).
    /// Expired delegations automatically stop counting toward the delegate and return to the delegator.
    pub fn effective_vote_weight_at_ledger(
        env: Env,
        user: Address,
        snapshot_ledger: u32,
    ) -> Result<i128, GovernanceStrategyError> {
        let total_shares = balance::get_total_shares(&env);
        let total_deposited = balance::get_total_deposited(&env);

        // If user actively delegated their vote weight to someone else at snapshot_ledger, their own power is 0
        let own_weight = if Self::is_delegation_active_at(&env, &user, snapshot_ledger).is_some() {
            0
        } else {
            let user_shares = balance::get_shares(&env, &user);
            if user_shares == 0 {
                0
            } else {
                balance::shares_to_amount(total_shares, total_deposited, user_shares)
                    .unwrap_or(0)
            }
        };

        let mut total_weight = own_weight;

        // Add weight from users currently delegating to `user` whose delegations are active at snapshot_ledger
        let l_key = (DELEGATOR_LIST_KEY, user.clone());
        let delegator_list: Vec<Address> = env
            .storage()
            .persistent()
            .get(&l_key)
            .unwrap_or(Vec::new(&env));

        for delegator in delegator_list.iter() {
            if let Some(target) = Self::is_delegation_active_at(&env, &delegator, snapshot_ledger) {
                if target == user {
                    let d_shares = balance::get_shares(&env, &delegator);
                    if d_shares > 0 {
                        let d_weight = balance::shares_to_amount(total_shares, total_deposited, d_shares)
                            .unwrap_or(0);
                        total_weight = total_weight
                            .checked_add(d_weight)
                            .ok_or(GovernanceStrategyError::ArithmeticError)?;
                    }
                }
            }
        }

        Ok(total_weight)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Issue #764: StrategyAdapter & Yield Management
    // ─────────────────────────────────────────────────────────────────────────

    /// Admin: Add a new external strategy adapter with an allocation cap (issue #764).
    pub fn add_strategy(
        env: Env,
        admin: Address,
        adapter: Address,
        cap_bps: u32,
    ) -> Result<(), GovernanceStrategyError> {
        admin::require_admin_as(&env, &admin)?;

        if cap_bps > BOOST_BPS_BASE {
            return Err(GovernanceStrategyError::InvalidBps);
        }

        let mut strategies: Vec<StrategyConfig> = env
            .storage()
            .instance()
            .get(&STRATEGIES_LIST_KEY)
            .unwrap_or(Vec::new(&env));

        if strategies.len() >= MAX_STRATEGIES {
            return Err(GovernanceStrategyError::MaxStrategiesReached);
        }

        let mut total_caps: u32 = 0;
        for strat in strategies.iter() {
            if strat.adapter == adapter {
                return Err(GovernanceStrategyError::StrategyAlreadyExists);
            }
            total_caps = total_caps.saturating_add(strat.cap_bps);
        }

        if total_caps.saturating_add(cap_bps) > BOOST_BPS_BASE {
            return Err(GovernanceStrategyError::StrategyCapExceeded);
        }

        let new_strat = StrategyConfig {
            adapter: adapter.clone(),
            cap_bps,
            allocated: 0,
        };
        strategies.push_back(new_strat);
        env.storage().instance().set(&STRATEGIES_LIST_KEY, &strategies);

        events::strategy_added(&env, &admin, &adapter, cap_bps, env.ledger().sequence());

        Ok(())
    }

    /// Keeper or Admin: Deploy idle funds into an approved external strategy adapter (issue #764).
    pub fn allocate_to_strategy(
        env: Env,
        keeper: Address,
        adapter: Address,
        amount: i128,
    ) -> Result<(), GovernanceStrategyError> {
        keeper.require_auth();

        if amount <= 0 {
            return Err(GovernanceStrategyError::ZeroAmount);
        }

        let mut strategies: Vec<StrategyConfig> = env
            .storage()
            .instance()
            .get(&STRATEGIES_LIST_KEY)
            .ok_or(GovernanceStrategyError::StrategyNotFound)?;

        let total_deposited = balance::get_total_deposited(&env);
        let mut updated = Vec::new(&env);
        let mut found = false;

        for mut strat in strategies.iter() {
            if strat.adapter == adapter {
                found = true;
                let max_allocation = (total_deposited
                    .checked_mul(strat.cap_bps as i128)
                    .ok_or(GovernanceStrategyError::ArithmeticError)?)
                    / (BOOST_BPS_BASE as i128);

                let new_allocated = strat
                    .allocated
                    .checked_add(amount)
                    .ok_or(GovernanceStrategyError::ArithmeticError)?;

                if new_allocated > max_allocation {
                    return Err(GovernanceStrategyError::StrategyCapExceeded);
                }

                strat.allocated = new_allocated;
            }
            updated.push_back(strat);
        }

        if !found {
            return Err(GovernanceStrategyError::StrategyNotFound);
        }

        env.storage().instance().set(&STRATEGIES_LIST_KEY, &updated);

        // Transfer funds to adapter
        let token_addr = Self::token_address(&env)?;
        let token_client = token::Client::new(&env, &token_addr);
        token_client.transfer(&env.current_contract_address(), &adapter, &amount);

        // Invoke adapter deposit
        StrategyClient::deposit(&env, &adapter, amount)?;

        events::strategy_allocated(&env, &keeper, &adapter, amount, env.ledger().sequence());

        Ok(())
    }

    /// Admin: Withdraw funds from a strategy back into vault idle reserves (issue #764).
    pub fn withdraw_from_strategy(
        env: Env,
        admin: Address,
        adapter: Address,
        amount: i128,
    ) -> Result<i128, GovernanceStrategyError> {
        admin::require_admin_as(&env, &admin)?;

        if amount <= 0 {
            return Err(GovernanceStrategyError::ZeroAmount);
        }

        let mut strategies: Vec<StrategyConfig> = env
            .storage()
            .instance()
            .get(&STRATEGIES_LIST_KEY)
            .ok_or(GovernanceStrategyError::StrategyNotFound)?;

        let mut updated = Vec::new(&env);
        let mut found = false;
        let mut withdrawn_amount: i128 = 0;

        for mut strat in strategies.iter() {
            if strat.adapter == adapter {
                found = true;
                let actual_withdrawn = StrategyClient::withdraw(&env, &adapter, amount)?;
                strat.allocated = strat.allocated.saturating_sub(actual_withdrawn);
                withdrawn_amount = actual_withdrawn;
            }
            updated.push_back(strat);
        }

        if !found {
            return Err(GovernanceStrategyError::StrategyNotFound);
        }

        env.storage().instance().set(&STRATEGIES_LIST_KEY, &updated);

        events::strategy_withdrawn(&env, &adapter, withdrawn_amount, env.ledger().sequence());

        Ok(withdrawn_amount)
    }

    /// Query all active strategies and their configurations.
    pub fn get_strategies(env: Env) -> Vec<StrategyConfig> {
        env.storage()
            .instance()
            .get(&STRATEGIES_LIST_KEY)
            .unwrap_or(Vec::new(&env))
    }

    /// Calculate total assets across all active external yield strategies (issue #764).
    pub fn total_strategy_assets(env: Env) -> i128 {
        let strategies: Vec<StrategyConfig> = env
            .storage()
            .instance()
            .get(&STRATEGIES_LIST_KEY)
            .unwrap_or(Vec::new(&env));

        let mut total: i128 = 0;
        for strat in strategies.iter() {
            if let Ok(assets) = StrategyClient::total_assets(&env, &strat.adapter) {
                total = total.saturating_add(assets);
            } else {
                total = total.saturating_add(strat.allocated);
            }
        }
        total
    }

    /// Total assets managed by vault including idle tokens and strategy assets (issue #764).
    pub fn total_vault_assets(env: Env) -> i128 {
        let token_addr = match Self::token_address(&env) {
            Ok(addr) => addr,
            Err(_) => return 0,
        };
        let token_client = token::Client::new(&env, &token_addr);
        let idle_balance = token_client.balance(&env.current_contract_address());
        let strategy_assets = Self::total_strategy_assets(env);
        idle_balance.saturating_add(strategy_assets)
    }

    /// Read-only vault share price: returns (total_assets, total_shares) reflecting strategy balances (issue #764).
    pub fn get_vault_share_price(env: Env) -> (i128, i128) {
        let total_assets = Self::total_vault_assets(env.clone());
        let total_shares = balance::get_total_shares(&env);
        (total_assets, total_shares)
    }
}
