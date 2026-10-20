# Mainnet Launch Checklist

Do not deploy until every required item is checked, blockers are resolved, and
the rollback/pause plan is signed off. Keep the completed copy with the
deployment record.

## Rollback and pause plan (complete before launch)

- [ ] Name the authorized admin, independent guardian (if configured), and
  signer threshold. Link the private signer/recovery policy:
  `[operator vault / policy link]`.
- [ ] Confirm the exact pause procedure and its effect: `pause` blocks deposits
  **and withdrawals**; `guardian_pause` is available only if a guardian was
  configured. Review the [incident response runbook](./INCIDENT_RESPONSE.md).
- [ ] Record the response for a pause that prevents ordinary withdrawals:
  use `emergency_withdraw(user)` only as a break-glass principal-only exit,
  with reward forfeiture and token-transfer caveats documented.
- [ ] Define who can authorize `unpause()` and what evidence is required before
  resuming.
- [ ] Record the rollback boundary: deployed Soroban contract state/transactions
  are not undone by a software rollback. Identify the reviewed prior WASM hash,
  upgrade authority, and the recovery/forward-fix decision process; never
  promise an automatic rollback.

## Code freeze

- [ ] Freeze the release commit and tag; record commit SHA and WASM hash.
  Complete [contribution/release guidance](./CONTRIBUTING_GUIDE.md).
- [ ] Confirm the release branch contains only approved changes and that the
  deployed artifact is built from that exact commit.
- [ ] Confirm version, contract interface, and network parameters; review
  [`src/interface.rs`](../src/interface.rs), the
  [interface documentation](./INTERFACE.md), and the compile-time
  [runbook interface snapshot](../src/test_runbook_interface_snapshot.rs).
- [ ] Record unresolved issues and explicitly accept or block each one; include
  [#800](https://github.com/Rickyy1017/Stellar-Defi-Vault/issues/800),
  [#801](https://github.com/Rickyy1017/Stellar-Defi-Vault/issues/801),
  [#802](https://github.com/Rickyy1017/Stellar-Defi-Vault/issues/802), and
  [#803](https://github.com/Rickyy1017/Stellar-Defi-Vault/issues/803) in the
  release review.

## Audit status

- [ ] Link the final audit report and remediation evidence:
  `[audit report / tracking issue]`.
- [ ] Verify every critical/high finding is closed or has a named maintainer's
  explicit risk acceptance.
- [ ] Review the [security model and known limitations](./SECURITY.md) and
  [storage/scale risks](./STORAGE.md).
- [ ] Confirm the incident roster and
  [response runbook](./INCIDENT_RESPONSE.md) are complete and reachable by
  responders.

## Configuration review

- [ ] Independently verify the stake token, reward token, admin, guardian,
  emergency admin, reward rate, fees, pause state, runway guard, and all
  relevant limits against the approved deployment sheet.
- [ ] Confirm the reward pool is funded and `get_projected_runway()` meets the
  policy threshold; record the source transaction.
- [ ] Confirm the circuit-breaker threshold and whether it is enabled; record
  expected behavior and any alerting gaps.
- [ ] Confirm the network, RPC URL, contract ID, token IDs, and starting ledger
  are supplied through deployment/operator secrets and environment, not
  committed configuration.
- [ ] Review [event names and payloads](./EVENTS.md); configure the
  [event watcher](../scripts/watch-events.ts), cursor persistence, thresholds,
  and private webhook endpoint.

## Key management

- [ ] Verify admin and guardian keys are controlled by the approved multisig or
  hardware-backed signer policy; test signer availability without exposing
  secrets.
- [ ] Verify the guardian is independently controlled and can call
  `guardian_pause`; document the contingency if no guardian is configured.
- [ ] Verify admin recovery, upgrade authority, and backup signers; record
  access owners in the private operator system.
- [ ] Confirm no seed phrase, secret key, RPC credential, or webhook URL is
  present in source, logs, CI output, or deployment records.

## Deployment

- [ ] Confirm the production WASM artifact hash and network before submitting
  any transaction.
- [ ] Use a reviewed mainnet deployment procedure. The repository's
  [`deploy_testnet.sh`](../scripts/deploy_testnet.sh) defaults to **testnet**
  and is not a mainnet launch procedure; do not switch its network casually.
- [ ] Deploy and initialize from the approved identity; capture transaction
  hashes, contract ID, token IDs, admin address, and ledger.
- [ ] Have a second maintainer independently verify the deployment record and
  initialization inputs before user traffic is enabled.

## Verification

- [ ] Query the deployed contract's admin, token addresses, rate, pause state,
  reward-pool balance, and runway; compare every value with the approved sheet.
- [ ] Confirm the deployed WASM hash matches the frozen artifact and interface.
- [ ] Verify `is_paused()` has the intended value and test a representative
  read-only query over the production RPC.
- [ ] Confirm event indexing from the deployment ledger and that the watcher
  cursor advances and survives a restart.
- [ ] Reconcile contract balances and total deposited/reward accounting with
  the deployment record before enabling deposits.

## Monitoring

- [ ] Start the event watcher as a supervised service or scheduled job with
  durable cursor storage and restricted write permissions.
- [ ] Confirm alerts reach the on-call channel for pause/admin change,
  large withdrawal, fee change, circuit-breaker signal, and low reward runway.
- [ ] Set explicit service health checks, RPC failure alerts, and on-call
  ownership; do not rely solely on contract events for failed transactions.
- [ ] Record the first monitoring review time and backup operator.

## Communications

- [ ] Publish the canonical contract ID, supported network, official UI, and
  support channel through approved channels.
- [ ] Prepare the initial incident notice and user FAQ using the
  [incident communication templates](./INCIDENT_RESPONSE.md#user-communication-templates).
- [ ] Confirm the communications lead and escalation contacts are reachable.
- [ ] Announce launch only after the deployment, configuration, and monitoring
  checks are signed off.

## Sign-off

| Role | Name | Decision | Date/time (UTC) | Signature / record link |
|---|---|---|---|---|
| Release maintainer | `[name]` | `[approve/block]` | `[UTC]` | `[link]` |
| Contract/security reviewer | `[name]` | `[approve/block]` | `[UTC]` | `[link]` |
| Deployment operator | `[name]` | `[approve/block]` | `[UTC]` | `[link]` |
| Incident commander/on-call | `[name]` | `[approve/block]` | `[UTC]` | `[link]` |
| Communications lead | `[name]` | `[approve/block]` | `[UTC]` | `[link]` |

**Launch decision:** `[GO / NO-GO]`  
**Contract ID / network:** `[value]`  
**Release commit / WASM hash:** `[value]`  
**Next review:** `[UTC date/time]`
