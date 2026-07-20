# Benchmarking `rvoip-sip`

> **Philosophy.** rvoip-sip benchmarks prioritise **deterministic
> low-latency behaviour under sustained realtime workloads** rather
> than peak synthetic throughput. The methodology surfaces the
> *operating-point* curve (sweep tables + the 80%-of-knee headline),
> **tail latency** (p99 / p99.9 from hdrhistogram), and **resource
> efficiency** (memory and CPU per call, RSS growth rate as the
> "no-leak" indicator) — the metrics carrier-grade and AI-voice
> engineers actually operate against.

This document describes the publishable-numbers benchmark suite under
`crates/sip/rvoip-sip/tests/perf/`. It is the companion to
[`PROFILING.md`](PROFILING.md): that one tells you how to *find* a
performance problem with samply / dhat / criterion; this one tells you
how to produce **absolute numbers** a VoIP engineer can cite when
evaluating the library's maturity (calls per second, concurrent active
calls, p99 setup latency, REGs/sec, memory per call).
For workload-specific `Config` recipes and SIPp matrix knobs, start with
[`TUNING.md`](TUNING.md). For carrier burst stress symptoms, knob choices, and
tradeoffs, use the
[`Stress Tuning Decision Guide`](TUNING.md#stress-tuning-decision-guide).

The numbers are produced by integration tests gated behind the
Cargo feature `perf-tests`. Default `cargo build` / `cargo test` are
untouched — turn the suite on explicitly only when you want to measure.
`perf-tests` is the clean measurement harness and does not compile media,
RTP, or memory diagnostic instrumentation into hot paths.

```bash
cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_call_setup_cps -- --nocapture
```

For the fixed in-process 2,000-CPS beta control and its matching CPU, timing,
and memory captures, use
`scripts/perf_call_setup_2k_profile.sh <clean|cpu|timing|memory>`. The script
resolves the exact executable from Cargo JSON, fixes the canonical runtime
profile, isolates each run's report, and writes a provenance manifest. `clean`
first conditions the same booted peers at `30,100,300,1000` CPS, then measures
the 2,000-CPS point. This reproduces the reviewed beta sweep's allocator and
retention state instead of comparing a cold process with a warmed fifth point.
It samples for 95 seconds after the measured calls drain, without endpoint
diagnostic scans in the active window. `clean` also verifies the exact
workload/configuration, runs
`scripts/perf_2k_acceptance.py` against the absolute beta limits, and gates
`perf_audit.py --fail-on-regression` against the reviewed `20260706T181609Z`
baseline. Only a `PASS` clean manifest is acceptance evidence; see
[`PROFILING.md`](PROFILING.md#canonical-2000-cps-reproduction).

Enable diagnostic features only for targeted investigation runs:

| Feature | Adds |
| --- | --- |
| `perf-infra-memory-diagnostics` | `rvoip-infra-common` memory registry and SIP-side memory hooks |
| `perf-media-diagnostics` | `rvoip-media-core` media setup/audio-quality counters |
| `perf-media-memory-diagnostics` | `rvoip-media-core` memory tracking |
| `perf-rtp-memory-diagnostics` | `rvoip-rtp-core` memory tracking |

For example, a media timing investigation can use
`--features perf-tests,perf-media-diagnostics`; an RSS-retention
investigation that needs SIP/infra memory samples can use
`--features perf-tests,perf-infra-memory-diagnostics` plus
`RVOIP_PERF_MEMORY_DIAGNOSTICS=1`. Do not use diagnostic feature sets for
headline beta performance comparisons.

Each test prints a stdout summary table and writes a machine-readable
JSON file to `target/perf-results/<scenario>.json` matching the schema
in [§4](#4-output-format-and-schema). Two JSON files can be diff'd
across commits to track regressions.

All generated benchmark artifacts live under the workspace root
`target/perf-results/`. `crates/target/` is not a valid output location.

## Clean Beta Comparison: 2026-06-15

The 2026-06-15 clean beta run is packaged at
`beta-report/20260615T105337Z`. It used the clean `perf-tests` feature set,
`BETA_SIPP_DIAGNOSTICS=0`, `RVOIP_PERF_MEMORY_DIAGNOSTICS=0`, and
`RVOIP_PERF_ALLOCATOR_DIAGNOSTICS=0`. No media/RTP/infra diagnostic features,
RTP/audio pacing default, or shared RTP TX scheduler default were enabled.

Use `20260612T211608Z` as the comparison baseline for this RCA, not the
diagnostic-contaminated `20260615T030513Z` run.

| Standard beta gate | 20260612 baseline | 20260615 clean fix run | Read |
| --- | --- | --- | --- |
| Full beta result | `0` failures, `0` skips | `0` failures, `0` skips | Pass |
| `pbx-media-server` @ 1000 CPS | p99 `12.4 ms`, ASR `1.0000`, no knee | p99 `13.0 ms`, ASR `1.0000`, no knee | Recovered |
| `pbx-media-server` headline | `1857.1` achieved CPS @ target `2000`, p99 `12.7 ms` | `1857.1` achieved CPS @ target `2000`, p99 `12.8 ms` | Equivalent |
| `signaling-only-server-high-performance` headline | `1857.1` achieved CPS, p99 `12.6 ms` | `1857.1` achieved CPS, p99 `12.9 ms` | Equivalent |
| `perf_backpressure_step` | max setup `113.6 ms`; p99 phases `13.2/12.7/13.5 ms` | max setup `23.8 ms`; p99 phases `14.1/12.9/14.1 ms` | 100 ms tail removed |
| `perf_soak_candidate` caller | p99 `49.9 ms`, max `114.9 ms`, CPU `60.5%` | p99 `44.1 ms`, max `95.1 ms`, CPU `57.2%` | Improved |
| `perf_soak_candidate` receiver | CPU `41.4%`, post-drain RSS gate `1.00 MB/hr` | CPU `38.8%`, post-drain RSS gate `0.10 MB/hr` | Improved |
| Artifact root | older logs referenced `crates/target/perf-results` | report packaged from `target/perf-results`; `crates/target/` absent | Fixed |

The root cause for the 100 ms setup tail was a fixed post-initiation wait in
`TransactionManager::send_request` for successful INVITE client sends. The
fix removes that wait for `InviteClient` transactions and keeps the old
non-INVITE safety window until the pending-options/auth-retry lifecycle is
cleaned up.

The standard beta run had two single-run p99 values higher than the baseline:
`perf_rtp_steady_state` (`28.7 ms` vs `17.5 ms`) and
`perf_concurrent_active_calls` (`55.1 ms` vs `51.6 ms`). Follow-up repeat checks
did not reproduce those as persistent regressions. RTP steady-state repeated at
`16.4`, `23.2`, `19.8`, `20.4`, and `16.8 ms`; sequential concurrent-call
repeats passed at `45.7`, `47.6`, and `47.8 ms`.

> **`--release` is mandatory.** Every scenario asserts
> `!cfg!(debug_assertions)` at startup. Debug-build numbers are not
> citable: dialog and transaction state machines run 5–20× slower
> without optimisation.

---

## Table of contents

1. [What each scenario measures](#1-what-each-scenario-measures)
2. [How to run](#2-how-to-run)
3. [How to interpret the metrics](#3-how-to-interpret-the-metrics)
   - 3.5 [Reading a sweep table & the knee point](#35-reading-a-sweep-table--the-knee-point)
4. [Output format and schema](#4-output-format-and-schema)
5. [Limitations](#5-limitations)
6. [Hardware spec template](#6-hardware-spec-template)
   - 6.5 [Industry calibration targets](#65-industry-calibration-targets)
7. [Results template (publication-ready)](#7-results-template-publication-ready)
8. [CI integration sketch](#8-ci-integration-sketch)
9. [Metric glossary (ASR / NER / PDD / RSR / MOS / TPS)](#9-metric-glossary)

---

## 1. What each scenario measures

Eight scenarios cover the headline questions a VoIP engineer asks when
sizing up a SIP library. The first three are **signalling-only** and
ship enabled today; scenarios 4–8 cover the media plane, security
overheads, and long-running soak — they live as stubs in the same
suite, scheduled for follow-up.

| # | Scenario | Headline metric | Why it matters |
| --- | --- | --- | --- |
| 1 | **`perf_call_setup_cps`** | Sustained calls/sec at ≥99% success | The single most-cited VoIP signalling benchmark. Mirrors the SIPp `-rate` workload. |
| 2 | **`perf_concurrent_active_calls`** | Max concurrent + RSS MB/call | Tells you how many calls one process can hold. Sets the ceiling for B2BUA / SBC deployments. |
| 3 | **`perf_registration_throughput`** | REGs/sec at ≥99% success | The headline for any deployment fronting registrations: PBX, carrier, OTT. |
| 4 | `perf_mid_call_signal_under_media` *(stub)* | Mid-call op latency p99 with live RTP | Re-INVITE / hold / DTMF latency while audio is flowing — covers the hold / transfer / IVR path. |
| 5 | `perf_rtp_steady_state` *(stub)* | Max concurrent calls with <0.1% RTP loss | The media-plane equivalent of #2: how many simultaneous calls can carry G.711 cleanly. |
| 6 | `perf_tls_overhead` *(stub)* | Δ CPS vs #1, Δ p99 setup vs #1 | Quantifies SIPS/TLS overhead so deployments can size the cost. |
| 7 | `perf_srtp_overhead` *(stub)* | Δ max-concurrent vs #5 | Quantifies SRTP overhead (AES-CM-128 / HMAC-SHA1-80) on the media plane. |
| 8 | `perf_soak_30min` *(stub, `#[ignore]`)* | Latency drift, RSS growth/hr | Catches slow leaks and steady-state degradation invisible in <1 min runs. |
| 9 | **`perf_burst_matrix`** | Burst ASR, setup p99, recovery, RSS slope | Carrier-style media burst evidence with split caller/receiver processes and short-to-long random call holds. |

Each scenario reads sizing knobs from `RVOIP_PERF_*` environment
variables. Defaults are tuned so a 4-core CI runner completes inside
10 min; publication-grade numbers come from cranking the dials up on
the reference hardware listed below.

---

## 2. How to run

### 2.1 Prereqs

- Rust toolchain pinned by the workspace (`rustc --version` — both the run-host
  rustc and the version reported in the JSON output should match).
- `--release` builds — debug builds will panic at startup.
- An idle host. CPU steal in a VM, an active Spotlight reindex, a noisy
  Docker daemon, or another perf run on the same machine will skew p99 numbers.

### 2.2 Quick-start commands

Signalling scenarios:

```bash
# Scenario 1 — sustained CPS
cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_call_setup_cps -- --nocapture

# Scenario 2 — concurrent active calls
cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_concurrent_active_calls -- --nocapture

# Scenario 3 — REGISTER throughput
cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_registration_throughput -- --nocapture
```

**Concurrency sweep** (the publication-shape Kamailio / OpenSIPS / SBC
vendors all use — runs the same scenario across a vector of operating
points and emits an aggregated table):

```bash
# Sweep CPS across 5 points; per-point JSONs + _sweep.{json,md} land
# under target/perf-results/perf_call_setup_cps/.
RVOIP_PERF_SWEEP_CPS=10,50,100,500,1000 \
  cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_call_setup_cps -- --nocapture

# Sweep concurrent-call ceiling
RVOIP_PERF_SWEEP_CONCURRENT=100,500,1000,5000 \
  cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_concurrent_active_calls -- --nocapture

# Sweep REGs/sec
RVOIP_PERF_SWEEP_REG_RPS=10,50,100,500 \
  cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_registration_throughput -- --nocapture
```

See §3.5 ("Reading a sweep table & the knee point") for interpretation.

Media/security/soak scenarios (currently stubs):

```bash
cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_mid_call_signal_under_media -- --nocapture
cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_rtp_steady_state -- --nocapture
cargo test -p rvoip-sip --features perf-tests,dev-insecure-tls --release \
    --test perf_tls_overhead -- --nocapture
cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_srtp_overhead -- --nocapture
# Scenario 8 is #[ignore] — requires --ignored to run the 30-min soak
cargo test -p rvoip-sip --features perf-tests --release \
    --test perf_soak_30min -- --ignored --nocapture

# Carrier media burst matrix — split caller/receiver processes
RVOIP_PERF_BURST_SCENARIOS=carrier-smoke \
  crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh
```

The burst smoke keeps beta perf time bounded. It records RSS slope in the
caller and receiver reports, but the RSS growth gate is enforced only when the
post-drain window reaches the scenario's `minRssGateWindowSecs`.
Burst reports also include the effective performance Config, SIP UDP
diagnostics, SIP dialog/transaction timing, server admission diagnostics,
media setup diagnostics, caller-side UAC INVITE 2xx/ACK counters, SIP UDP
per-Call-ID first/last wire timestamps, and dialog `call_timing_traces` when
the profile enables those diagnostics.

For `access-edge-microburst`, 2026-06-09 admission pacing follow-up runs with
server `softLimit=4500` did not produce a promotable recipe. `delay=1 ms`
improved ASR only to `0.9862`, and `delay=2 ms` regressed to `0.9826` while
failing the caller RSS gate. Use these artifacts as evidence for library-side
receive/admission pacing diagnostics rather than more static Config tuning.

Later 2026-06-09 isolation runs narrowed that conclusion. Bounded SIP UDP
receive draining improved ASR only from `0.9896` to `0.9916`, so it is useful
but not a root fix. The same burst shape with signaling-only media had setup
p95 near `13 ms` and no answer timeouts. Full media allocation with generated
RTP disabled via diagnostic-only `RVOIP_PERF_BURST_SKIP_AUDIO_SOURCE=1` also
kept setup p95 under `14 ms`, delivered all INVITE/2xx/ACK legs, and drained
receiver media sessions cleanly. Treat generated RTP traffic and control-plane
scheduling fairness as the next library investigation before promoting any
static access-edge Config recipe.

The first RTP scheduling experiment spread generated-audio transmitter start
phases over the 20 ms packet interval and set missed ticks to `Skip`. It
improved the clean full-RTP run to ASR `0.9932` with `50` answer timeouts, but
caller SIP receive-loop gaps still reached p95 `1 s`, p99/p999 `5 s`, and max
`36.08 s`. Do not cite that as a passing profile. A later cached-tone/payload
copy experiment regressed the same clean run to ASR `0.9886` with `84` answer
timeouts and was rejected; receiver cleanup, receiver media receive, and host
full-socket-buffer drops remained clean.

Audio TX pacing is the first full-media library candidate to pass the
`access-edge-microburst` gates. With `RVOIP_MEDIA_AUDIO_TX_PACING=1` and target
active `3000`, three repeat runs completed `7400/7400` calls with ASR `1.0000`,
zero answer timeouts, zero media setup failures, zero teardown failures, zero
retained objects, zero receiver active audio receivers, and RSS gates below
`10 MB/hr`. A lighter target active `4000` also passed once, but had a worse
setup tail (p95 `3.63 s`, p99 `8.20 s`) and higher host `no socket` drops than
target `3000`, so target `3000` remains the current candidate. A follow-up
shared generated-audio TX scheduler probe did not supersede it: shared-only
regressed to ASR `0.9866`, and shared plus target-`3000` pacing passed three
guarded runs but still lacks a clear CPU or tail-latency advantage over the
simpler pacing-only candidate.

When the burst matrix runs with SIP UDP diagnostics, the scenario directory also
captures host UDP snapshots as `host_udp_before.txt`, `host_udp_after.txt`, and
`host_udp_delta.txt`. Use those with the caller/receiver `results.sip_udp`
blocks before attributing answer timeouts to Rust queue sizing.

### 2.3 Sizing knobs

| Env var | Scenarios | Default | Notes |
| --- | --- | --- | --- |
| `RVOIP_PERF_SWEEP_CPS` | 1 | unset | Comma-separated CPS list (e.g. `10,50,100,500`). When set, the scenario sweeps the points and emits the aggregated `_sweep.{json,md}`. |
| `RVOIP_PERF_SWEEP_CONCURRENT` | 2 | unset | Same shape, sweeps concurrent-call ceiling. |
| `RVOIP_PERF_SWEEP_REG_RPS` | 3 | unset | Same shape, sweeps REGs/sec. |
| `RVOIP_PERF_TARGET_CPS` | 1, 3 | 100 | Single-point default if no sweep var is set. |
| `RVOIP_PERF_RAMP_SECS` | 1, 3 | 5 | Ramp from 0 → target CPS — applies to every sweep point too. |
| `RVOIP_PERF_STEADY_SECS` | 1, 3 | 30 | Steady-state window. Publish only over a representative window — 5 s runs are dominated by ramp / cooldown noise. |
| `RVOIP_PERF_COOLDOWN_SECS` | 1, 3 | 5 | Drain window before snapshotting. |
| `RVOIP_PERF_POST_DRAIN_SAMPLE_SECS` | 1 | 0 | Continue resource sampling after the final sweep point drains. The canonical clean control fixes this at 95 seconds; ordinary sweeps leave it off. |
| `RVOIP_PERF_CALL_TIMEOUT_SECS` | 1, 2 | 15 / 30 | Per-call deadline. Distinguishes "slow" from "stuck". |
| `RVOIP_PERF_CONCURRENT_TARGET` | 2 | 500 | Single-point default for concurrent-call ceiling. |
| `RVOIP_PERF_HOLD_SECS` | 2 | 10 | Steady-state hold before teardown. |
| `RVOIP_PERF_REG_TIMEOUT_SECS` | 3 | 10 | Per-REGISTER deadline. |

For publishable runs, override the defaults to size the workload to the
target machine: 4-core CI takes ~100 CPS comfortably; an M3 Max or
modern Xeon can hold several hundred CPS sustained.

---

## 3. How to interpret the metrics

### 3.1 Success and failure (ASR / RSR)

A call **succeeds** in scenario 1/2 when the full lifecycle completes:
INVITE dispatched → `Event::CallAnswered` observed → BYE dispatched →
`Event::CallEnded` observed. Any other terminal state is a failure.
The ratio is emitted as **`asr`** in the JSON `results` block — ITU
E.411's **Answer-Seizure Ratio**. **`ner`** (Network Efficiency Ratio)
is currently equal to `asr`; once we ship a user-driven-rejection
scenario the two diverge (see §9 glossary). The error breakdown
distinguishes:

- `invite_send_failed`: the local `invite().send()` returned `Err` (usually a
  transport-bind failure or session-conflict).
- `answer_failed`: a `CallFailed` event arrived before `CallAnswered` (4xx/5xx).
- `bye_failed`: BYE round-trip failed.
- `timeout`: the per-call deadline elapsed; the call may still complete after
  the snapshot but it's not counted as a success.

For split-process burst runs, interpret answer timeouts with both caller and
receiver diagnostics. If the caller reports UAC INVITE 2xx responses and ACK
sends but the receiver ACK count is much lower, first collect host UDP drop
counters and per-method inbound socket/source counters before changing server
queue sizes. Rust-level `sip_udp` backpressure counters at zero mean the loss or
delay is below the current app queues, or in routing/matching that needs
method-level evidence. A useful diagnostic join is caller failure JSONL
Call-IDs against caller and receiver `results.sip_udp.call_traces` plus
`results.sip_dialog_timing.call_timing_traces`, normalized for the optional
`@host` suffix on wire Call-IDs.

A REGISTER **succeeds** when `Event::RegistrationSuccess` arrives
before the per-REGISTER timeout. The ratio is emitted as **`rsr`** —
Register-Success Ratio, the REG-side analogue of ASR; same name
Kamailio uses in its OpenSER performance-test docs.

### 3.2 Latency definitions

All histograms record nanoseconds and report the canonical p50 / p95 /
p99 / p99.9 / max plus mean + min and the sample count.

| Histogram | t = 0 | t = end | Notes |
| --- | --- | --- | --- |
| `setup_latency` (scenarios 1, 2) | Just before `coord.invite(...).send().await` | `SessionHandle::wait_for_answered` returns `Ok` | INVITE → 200 OK round-trip from the API caller's perspective. |
| `full_cycle` (scenario 1) | Same as `setup_latency` | `SessionHandle::hangup_and_wait` returns `Ok` (CallEnded observed) | INVITE → 200 → ACK → BYE → 200 from the API caller's perspective. |
| `teardown_latency` (scenario 2) | Just before each task's `hangup_and_wait` | Same, after `Ok` | Isolated BYE round-trip without the call-setup leg. |
| `register_latency` (scenario 3) | Just before `coord.register(...).send().await` | First `Event::RegistrationSuccess` observed | One full client-side REGISTER round-trip. Mock registrar replies 200 OK with no challenge. |

**post-dial delay (PDD)** as classically defined is INVITE → first
provisional (180 Ringing). The current `AutoAccept` handler answers
without a provisional, so PDD is undefined; it will become a distinct
histogram once a Ringing-handler is added. `setup_latency` is the
INVITE → 200 OK time and is the right number to publish today.

### 3.3 Throughput

`achieved_cps` = `calls_succeeded / active_wall_secs`. The active
window covers ramp + steady (not cooldown — cooldown is a drain
window, not new offered load). Undershoot at low CPS (e.g. 78% of
target at 30 CPS) is normal — the pacer's sub-millisecond granularity
dominates per-tick error.

`rss_mb_per_call` is `(peak_rss - baseline_rss) / achieved_concurrent`.
This is **per-process** memory, not per-call working-set in a
production-shaped deployment. Treat it as an upper bound: a real
deployment running rvoip-sip alongside other workloads will share
allocator overhead, OS page caches, and dynamic libs.

Resource reports distinguish the configured request from what was actually
observed. `rss_tail_window_requested_secs` is the requested tail length;
`rss_tail_window_secs`, `rss_tail_sample_count`, and
`rss_tail_window_complete` describe actual coverage. A 35-second run therefore
cannot silently label its all-run slope as a 60-second tail. Named windows make
the same distinction: `requested_coverage_secs` comes from configuration, while
`actual_coverage_secs` comes only from the first and last observed samples.
Scheduler overshoot at sampler shutdown never changes the configured request.

The fixed beta threshold of `2378.44 MB/min` applies to
`rss_active_growth_mb_per_min`, selected from `point_start` through
`calls_drained`. That is the window represented by the reviewed historical
sweep. It is not an idle leak threshold. The 95-second cleanup window still
reports `rss_cleanup_growth_mb_per_hour`, but that hour-projected least-squares
slope is diagnostic: on a multi-gigabyte process, roughly one megabyte of
allocator or RSS sampling movement can appear as tens of MB/hour.

The short cleanup gate instead uses
`rss_cleanup_endpoint_growth_mb_per_hour`. It first calculates
`rss_cleanup_retained_growth_mb`, the signed difference between median RSS in
the first and last sixth of the observed window (each endpoint band is capped
at 15 seconds), then divides that delta by the actual separation between the
two bands' median sample timestamps. This preserves the 10 MB/hour limit
without treating the unobserved outer edges of the 95-second window as growth
time and without an additive allowance.

A compact structural snapshot is taken only after resource sampling stops and
must independently report zero retained call structures. The 30-minute
monolithic and one-hour split soaks enforce the same unadjusted 10 MB/hour slope
and remain authoritative for sustained-growth claims.

`diagnostics.measurement_identity` records the ordered conditioning points,
their offered/succeeded calls, shared-peer lifetime, resource phase names, and
sampling cadence. `perf_audit.py` refuses the scalar comparison as
`NON_COMPARABLE` when either conditioning or window identity differs. For the
reviewed pre-schema baseline only, the auditor reconstructs this identity from
its complete `_sweep.json` and per-point reports; it reports that inference and
the available sample-count evidence explicitly.

### 3.4 What the numbers don't tell you

- **Loopback excludes the NIC, the driver, and the kernel UDP queue.**
  Real-world deployments add 50–500 µs of network latency at minimum
  and lose ≥0.01% of packets on any production link. The methodology
  hands you the library cost; real-world cost is a strict superset.
- **One process measures library cost, not deployment cost.** A
  fully-loaded B2BUA holds many calls behind a load balancer with
  call-state replication, audit logging, and codec transcoding — none
  of which these benchmarks model.
- **A VM with steal time invalidates results.** Run on bare metal or
  dedicated cloud instances (AWS `c7i.metal`, Hetzner dedicated, etc.)
  for citable numbers.
- **The benchmarks measure the *current* `main` branch.** They are not
  a stable contract; numbers drift as we ship optimisations. The JSON
  output includes the `git_rev` so two runs can be compared exactly.

### 3.5 Reading a sweep table & the knee point

Every comparable product — Kamailio, OpenSIPS, AudioCodes Mediant
(Miercom), Asterisk dimensioning guides — reports a curve, not a
single point. Running a scenario with one of the `RVOIP_PERF_SWEEP_*`
env vars produces the matching curve as a markdown table at
`target/perf-results/<scenario>/_sweep.md` plus an aggregated JSON at
`_sweep.json` (per-point JSONs are also written alongside).

Example markdown output (from a real Apple M3 Max sweep, abbreviated):

```markdown
| CPS target | Achieved | ASR | Latency p50 | Latency p95 | Latency p99 | Full-cycle p99 | RSS Δ MB | Errors |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
|   10 |    9.8 | 1.0000 | 11.5 ms | 12.7 ms |  12.8 ms | 124.3 ms |  2.1 | 0 |
|   50 |   49.4 | 1.0000 | 11.8 ms | 13.3 ms |  13.4 ms | 124.6 ms | 12.0 | 0 |
|  100 |   99.0 | 0.9998 | 12.1 ms | 17.8 ms |  18.2 ms | 125.1 ms | 22.4 | 1 |
|  500 |  443.2 | 0.987  | 18.9 ms | 67.2 ms |  87.4 ms | 142.0 ms | 89.7 | 6 |
| 1000 |  612.7 | 0.812  | 31.5 ms | 380 ms  | 412   ms | 290   ms | 168  | 188 — **knee (ratio<0.95)** |
```

**Knee point** — the first sweep point where any of these fires:

1. The success ratio (`asr` or `rsr`) drops below **0.95**. This is
   the threshold ITU peering reports commonly cite as "below carrier
   grade". The runner labels the trigger `ratio<0.95`.
2. The setup-latency p99 grows to **>5×** the first-point baseline.
   Tail-latency blow-up at the same offered load tells you queues
   started backing up. Trigger label: `setup_p99>5×baseline`.
3. The error rate exceeds **0.5%** of attempted operations. Trigger
   label: `errors>0.5%`.

The sweep **does not stop** at the knee — it keeps running so the
markdown table shows the full degradation shape. This matches OpenSIPS
3.4's 14-test sweep methodology and what every commercial SBC vendor
publishes.

**Achieved vs target** — at low CPS the pacer's sub-millisecond
granularity dominates per-tick error; expect ~80% of target at offered
rates below 50 CPS and >95% at offered rates above 100 CPS. The
*achieved* number is what you cite; the *target* is just the offered
load.

**Citing a result** — pick the highest sweep point where ASR ≥ 0.99
and setup-p99 stays inside your SLA budget, and report that as the
sustained capacity. Example: "rvoip-sip 0.2.2 sustains 500 CPS at
98.7% ASR with setup p99 < 90 ms on Apple M3 Max." Always include the
hardware spec block from §6.

---

## 4. Output format and schema

Every scenario writes
`target/perf-results/<scenario>.json`. The canonical top-level keys
(`scenario`, `environment`, `load`, `results`, `latency_ns`,
`resources`) are asserted in `tests/perf/support/report.rs`; adding
extra keys is fine, removing a canonical one is a breaking change.

```json
{
  "scenario": "perf_call_setup_cps",
  "duration_secs": 41,
  "environment": {
    "rustc": "rustc 1.85.0 (...)",
    "os": "Darwin 25.2.0",
    "cpu_model": "Apple M3 Max",
    "cpu_count_physical": 12,
    "cpu_count_logical": 12,
    "total_ram_gb": 64.0,
    "build_profile": "release",
    "global_allocator": "mimalloc",
    "rvoip_sip_version": "0.2.2",
    "git_rev": "a9a3383c",
    "git_commit": "a9a3383c...",
    "git_dirty": false,
    "source_fingerprint_sha256": "...",
    "cargo_features": ["perf-tests"],
    "requested_cargo_features": "perf-tests"
  },
  "load": {
    "target_cps": 200.0,
    "ramp_secs": 5,
    "steady_secs": 30,
    "cooldown_secs": 5
  },
  "results": {
    "achieved_cps": 198.4,
    "asr": 0.9992,
    "ner": 0.9992,
    "calls_offered": 7000,
    "calls_succeeded": 6994,
    "errors": {
      "invite_send_failed": 0,
      "answer_failed": 2,
      "bye_failed": 0,
      "timeout": 4
    }
  },
  "latency_ns": {
    "setup_latency": {
      "count":  6994,
      "min":    8_900_000,
      "max":    47_000_000,
      "mean":   1.18e7,
      "p50":   11_800_000,
      "p95":   17_200_000,
      "p99":   24_800_000,
      "p99_9": 39_000_000
    },
    "full_cycle":     { "count": 6994, "min": ..., "p50": 124_400_000, ... }
  },
  "resources": {
    "peak_rss_mb": 132.4,
    "avg_cpu_pct": null
  }
}
```

Latency values are in **nanoseconds**. Consumers should convert as
needed for display. Resource fields are `null` when the scenario
doesn't populate them.

Call-setup reports additionally place the performance-recipe SHA-256, an
allowlisted runtime-switch snapshot, complete effective endpoint configuration,
and workload phase markers under `diagnostics`. These fields make a dirty-tree
profile reproducible without adding provenance to headline result keys.

### 4.1 New result keys (Phase 1.5)

Every scenario emits these on top of the headline number:

- `results.cps_per_core` / `dialogs_per_core` / `regs_per_core_per_sec`
  — per-physical-core normalisation. Carrier engineers compare these
  directly across products; matches the "calls/core" KPI rtpengine
  and OpenSIPS publish.
- `resources.baseline_rss_mb` — RSS at the start of the active phase.
- `resources.peak_rss_mb` — RSS at the highest sampled point.
- `resources.rss_growth_mb_per_min` — least-squares slope of RSS over
  time. Near zero on a healthy run; positive growth is the leak
  indicator that backs the "no leaks" Rust pitch.
- `resources.rss_tail_window_requested_secs` and
  `resources.rss_tail_window_secs` — configured versus actually observed tail
  coverage; completeness and sample count are reported alongside them.
- `resources.rss_windows.<name>` — phase names, requested/actual boundaries,
  sample count, completeness, slope, and robust endpoint medians for explicit
  windows. The canonical names are `active_load` and `post_drain_cleanup`.
- `resources.rss_active_growth_mb_per_min` and
  `resources.rss_cleanup_growth_mb_per_hour` — convenient scalar slope
  projections of those two named windows. The cleanup slope is diagnostic for
  the 95-second control.
- `resources.rss_cleanup_retained_growth_mb` — robust absolute cleanup delta;
  the nested window also records endpoint medians, representative timestamps,
  their separation, band duration, and sample counts.
- `resources.rss_cleanup_endpoint_growth_mb_per_hour` — the robust delta
  normalized by the representative timestamp separation and used by the
  10 MB/hour short-window gate.
- `resources.avg_cpu_pct` — process-level CPU% averaged across the
  steady window (excludes the first sysinfo sample, which is always 0).
- `resources.rss_samples_mb` — the raw `(t_secs, rss_mb, cpu_pct)`
  time series, suitable for plotting "RSS vs time".

The aggregated `_sweep.json` also gains a top-level `headline` block
(see §3.5):

```json
"headline": {
  "operating_point": 400,
  "achieved": 392.4,
  "ratio_label": "ASR",
  "ratio": 0.9997,
  "setup_p99_ns": 24_800_000,
  "framing": "sustained_at_80pct_of_knee"
}
```

### 4.2 Optional pcap artifact

Set `RVOIP_PERF_PCAP=1` before any scenario to also capture the
loopback wire to `target/perf-results/<scenario>.pcap`. Uses `tcpdump`
on `lo0` (macOS) / `lo` (Linux) — gracefully no-ops if `tcpdump` is
absent or unprivileged. The pcap is a Wireshark-loadable proof
artifact alongside the JSON metrics.

**Sweep mode** writes the same per-point JSON shape into
`target/perf-results/<scenario>/<point>.json` plus two aggregated
files alongside:

```text
target/perf-results/<scenario>/
├── 10.json              # per-point report
├── 50.json
├── 100.json
├── _sweep.json          # aggregated array + sweep_summary block
└── _sweep.md            # publication-ready markdown table
```

`_sweep.json` shape:

```json
{
  "scenario": "perf_call_setup_cps",
  "environment": { ... },
  "sweep_summary": {
    "points": [10, 50, 100, 500, 1000],
    "point_label": "CPS target",
    "knee_point": 1000,
    "knee_reason": "ratio<0.95"
  },
  "points": [ /* per-point reports in order */ ]
}
```

---

## 5. Limitations

The suite focuses on the rvoip-sip library running in isolation on
loopback. A non-exhaustive list of things it does **not** measure:

- **Packet loss / jitter / reordering on the network.** rtp-core has
  no in-tree impairment injector; if you want impaired-network numbers,
  drive a `tc qdisc netem` (Linux) or `pfctl` (macOS) rule against the
  loopback interface around the perf run.
- **NIC offload effects** (GRO, checksum offload, NAPI). All UDP
  traffic stays in-kernel on loopback.
- **TLS truststore performance.** The `dev-insecure-tls` feature used
  by scenario 6 skips server-cert validation — that's a measurement of
  the SIPS handshake itself, not of cert-chain processing.
- **SRTP key management.** Scenario 7 (when implemented) uses the
  SDES path (`a=crypto: inline:`) from `examples/regression/03_srtp/`,
  not DTLS-SRTP — see `crates/rvoip-rtp-core/benches/srtp_protect_unprotect.rs`
  for isolated SRTP crypto numbers.
- **Auth-challenged REGISTER cost.** Scenario 3 measures plain
  REGISTER → 200 OK. A 401 → digest-Authorization → 200 OK variant is
  the obvious follow-up.
- **Multi-process / multi-host deployment cost.** Everything runs in
  one process today.
- **Per-call CPU breakdown.** A full per-call CPU attribution requires
  `samply` / `cargo flamegraph` (see `PROFILING.md`). The perf suite
  reports process-level CPU only.

---

## 6. Hardware spec template & industry calibration

Paste this block alongside any published number. Without it, the
number can't be compared against anything.

```text
Host          : <model and CPU, e.g. AWS c7i.metal / Apple M3 Max>
CPU           : <model>; <physical>P / <logical>L cores; <base GHz> base, <turbo> turbo
RAM           : <GB>, <generation, e.g. DDR5-5600>
OS            : <kernel and version, e.g. Linux 6.6.0-25 amd64 / Darwin 25.2 arm64>
rustc         : <output of `rustc --version`>
mimalloc      : on / off (rvoip-infra-common::no-global-allocator feature)
isolation     : <none / dedicated host / VM with N CPUs pinned>
network       : N/A (loopback only)
rvoip-sip     : <crate version> @ <git rev>
duration      : <ramp + steady + cooldown of each scenario>
```

### 6.5 Industry calibration targets

What the comparable open-source and commercial products publish for
the same kinds of workloads. Use these as **calibration anchors** when
sizing your sweep — if rvoip-sip's curve sits in the same order of
magnitude on similar hardware, the result is plausible; if it's two
orders of magnitude off, suspect a measurement bug (e.g. forgot
`--release`, blocked on a debug-only assertion, single-threaded
runtime) before citing the number.

| Product (source) | Headline number | Methodology | What they varied |
| --- | --- | --- | --- |
| **Kamailio** ([OpenSER perf-tests](http://www.kamailio.org/docs/openser-performance-tests/)) | 15 000 stateful tx/sec; **8 060 complete calls/sec**; **7 600 new REG/sec**; 10 500 location lookups/sec | Two parallel UAC pairs via SIPp, AMD/Intel servers, Debian, UDP | Max concurrent 27–70; rate 7 k–10 k |
| **OpenSIPS 3.4** ([perf tests](https://www.opensips.org/About/PerformanceTests-3-4)) | 900 → **13 000 CPS** across 14 sweep scenarios; **2 500–3 000 CPS** per B2BUA instance | i7-7700 4C/8T 3.6 GHz, 16 GB; sustained 30 s mean call duration; F_MALLOC | 14-scenario complexity sweep, then 6 sustained-at-6k tests |
| **rtpengine** ([scaling architecture](https://www.ecosmob.com/blog/rtp-scaling-architecture-concurrent-media-streams/)) | **15 000 concurrent calls/host** in kernel mode; 30 000+ cluster (1&1); **3 M pps** at 30 k calls | G.711 50 pps/stream, MOS ≥ 4.3 quality threshold | Stream count and kernel-vs-userspace path |
| **FreeSWITCH** ([Real-world results](https://developer.signalwire.com/freeswitch/FreeSWITCH-Explained/Configuration/Performance-Testing-and-Configurations/Real-world-results_13173614/)) | Community-submitted: 1 → 247 concurrent / 1 → 20 CPS at the edge; **5 000+ concurrent** with tuning | Production deployments, B2BUA / IVR / conferencing | Hardware tier, codec, B2BUA mode |
| **Asterisk** (community / [VitalPBX](https://vitalpbx.com/blog/asterisk-pbx-multicore-4500-calls-test/)) | **1 600 concurrent / Xeon E5506**; up to 5 000 G.711 on bare metal; –25 % capacity with recording enabled | Concurrent-channel ramp until quality degrades | Hardware, codec (G.711 vs G.729), recording on/off |
| **AudioCodes Mediant 9000** ([Miercom report](https://www.scribd.com/document/890531115/Miercom-Repot-AudioCodes-Mediant-9000-and-Mediant-Virtual-Edition-VE-Session-Border-Controller-SBC)) | **500 CPS / 32 000 signalling sessions / 24 000 concurrent G.711 / 120 000 registered users** sustained | Vendor-published commercial SBC baseline | Sustained at the rated point |

What every comparable product **also** does that we plan in Phase 3:

- **Sustained long-duration calls** (OpenSIPS uses 30 s mean call
  duration as the default load shape; today's `perf_call_setup_cps`
  does back-to-back INVITE-BYE bursts which under-represents
  memory/table cost).
- **Registrar binding scale** separate from REG churn (Kamailio reports
  REGs/sec and location-lookups/sec as distinct KPIs).
- **Mixed workload** (carrier RFPs require numbers under realistic
  cross-load: ~70 % active calls + 20 % REG refresh + 10 % mid-call).
- **B2BUA forwarding throughput** (rvoip-sip-proxy ships the surface;
  vendors quote this separately from UA throughput).

---

## 7. Results template (publication-ready)

Pull the relevant numbers out of the JSON files and post a markdown
table like this in release notes, blog posts, or the project README:

```markdown
### Historical baseline: rvoip-sip 0.2.0 — measured on Apple M3 Max (12P / 12L, 64 GB)

| Workload | Throughput | p50 / p95 / p99 latency | Notes |
| --- | --- | --- | --- |
| Call setup (INVITE → 200 → ACK → BYE → 200) | 198 CPS @ 99.92% success | 11.8 / 17.2 / 24.8 ms (setup) · 124 / 128 / 134 ms (full cycle) | UDP loopback, no SDP processing on remote, AutoAccept handler |
| Concurrent active calls | 500 calls held @ 100% success | 16.7 / 20.6 / 20.9 ms (setup) · 115 / 116 / 116 ms (teardown) | 0.69 MB RSS / call |
| REGISTER throughput | 198 RPS @ 100% success | 114 / 116 / 117 ms | Plain REGISTER (no 401 challenge), mock UDP registrar |
```

Cite the hardware spec block from §6 alongside the table. Without it
the numbers are not comparable.

For a sweep result, paste the `_sweep.md` table verbatim — it already
includes the host block in the header. Example:

```markdown
### Historical baseline: rvoip-sip 0.2.0 — `perf_call_setup_cps` sweep on Apple M3 Max (16P / 16L, 128 GB)

| CPS target | Achieved | ASR | Latency p50 | Latency p95 | Latency p99 | Full-cycle p99 | RSS Δ MB | Errors |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
|   10 |   9.8 | 1.0000 | 11.5 ms | 12.7 ms |  12.8 ms | 124.3 ms |  2.1 | 0 |
|  100 |  99.0 | 0.9998 | 12.1 ms | 17.8 ms |  18.2 ms | 125.1 ms | 22.4 | 1 |
| 1000 | 612.7 | 0.812  | 31.5 ms | 380 ms  | 412   ms | 290   ms |  168 | 188 — **knee** |

Knee at CPS=1000 (ratio<0.95).
```

---

## 8. CI integration sketch

A tiny CI smoke job catches harness breaks on every PR without paying
for a full perf run:

```yaml
# .github/workflows/perf-smoke.yaml
name: perf smoke
on: [pull_request]

jobs:
  smoke:
    runs-on: ubuntu-latest
    timeout-minutes: 10
    env:
      RVOIP_PERF_TARGET_CPS: "20"
      RVOIP_PERF_RAMP_SECS:  "1"
      RVOIP_PERF_STEADY_SECS: "5"
      RVOIP_PERF_CALL_TIMEOUT_SECS: "10"
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: cargo build (perf-tests)
        run: cargo build -p rvoip-sip --features perf-tests --tests --release
      - name: perf scenario 1
        run: cargo test -p rvoip-sip --features perf-tests --release
              --test perf_call_setup_cps -- --nocapture
      - name: perf scenario 3
        run: cargo test -p rvoip-sip --features perf-tests --release
              --test perf_registration_throughput -- --nocapture
      - name: upload JSON
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: perf-results
          path: target/perf-results/*.json
```

For a tracked-over-time perf dashboard, ship the JSON artifacts to a
storage bucket and diff p99 latency between commits. A regression gate
like "fail PR if p99 setup latency on scenario 1 grows >25% vs `main`"
catches accidental slowdowns at PR time. The JSON schema is forward-
compatible: dashboards reading any keys other than the canonical
top-level ones must tolerate `null`s and absent keys.

For a quarterly **publishable-numbers** run, replace the CI smoke knobs
with a sweep so the artifact uploaded is the `_sweep.md` table itself:

```yaml
env:
  RVOIP_PERF_SWEEP_CPS:   "50,100,200,500,1000"
  RVOIP_PERF_STEADY_SECS: "60"
steps:
  - run: cargo test -p rvoip-sip --features perf-tests --release
          --test perf_call_setup_cps -- --nocapture
  - uses: actions/upload-artifact@v4
    with:
      name: perf-sweep
      path: target/perf-results/perf_call_setup_cps/
```

---

## 9. Metric glossary

The KPI vocabulary used in this document and emitted in the JSON
output. Standards cited where applicable.

| Term | Full name | Definition | Citation |
| --- | --- | --- | --- |
| **ASR** | Answer-Seizure Ratio | Successfully-answered calls ÷ attempted calls. Emitted as `results.asr`. | [ITU-T E.411](https://www.itu.int/rec/T-REC-E.411/) §6 |
| **NER** | Network Efficiency Ratio | Calls reaching a *network* terminal state (answered + busy + no-answer) ÷ attempted. Excludes user-side rejection from the denominator so the metric reflects network performance rather than user behaviour. For rvoip-sip's auto-accepting peers, NER == ASR until Phase 3's user-rejection scenarios land. Emitted as `results.ner`. | Proposed addition to ITU-T E.411 |
| **PDD** | Post-Dial Delay | INVITE sent → first 18x provisional received (typically 180 Ringing). Industry rule of thumb: <2 s excellent, <4–5 s acceptable. Currently undefined in our suite because the `AutoAccept` handler answers without a provisional; Phase 3's 180-first handler will populate `latency_ns.pdd`. | [ITU-T E.411](https://www.itu.int/rec/T-REC-E.411/); per-trunk SLA standard |
| **RSR** | Register-Success Ratio | REG-side analogue of ASR — registrations that hit `RegistrationSuccess` ÷ attempted. Emitted as `results.rsr`. | Used by Kamailio OpenSER performance docs |
| **MOS** | Mean Opinion Score | Perceived voice quality 1 (bad) – 5 (excellent). Properly measured via PESQ (P.862) or POLQA (P.863) against reference audio — not what our suite emits. Phase 2 will publish a `frame_loss_pct` proxy that correlates with R-factor / MOS but is not a substitute for it. | [ITU-T P.862](https://www.itu.int/rec/T-REC-P.862/) (PESQ); [P.863](https://www.itu.int/rec/T-REC-P.863/) (POLQA) |
| **CPS** | Calls Per Second | Offered or achieved call-setup rate. Emitted as `load.target_cps` and `results.achieved_cps`. | de facto standard across all VoIP load tools |
| **TPS** | Transactions Per Second | Stateless message-forwarding rate. **Not measured** by this suite — rvoip-sip is a UA stack, not a stateless proxy. | de facto |

**Setup latency** (`latency_ns.setup_latency`) is INVITE → 200 OK from
the API caller's perspective. It is **not** PDD: PDD stops at 180/183,
setup latency stops at 200. Both are useful; PDD is the user-perceived
"how long until it rings" number, setup latency is the
"how long until the call is fully up" number.

**Full-cycle latency** (`latency_ns.full_cycle`) is INVITE → CallEnded
(BYE round-trip complete). Captures the end-to-end signalling cost of
a transient call.

---

## See also

- [`PROFILING.md`](PROFILING.md) — flamegraph and dhat recipes for
  finding the root cause once a number regresses.
- `crates/sip/rvoip-sip/benches/` — criterion benches for *relative*
  comparisons (e.g. "did my optimisation help?").
- `crates/sip/rvoip-sip/examples/profiling/` — long-running profiling
  drivers for samply / dhat / tokio-console (stdout output, no
  JSON / percentiles).
