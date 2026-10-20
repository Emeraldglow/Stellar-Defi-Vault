# Storage type audit (issue #609)

Soroban gives each storage entry one of three lifetimes, and the vault must
pick the right one per key:

- **Instance** — bundled with the contract instance itself. Cheap to read
  (loaded once per invocation regardless of how many instance keys exist) and
  its TTL is bumped as a single unit, but every instance key is loaded on
  *every* contract call whether that call needs it or not, and the whole
  bundle has a hard size ceiling. Right fit: small, singleton, always-relevant
  state — admin address, global config, pool-wide totals.
- **Persistent** — one entry per key, billed and TTL-extended independently,
  archived (not deleted) if its TTL lapses, and restorable. Right fit:
  per-user / per-position data that only the calls touching that specific
  user need to load, and that must never silently disappear even if nobody
  extends its TTL for a long time.
- **Temporary** — one entry per key, deleted (not archived) once its TTL
  lapses, with no restore path. Right fit: data that is genuinely fine to
  lose once it's stale — a one-time nonce, a short-lived price quote — where
  losing it early has no correctness impact.

## Core `DataKey` enum

`DataKey` (`storage.rs`) is the primary storage-key enum, and is at Soroban's
50-variant cap for `#[contracttype]` enums, so no more variants can be added
to it (new keys use raw `Symbol` keys instead — see below). Every variant's
storage type below was verified by cross-referencing every `.instance()` /
`.persistent()` call site against the `DataKey` variant it operates on: no
variant is accessed under more than one storage type anywhere in the crate.

| Key | Type | Rationale |
| --- | --- | --- |
| `Admin` | instance | Single global admin address. |
| `Token` | instance | Single global stake/reward token address. |
| `TotalShares`, `TotalDeposited`, `TotalStakers`, `TotalRewardsPaid` | instance | Pool-wide running totals, one value each, read on nearly every call. |
| `MinStake`, `RewardRateBps`, `RewardPoolBalance`, `BoostSchedule`, `WithdrawalLimit`, `LockPeriod`, `EarlyExitPenaltyBps`, `UnstakeFeeBps`, `WhitelistEnabled`, `CooldownPeriod`, `InactivityThreshold`, `KycRequired`, `Paused`, `Stopped`, `ShuttingDown` | instance | Admin-set global config/flags, not per-user. |
| `RateHistory`, `BoostCampaign`, `Leaderboard`, `LeaderboardSize`, `AllStakers` | instance | Pool-wide aggregates/registries (single value each — see "Known scaling risks" below for the cost of `AllStakers` specifically). |
| `ShareBalance(Address)`, `StakeHistory(Address)`, `RewardCheckpointLedger(Address)`, `LastClaimLedger(Address)`, `AccruedReward(Address)`, `StakedAtLedger(Address)`, `Delegate(Address)`, `LastUnstakeLedger(Address)`, `Restaked(Address)`, `Whitelisted(Address)`, `UnbondingPosition(Address)`, `RewardRemainder(Address)`, `UserClaimWindow(Address)`, `FrozenAt(Address)`, `KycApproved(Address)`, `FirstStakedAt(Address)`, `VestingEntries(Address)` | persistent | One entry per user; only calls touching that user need to load it, and a dormant user's data must survive (and be restorable) even if nobody touches it for a long time. |
| `TotalEverClaimed`, `EpochMode`, `CurrentEpoch`, `EpochLedgers`, `EpochRewardPerEpoch`, `VestingPeriod`, `EpochRewardFactor(u32)` | instance / persistent (unused) | Declared but never read or written anywhere in the crate today (dead variants left over from earlier feature work). Not a storage-type bug — there's nothing to misclassify — but worth pruning next time `DataKey` needs headroom, since it's at the 50-variant cap. |

## Extension modules (raw `Symbol`-keyed storage)

Every feature added after `DataKey` hit its cap (see the note at the bottom
of `storage.rs`) uses a raw `Symbol` constant instead, generally as a bare key
for global/config data (`.instance()`) or as `(SYMBOL, Address)` /
`(SYMBOL, Address, u32)` tuple keys for per-user or per-position data
(`.persistent()`), matching the same instance-vs-persistent rule as the core
`DataKey` enum above.

An automated cross-reference of every `symbol_short!`-declared storage
constant against every `.instance()`/`.persistent()` call site in the crate
(103 modules use this pattern) found no key accessed under more than one
storage type. `gpd_lv` appears in both `batch_vote.rs` and
`governance_power_decay.rs` — that's intentional, documented cross-module key
sharing (`batch_vote.rs`'s own comment: "the key used by the governance
power-decay layer... so inactivity [decay applies]"), not a collision.

| Module | Keys (type) |
| --- | --- |
| `activity_log` | `"act_log"`=persistent |
| `admin_action_nonce` | `"adm_nonce"`=persistent |
| `admin_recovery` | `"adm_rec"`=instance |
| `admin_roles` | `"adm_roles"`=instance, `"emg_cont"`=instance, `"pool_nm"`=instance |
| `admin_succession` | `"succ_pln"`=instance |
| `anti_dump_claim_cooldown` | `"ad_cfg"`=instance, `"ad_cool"`=persistent |
| `batch_vote` | `"gpd_lv"`=persistent |
| `boost_activation_age` | `"bam_age"`=instance |
| `burn_milestone_tracker` | `"burn_hit"`=instance, `"burn_thr"`=instance |
| `capacity_forecast` | `"cuf_infl"`=instance, `"cuf_warn"`=instance |
| `charitable_donation_routing` | `"ch_dcfg"`=persistent, `"ch_don"`=persistent, `"ch_lst"`=instance |
| `claim_fee` | `"clm_fee"`=instance, `"clm_res"`=instance |
| `claim_vesting` | `"cv_dur"`=instance |
| `co_sponsor` | `"co_spf"`=persistent, `"co_spg"`=persistent |
| `collateral_swap` | `"col_swp"`=persistent |
| `collusion_detector` | `"cld_alrt"`=instance |
| `combined_vesting` | `"cmb_vst"`=instance |
| `comfort_score` | `"cs_audit"`=instance, `"cs_lockd"`=instance, `"cs_prof"`=persistent, `"cs_slash"`=instance |
| `commitment` | `"cmt_rec"`=persistent, `"cmt_wnd"`=instance |
| `community_treasury` | `"ct_bal"`=instance, `"ct_bps"`=instance, `"ct_next"`=instance |
| `competitive_season` | `"cur_seas"`=instance, `"seas_hst"`=persistent, `"seas_nid"`=instance |
| `compliance_report` | `"cr_hist"`=instance, `"cr_next"`=instance |
| `compound_optimizer` | `"cmp_opt"`=persistent |
| `content_curation` | `"cc_items"`=instance, `"cc_vote"`=persistent |
| `balance_checkpoints` | `"ck_tot"`=persistent, `"ck_usr"`=persistent |
| `cross_pool_identity` | `"cp_gov_w"`=instance, `"cp_id"`=persistent |
| `daily_community_tip` | `"tip_day"`=persistent, `"tip_flt"`=instance, `"tip_ftd"`=persistent, `"tip_nid"`=instance, `"tip_usr"`=persistent, `"tip_vot"`=persistent |
| `daily_token_velocity_limiter` | `"dv_cfg"`=instance, `"dv_dfr"`=persistent, `"dv_trk"`=instance |
| `daily_withdrawal_limit` | `"dw_cfg"`=instance, `"dw_trk"`=persistent |
| `dex_limit_order_buyback` | `"lo_ctr"`=instance, `"lo_list"`=instance, `"lo_ord"`=persistent |
| `dynamic_reward_rate` | `"dyn_rate"`=instance |
| `emission_schedule_history` | `"em_hist"`=instance |
| `epoch_alignment` | `"epc_algn"`=instance |
| `epoch_reward_cap` | `"epc_cap"`=instance, `"epc_dfr"`=persistent, `"epc_trk"`=instance |
| `external_price_oracle` | `"ext_orcl"`=instance |
| `governance_params` | `"gv_cfg"`=instance, `"gv_snap"`=persistent |
| `governance_power_decay` | `"gpd_cfg"`=instance, `"gpd_lv"`=persistent |
| `guardian_pause` | `"guardian"`=instance |
| `insurance` | `"gtor"`=instance, `"gtor_cov"`=instance, `"gtor_ins"`=instance, `"gtor_reg"`=instance, `"gtor_res"`=instance |
| `keeper_registry` | `"kpr_all"`=instance, `"kpr_rec"`=persistent |
| `liquidity_bridge` | `"brdg_tgt"`=instance |
| `lock_extension` | `"lockboost"`=persistent, `"lockxcfg"`=instance |
| `lockdrop_campaign` | `"ldrp_cfg"`=instance, `"ldrp_scr"`=instance, `"ldrp_usr"`=instance |
| `loyalty_points` | `"loy_bal"`=persistent, `"loy_cfg"`=instance, `"loy_earn"`=persistent |
| `max_deposit_cap` | `"usr_cap"`=instance |
| `meta_staking` | `"meta_acc"`=persistent, `"meta_pos"`=persistent, `"meta_rate"`=instance, `"meta_tot"`=instance |
| `mev_claim_protection` | `"mev_thr"`=instance |
| `minimum_reserve_ratio` | `"mrr_bps"`=instance |
| `minimum_unstake_amount` | `"mn_unstk"`=instance |
| `multi_token` | `"sup_toks"`=instance |
| `mutual_insurance_pool` | `"mi_cfg"`=instance, `"mi_evt"`=persistent, `"mi_fund"`=instance, `"mi_memb"`=persistent, `"mi_next"`=instance |
| `nft_fractionalize` | `"fr_bal"`=persistent, `"fr_hold"`=persistent, `"fr_lock"`=persistent, `"fr_meta"`=persistent |
| `operator_reputation_score` | `"op_rep"`=persistent |
| `partial_freeze` | `"pf_frzn"`=persistent |
| `pause_grace_period` | `"fpse_cd"`=instance, `"max_pse"`=instance |
| `peg_stabilization` | `"peg_cfg"`=instance, `"peg_hlt"`=instance, `"peg_max"`=instance |
| `performance_league_table` | `"lg_pools"`=instance, `"lg_stats"`=persistent |
| `pool_clone_factory` | `"pcf_cln"`=instance |
| `pool_presale` | `"ps_cfg"`=instance, `"ps_res"`=persistent |
| `position_dna` | `"pos_dna"`=persistent |
| `position_fee_tiers` | `"fee_tiers"`=instance |
| `position_heartbeat` | `"hb_log"`=persistent, `"hb_max"`=instance, `"hb_susp"`=persistent |
| `position_mirroring` | `"mirr_cfg"`=persistent, `"mirr_flw"`=persistent |
| `position_sealed_bid_auction` | `"psa_bdrs"`=persistent, `"psa_bid"`=persistent, `"psa_list"`=persistent, `"psa_next"`=instance, `"psa_slr"`=persistent |
| `position_shadow_clone` | `"sc_clone"`=persistent, `"sc_ctr"`=instance, `"sc_ucl"`=persistent |
| `position_value_appreciation_log` | `"val_app"`=persistent |
| `price_oracle` | `"pos_prc"`=persistent |
| `proof_of_humanity_hook` | `"hmn_cfg"`=instance, `"hmn_fbm"`=instance |
| `proposal_comment_thread` | `"prop_cmt"`=persistent |
| `reputation_decay` | `"rd_act"`=persistent, `"rd_cfg"`=instance |
| `reward_rate_ceiling` | `"max_rate"`=instance |
| `reward_token_audit_trail` | `"aud_cnt"`=persistent, `"aud_pg"`=persistent, `"aud_sum"`=persistent |
| `reward_waterfall` | `"rw_cred"`=persistent, `"rw_order"`=instance |
| `roadmap_voting` | `"rdmp_itm"`=instance, `"rdmp_nid"`=instance |
| `runway_guard` | `"min_run"`=instance |
| `scheduled_exit` | `"sch_exit"`=persistent |
| `slash_dispute` | `"sd_disp"`=persistent, `"sd_next"`=instance, `"sd_open"`=instance, `"sd_s2d"`=persistent, `"sd_slash"`=persistent, `"sd_win"`=instance |
| `snapshot_airdrop` | `"air_clmd"`=persistent, `"air_cnt"`=instance, `"air_rgst"`=persistent |
| `stake_funded_bug_bounty` | `"bb_bps"`=persistent, `"bb_fund"`=instance |
| `stake_gated_ipfs_storage` | `"ipfscfg"`=instance, `"ipfsrec"`=persistent |
| `stake_hooks` | `"stk_hooks"`=instance |
| `stake_quota` | `"qt_cfg"`=instance, `"qt_use"`=persistent |
| `stake_weighted_news_feed` | `"news_cnt"`=instance, `"news_itm"`=instance, `"news_vt"`=persistent |
| `stake_weighted_tip_jar` | `"tip_recv"`=persistent, `"tip_sent"`=persistent |
| `staker_favor_rounding` | `"sfr_on"`=instance |
| `staker_network_graph` | `"mirr_tgt"`=persistent |
| `staker_region_tag` | `"rgn_idx"`=persistent |
| `staker_sentiment_index` | `"si_clm"`=instance, `"si_infl"`=instance, `"si_msg"`=instance, `"si_rat"`=instance, `"si_vote"`=instance |
| `staking_covenant` | `"cov_rec"`=persistent, `"cov_trm"`=instance |
| `sub_pool_delegation` | `"sub_cnt"`=instance, `"sub_pool"`=instance, `"sub_pos"`=persistent |
| `sub_unit_reward_accumulator` | `"sua_ckp"`=persistent, `"sua_rem"`=persistent |
| `time_locked_admin_proposal` | `"acp_lst"`=instance, `"acp_nid"`=instance |
| `transfer_cooldown` | `"tc_cool"`=instance, `"tc_recv"`=persistent |
| `ttl_management` | `"ttl_thr"`=instance |
| `tvl_rate_rebalance` | `"tvl_thr"`=instance |
| `twa_reward_rate` | `"twa_cps"`=instance |
| `validator_delegation` | `"vd_dele"`=persistent, `"vd_wght"`=instance |
| `validator_rewards` | `"vr_bal"`=persistent, `"vr_node"`=instance, `"vr_pool"`=instance |
| `vault_extensions_490_493` | `"feerec_492"`=instance, `"p_dep"`=instance, `"p_with"`=instance |
| `vault_extensions_498_501` | `"swp_rte"`=instance, `"ts_split"`=instance |
| `vault_extensions_502_505` | `"auto_comp"`=persistent, `"nft_bals"`=instance, `"nft_nxt"`=instance, `"nft_owns"`=instance, `"tl_acts"`=instance, `"tl_nxt_id"`=instance, `"wq_en"`=instance, `"wq_q"`=instance |
| `vault_extensions_538_541` | `"rate_rmp"`=instance |
| `vault_extensions_546_549` | `"lg_hold"`=instance, `"lg_thr"`=instance, `"lg_tr"`=persistent, `"mn_rwdf"`=instance, `"mx_pos"`=instance, `"shr_dec"`=instance |
| `vesting_cliff` | `"clf_evt"`=persistent, `"vst_clff"`=instance |
| `vote_weight_delegation` | `"vwd_dele"`=persistent, `"vwd_drs"`=persistent |
| `xlm_wrapper_integration` | `"xlm_sac"`=instance, `"xlm_wst"`=instance |

Bare (non-tuple) keys here are pool-wide config/counters (correctly
instance); every key listed as persistent is used exclusively as part of a
`(SYMBOL, Address, ...)` tuple, i.e. genuinely per-user/per-position data.

## Fix applied: `auto_comp` (issue #504 auto-compound) was misclassified

`vault_extensions_502_505.rs`'s `set_auto_compound` / `compound` stored a
single `Map<Address, bool>` — one boolean per user — under **one instance
key**, rewriting the entire map on every call to `set_auto_compound`. That's
per-user data stored as if it were global config: it fails the "per-user data
uses persistent storage" criterion directly, and it doesn't scale — the
single instance value grows with every new user who opts in, and every
unrelated contract call still pays to load it (instance keys are loaded
unconditionally on every invocation) even though only `compound()` and
`set_auto_compound()` ever need it.

Fixed by switching to a `(Symbol, Address)` tuple key per user under
`.persistent()`, matching the convention used everywhere else in the crate —
see the diff in `vault_extensions_502_505.rs`. No other call sites read the
old map's full contents (no enumeration/iteration existed), so this is a
drop-in change.

## Temporary storage: correctly unused

Nothing in the vault uses `.temporary()`, and that's the right call, not an
oversight. Soroban's temporary storage silently *deletes* (no archive, no
restore) once its TTL lapses. Every candidate that looked "ephemeral" at a
glance turns out to need to survive past its own nominal expiry for
settlement or correctness:

- Auction bids, sealed-bid data, sale offers, lottery/prediction-market
  config (`position_sealed_bid_auction`, `dex_limit_order_buyback`, etc.) —
  the *business* deadline (`ends_at`/`draw_at_ledger`/...) is unrelated to
  Soroban's storage TTL. If the storage entry were temporary and got swept by
  the network before someone calls the finalize/refund/settle entrypoint, the
  bid or escrowed funds would become unrecoverable — a real fund-loss bug, not
  a savings.
- Timelocked/queued admin actions (`PendingAction`, `AdminProposal`,
  `time_locked_admin_proposal`) — must still be present and readable once
  their `executable_at` ledger arrives, which is the entire point of queuing
  them.
- Rolling-window trackers (`ClaimWindow`, `daily_withdrawal_limit`,
  `daily_token_velocity_limiter`) — read and compared against on every
  relevant call; losing one early would silently reset a user's limit to
  "fresh window," which is a security/business-logic regression, not a cost
  optimization.
- The one truly single-transaction-lifetime value in the contract, the
  reentrancy guard (`vault.rs`'s `REENTRANCY_KEY`), is set and removed again
  within the same call via `ReentrancyGuard`'s `Drop` impl — it never survives
  to a second ledger regardless of which storage type backs it, so instance
  vs. temporary makes no observable difference there.

If a genuinely short-lived, safe-to-drop value (e.g. a single-use off-chain
price quote with its own short expiry, never referenced again after use) is
added in the future, temporary storage is the right place for it — the
absence of any use today is a reflection of what this contract actually
stores, not a gap.

## Known scaling risks (feeds into issue #623's benchmarks)

Instance-vs-persistent classification is correct everywhere audited above,
but one instance key has a data-*shape* problem regardless of which storage
type it uses: `DataKey::AllStakers` (`balance::get_all_stakers` /
`balance::register_staker`) is a single instance value holding **one
`Vec<Address>` of every address that has ever staked**, appended to and never
pruned.

- `balance::register_staker` — called once per address, on that address's
  first stake — reads the whole vec, does a linear de-dup scan, and (for a
  genuinely new address) writes the whole, now-one-longer vec back. So every
  *new* depositor's first stake costs more as the pool grows; a returning
  depositor's top-up stake never touches this path and stays flat.
- `VaultContract::stake_weighted_average_duration` (public, no auth) and
  `VaultContract::export_state` (admin-only) both read and loop over the
  *entire* vec with no cap.
  `VaultContract::view_all_positions` paginates its *output*, but still loads
  the full vec to do so. `VaultContract::get_reward_gini_coefficient` bounds
  itself by reverting above `MAX_GINI_STAKERS`.
  `VaultContract::get_top_depositors` is the one fully bounded example: it
  caps its scan at `MAX_DEPOSITOR_SCAN` (200) registration-order entries
  regardless of pool size.
- `VaultContract::deposit`'s `is_first_deposit` check has the identical
  unbounded-scan shape, but `deposit()` itself currently fails to compile on
  `main` for an unrelated, pre-existing reason (see the repo's baseline-build
  notes) and so isn't exercised by the benchmarks below.

`src/test_issue_623_scale.rs` (issue #623) measures this empirically at
100/500/1000 depositors using `env.budget()`'s real CPU-instruction
accounting and asserts on the trend described above: bounded functions stay
flat, unbounded ones grow. Fixing the underlying `AllStakers` shape (e.g. a
maintained top-N index instead of a full-registry scan, as `get_top_depositors`'s
own doc comment already suggests) is future work — out of scope for a
documentation-plus-audit issue — but is exactly what those tests would need
to keep passing if a future change tries to bound these functions further.

## Storage Read/Write Cost Audit for Per-User Scan Operations (Issue #600)

### Executive Summary

As pool size grows, full-registry and multi-user iteration pathways incur quadratic or linear gas and storage read/write amplification. This audit catalogs every iteration path across the vault, documents read/write amplification metrics, establishes bounding mechanisms, and outlines architectural migration from full-scan aggregations to incrementally-maintained running totals.

### Multi-User Iteration Pathways & Amplification Matrix

| Function / Operation | Module / Entrypoint | Storage Keys Accessed | Read Amplification | Write Amplification | Current Bound / Mitigation | Recommended Next Architecture |
|---|---|---|---|---|---|---|
| `balance::register_staker` | `src/balance.rs` | `DataKey::AllStakers` | $O(N)$ instance read | $O(N)$ instance write | Linearly rewrites full staker list | Sharded staker bitmap or set |
| `get_top_depositors` | `src/vault.rs` | `DataKey::AllStakers`, `ShareBalance(Address)` | $O(\min(N, 200))$ persistent reads | $O(0)$ (read-only) | Hard-capped at `MAX_DEPOSITOR_SCAN = 200` | Incrementally-maintained top-N sorted skip-list on deposit/withdraw |
| `get_reward_gini_coefficient` | `src/vault.rs` | `DataKey::AllStakers`, `ShareBalance(Address)` | $O(\min(N, 100))$ persistent reads | $O(0)$ (read-only) | Reverts above `MAX_GINI_STAKERS = 100` | Sample-based Monte Carlo estimation for pools > 100 users |
| `scan_for_collusion` | `src/collusion_detector.rs` | `DataKey::AllStakers`, `"cld_snap"` | $O(\min(N, 100))$ persistent reads | $O(K)$ alerts | Bounded to `MAX_GINI_STAKERS` | Event emission trigger with off-chain indexer analysis |
| `stake_weighted_average_duration` | `src/vault.rs` | `DataKey::AllStakers`, `ShareBalance`, `StakedAtLedger` | $O(N)$ persistent reads | $O(0)$ (read-only) | Unbounded scan on full staker registry | Maintain running accumulators `TotalStakeTimeWeight` & `TotalShares` |
| `view_all_positions` | `src/vault.rs` | `DataKey::AllStakers`, `ShareBalance(Address)` | $O(N)$ instance read + $O(\text{limit})$ persistent reads | $O(0)$ (read-only) | Paginated output, but loads full `AllStakers` vector | Index-based pagination with range cursors |
| `export_state` | `src/vault.rs` | All instance & persistent keys | $O(N)$ exhaustive read | $O(0)$ (read-only) | Admin-only emergency diagnostic path | Chunked export with cursor continuation tokens |

### Mitigation Architectures: Running Totals vs. Full Scans

1. **Incremental Running Totals (Zero-Scan Pattern)**:
   - Aggregations such as total deposits (`DataKey::TotalDeposited`), total shares (`DataKey::TotalShares`), and total stakers count (`DataKey::TotalStakers`) are strictly maintained incrementally upon every deposit, withdrawal, and stake action.
   - Any new aggregation metric must follow this pattern rather than traversing the registry at query time.

2. **Hard Bounding & Safe Truncation**:
   - Diagnostic and analytics operations (`get_top_depositors`, `collusion_detector`) enforce deterministic ceilings (`MAX_DEPOSITOR_SCAN = 200`, `MAX_GINI_STAKERS = 100`).
   - If user count exceeds the ceiling, functions either truncate with clear API caveats or require off-chain indexing.

3. **Pagination & Range Limits**:
   - All multi-user view functions must require `start_after: Option<Address>` or `offset: u32` along with `limit: u32` clamped to at most 50 entries.

## Ledger Entry Size Audit for Rolling-Buffer `Vec` Storage (Issue #649)

### Executive Summary
Soroban enforces a maximum ledger entry size of 64KB (65,536 bytes). Rolling-buffer storage vectors (histories, logs, snapshots, queues) risk hitting this ceiling at full capacity if unbounded or inadequately sized. This audit cataloged every `Vec`-backed storage key, measured element sizes and capacity ceilings, and empirically verified that writes and reads succeed safely at capacity.

### Rolling-Buffer Vector Ledger Entry Size Matrix

| Storage Key | Storage Scope | Element Type | Approx Element Size | Max Length / Cap | Max Entry Size (XDR) | Limit Safety Margin | Status / Mitigation |
|---|---|---|---|---|---|---|---|
| `act_log` | Instance | `DayBucket` (4 `u32`s) | 16 bytes | 7 days | ~140 bytes | > 99.7% | Bounded rolling window |
| `RateHistory` | Instance | `(u32, u32, u32)` | 12 bytes | 50 entries (`MAX_RATE_HISTORY_ENTRIES`) | ~680 bytes | > 98.9% | FIFO pruned upon append |
| `rr_hist` | Instance | `RateHistoryEntry` | 16 bytes | 20 entries (`MAX_RICH_RATE_HISTORY`) | ~360 bytes | > 99.4% | FIFO pruned upon append |
| `Changelog` | Instance | `ChangelogEntry` | ~68 bytes | 10 entries (`MAX_CHANGELOG_ENTRIES`) | ~720 bytes | > 98.8% | FIFO pruned upon record |
| `StakeHistory(Address)` | Persistent | `(u32, i128)` | 20 bytes | 5 entries (`MAX_STAKE_HISTORY`) | ~120 bytes | > 99.8% | Bounded per user |
| `clm_hist` | Persistent | `(u32, i128)` | 20 bytes | 100 entries (`MAX_CLAIM_HISTORY`) | ~2,200 bytes | > 96.6% | FIFO pruned upon record |
| `BoostSchedule` | Instance | `(u32, u32)` | 8 bytes | 5 tiers (`MAX_BOOST_TIERS`) | ~60 bytes | > 99.9% | Validated on set |
| `VestingEntries(Address)` | Persistent | `VestingEntry` | 20 bytes | 10 entries (`VestingQueueFull`) | ~240 bytes | > 99.6% | Reverts when queue full |
| `acc_tier` | Instance | `AccessTier` | 20 bytes | 10 tiers | ~250 bytes | > 99.6% | Bounded admin tiers |
| `anniv_pd` | Persistent | `u32` | 4 bytes | 20 years | ~100 bytes | > 99.8% | Naturally bounded |
| `u_rcpt` | Persistent | `u64` | 8 bytes | 50 receipts | ~450 bytes | > 99.3% | Bounded per user |

### Findings & Mitigations
- All rolling buffer collections have explicit capacity bounds (`MAX_*`) and FIFO pruning mechanisms on append.
- The largest rolling buffer entry (`clm_hist` at 100 entries) consumes ~2.2 KB in XDR format, providing a safety margin of over 96.6% below Soroban's 64 KB ledger entry limit.
- Verified via `src/test_vec_storage_capacity.rs`: filling every rolling buffer to its maximum capacity succeeds without error.
- No rolling buffer key risks exceeding ledger entry capacity.
