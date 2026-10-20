# Event Schema

All events include the current ledger sequence number (`u32`) in their data payload so
that indexers can reconstruct full pool history from events alone.

## Topic conventions

Every event starts with two indexed topics:

1. `topic[0]`: a stable event-type `Symbol` (at most 9 ASCII characters).
2. `topic[1]`: the primary affected `Address` (user, administrator, recipient,
   guarantor, or the vault contract for pool-wide/system events).

Admin audit events use `topic[2]` for the `AdminAction` discriminator. Other
event data remains in the non-indexed payload; an address may also remain in
the payload when needed to preserve the event's existing data shape.

Indexers should filter by the event symbol first, then by the address in
`topic[1]`. Pool-wide events use the current vault contract address in that
slot because no individual account is the event's primary subject.

## Event Reference

### `deposit`
Emitted on every successful stake/deposit.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"deposit"` |
| topic[1] | Address | depositor |
| data.0 | i128 | token amount deposited |
| data.1 | i128 | shares minted |
| data.2 | u32 | ledger sequence |

### `withdraw`
Emitted on every successful unstake/withdraw.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"withdraw"` |
| topic[1] | Address | withdrawer |
| data.0 | i128 | shares burned |
| data.1 | i128 | token amount returned |
| data.2 | u32 | ledger sequence |

### `pos_open` — position_opened
Emitted only on the **first stake** for a user (when their position goes from 0 → non-zero).

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"pos_open"` |
| topic[1] | Address | user |
| data.0 | i128 | initial stake amount |
| data.1 | u32 | ledger sequence |

### `pos_clos` — position_closed
Emitted when a user fully unstakes (position reaches 0).

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"pos_clos"` |
| topic[1] | Address | user |
| data.0 | u32 | ledger sequence |

### `paused`
Emitted when the vault is paused by the admin.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"paused"` |
| topic[1] | Address | admin |
| data.0 | u32 | ledger sequence |

### `unpaused`
Emitted when the vault is unpaused by the admin.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"unpaused"` |
| topic[1] | Address | admin |
| data.0 | u32 | ledger sequence |

### `rate_chg` — rate_changed
Emitted when the admin changes the reward rate.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"rate_chg"` |
| topic[1] | Address | vault contract |
| data.0 | u32 | old rate in basis points |
| data.1 | u32 | new rate in basis points |
| data.2 | u32 | ledger sequence |

### `yield_add` — yield_added
Emitted when the admin injects yield into the vault.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"yield_add"` |
| topic[1] | Address | admin |
| data.0 | i128 | amount added |
| data.1 | u32 | ledger sequence |

### `tk_rescue` — token_rescued
Emitted when the admin rescues a non-stake, non-reward token from the vault.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"tk_rescue"` |
| topic[1] | Address | recipient |
| data.0 | Address | token address rescued |
| data.1 | i128 | amount rescued |
| data.2 | Address | recipient address |
| data.3 | u32 | ledger sequence |

### `adm_act` — admin_action
Emitted for on-chain admin audit logging when an admin action is taken.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"adm_act"` |
| topic[1] | Address | admin actor |
| topic[2] | AdminAction | admin action type |
| data.0 | Address | admin actor |
| data.1 | u32 | ledger sequence |
| data.2+ | mixed | action-specific parameters |

### `admin_set` — admin_transferred
Emitted when the admin role is transferred to a new address.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"admin_set"` |
| topic[1] | Address | old admin |
| data.0 | Address | new admin |
| data.1 | u32 | ledger sequence |

### `wd_limit` — withdrawal_limit_updated
Emitted when the admin sets a new per-transaction withdrawal limit.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"wd_limit"` |
| topic[1] | Address | admin |
| data.0 | i128 | new limit in shares |
| data.1 | u32 | ledger sequence |

### `hook_fail` — hook_failed
Emitted when a registered stake/unstake hook contract fails; the user's action still succeeds.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"hook_fail"` |
| topic[1] | Address | user |
| data.0 | Address | hook contract |
| data.1 | Symbol | `"stake"` or `"unstake"` |
| data.2 | i128 | amount |
| data.3 | u32 | ledger sequence |

### `gov_par` — governance_params_updated
Emitted when the admin sets the governance quorum and pass threshold.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"gov_par"` |
| topic[1] | Address | admin |
| data.0 | u32 | quorum in basis points |
| data.1 | u32 | pass threshold in basis points |
| data.2 | u32 | ledger sequence |


### `token_fee_set` — token_fee_override_set
Emitted when a per-token deposit/unstake fee override is configured or cleared.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"token_fee_set"` |
| topic[1] | Address | token |
| data.0 | Option<u32> | deposit fee in basis points |
| data.1 | Option<u32> | unstake fee in basis points |
| data.2 | u32 | ledger sequence |

### `rfil_alt` — refill_alert
Emitted when reward runway is below 30 days after a claim.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"rfil_alt"` |
| topic[1] | Address | vault contract |
| data.0 | i128 | reward balance |
| data.1 | u32 | ledgers until empty |
| data.2 | u32 | ledger sequence |

### `rwpl_low` — reward_pool_low
Emitted once per low-balance episode when the pool falls below the configured
ratio of outstanding obligations.

| Field | Type | Description |
|-------|------|-------------|
| topic[0] | Symbol | `"rwpl_low"` |
| data.0 | i128 | reward pool balance |
| data.1 | i128 | outstanding reward obligations |
| data.2 | u32 | configured threshold in basis points |
| data.3 | u32 | ledger sequence |

### `cb_set` — circuit-breaker threshold updated
Emitted when `set_circuit_breaker_threshold_bps` changes the configured
single-withdrawal threshold. This reports configuration, not a circuit-breaker
trip.

### Fee-change notice events

The global unstake-fee notice mechanism emits these events:

| Topic | Data | Meaning |
|-------|------|---------|
| `fee_set` | `(new_bps: u32, ledger: u32)` | Fee decrease or immediate fee update applied |
| `fee_queued` | `(new_bps: u32, effective_at: u32, ledger: u32)` | Fee increase queued for its notice period |
| `fee_apld` | `(new_bps: u32, ledger: u32)` | Queued fee increase applied |
| `fn_cfg` | `(notice_ledgers: u32, ledger: u32)` | Fee-notice delay configuration changed |

## Operator event watcher

[`scripts/watch-events.ts`](../scripts/watch-events.ts) polls the Stellar RPC
`getEvents` endpoint for the configured contract and advances a durable cursor.
It prints alerts to stdout and optionally sends Slack/Discord-compatible
webhooks.

```powershell
npm install
$env:RPC_URL = "https://your-stellar-rpc.example"
$env:CONTRACT_ID = "C..."
$env:START_LEDGER = "123456" # required only on the first run
$env:CURSOR_FILE = "C:\vault-ops\watch-events-cursor.json"
$env:ALERT_RULES = "pause,admin_change,large_withdrawal,fee_change,circuit_breaker_trip,low_reward_runway"
$env:LARGE_WITHDRAWAL_THRESHOLD = "1000000000" # smallest token units
$env:WEBHOOK_URL = "<load from secret manager; never commit>"
npm run watch-events
```

`ALERT_RULES` can contain any comma-separated subset of those six names.
`POLL_INTERVAL_MS` defaults to 5000 and `PAGE_LIMIT` to 100. Preserve the cursor
file on durable storage and restrict its permissions. Delivery is at-least-once:
a webhook failure retains the event cursor for retry, so a previously printed
stdout alert may appear again.

For a small service, run `npm install` in a private operator directory and supervise
`npm run watch-events` with systemd, a container restart policy, or an
equivalent service manager. Inject `RPC_URL`, `CONTRACT_ID`, and `WEBHOOK_URL`
from the service's secret/config manager; mount the cursor path on persistent
storage. Do not use a cron job that starts a second watcher while the first is
still running.

Example systemd unit (store the environment file outside the repository with
operator-only permissions):

```ini
[Unit]
Description=Stellar vault event watcher
After=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/stellar-defi-vault
EnvironmentFile=/etc/stellar-vault-watcher.env
ExecStart=/usr/bin/npm run watch-events
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

The watcher supports `cb_trip` events and alerts on `cb_set` threshold changes.
The current circuit breaker only emits `cb_set` configuration events; a failed
withdrawal reverts its transaction, so an event-only watcher cannot reliably
observe or classify an actual breaker trip. Monitor failed transaction results
separately until the contract exposes a durable trip signal.

Global and per-token fee changes emit `fee_set`, `fee_queued`, `fee_apld`, or
`token_fee_set` events. Other fee-related configuration paths may not have a
dedicated event; keep off-chain monitoring of admin transactions in addition
to this event watcher.

### Pool-wide event topics

Events that do not have a single user or administrator as their primary
subject use the vault contract address as `topic[1]`. This applies to:
`boot_str`, `boot_end`, `act_queue`, `act_exec`, `rate_chg`, `rate_rbk`,
`auto_ps`, `rfil_alt`, `prop_enct`, `buyback`, `pen_rdst`,
`halving`, `pool_act`, `pool_dact`, `rwd_act`, `smth_sch`, `smth_rel`,
`auct_st`, `auct_fin`, `gini_cmp`, `seas_str`, `seas_end`, `snst_chg`,
`mkt_rsl`, and `rev_dist`.

Other event types index their affected account in `topic[1]`, including
deposit/withdraw/claim, admin configuration and audit events, governance
votes, guarantees, validator rewards, content curation, and position/NFT
operations. For example, `fee_buyb` indexes the admin actor, while `cc_apprv`
indexes the admin who closed the vote. `ins_dep`, `rd_set`, `vr_set`,
`vr_dist`, and `updated` likewise index the administrator; `test_ev` indexes
the vault contract address.
