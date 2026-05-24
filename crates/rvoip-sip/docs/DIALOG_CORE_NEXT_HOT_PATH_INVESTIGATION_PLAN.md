# Dialog Core Next Hot Path Investigation Plan

## Summary

The first hot-path fix improved overload behavior by pacing INVITE 2xx retransmission maintenance and retaining ACKed 2xx responses briefly for duplicate INVITE idempotence. That removed duplicate INVITE cache misses, ACK misses, host UDP drops, and the diagnostics overflow display.

We are not done. The 20k CPS shape still shows high RTT, high retransmits, and many SIPp `Dead call, received 200 OK` messages. The next investigation should focus on where the remaining backlog forms and why late 200 OK traffic is still produced after calls complete.

## What Improved

- INVITE 2xx maintenance is now bounded to a fixed batch per tick instead of draining the whole due backlog.
- ACKed 2xx responses remain cached for a short retention window, so duplicate INVITEs still hit the cache after ACK.
- ACKed entries no longer proactively retransmit.
- Diagnostics now report capped maintenance ticks, due queue depth, and finite high-latency buckets.
- Validation showed `dup_invite_cache_miss=0`, `ack_unmatched=0`, `worker_mismatch=0`, and zero host UDP drops in the tested shapes.

## Next Investigation

- Trace the remaining overload queue path from UDP receive through transaction dispatch and dialog/session publish.
- Compare `tx=1 dialog=4` and `tx=2 dialog=4`; `tx=2 dialog=4` is now the stronger 20k candidate.
- Attribute late 200 OK traffic by source:
  - first INVITE 200 OK
  - duplicate INVITE cache response
  - proactive INVITE 2xx retransmit
  - BYE 200 OK
  - post-completion/late send
- Re-run flamegraphs under the fixed build for:
  - 18k promoted baseline
  - 20k `tx=1 dialog=4`
  - 20k `tx=2 dialog=4`
- Use diagnostics and flamegraphs to decide whether the next fix is queue admission, worker topology, duplicate-response pacing, cleanup isolation, or BYE/termination fast-path work.

## Test Plan

- Run repo checks:
  - `cargo fmt`
  - `cargo test -p rvoip-sip-dialog`
  - `cargo test -p rvoip-sip-transport`
  - `cargo test -p rvoip-sip --test config_channel_capacity_integration`
  - `git diff --check`
- Run load validation:
  - 18k: `tx=1 dialog=4`, `tx=1 dialog=8`, `tx=2 dialog=4`
  - 20k: `tx=1 dialog=4`, `tx=2 dialog=4`
- Capture for each load run:
  - SIPp rc
  - achieved CPS
  - retransmits
  - host UDP drop delta
  - `dup_invite_cache_miss`
  - `ack_unmatched`
  - `worker_mismatch`
  - dead-call 200 OK count
  - final `sip_udp_diag` and `sip_retrans_diag` summaries
  - flamegraph artifact path

## Acceptance Criteria

- Keep `dup_invite_cache_miss=0`.
- Keep `ack_unmatched=0`.
- Keep `worker_mismatch=0`.
- Keep host UDP socket drops at zero.
- Keep diagnostics free of `u64::MAX` latency output.
- Materially reduce 20k dead-call 200 OK volume.
- Materially reduce 20k RTT and retransmits.
- Do not regress the clean 18k promoted shape.

## Assumptions

- The INVITE 2xx maintenance/cache issue is fixed enough to move to the next bottleneck.
- `tx=2 dialog=4` should be treated as the primary 20k comparison candidate.
- `tx=1 dialog=4` remains useful as the stress/failure shape.
- No public API changes should be introduced for the next investigation unless diagnostics require an internal opt-in field.
