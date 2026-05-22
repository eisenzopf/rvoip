# rvoip vs Asterisk — sipp-driven performance comparison

Date: 2026-05-21. Driver: SIPp 3.7.3 from a sidecar Alpine container
on the asterisk docker bridge. Each scenario drives `uac_perf.xml`
(INVITE → 200 → ACK → 100 ms pause → BYE → 200) at the target CPS for
15 s of steady load (`15 × CPS` total calls).

## TL;DR

| | 30 CPS | 100 CPS | 300 CPS |
|---|---|---|---|
| **Asterisk** | ✅ 100 % | ✅ 100 % | ✅ 100 % |
| **rvoip-sip** | ✅ 100 % | ⚠️ 64.7 % (stall after ~550 calls) | ❌ 0 % (no response) |

At 30 CPS the two stacks are equivalent (100 % ASR, 2 ms INVITE→200 OK
median). Above ~30 CPS rvoip-sip falls off a cliff — the cliff is
exactly the bottleneck described in
[`NEXT_STEPS.md`](NEXT_STEPS.md) Area B (the sipp-driven path serial-
izes somewhere that the in-process call-setup benchmarks bypass).
Asterisk is bottleneck-free through 300 CPS on the same host.

Asterisk's median INVITE→200 OK latency is **1 ms** at 100/300 CPS
(2 ms at 30 CPS because the warm-up dominates); rvoip-sip matches at
the rates it can sustain (1–2 ms at 30 CPS) before it stalls
entirely.

## Methodology

- **Driver**: a one-off Alpine 3.20 docker image (`local-sipp`) with
  `sipp 3.7.3` from the Alpine community repo, run as a sidecar on
  the `asterisk_asterisk-local` docker bridge network. Sidecar
  approach is forced by Docker Desktop on macOS — UDP responses from
  a bridge-networked container don't reach a host-bound sipp because
  the reply's source IP is the container's bridge IP rather than the
  forwarded host IP.
- **Scenario**: `uac_perf.xml` (this directory). One INVITE → wait
  200 → ACK → 100 ms `<pause>` → BYE → wait 200. 100 ms hold keeps
  the in-flight dialog count bounded at high CPS; the per-call
  histogram bins are pinned at the top of the file.
- **Asterisk target**: the `[anonymous]` PJSIP endpoint on UDP 5060
  (no auth) routing to the `[perf-bench]` dialplan context
  (catch-all Answer/Wait 1/Hangup). Already shipped in the
  `~/Developer/asterisk/` config.
- **rvoip target**: `cargo build --release -p rvoip-sip --example
  perf_listener` then run on host port 35060. The sipp container
  reaches it through `host.docker.internal:35060` so the routing
  path is symmetric to the Asterisk run (sipp leaves the same docker
  bridge, network hop length is identical).
- **Sweep**: 30 / 100 / 300 CPS for 15 s steady load each (`15 × CPS`
  total calls). Each band gets `(target_cps × 30 s)` budget for
  retransmits and tear-down before sipp's `-timeout` aborts the run.
- **Stat capture**: `sipp -trace_stat -stf <prefix>` writes a per-
  scenario CSV; the run script `docker cp`s the file out at the end
  (necessary because Docker Desktop on macOS' bind-mount sync drops
  short-lived container writes).
- **Aggregator**: `analyze.py` reads each CSV's final cumulative
  row, computes achieved CPS, success rate, INVITE→200 OK latency
  (`ResponseTime1`), and retransmissions.

To reproduce:

```sh
# 1. Build the sipp sidecar.
docker build -t local-sipp /tmp/sipp_runner  # Dockerfile in this tree

# 2. Build rvoip listener.
cargo build --release -p rvoip-sip --example perf_listener

# 3. Asterisk sweep (assumes `~/Developer/asterisk/` compose is up).
RVOIP_PERF_RESULTS=/tmp/perf_results \
    ./run_comparison_dockerized.sh rvoip-asterisk 5060 asterisk

# 4. rvoip sweep (assumes host port 5060 is free; Asterisk stopped).
/path/to/target/release/examples/perf_listener 35060 &
RVOIP_PERF_RESULTS=/tmp/perf_results \
    ./run_comparison_dockerized.sh host.docker.internal 35060 rvoip

# 5. Aggregate.
python3 ./analyze.py /tmp/perf_results
```

## Findings vs NEXT_STEPS.md Area B

The doc's claim — "rvoip hangs at 100 CPS over sipp even though the
internal benchmarks sustain 100+ CPS at 100 % ASR" — is reproduced
exactly here. The listener accepts ~550 calls at 100 CPS, then stalls.
At 300 CPS it accepts 0. This is the in-process vs sipp-driven
serialization point Area B is meant to fix. The diagnose step (samply
flamegraph) is the next action; the bug is now cleanly reproducible
in a dockerized rig that anyone can re-run.

## Summary table

| Target | Target CPS | Total calls | Success | Success % | Achieved CPS | Avg RTT (ms) | Retrans |
|---|---|---|---|---|---|---|---|
| asterisk | 30 | 450 | 450 | 100.0% | 30.0 | 2.0 | 0 |
| asterisk | 100 | 1500 | 1500 | 100.0% | 100.0 | 1.0 | 0 |
| asterisk | 300 | 4500 | 4500 | 100.0% | 300.0 | 1.0 | 0 |
| rvoip | 30 | 450 | 450 | 100.0% | 30.0 | 2.0 | 0 |
| rvoip | 100 | 851 | 551 | 64.7% | 12.2 | 1.0 | 0 |
| rvoip | 300 | 900 | 0 | 0.0% | 0.0 | 0.0 | 0 |

## asterisk

### asterisk @ 30 CPS

- Elapsed: **15.0 s**
- Calls: 450 success / 0 failed / 450 total (100.0% success)
- Achieved CPS: **30.0** (target 30)
- INVITE→200 OK latency: avg **2.0 ms** (σ 0.0 ms)
- Call length: 106.0 ms
- Retransmissions: 0

### asterisk @ 100 CPS

- Elapsed: **15.0 s**
- Calls: 1500 success / 0 failed / 1500 total (100.0% success)
- Achieved CPS: **100.0** (target 100)
- INVITE→200 OK latency: avg **1.0 ms** (σ 0.0 ms)
- Call length: 105.0 ms
- Retransmissions: 0

### asterisk @ 300 CPS

- Elapsed: **15.0 s**
- Calls: 4500 success / 0 failed / 4500 total (100.0% success)
- Achieved CPS: **300.0** (target 300)
- INVITE→200 OK latency: avg **1.0 ms** (σ 0.0 ms)
- Call length: 104.0 ms
- Retransmissions: 0

## rvoip

### rvoip @ 30 CPS

- Elapsed: **15.0 s**
- Calls: 450 success / 0 failed / 450 total (100.0% success)
- Achieved CPS: **30.0** (target 30)
- INVITE→200 OK latency: avg **2.0 ms** (σ 0.0 ms)
- Call length: 106.0 ms
- Retransmissions: 0

### rvoip @ 100 CPS

- Elapsed: **45.0 s**
- Calls: 551 success / 0 failed / 851 total (64.7% success)
- Achieved CPS: **12.2** (target 100)
- INVITE→200 OK latency: avg **1.0 ms** (σ 0.0 ms)
- Call length: 106.0 ms
- Retransmissions: 0
- ⚠️ 300 calls still in-flight at sweep end (SUT stalled mid-run)

### rvoip @ 300 CPS

- Elapsed: **45.0 s**
- Calls: 0 success / 0 failed / 900 total (0.0% success)
- Achieved CPS: **0.0** (target 300)
- INVITE→200 OK latency: avg **0.0 ms** (σ 0.0 ms)
- Call length: 0.0 ms
- Retransmissions: 0
- ⚠️ 900 calls still in-flight at sweep end (SUT stalled mid-run)
