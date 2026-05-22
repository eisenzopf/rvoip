# 5000 CPS Cleanup Backlog Investigation

## Summary

This investigation focuses on the current signaling-only SIPp result:
`rvoip-sip` is clean through `2000 CPS` but collapses at `5000 CPS`
with cleanup lag, high in-flight call count, and retransmission storms.

The goal is to prove which cleanup subpath creates the backlog, fix it
without regressing the clean `1000/2000 CPS` path, and establish whether
the historical `10000 CPS` result was comparable.

Current reference report:

```text
/tmp/rvoip_asterisk_sipp_full_20260522_014255/summary.md
```

Important benchmark boundary: this is signaling-only. SIPp sends SIP
messages with SDP bodies and holds the dialog for 100 ms, but it does
not send RTP media packets.

## Current Signal

Reference SIPp comparison, 15 seconds steady load, SIPp sharded at
about `1000 CPS` per runner:

| Target | 1000 CPS | 2000 CPS | 5000 CPS |
| --- | ---: | ---: | ---: |
| rvoip | 100%, p99 `<10 ms`, 0 retrans | 100%, p99 `<150 ms`, 0 retrans | 5.8%, heavy retrans, cleanup backlog |
| Asterisk | 48.4%, achieved 484 CPS | 24.2%, achieved 484 CPS | 4.9%, heavy retrans |

During the rvoip `5000 CPS` point, `perf_listener` showed accepted
calls continuing to rise while cleanup fell behind. The in-flight gap
grew to roughly `12k+` calls before SIPp timed out and retransmissions
exploded.

Working hypothesis: cleanup is the first load-bearing bottleneck above
`2000 CPS`. This is not yet proven as the only cause; the investigation
must identify the specific cleanup stage.

## Investigation Plan

### 1. Baseline the Current Knee

Run the corrected sharded SIPp harness against rvoip with this ladder:

```bash
RVOIP_PERF_RESULTS=/tmp/rvoip_cleanup_ladder_$(date +%Y%m%d_%H%M%S) \
RVOIP_PERF_CPS="1000 2000 3000 4000 5000" \
RVOIP_PERF_STEADY_SECS=15 \
RVOIP_PERF_SIPP_SHARD_CPS=1000 \
RVOIP_PERF_TRACE_SCREEN=0 \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_comparison_dockerized.sh \
  host.docker.internal 35060 rvoip
```

Capture for every point:

- accepted calls, cleaned calls, and in-flight gap from `perf_listener`
- SIPp total calls, success calls, failed calls, current calls, and retransmits
- achieved CPS, ASR, p95, and p99 from `analyze.py`
- RSS and CPU if available from the process sampler or host tools

Keep Asterisk as secondary calibration. The blocker is the rvoip cleanup
knee, not Asterisk parity.

### 2. Add Cleanup-Stage Instrumentation

Add temporary diagnostics behind perf-only env flags or log targets. Do
not change public API behavior.

Required measurements:

- per-call cleanup start/end timestamp
- cleanup active count and queue depth where applicable
- p50/p95/p99/max duration for each cleanup stage
- count of cleanup operations started, completed, failed, and still in flight

Stage breakdown to measure:

- dialog and transaction termination
- media adapter cleanup and RTP/media session teardown
- session store removal and index cleanup
- global/session event publication during terminal events
- timer cancellation and task shutdown

The output should be easy to correlate with SIPp points. Prefer periodic
log lines from `perf_listener` plus a final summary at shutdown.

### 3. Profile the Failing Point

Run `samply` against `perf_listener` during the `5000 CPS` point:

```bash
cargo build --profile flamegraph -p rvoip-sip --example perf_listener

RVOIP_EVENT_BUS_CHANNEL_CAPACITY=100000 \
RVOIP_PERF_CHANNEL_CAPACITY=20000 \
RVOIP_PERF_SESSION_EVENT_WORKERS=16 \
RVOIP_PERF_SESSION_EVENT_CHANNEL_CAPACITY=50000 \
samply record -- \
  target/flamegraph/examples/perf_listener 35060 192.168.5.2
```

Confirm whether CPU time is primarily in:

- cleanup logic
- lock contention
- task scheduling or wakeups
- allocator pressure
- SIP retransmission handling after the backlog forms

Also capture heap/RSS behavior after the pressure point to identify
retained sessions, timers, or tasks.

### 4. Isolate Media and SDP Cleanup

Run the same ladder with the normal SDP/media allocation path first.
Then run a minimal/no-media variant:

- If a no-media or signaling-only accept path already exists, use it.
- If it does not exist, add a perf-only config gate that skips RTP/media
  session allocation while preserving SIP + SDP signaling behavior as
  much as possible.

Interpretation:

- If the no-media variant restores high CPS, focus fixes on media
  adapter allocation/cleanup and RTP session teardown.
- If the no-media variant still collapses, focus on dialog/transaction,
  session store, event publication, timers, or runtime scheduling.

### 5. Validate Against the Historical 10000 CPS Result

Locate the historical `10000 CPS` benchmark scenario and result. Record:

- commit SHA and date
- benchmark driver: SIPp, in-process, criterion, or synthetic loop
- whether it included BYE and full session cleanup
- whether it included SDP negotiation
- whether it allocated media/RTP sessions
- whether success criteria included ASR, retransmits, p95/p99, and cleanup drain
- whether the workload was sharded, single-process, or single-runner

Only call the current result a regression if the historical workload is
equivalent to the current SIPp scenario.

## Acceptance Criteria

Before making a fix, the investigation is complete when we can answer:

- Which cleanup stage first falls behind at `5000 CPS`?
- Does the backlog begin before or after SIP retransmissions start?
- Does disabling/minimizing media allocation materially change the knee?
- Is the historical `10000 CPS` result comparable to this SIPp workload?

After a fix, acceptance is:

- `1000 CPS`: 100% ASR, p99 `<10 ms`, 0 retrans, cleanup drains to near zero
- `2000 CPS`: 100% ASR, p99 `<150 ms`, 0 retrans, cleanup drains to near zero
- higher ladder points: knee moves upward or degradation is explained by a
  newly identified bottleneck
- no public event semantics or public API behavior changes

## Test Plan

- Unit tests for any diagnostic-only counters, histograms, or env-gated
  instrumentation.
- Existing lifecycle/config tests remain passing.
- Release smoke:

```bash
RVOIP_PERF_SWEEP_CPS=10,50 \
RVOIP_PERF_RAMP_SECS=1 \
RVOIP_PERF_STEADY_SECS=2 \
RVOIP_PERF_COOLDOWN_SECS=2 \
RVOIP_PERF_CALL_TIMEOUT_SECS=5 \
cargo test -p rvoip-sip --features perf-tests --release \
  --test perf_call_setup_cps -- --nocapture
```

- Release SIPp pressure ladder:

```bash
RVOIP_PERF_CPS="1000 2000 3000 4000 5000" \
RVOIP_PERF_STEADY_SECS=15 \
RVOIP_PERF_SIPP_SHARD_CPS=1000 \
RVOIP_PERF_TRACE_SCREEN=0 \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_comparison_dockerized.sh \
  host.docker.internal 35060 rvoip
```

## Assumptions

- Primary scope is cleanup backlog at high signaling CPS.
- The `5000 CPS` failure is not RTP media throughput; no RTP packets are
  exchanged in this SIPp scenario.
- Public event semantics must remain unchanged.
- Diagnostic instrumentation may be temporary or gated behind perf-only
  env/config.
- The first implementation target is measurement; the fix should be the
  smallest change proven by that measurement.
