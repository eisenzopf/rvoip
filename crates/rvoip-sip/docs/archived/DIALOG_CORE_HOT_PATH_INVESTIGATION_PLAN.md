# Dialog-Core Hot Path Investigation Plan

Date: 2026-05-23

## Summary

The 18k signaling-only sharding matrix shows that dialog dispatch fanout is
not a simple CPU bottleneck. Dialog workers help one shape
(`transport=1`, `transaction=1`, `dialog=8`), but do not compose cleanly with
transaction fanout. The best repeated clean profile remains:

```text
--udp-parse-workers 4 --udp-parse-round-robin
--sip-transport-dispatch-workers 1
--transaction-dispatch-workers 2
--sip-dialog-dispatch-workers 1
--session-event-dispatcher-workers 4
--signaling-only-media
```

The next target is to find and remove interference around dialog-core rather
than increasing worker counts blindly.

## Implemented Fix Finding

The first fix confirmed that INVITE 2xx maintenance and cache timing were part
of the overload amplifier. Bounding proactive INVITE 2xx retransmit
maintenance and retaining ACKed 2xx responses briefly made the high-CPS path
more stable:

- Maintenance no longer drains the entire due backlog in one tick.
- ACKed 2xx responses remain available for duplicate INVITE idempotence during
  a short retention window.
- ACKed entries no longer proactively retransmit.
- Validation showed `dup_invite_cache_miss=0`, `ack_unmatched=0`,
  `worker_mismatch=0`, and zero host UDP drops in the tested shapes.

The important follow-up lesson is that overload-sensitive SIP correctness work
must be bounded and should not remove idempotence state too eagerly. Remaining
20k CPS issues should be investigated as another backlog/amplification path,
not as a simple worker-count problem.

## Findings From Code Review

### Dialog Event Affinity Can Split A Call

Dialog dispatch currently routes request-bearing transaction events by
`Call-ID + From tag`, but lifecycle events without a request fall back to
`TransactionKey`.

- `manager/core.rs`: `request_dialog_route_hash`.
- `manager/core.rs`: `dialog_event_route_hash`.
- `manager/core.rs`: `dialog_event_dispatch_worker_index`.

That means INVITE/ACK/BYE request events for a call can land on one dialog
worker while `StateChanged` or `TransactionTerminated` events for the same
transaction land on another. The shared DashMap indexes make this mostly work,
but under combined transaction+dialog fanout it can reorder create/link/cleanup
work enough to explain duplicate-cache misses and worse tail latency.

### Dialog Handler Work Is Not The Main Cost

Existing diagnostics show dialog handler and lookup spans are usually
microseconds. The large `udp_receive_to_incoming_call_emit` and
`bye_receive_to_200` tails are therefore likely before dialog handler entry,
around global transaction dispatch, transaction event broadcast, cleanup
side-work, or event-affinity contention.

### Transaction Termination Cleanup Is Too Task-Heavy

`broadcast_event` spawns cleanup handling for every
`TransactionTerminated`. `process_transaction_terminated` then spawns another
task per transaction, polls lifecycle state every 100 ms, and after removal can
spawn an indexed cleanup sweep.

- `transaction/manager/mod.rs`: `broadcast_event`.
- `transaction/manager/mod.rs`: `process_transaction_terminated`.
- `transaction/manager/functions.rs`: `cleanup_indexed_terminated_transactions`.
- `transaction/manager/functions.rs`: `cleanup_terminated_transactions`.

At 270k calls this can create hundreds of thousands of background tasks and
repeated DashMap scans during the hot window. That is exactly the kind of
scheduler/cache churn that can prevent dialog fanout from improving throughput.

### INVITE 2xx Proactive Retransmit Scan Is Global

The INVITE 2xx response cache is scanned every 100 ms with
`invite_2xx_response_cache.iter_mut()`. In a high-CPS run that cache can hold a
large fraction of active calls, so each tick touches many DashMap shards even
when only a small number of entries are due.

- `transaction/manager/mod.rs`: `retransmit_due_invite_2xx_responses`.
- `transaction/manager/mod.rs`: `prune_invite_2xx_response_cache`.

This is RFC-useful behavior, but the implementation should be due-driven
instead of full-map-scan driven.

### Session Publish Path Has Avoidable Hot-Path Friction

`emit_session_coordination_event` and `try_emit_session_coordination_event`
read `event_hub` or `session_coordinator` and then await while the RwLock read
guard is still live. In the common initialized case this probably does not
dominate, but it is a low-risk cleanup and removes one unnecessary async lock
interaction from a high-frequency path.

- `manager/core.rs`: `emit_session_coordination_event`.
- `manager/core.rs`: `try_emit_session_coordination_event`.
- `events/event_hub.rs`: `try_publish_session_coordination_event`.

The same area also has noisy `info!` logs in the publish path. The perf
listener default filter keeps them disabled, but they should not be `info!` on
a hot path.

## Next Instrumentation

Add these diagnostics behind existing timing/diagnostic gates:

1. Dialog route affinity:
   - request route hash and selected dialog worker.
   - transaction-key route hash and selected dialog worker.
   - count when a lifecycle event for a transaction uses a different worker
     than the original request for that transaction.
   - counts by event kind: INVITE, ACK, BYE, StateChanged, Terminated, other.

2. Transaction termination cleanup:
   - number of termination cleanup tasks spawned.
   - current cleanup task in-flight count and max.
   - lifecycle poll attempts before cleanup.
   - cleanup batch size.
   - indexed cleanup scan duration and number of keys scanned.
   - full cleanup scan duration and number of client/server keys scanned.
   - timer unregister latency during cleanup.

3. INVITE 2xx cache maintenance:
   - cache length at scan start.
   - entries scanned per tick.
   - due entries per tick.
   - expired entries per tick.
   - scan duration p50/p99/p999.
   - proactive retransmit send duration.

4. Global event publish:
   - dialog-to-global publish handler count.
   - time spent in registered handlers vs broadcast send.
   - event type counts for dialog-to-session events.

5. Runtime hot-path and flamegraph capture:
   - add optional matrix-runner profiling modes that attach during the active
     SIPp window and save artifacts beside `summary.md`.
   - use macOS `sample` for quick thread-state snapshots
     (`RVOIP_SHARDING_SAMPLE=1`).
   - use `samply` CPU profiles for flamegraph/icicle analysis
     (`RVOIP_SHARDING_SAMPLY=1`); keep all worker threads visible by default
     so transaction/dialog/Tokio scheduling distribution is inspectable.
     On macOS, run `samply setup` once if PID attach is denied.
   - capture at least one baseline and one dialog-fanout run before changing
     worker-count recommendations.
   - inspect flamegraphs for Tokio scheduler/context-switch churn, DashMap
     shard contention, transaction cleanup work, timer unregister latency,
     INVITE 2xx maintenance, dialog dispatch/link/unlink work, and global
     coordinator publish handlers.

## Optimization Candidates

### 1. Preserve Dialog Affinity For Lifecycle Events

Store the chosen call route for a transaction when dialog-core sees the first
request-bearing event. For later non-request lifecycle events, route by that
stored call route before falling back to raw `TransactionKey`.

Acceptance:

- INVITE, ACK, BYE, CANCEL, StateChanged, and TransactionTerminated for one
  call route to the same dialog worker when a call route is known.
- `dup_invite_cache_miss` remains `0` with transaction and dialog fanout.
- Existing single-worker behavior is unchanged.

### 2. Replace Per-Transaction Cleanup Spawns With A Batch Cleanup Worker

Keep immediate marking of terminated transactions, but move cleanup to one
bounded worker that drains terminated transaction ids in batches. The worker can
requeue transactions that have not reached `Destroyed` yet, using a delayed
retry bucket instead of one sleeping task per transaction.

Remove the per-transaction follow-up call to
`cleanup_indexed_terminated_transactions`; keep periodic full cleanup as a
defensive fallback, not as part of every termination.

Acceptance:

- Cleanup still reaches accepted-call count.
- BYE cleanup delivered remains exact.
- In-flight cleanup task count stays bounded.
- Indexed cleanup scans are batch-sized, not per-transaction.

### 3. Replace Full INVITE 2xx Cache Scans With A Due Queue

On cache insert, schedule the transaction key in a due queue or timing wheel.
On ACK removal, remove from the cache and let stale due entries no-op when they
pop. The periodic worker should pop only due entries instead of scanning the
whole DashMap.

Acceptance:

- Retransmitted INVITEs still get cached 2xx responses.
- Proactive 2xx retransmits still occur until ACK or TTL.
- Cache miss count remains `0`.
- Scan duration no longer grows with total active cache size.

### 4. Clean Up Session Publish Locking And Logging

Clone the event hub or legacy sender out of the RwLock before awaiting. Downgrade
hot-path `info!` logs in dialog event publication to `trace!` or `debug!`.

Acceptance:

- No behavior change.
- Dialog session publish timings stay flat under fanout.
- Tests covering fallback publish semantics still pass.

### 5. Only Then Revisit Wider Dialog Index Structure

If route affinity and cleanup/cache work do not fix fanout composition, inspect
the dialog indexes next. The current link/unlink path updates several DashMaps
and vector lists per transaction. A later optimization could consolidate related
per-dialog transaction indexes into one sharded entry, but this is more invasive
and should wait for route/cleanup evidence.

## Test Matrix

Run each change at 18k first, then promote only clean candidates to 20k.

Baseline:

```text
UDP RR 4, transport 1, transaction 2, dialog 1, session dispatcher 4
```

Instrumentation-only:

```text
baseline + new route/cleanup/cache diagnostics
transaction 1 + dialog 8 + new diagnostics
transaction 2 + dialog 8 + new diagnostics
```

Optimization probes:

```text
affinity fix only:      transaction 1/2, dialog 1/4/8
batch cleanup only:     transaction 1/2, dialog 1/8
2xx due queue only:     transaction 1/2, dialog 1/8
combined candidate:     transaction 2, dialog 4/8
```

Acceptance for each run:

- SIPp calls complete at target count.
- Listener accepted and cleanup counts match target.
- warnings/dead stay `0/0`.
- ACK matched and delivered match accepted calls.
- BYE 200 and BYE cleanup delivered match accepted calls.
- `dup_invite_cache_miss=0`.
- `udp queue_full=0`.
- host UDP full-socket drops `0`.
- retransmits improve over the same worker profile before the change.
- when profiling is enabled, `sample`/`samply` artifacts are captured for both
  baseline and candidate runs and compared before promoting a new worker shape.

Do not promote dialog dispatch above `1` until the affinity diagnostics prove
that lifecycle events are not split across workers and the duplicate-cache-miss
signal stays at zero.
