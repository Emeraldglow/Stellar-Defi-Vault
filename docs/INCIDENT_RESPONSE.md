# Mainnet Incident Response Runbook

This runbook is for the deployed Stellar/Soroban vault. Keep the deployment
record, current contract ID, token IDs, signer policy, RPC endpoint, and contact
roster alongside the on-call handoff. Do not put secret keys, seed phrases, or
webhook URLs in this document.

## First five minutes

1. Open an incident channel and assign an incident commander (IC), contract
   operator, communications lead, and scribe. One person may hold more than one
   role only until additional responders arrive.
2. Record the contract ID, network, ledger/time, transaction hashes, observed
   balances, and an unedited copy of relevant RPC responses.
3. Classify severity using the table below. If uncertain, use the higher level.
4. Decide whether to pause. For an active exploit, unexplained outflow, or
   compromised operator key, pause immediately using the independent guardian
   when configured; otherwise use the admin. Do not wait for consensus to
   preserve funds.
5. Notify the on-call contacts through the pre-agreed secure channel. Never ask
   anyone to send a private key or seed phrase.

## Severity and decision tree

| Severity | Criteria | Response target |
|---|---|---|
| **SEV-1 Critical** | Confirmed/credible exploit, unauthorized admin action, active loss of user funds, or compromised admin/guardian key | Pause immediately; page every maintainer and security contact; publish a user notice after the initial containment update |
| **SEV-2 High** | Reward claims failing broadly, token behavior breaking deposits/withdrawals, or suspected accounting/invariant issue without confirmed loss | Incident commander immediately; pause if state changes could worsen loss; notify maintainers and affected users |
| **SEV-3 Moderate** | Degraded RPC/indexer/monitoring, isolated transaction failures, or low runway with no failed payouts | Assign an owner; investigate and publish an update if user action is needed |
| **SEV-4 Low** | Non-impacting alert, documentation/configuration issue, or resolved transient | Record and resolve in the normal operational channel |

```text
Is there a credible exploit, unauthorized state change, or active loss?
  yes -> SEV-1; pause with guardian_pause (if configured), otherwise pause.
  no  -> Are claims, withdrawals, or token transfers failing for many users?
           yes -> SEV-2; pause if further calls can worsen the incident.
           no  -> Is the reward runway below policy or an operational service degraded?
                    yes -> SEV-3; top up/recover service or escalate if payouts fail.
                    no  -> SEV-4; record and monitor.
After any pause: preserve evidence -> diagnose -> obtain independent approval
                -> verify balances and code/config -> unpause only when safe.
```

## Roles and decision rights

- **Incident commander:** owns severity, timeline, decisions, and handoffs.
- **Contract operator:** submits transactions from the approved signer set and
  confirms results from RPC; never shares signing material.
- **Security lead:** validates exploit scope, evidence preservation, and
  remediation.
- **Communications lead:** posts only approved updates using the templates below.
- **Scribe:** records timestamps, transaction hashes, decisions, owners, and
  next update time.

The IC may declare/escalate an incident and request a pause. Only the deployed
contract's authorized admin or configured guardian can execute the respective
on-chain pause function. A second maintainer must independently verify
high-impact admin transactions where signer policy permits.

## Playbooks

The function names below are the deployed vault entrypoints in
[`src/vault.rs`](../src/vault.rs) and its extension modules. Check the deployed
WASM/interface for the specific contract before submitting any transaction.
The compile-time entrypoint snapshot for the runbook is
[`src/test_runbook_interface_snapshot.rs`](../src/test_runbook_interface_snapshot.rs).

### Suspected exploit or unexplained outflow

1. Confirm the contract ID, suspicious transaction hashes, affected addresses,
   token balances, and whether the calls succeeded. Preserve RPC responses and
   ledger numbers.
2. If configured, call `guardian_pause(guardian, message)` with a concise
   security-incident message. Otherwise call
   `pause(SecurityIncident, message)` from the admin. Verify with `is_paused()`
   and confirm the `paused`/`grd_paus` event.
3. Remember: `pause` blocks both deposits and withdrawals. Do not promise that
   normal exits remain available. `emergency_withdraw(user)` is a
   break-glass principal-only exit available only while paused; it forfeits
   accrued rewards and is not a substitute for investigating token-transfer
   risk.
4. Stop discretionary admin actions and preserve the signer audit trail. Do
   not attempt `unpause()` until exploit path, balances, and remediation have
   been independently reviewed.
5. Escalate as SEV-1. Publish the initial user notice; post updates at the
   interval agreed by the IC.

**Functions:** `guardian_pause`, `pause`, `is_paused`, `emergency_withdraw`,
`unpause`.

### Reward pool drained or claims failing

1. Confirm the reward token address and balance; query `get_reward_pool_balance()`
   and, when enabled, `get_projected_runway()`. Compare with the latest
   `rwpl_low`/`rfil_alt` alert and the configured runway policy.
2. Distinguish reward insolvency from principal-token impairment. Do not label
   `add_yield` as reward funding: it increases the deposited/strategy total.
3. If claims are failing but principal accounting is sound, pause only if
   continued activity can worsen the issue. Confirm the effect of a pause on
   withdrawals before acting.
4. If funding is verified and authorized, use `fund_reward_pool(admin_addr,
   amount)` (or the configured role-gated funding path). Recheck
   `get_reward_pool_balance()` and `get_projected_runway()`, and test a
   representative claim before declaring recovery.
5. If rate configuration caused the drain, lower it using
   `set_reward_rate_bps(rate_bps)` only after confirming the value and runway
   guard. Do not make a speculative rate change during an exploit.

**Functions:** `get_reward_pool_balance`, `get_projected_runway`,
`fund_reward_pool`, `set_reward_rate_bps`, `claim`, `pause`, `unpause`.

### Admin key compromised or suspected compromised

1. Treat any unrecognized `admin_set`, `adm_act`, guardian, or emergency-admin
   change as SEV-1. Verify it against the approved change record and notify the
   IC through the independent channel.
2. If an uncompromised configured guardian is available, use
   `guardian_pause(guardian, message)` immediately. A compromised admin may
   still be able to submit admin transactions; pause is containment, not key
   recovery.
3. From the approved recovery signer/multisig, transfer control with
   `transfer_admin(new_admin)` only if the current contract authorization
   policy allows it. Verify `get_admin()` and the `admin_set` event. Do not
   assume that `set_emergency_admin` or `revoke_emergency_admin` can recover a
   compromised primary admin: both require the primary admin.
4. If no uncompromised authorized signer remains, do not improvise or claim
   that the contract can be recovered on-chain. Escalate to the pre-agreed
   governance/recovery process and publish a factual notice.
5. Rotate affected off-chain credentials separately. Never paste secret
   material into the incident channel or this runbook.

**Functions:** `guardian_pause`, `transfer_admin`, `get_admin`,
`set_emergency_admin`, `revoke_emergency_admin`, `set_guardian`,
`remove_guardian`.

### Stake/reward token contract issue

1. Verify the configured token address, issuer status, transfer behavior, and
   whether failures affect deposits, claims, or exits. Save failed transaction
   hashes and token issuer communications.
2. If vault operations may worsen the impact, call `guardian_pause` or `pause`
   and verify `is_paused()`. Determine explicitly whether the token contract
   itself can still transfer funds; a vault pause cannot repair or override a
   token contract.
3. Do not call `rescue_token` for the stake token or registered reward token:
   the contract rejects both. It is only for unrelated, accidentally stuck
   tokens. Never promise `emergency_withdraw` will succeed if the token itself
   rejects transfers.
4. Coordinate with the token issuer and security lead. Resume with `unpause()`
   only after token behavior and representative transactions have been
   independently verified.

**Functions:** `pause`, `guardian_pause`, `is_paused`, `rescue_token`,
`emergency_withdraw`, `unpause`.

### Yield-source/strategy failure (only if enabled for this deployment)

The vault supports optional external strategy adapters. Use this playbook only
when the deployment record confirms that an adapter or yield source is
configured and funded.

1. Confirm the affected source address, its allowlist status, current vault and
   source balances, and whether the failure affects principal or rewards.
2. Stop new exposure by halting calls to `allocate_to_strategy(keeper, adapter,
   amount)` and the integration's own control plane. Do not remove a configured
   source or request a withdrawal until the source operator confirms the
   effects.
3. If recovery is approved, use `withdraw_from_strategy(admin, adapter,
   amount)` for the adapter, or `withdraw_from_yield(admin, amount)` for the
   legacy yield-source path. Verify the adapter implementation and returned
   amount before reconciling vault accounting.
4. Compare `get_strategies()`, `total_strategy_assets()`, and
   `total_vault_assets()` with token balances. If accounting is uncertain or
   an active exploit is possible, pause the vault and do not reopen until the
   share-price impact is reviewed.

**Functions:** `get_strategies`, `add_strategy`, `allocate_to_strategy`,
`withdraw_from_strategy`, `total_strategy_assets`, `total_vault_assets`,
`add_yield_source`, `remove_yield_source`, `withdraw_from_yield`, `pause`,
`guardian_pause`, `is_paused`, `unpause`.

## Contacts and escalation roster template

Fill this in before mainnet deployment. Store actual contact details in the
approved private operator system, not in a public repository.

| Role | Primary | Backup | Secure contact/channel | Escalation trigger |
|---|---|---|---|---|
| Incident commander / maintainer | `[name, handle]` | `[name, handle]` | `[private channel]` | Any SEV-1/2 |
| Contract operator / signer coordinator | `[name, handle]` | `[name, handle]` | `[private channel]` | Pause, admin, or recovery transaction |
| Security/audit contact | `[team/contact]` | `[team/contact]` | `[private channel]` | Exploit, key compromise, token issue |
| Token issuer contact | `[organization/contact]` | `[backup]` | `[private channel]` | Token transfer/freeze issue |
| Communications lead | `[name, handle]` | `[name, handle]` | `[private channel]` | Any user-facing incident |
| RPC/hosting provider | `[provider/contact]` | `[support route]` | `[account/ticket route]` | Data or endpoint outage |

Escalation order: on-call operator -> incident commander -> security and
maintainers -> affected issuer/provider -> users/public status channel.
For SEV-1, page all listed roles in parallel.

## User communication templates

Use only confirmed facts; do not speculate about root cause, recovery time, or
fund safety. Include the next update time even if there is no new information.

**Initial notice**

> We are investigating an issue affecting the Stellar DeFi Vault on
> `[network]` (`[contract ID]`). At `[UTC time]` we `[paused/not paused]`
> vault operations while we verify the impact. Please do not interact with
> unofficial links or share signing credentials. Next update: `[UTC time]`.

**Progress update**

> Update `[UTC time]`: `[confirmed impact and actions taken]`. The current
> status is `[status]`. `[User action, or "No action is requested at this
> time."]` We will update again by `[UTC time]`.

**Resolution**

> Resolved at `[UTC time]`: `[verified fix/recovery]`. Vault status:
> `[paused/unpaused]`. Verified transaction(s): `[hashes]`. Any remaining
> user action: `[instructions or none]`. Post-incident review will be
> published by `[date/channel]`.

## Post-incident review template

- **Incident ID / severity:**
- **Incident commander / participants:**
- **Start, detection, containment, and resolution times (UTC):**
- **Affected network, contract ID, tokens, and users:**
- **Summary of confirmed impact (include amounts and ledger/tx references):**
- **Detection source and what worked/did not work:**
- **Timeline of facts and decisions:**
- **Root cause and contributing factors (separate confirmed facts from hypotheses):**
- **Containment and recovery transactions:**
- **User communications and outstanding requests:**
- **What went well / what was difficult:**
- **Corrective actions** (owner, due date, tracking issue):
- **Runbook/monitoring changes required:**
- **Review date and approvers:**

Do not include keys, tokens, private user data, or unredacted secrets in the
review.
