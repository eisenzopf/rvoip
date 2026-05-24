# Dialog Core BYE And Transaction Dispatch Hot Path Plan

Date: 2026-05-24

## Summary

The latest 20k CPS investigation shows that the remaining bottleneck is no
longer primarily duplicate INVITE recovery. The clean validation runs kept
`dup_invite_cache_miss=0`, `worker_mismatch=0`, and usually
`ack_unmatched=0`, while dead-call `200 OK` traffic was mostly BYE-attributed
and BYE receive-to-200 tails tracked backlog.

Transaction dispatch queue delay also dominated the stressed `tx=2 dlg=4`
shape. The next phase should instrument, profile, and stress test
BYE/termination fast paths and transaction dispatch queue isolation/admission
before changing worker-count recommendations.

## Current Findings

- 18k clean shapes remain healthy: low RTT, near-zero dead-call `200 OK`, zero
  host UDP drops, and clean duplicate/ACK/worker diagnostics.
- 20k `tx=1 dlg=4` clean validation showed high late BYE response volume:
  `204,635` dead-call `200 OK`, with `164,593 BYE` and BYE receive-to-200
  `over_500ms=234,058`.
- 20k `tx=2 dlg=4` shifted pressure toward transaction dispatch: dispatch queue
  `over_500ms` stayed high, queue depth reached tens of thousands, and repeated
  runs showed sensitivity to host UDP drops under stress/profiling.
- INVITE duplicate-cache/proactive retransmit remains present but is not the
  primary next target; `dup_invite_cache_miss=0` held in validation.
- `sample` and `samply` captures show heavy time in BYE response send paths,
  transaction dispatch/dialog dispatch workers, and UDP send/serialization
  under overload.

## Proposed Next Work

### BYE And Termination Diagnostics

Add diagnostics behind the existing diagnostics and timing gates. Do not change
public behavior or public APIs.

- Classify BYE `200 OK` sends as fresh in-dialog, tombstone retransmit,
  duplicate terminated dialog, or late post-cleanup path.
- Measure BYE ingress-to-dialog-handler, dialog-handler-to-transaction-send,
  transaction-send-to-UDP-send, and cleanup-release timing.
- Record tombstone lookup hit/miss latency, tombstone table size, BYE server
  transaction release latency, and termination cleanup fanout.
- Keep existing idempotence counters intact so every new BYE timing can be
  correlated with `bye_200_sent`, BYE tombstone counters, and dead-call
  `CSeq: BYE` analyzer output.

### Transaction Dispatch Queue Diagnostics

Extend queue visibility before introducing new worker topology or admission
behavior.

- Report per-worker queue depth and dequeue delay, not only aggregate max.
- Add event-kind splits for INVITE, ACK, BYE, CANCEL, lifecycle, and other.
- If an admission policy is prototyped, count admitted, deferred, and dropped
  events by event kind and worker.
- Correlate transaction queue delay with BYE receive-to-200 tails in the
  analyzer summary.

### Optimization Prototypes

Prototype only after the new counters identify the hot segment.

- Prioritize BYE handling or isolate BYE events from INVITE duplicate/retransmit
  work if BYE tails continue to dominate dead-call `200 OK` volume.
- Avoid cleanup work on the BYE `200 OK` response path where correctness
  permits; release and cleanup should be bounded and should not delay the
  response.
- Keep tombstone idempotence, but make tombstone lookup and reply bounded and
  low-contention.
- Evaluate transaction dispatch queue isolation/admission as an experiment:
  separate lifecycle/cleanup work from request-bearing SIP events, or separate
  BYE from INVITE pressure.
- Do not promote a worker topology from this phase unless clean validation,
  profiler artifacts, and diagnostics all point to the same shape.

## Investigation Runs

Run repo checks before load testing:

```text
cargo fmt
cargo test -p rvoip-sip-dialog
cargo test -p rvoip-sip-transport
cargo test -p rvoip-sip --test config_channel_capacity_integration
python3 -m py_compile crates/rvoip-sip/tests/perf/sipp_scenarios/analyze.py
git diff --check
```

Run validation with diagnostics enabled and
`--sip-udp-recv-buffer-size 8388608`:

```text
18k: tx=1 dlg=4
18k: tx=1 dlg=8
18k: tx=2 dlg=4
20k: tx=1 dlg=4
20k: tx=2 dlg=4
```

Capture profiles for these fixed-build shapes:

```text
18k promoted baseline: UDP RR 4, transport 1, transaction 2, dialog 1, session dispatcher 4
20k candidate A:       UDP RR 4, transport 1, transaction 1, dialog 4, session dispatcher 4
20k candidate B:       UDP RR 4, transport 1, transaction 2, dialog 4, session dispatcher 4
```

Each load summary must include SIPp rc, achieved CPS, retransmits, host UDP
drop delta, dead-call `200 OK` attribution by CSeq method, final
`sip_udp_diag`, final `sip_retrans_diag`, BYE/termination diagnostics, queue
diagnostics, and `sample`/`samply` artifact paths.

## Decision Rules

- If dead-call `200 OK` remains mostly `CSeq: BYE` and BYE receive-to-200 tails
  track BYE/termination timing, target BYE response fast-path and termination
  cleanup isolation first.
- If transaction dispatch queue delay dominates while dialog/session queues stay
  flat, test transaction queue isolation/admission before changing worker
  counts.
- If dialog event dispatch or dialog-to-session publish queues dominate, isolate
  dialog-to-session publication or cleanup fanout before tuning transaction
  workers.
- If proactive or duplicate-cache INVITE 2xx counters become dominant again,
  revisit INVITE 2xx pacing, but do not treat it as the primary hypothesis for
  this phase.

## Acceptance Criteria

- Preserve `dup_invite_cache_miss=0`, `worker_mismatch=0`, and host UDP drops at
  zero in clean validation shapes.
- Keep `ack_unmatched=0` in promoted clean shapes; any nonzero count requires a
  follow-up before promoting the result.
- Reduce 20k dead-call BYE `200 OK`, BYE receive-to-200 `over_500ms`, and
  transaction dispatch `over_500ms`.
- Keep all new diagnostics gated by existing diagnostics/timing flags.
- Do not add public API or public config changes unless counters and profiles
  justify a production-facing knob.
- Do not promote a new worker topology based only on a single profiled run.

## Assumptions

- This is a follow-up plan and does not replace the prior hot-path investigation
  documents.
- The next implementation pass remains instrumentation-first, then optimization
  prototypes, then validation.
- Public behavior remains unchanged while the next bottleneck is being isolated.
