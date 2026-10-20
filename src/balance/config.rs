//! Protocol configuration, staker registries, and feature-specific storage accessors.
//!
//! Handles admin configuration, paused state, changelogs, staker lists,
//! activity logs, vesting queues, whitelists, and modular feature state.

use super::positions::assert_non_negative;
use crate::storage::{
    AccessTier, BrandingConfig, ChangelogEntry, DayBucket, InsurancePolicy, InsuranceProduct,
    Loan, LoanConfig, Milestone, OnboardingChecklist, PriceCondition, PriorityBidRecord,
    Quiz, ReferralStats, RevenueShareMerkleRoot, RevenueSharingConfig, Season, SunsetState,
    VestingEntry, DataKey,
};
use soroban_sdk::{symbol_short, Address, Env, String, Symbol, Vec};

pub const MAX_QUIZZES: u32 = 20;

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::Paused, &paused);
}

pub fn get_withdrawal_limit(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::WithdrawalLimit)
        .unwrap_or(0)
}

pub fn set_withdrawal_limit(env: &Env, limit: i128) {
    assert_non_negative(env, limit);
    env.storage()
        .instance()
        .set(&DataKey::WithdrawalLimit, &limit);
}

pub fn get_total_stakers(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::TotalStakers)
        .unwrap_or(0)
}

pub fn set_total_stakers(env: &Env, total: u32) {
    env.storage().instance().set(&DataKey::TotalStakers, &total);
}

pub fn is_whitelist_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::WhitelistEnabled)
        .unwrap_or(false)
}

pub fn set_whitelist_enabled(env: &Env, enabled: bool) {
    env.storage()
        .instance()
        .set(&DataKey::WhitelistEnabled, &enabled);
}

pub fn is_whitelisted(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::Whitelisted(user.clone()))
        .unwrap_or(false)
}

pub fn set_whitelisted(env: &Env, user: &Address, status: bool) {
    env.storage()
        .persistent()
        .set(&DataKey::Whitelisted(user.clone()), &status);
}

pub fn remove_whitelisted(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::Whitelisted(user.clone()));
}

pub fn get_cooldown_period(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::CooldownPeriod)
        .unwrap_or(0)
}

pub fn set_cooldown_period(env: &Env, ledgers: u32) {
    env.storage()
        .instance()
        .set(&DataKey::CooldownPeriod, &ledgers);
}

pub fn get_inactivity_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::InactivityThreshold)
        .unwrap_or(0)
}

pub fn set_inactivity_threshold(env: &Env, ledgers: u32) {
    env.storage()
        .instance()
        .set(&DataKey::InactivityThreshold, &ledgers);
}

pub fn get_all_stakers(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::AllStakers)
        .unwrap_or(Vec::new(env))
}

pub fn set_all_stakers(env: &Env, stakers: &Vec<Address>) {
    env.storage().instance().set(&DataKey::AllStakers, stakers);
}

pub fn register_staker(env: &Env, user: &Address) {
    let mut stakers = get_all_stakers(env);
    for s in stakers.iter() {
        if &s == user {
            return;
        }
    }
    stakers.push_back(user.clone());
    set_total_stakers(env, stakers.len());
    set_all_stakers(env, &stakers);
}

pub fn get_changelog(env: &Env) -> Vec<ChangelogEntry> {
    env.storage()
        .instance()
        .get(&DataKey::Changelog)
        .unwrap_or(Vec::new(env))
}

pub fn set_changelog(env: &Env, log: &Vec<ChangelogEntry>) {
    env.storage().instance().set(&DataKey::Changelog, log);
}

pub fn record_change(env: &Env, admin: &Address, action: &Symbol, ledger: u32) {
    let mut log = get_changelog(env);
    log.push_back(ChangelogEntry {
        admin: admin.clone(),
        action: action.clone(),
        ledger,
    });
    set_changelog(env, &log);
}

pub fn get_initialized_at_ledger(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("init_at"))
        .unwrap_or(0)
}

pub fn set_initialized_at_ledger(env: &Env, ledger: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("init_at"), &ledger);
}

pub fn get_activity_log(env: &Env) -> Vec<DayBucket> {
    env.storage()
        .instance()
        .get(&symbol_short!("act_log"))
        .unwrap_or(Vec::new(env))
}

pub fn set_activity_log(env: &Env, log: &Vec<DayBucket>) {
    env.storage().instance().set(&symbol_short!("act_log"), log);
}

pub fn get_vesting_entries(env: &Env, user: &Address) -> Vec<VestingEntry> {
    let key = (Symbol::new(env, "vest"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_vesting_entries(env: &Env, user: &Address, entries: &Vec<VestingEntry>) {
    let key = (Symbol::new(env, "vest"), user.clone());
    env.storage().persistent().set(&key, entries);
}

pub fn get_max_quizzes(_: &Env) -> u32 {
    MAX_QUIZZES
}

pub fn get_quiz(env: &Env, quiz_id: u32) -> Option<Quiz> {
    let key = (Symbol::new(env, "quiz"), quiz_id);
    env.storage().persistent().get(&key)
}

pub fn set_quiz(env: &Env, quiz: &Quiz) {
    let key = (Symbol::new(env, "quiz"), quiz.id);
    env.storage().persistent().set(&key, quiz);
}

pub fn get_quiz_attempts_remaining(env: &Env, user: &Address, quiz_id: u32) -> u32 {
    let key = (Symbol::new(env, "quiz_attempts"), user.clone(), quiz_id);
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn get_quiz_attempts_remaining_opt(env: &Env, user: &Address, quiz_id: u32) -> Option<u32> {
    let key = (Symbol::new(env, "quiz_attempts"), user.clone(), quiz_id);
    env.storage().persistent().get(&key)
}

pub fn set_quiz_attempts_remaining(env: &Env, user: &Address, quiz_id: u32, attempts: u32) {
    let key = (Symbol::new(env, "quiz_attempts"), user.clone(), quiz_id);
    env.storage().persistent().set(&key, &attempts);
}

pub fn get_completed_quizzes(env: &Env, user: &Address) -> Vec<u32> {
    let key = (Symbol::new(env, "quiz_done"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_completed_quizzes(env: &Env, user: &Address, completed_quizzes: &Vec<u32>) {
    let key = (Symbol::new(env, "quiz_done"), user.clone());
    env.storage().persistent().set(&key, completed_quizzes);
}

pub fn get_branding(env: &Env) -> Option<BrandingConfig> {
    env.storage().instance().get(&symbol_short!("branding"))
}

pub fn set_branding(env: &Env, config: &BrandingConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("branding"), config);
}

pub fn get_insurance_product(env: &Env) -> Option<InsuranceProduct> {
    env.storage().instance().get(&symbol_short!("ins_prod"))
}

pub fn set_insurance_product(env: &Env, product: &InsuranceProduct) {
    env.storage()
        .instance()
        .set(&symbol_short!("ins_prod"), product);
}

pub fn get_insurance_policy(env: &Env, user: &Address) -> Option<InsurancePolicy> {
    let key = (symbol_short!("ins_pol"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_insurance_policy(env: &Env, user: &Address, policy: &InsurancePolicy) {
    let key = (symbol_short!("ins_pol"), user.clone());
    env.storage().persistent().set(&key, policy);
}

pub fn remove_insurance_policy(env: &Env, user: &Address) {
    let key = (symbol_short!("ins_pol"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_loan_config(env: &Env) -> Option<LoanConfig> {
    env.storage().instance().get(&symbol_short!("loan_cfg"))
}

pub fn set_loan_config(env: &Env, config: &LoanConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("loan_cfg"), config);
}

pub fn get_loan(env: &Env, user: &Address) -> Option<Loan> {
    let key = (symbol_short!("loan"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_loan(env: &Env, user: &Address, loan: &Loan) {
    let key = (symbol_short!("loan"), user.clone());
    env.storage().persistent().set(&key, loan);
}

pub fn remove_loan(env: &Env, user: &Address) {
    let key = (symbol_short!("loan"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_seasons(env: &Env) -> Vec<Season> {
    env.storage()
        .instance()
        .get(&symbol_short!("seasons"))
        .unwrap_or(Vec::new(env))
}

pub fn set_seasons(env: &Env, seasons: &Vec<Season>) {
    env.storage()
        .instance()
        .set(&symbol_short!("seasons"), seasons);
}

pub fn get_last_active_season_marker(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("seas_act"))
}

pub fn set_last_active_season_marker(env: &Env, marker: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("seas_act"), &marker);
}

pub fn clear_last_active_season_marker(env: &Env) {
    env.storage()
        .instance()
        .remove(&symbol_short!("seas_act"));
}

pub fn get_sunset_state(env: &Env) -> SunsetState {
    env.storage()
        .instance()
        .get(&symbol_short!("sunset"))
        .unwrap_or(SunsetState::Normal)
}

pub fn set_sunset_state(env: &Env, state: SunsetState) {
    env.storage().instance().set(&symbol_short!("sunset"), &state);
}

pub fn get_grace_period_end(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("snst_grc"))
}

pub fn set_grace_period_end(env: &Env, ledger: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("snst_grc"), &ledger);
}

pub fn get_sunset_exit_deadline(env: &Env) -> Option<u32> {
    env.storage().instance().get(&symbol_short!("snst_dln"))
}

pub fn set_sunset_exit_deadline(env: &Env, deadline: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("snst_dln"), &deadline);
}

pub fn get_revenue_sharing_config(env: &Env) -> Option<RevenueSharingConfig> {
    env.storage().instance().get(&symbol_short!("rev_cfg"))
}

pub fn set_revenue_sharing_config(env: &Env, config: &RevenueSharingConfig) {
    env.storage()
        .instance()
        .set(&symbol_short!("rev_cfg"), config);
}

pub fn get_revenue_share_pool(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("rev_pool"))
        .unwrap_or(0)
}

pub fn set_revenue_share_pool(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&symbol_short!("rev_pool"), &amount);
}

pub fn get_revenue_share_epoch(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("rev_epch"))
        .unwrap_or(0)
}

pub fn set_revenue_share_epoch(env: &Env, epoch: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("rev_epch"), &epoch);
}

pub fn get_revenue_share_merkle_root(env: &Env) -> Option<RevenueShareMerkleRoot> {
    env.storage().instance().get(&symbol_short!("rev_root"))
}

pub fn set_revenue_share_merkle_root(env: &Env, root: &RevenueShareMerkleRoot) {
    env.storage()
        .instance()
        .set(&symbol_short!("rev_root"), root);
}

pub fn is_revenue_share_claimed(env: &Env, user: &Address, epoch: u32) -> bool {
    let key = (symbol_short!("rev_clm"), user.clone(), epoch);
    env.storage().persistent().has(&key)
}

pub fn set_revenue_share_claimed(env: &Env, user: &Address, epoch: u32) {
    let key = (symbol_short!("rev_clm"), user.clone(), epoch);
    env.storage().persistent().set(&key, &true);
}

pub fn get_escrow_period(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("escr_prd"))
        .unwrap_or(0)
}

pub fn set_escrow_period(env: &Env, ledgers: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("escr_prd"), &ledgers);
}

pub fn get_escrow_balance(env: &Env, user: &Address) -> i128 {
    let key = (symbol_short!("escr_bal"), user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_escrow_balance(env: &Env, user: &Address, amount: i128) {
    assert_non_negative(env, amount);
    let key = (symbol_short!("escr_bal"), user.clone());
    env.storage().persistent().set(&key, &amount);
}

pub fn remove_escrow_balance(env: &Env, user: &Address) {
    let key = (symbol_short!("escr_bal"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_escrow_release_ledger(env: &Env, user: &Address) -> Option<u32> {
    let key = (symbol_short!("escr_rls"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_escrow_release_ledger(env: &Env, user: &Address, ledger: u32) {
    let key = (symbol_short!("escr_rls"), user.clone());
    env.storage().persistent().set(&key, &ledger);
}

pub fn remove_escrow_release_ledger(env: &Env, user: &Address) {
    let key = (symbol_short!("escr_rls"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_access_tiers(env: &Env) -> Vec<AccessTier> {
    env.storage()
        .instance()
        .get(&symbol_short!("acc_tier"))
        .unwrap_or(Vec::new(env))
}

pub fn set_access_tiers(env: &Env, tiers: &Vec<AccessTier>) {
    env.storage()
        .instance()
        .set(&symbol_short!("acc_tier"), tiers);
}

pub fn get_user_access_tier(env: &Env, user: &Address) -> Option<u32> {
    let key = (symbol_short!("u_tier"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_user_access_tier(env: &Env, user: &Address, tier_index: u32) {
    let key = (symbol_short!("u_tier"), user.clone());
    env.storage().persistent().set(&key, &tier_index);
}

pub fn remove_user_access_tier(env: &Env, user: &Address) {
    let key = (symbol_short!("u_tier"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_onboarding_checklist(env: &Env, user: &Address) -> OnboardingChecklist {
    let key = (symbol_short!("onb_chk"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(OnboardingChecklist {
            staked: false,
            claimed: false,
            delegated: false,
            voted: false,
        })
}

pub fn set_onboarding_checklist(env: &Env, user: &Address, checklist: &OnboardingChecklist) {
    let key = (symbol_short!("onb_chk"), user.clone());
    env.storage().persistent().set(&key, checklist);
}

pub fn get_matching_program(env: &Env) -> Option<crate::storage::MatchingProgram> {
    env.storage().instance().get(&symbol_short!("mtch_prg"))
}

pub fn set_matching_program(env: &Env, program: &crate::storage::MatchingProgram) {
    env.storage()
        .instance()
        .set(&symbol_short!("mtch_prg"), program);
}

pub fn get_output_tokens(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&symbol_short!("out_toks"))
        .unwrap_or(Vec::new(env))
}

pub fn set_output_tokens(env: &Env, tokens: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&symbol_short!("out_toks"), tokens);
}

pub fn get_cohort_ids(env: &Env) -> Vec<u32> {
    env.storage()
        .instance()
        .get(&symbol_short!("chrt_ids"))
        .unwrap_or(Vec::new(env))
}

pub fn set_cohort_ids(env: &Env, ids: &Vec<u32>) {
    env.storage()
        .instance()
        .set(&symbol_short!("chrt_ids"), ids);
}

pub fn get_cohort_stats(env: &Env, cohort_id: u32) -> Option<crate::storage::CohortStats> {
    let key = (symbol_short!("chrt_st"), cohort_id);
    env.storage().instance().get(&key)
}

pub fn set_cohort_stats(env: &Env, cohort_id: u32, stats: &crate::storage::CohortStats) {
    let key = (symbol_short!("chrt_st"), cohort_id);
    env.storage().instance().set(&key, stats);
}

pub fn get_exit_queue(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&symbol_short!("exitq"))
        .unwrap_or(Vec::new(env))
}

pub fn set_exit_queue(env: &Env, queue: &Vec<Address>) {
    env.storage().instance().set(&symbol_short!("exitq"), queue);
}

pub fn get_min_priority_bid(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("minpbid"))
        .unwrap_or(0)
}

pub fn set_min_priority_bid(env: &Env, amount: i128) {
    assert_non_negative(env, amount);
    env.storage()
        .instance()
        .set(&symbol_short!("minpbid"), &amount);
}

pub fn get_priority_bids(env: &Env) -> Vec<PriorityBidRecord> {
    env.storage()
        .instance()
        .get(&symbol_short!("pbidrec"))
        .unwrap_or(Vec::new(env))
}

pub fn set_priority_bids(env: &Env, records: &Vec<PriorityBidRecord>) {
    env.storage()
        .instance()
        .set(&symbol_short!("pbidrec"), records);
}

pub fn get_webhook_url(env: &Env) -> Option<String> {
    env.storage().instance().get(&symbol_short!("whk_url"))
}

pub fn set_webhook_url(env: &Env, url: &String) {
    env.storage().instance().set(&symbol_short!("whk_url"), url);
}

pub fn clear_webhook_url(env: &Env) {
    env.storage().instance().remove(&symbol_short!("whk_url"));
}

pub fn get_pool_description(env: &Env) -> Option<String> {
    env.storage().instance().get(&symbol_short!("pool_dsc"))
}

pub fn set_pool_description(env: &Env, description: &String) {
    env.storage()
        .instance()
        .set(&symbol_short!("pool_dsc"), description);
}

pub fn get_daily_deposit_limit(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("dd_limit"))
        .unwrap_or(0)
}

pub fn set_daily_deposit_limit(env: &Env, limit: i128) {
    assert_non_negative(env, limit);
    env.storage()
        .instance()
        .set(&symbol_short!("dd_limit"), &limit);
}

pub fn get_low_balance_alert_threshold(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&symbol_short!("lba_thr"))
        .unwrap_or(0)
}

pub fn set_low_balance_alert_threshold(env: &Env, threshold: i128) {
    assert_non_negative(env, threshold);
    env.storage()
        .instance()
        .set(&symbol_short!("lba_thr"), &threshold);
}

pub fn get_staker_bio(env: &Env, user: &Address) -> Option<String> {
    let key = (symbol_short!("bio"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_staker_bio(env: &Env, user: &Address, bio: &String) {
    let key = (symbol_short!("bio"), user.clone());
    env.storage().persistent().set(&key, bio);
}

pub fn remove_staker_bio(env: &Env, user: &Address) {
    let key = (symbol_short!("bio"), user.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_milestones(env: &Env) -> Vec<Milestone> {
    env.storage()
        .instance()
        .get(&symbol_short!("milstns"))
        .unwrap_or(Vec::new(env))
}

pub fn set_milestones(env: &Env, milestones: &Vec<Milestone>) {
    env.storage()
        .instance()
        .set(&symbol_short!("milstns"), milestones);
}

pub fn get_user_milestones(env: &Env, user: &Address) -> Vec<u32> {
    let key = (symbol_short!("u_mlstn"), user.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_user_milestones(env: &Env, user: &Address, ids: &Vec<u32>) {
    let key = (symbol_short!("u_mlstn"), user.clone());
    env.storage().persistent().set(&key, ids);
}

pub fn get_price_condition(env: &Env, user: &Address) -> Option<PriceCondition> {
    let key = (symbol_short!("prc_cnd"), user.clone());
    env.storage().persistent().get(&key)
}

pub fn set_price_condition(env: &Env, user: &Address, condition: &PriceCondition) {
    let key = (symbol_short!("prc_cnd"), user.clone());
    env.storage().persistent().set(&key, condition);
}

pub fn get_auction_bids(env: &Env) -> Vec<crate::storage::AuctionBid> {
    env.storage()
        .instance()
        .get(&symbol_short!("auc_bids"))
        .unwrap_or(Vec::new(env))
}

pub fn set_auction_bids(env: &Env, bids: &Vec<crate::storage::AuctionBid>) {
    env.storage()
        .instance()
        .set(&symbol_short!("auc_bids"), bids);
}

pub fn get_referral_registry(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&symbol_short!("ref_reg"))
        .unwrap_or(Vec::new(env))
}

pub fn set_referral_registry(env: &Env, registry: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&symbol_short!("ref_reg"), registry);
}

pub fn get_referral_stats(env: &Env, referrer: &Address) -> ReferralStats {
    let key = (symbol_short!("ref_stat"), referrer.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(ReferralStats {
            referee_count: 0,
            total_referred_deposits: 0,
            earned: 0,
            claimed: 0,
        })
}

pub fn set_referral_stats(env: &Env, referrer: &Address, stats: &ReferralStats) {
    let key = (symbol_short!("ref_stat"), referrer.clone());
    env.storage().persistent().set(&key, stats);
}

pub fn get_referral_commission_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&symbol_short!("ref_bps"))
        .unwrap_or(0)
}

pub fn set_referral_commission_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&symbol_short!("ref_bps"), &bps);
}

pub fn get_referees(env: &Env, referrer: &Address) -> Vec<Address> {
    let key = (symbol_short!("referees"), referrer.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}
