# Dialog Core BYE And Transaction Dispatch Hot Path Plan

Date: 2026-05-24

## Summary

The latest 20k CPS investigation shows that the remaining bottleneck is no
longer primarily duplicate INVITE recovery. The clean validation runs kept
`dup_invite_cache_miss=0`, `worker_mismatch=0`, and usually
`ack_unmatched=0`, while dead-call `200 OK` traffic was mostly BYE-attributed
and BYE receive-to-200 tails tracked backlog.

The follow-up instrumentation confirmed the first actionable issue:
BYE requests were waiting in the shared transaction dispatch backlog before the
BYE handler ran. BYE tombstone lookup, transaction release, response send
segments, and cleanup removal were measurable but did not explain the largest
tail. The first implemented fix is therefore ACK/BYE priority inside each
transaction dispatch worker, preserving the existing worker hash and call
affinity.

The later cache/pacing experiments showed a second pressure point: INVITE 2xx
retransmission send volume competes with teardown work under stress. Prebuilt
cached wire bytes are worth keeping, but the resend pacing budget is workload
sensitive and should remain tunable instead of being promoted as a single
universal constant.

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

## Implementation Findings

- Added gated diagnostics for BYE path segments, BYE tombstones, transaction
  dispatch queue delay by event kind, and transaction dispatch queue delay by
  worker.
- Analyzer output now surfaces the new bracketed BYE/dispatch diagnostics and
  preserves `sample`/`samply` artifact paths.
- In the clean 20k `tx=2 dlg=4` baseline, BYE receive-to-200 tails matched
  `tx_received_to_handler`, and transaction dispatch BYE `over_500ms` accounted
  for nearly the same tail. That identifies the queue before the BYE handler as
  the first fix target.
- BYE-only priority removed most BYE dispatch delay, but it starved ACKs:
  `ack_unmatched=94495` at 20k. That shape is rejected.
- ACK+BYE priority preserved call affinity and fixed the ACK starvation issue.
  It reduced BYE dead-call volume, but 20k still showed UDP drops and high
  total dead-call volume, now mostly INVITE-attributed.
- Prebuilt raw cached INVITE 2xx sends reduced serialization/send overhead for
  duplicate and proactive cached responses. This is worth keeping.
- Aggressive INVITE 2xx resend pacing at `512` per 100 ms tick was excellent
  for the clean 18k shape, reaching zero dead-call `200 OK`, zero BYE
  over-500 ms, and zero dispatch over-500 ms. The same pacing hurt the lossy
  20k shape: the run completed only about 91.5 percent of expected successful
  calls and had `47214` host UDP drops. That value is not a safe default.
- Restoring the default resend budget to `2048` kept 20k completion at
  285000/285000 successful calls and reduced UDP drops versus priority-only,
  but 20k was still not acceptance-clean.

## Current Tunable Parameters

These are now exposed through `rvoip_sip::Config` and surfaced by the
`perf_listener` and SIPp sharding matrix harness.

- `sip_transaction_dispatch_priority_burst_max`
  - Default when unset: transaction-layer default `64`.
  - Applies only when `sip_transaction_dispatch_workers > 1`.
  - Lower values give INVITE/CANCEL/response work more fairness during ACK/BYE
    storms.
  - Higher values favor ACK/BYE latency when teardown tail dominates and normal
    lane delay is acceptable.
  - Perf CLI: `--transaction-dispatch-priority-burst-max`.
  - Matrix env:
    `RVOIP_SHARDING_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX`.
- `sip_invite_2xx_retransmit_max_due_per_tick`
  - Default when unset: transaction-layer default `2048`.
  - Controls proactive cached INVITE 2xx retransmits per 100 ms maintenance
    tick.
  - Lower values pace UDP send bursts and can protect teardown work in clean
    high-CPS shapes.
  - Higher values clear INVITE 2xx retransmit backlog faster when the host send
    path has headroom or packet loss is high.
  - Perf CLI: `--invite-2xx-retransmit-max-due-per-tick`.
  - Matrix env:
    `RVOIP_SHARDING_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK`.

The short-lived cache TTL remains internal. Current evidence points at
dispatch ordering and send pacing, not stale-cache lifetime or cache memory
pressure.

## Stress Results So Far

- Baseline instrumented 20k `tx=2 dlg=4`:
  - `dup_invite_cache_miss=0`, `worker_mismatch=0`, `ack_unmatched=0`.
  - Dead-call `200 OK`: `137730` total, `120220 BYE`, `17510 INVITE`.
  - BYE receive-to-200 `over_500ms=232608`.
  - BYE `tx_received_to_handler over_500ms=232608`.
  - Transaction dispatch BYE `over_500ms=232277`.
- BYE-only priority:
  - BYE dispatch tail improved, but `ack_unmatched=94495` at 20k.
  - Rejected because it violates ACK correctness/clean-shape requirements.
- ACK+BYE priority:
  - 18k: rc `0`, host UDP drops `0`, dead-call `200 OK=14198`
    (`3257 BYE`, `10941 INVITE`), `ack_unmatched=5`.
  - 20k: rc `1`, host UDP drops `44958`, dead-call `200 OK=22707`
    (`1539 BYE`, `21168 INVITE`), `ack_unmatched=0`.
- ACK+BYE priority plus raw cached INVITE 2xx responses, default resend budget
  `2048`:
  - 18k: rc `0`, host UDP drops `0`, dead-call `200 OK=5315`
    (`3533 BYE`, `1782 INVITE`), `ack_unmatched=2`, raw cached sends
    `44534`, proactive cached sends `21379`.
  - 20k: rc `1`, host UDP drops `35371`, success `285000/285000`,
    dead-call `200 OK=20012` (`1968 BYE`, `18044 INVITE`),
    `ack_unmatched=0`, raw cached sends `280289`, proactive cached sends
    `101708`.
- ACK+BYE priority plus raw cached INVITE 2xx responses, resend budget `512`:
  - 18k: rc `0`, host UDP drops `0`, dead-call `200 OK=0`,
    `ack_unmatched=0`, raw cached sends `9055`, proactive cached sends `2`,
    BYE receive-to-200 `over_500ms=0`, transaction dispatch `over_500ms=0`.
  - 20k: rc `1`, host UDP drops `47214`, success `260697/285000`,
    dead-call `200 OK=5506` (`3953 BYE`, `1553 INVITE`),
    `ack_unmatched=2`, raw cached sends `110796`, proactive cached sends
    `2960`.

## Current Conclusion

We have identified two real problems and one rejected approach:

- Real problem: BYEs can sit behind shared transaction dispatch backlog before
  the BYE handler runs. Fix: ACK/BYE priority lane per transaction worker with
  starvation protection.
- Real problem: cached INVITE 2xx retransmission send volume can compete with
  teardown under stress. Fix: prebuilt raw cached sends plus a configurable
  proactive resend budget.
- Rejected approach: BYE-only priority. It improves BYE latency but breaks ACK
  matching under stress.

This is not a final 20k fix yet. The 18k shape can be made very clean with
lower resend pacing, but the 20k shape still crosses host UDP send/receive
limits and remains sensitive to retransmit volume.

## Completed Work

- BYE and termination diagnostics were added behind existing diagnostics and
  timing gates. They classify BYE response paths, measure BYE handler segments,
  track tombstone lookup/prune behavior, and keep the existing idempotence
  counters intact.
- Transaction dispatch diagnostics now report dequeue delay and depth by event
  kind and by worker, which made the BYE pre-handler backlog visible.
- Transaction dispatch workers now use high and normal lanes per worker. ACK
  and BYE requests enter the high lane; INVITE, CANCEL, responses, and other
  events enter the normal lane. Worker selection is unchanged, preserving call
  affinity.
- Starvation protection processes one ready normal item after the configured
  priority burst.
- INVITE 2xx cache entries now store prebuilt wire bytes so cached duplicate
  and proactive retransmits can avoid rebuilding the SIP response.
- The ACK/BYE priority burst and INVITE 2xx proactive retransmit budget are
  configurable through `Config`, `SessionBuilder`, `perf_listener`, and the SIPp
  sharding matrix.

## Remaining Work

- Tune the two new Config parameters against fixed 18k and 20k shapes.
- Decide whether a lower INVITE 2xx resend budget should become part of a named
  high-CPS profile, or remain only an explicit benchmark/deployment knob.
- If 20k still drops host UDP packets after moderate resend pacing, investigate
  UDP send admission or transaction/dialog backpressure rather than further BYE
  handler micro-optimization.
- If BYE tails reappear after pacing is tuned, use the existing BYE segment
  metrics to decide whether response send, cleanup, or tombstone lookup has
  become the new dominant segment.
- Do not promote a worker topology or default pacing value based only on a
  profiled run; keep same-shape clean controls.

## Investigation Runs

Run repo checks before load testing:

```text
cargo fmt --check
cargo test -p rvoip-sip-dialog
cargo test -p rvoip-sip-transport
cargo test -p rvoip-sip --test config_channel_capacity_integration
cargo build -p rvoip-sip --release --example perf_listener
python3 -m py_compile crates/rvoip-sip/tests/perf/sipp_scenarios/analyze.py crates/rvoip-sip/tests/perf/sipp_scenarios/test_analyze.py
git diff --check
```

Run validation with diagnostics enabled, `--sip-udp-recv-buffer-size 8388608`,
and the fixed signaling-only media shape:

```text
18k: tx=2 dlg=4, invite_2xx_due_budget=512/1024/1536/2048, priority_burst=16/32/64/128
20k: tx=2 dlg=4, invite_2xx_due_budget=512/1024/1536/2048, priority_burst=16/32/64/128
```

Capture profiles for these fixed-build shapes:

```text
18k control/candidate: UDP RR 4, transport 1, transaction 2, dialog 4, session dispatcher 4
20k control/candidate: UDP RR 4, transport 1, transaction 2, dialog 4, session dispatcher 4
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
  tune `sip_invite_2xx_retransmit_max_due_per_tick` and compare same-shape
  clean/profiler runs before changing the default.

## Next Recommended Experiments

- Sweep `sip_invite_2xx_retransmit_max_due_per_tick` between the known points:
  `512`, `1024`, `1536`, and `2048`.
- Sweep `sip_transaction_dispatch_priority_burst_max` around `16`, `32`, `64`,
  and `128` for `tx=2 dlg=4`.
- Keep the same fixed shape for comparison: UDP RR `4`, transport `1`,
  transaction `2`, dialog `4`, session dispatcher `4`, capacity `20000`,
  UDP receive buffer `8388608`, signaling-only media.
- Every profiled run still needs a same-shape non-profiled control because the
  20k shape is sensitive to host UDP drops.
- If 20k still drops UDP with moderate pacing, investigate UDP send admission
  or transaction/dialog backpressure rather than further optimizing BYE handler
  internals.

## Acceptance Criteria

- Preserve `dup_invite_cache_miss=0`, `worker_mismatch=0`, and host UDP drops at
  zero in clean validation shapes.
- Keep `ack_unmatched=0` in promoted clean shapes; any nonzero count requires a
  follow-up before promoting the result.
- Reduce 20k dead-call BYE `200 OK`, BYE receive-to-200 `over_500ms`, and
  transaction dispatch `over_500ms`.
- Keep all new diagnostics gated by existing diagnostics/timing flags.
- Public config changes are now justified for the dispatch priority burst and
  INVITE 2xx resend budget because the best values differ by stress shape.
- Do not promote a new worker topology based only on a single profiled run.

## Assumptions

- This is a follow-up plan and does not replace the prior hot-path investigation
  documents.
- The next implementation pass remains instrumentation-first, then optimization
  prototypes, then validation.
- Public behavior remains unchanged while the next bottleneck is being isolated.
