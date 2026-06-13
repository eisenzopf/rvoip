# Carrier Burst Tuning Plan

For the run-by-run experiment table, see
[`CARRIER_BURST_TUNING.md`](CARRIER_BURST_TUNING.md) and the machine-readable
CSV ledger [`CARRIER_BURST_TUNING_LEDGER.csv`](CARRIER_BURST_TUNING_LEDGER.csv).

## Purpose

This document tracks the current rvoip-sip carrier burst tuning work. The goal
is to prove that the SIP/media stack handles realistic bursty media traffic
without leaks, retained calls, runaway RSS growth, or unacceptable answer/setup
latency.

The immediate focus is `access-edge-microburst`, because it has shown receiver
cleanup is now healthy but caller-side answer timeouts still need tuning. The
same method applies to the broader burst matrix before any tuning profile is
promoted.

## Current Questions

We are testing these questions:

- Does the split caller/receiver burst harness accurately show which side is
  failing: caller, receiver, media, cleanup, or host capacity?
- Do the zero-copy RTP/media changes improve allocation behavior without
  harming burst latency?
- Did any added RTP/media buffer or pool add overhead that is not justified by
  better stability? If yes, remove it or keep it out of public tuning profiles.
- Can Config and YAML recipes expose all useful SIP/media tuning knobs so users
  can tune without patching code?
- Can `access-edge-microburst` reach the required acceptance gates through
  configuration tuning rather than hidden harness behavior?

## Test Harness

Carrier burst testing uses split processes:

- `tests/perf/perf_burst_receiver.rs` runs the answering side.
- `tests/perf/perf_burst_caller.rs` runs the calling side.
- `scripts/perf_burst_matrix.sh` launches both, applies scenario/profile YAML,
  samples per-process resources, and writes reports.

Scenarios live in `config/perf-burst-scenarios.yaml`. Performance recipes live
in `config/performance-recipes.yaml`.

Reports are written under:

```bash
target/perf-results/perf_burst_matrix/
```

Each run should preserve:

- caller JSON report
- receiver JSON report
- carrier-readable `_burst.md` summary
- effective caller/server config snapshots
- resource samples and diagnostic JSONL paths
- git revision and scenario seed

## Scenario Runtime

Wall time is the phase duration plus call hold drain time and the configured
post-drain retention wait. The table below lists the deterministic traffic
window from the scenario file and the likely extra hold tail.

| Scenario | Traffic window | Hold tail | Purpose |
| --- | ---: | ---: | --- |
| `carrier-smoke` | 12s | up to 5s | Required beta-gate harness smoke. |
| `access-edge-microburst` | 130s | default up to 360s | Repeated access-edge bursts over baseline traffic. |
| `contact-center-flash` | 135s | up to 360s | High-arrival flash event with mostly short calls. |
| `shift-change-long-hold` | 210s | up to 600s | Moderate burst with many long calls. |
| `overload-recovery` | 110s | default up to 360s | Intentional overload, controlled 503/recovery behavior. |
| `high-density-media-burst` | 210s | default up to 360s | Dense media burst stressing RTP/media queues. |
| `buffer-ab-legacy` | 80s | up to 15s | Controlled A/B for RTP/media buffer behavior. |

For quick tuning, run one scenario at a time. For promotion, repeat the final
candidate at least three times with the same shape.

## Metrics We Care About

Acceptance metrics:

- ASR meets scenario threshold, normally `>= 0.999`.
- NER stays high and failures are attributable.
- Media setup failures are `0`.
- Teardown failures are `0`.
- Caller and receiver retained objects after drain are `0`.
- Receiver active audio receivers after drain are `0`.
- Post-drain RSS slope is below the configured gate, currently `10 MB/hr`.

Latency and burst metrics:

- offered CPS, achieved CPS, accepted CPS
- setup latency p50, p95, p99, p99.9
- PDD when ringing is observed
- answer latency and answer timeout count
- teardown latency
- recovery time after each burst

Scaling metrics:

- peak active calls
- RSS per peak active call
- CPU per achieved CPS
- CPU per active call
- caller RSS slope and receiver RSS slope
- post-drain RSS behavior

Media and cleanup metrics:

- received audio frames
- completed audio receivers
- active audio receivers after drain
- RTP/media retained objects after drain
- session/dialog/media retained objects after drain

## Config And YAML Knob Coverage

The Config layer should expose every meaningful runtime tuning knob. The YAML
recipe layer should expose the same knobs where they are useful for repeatable
performance profiles.

Knob categories that must be available through Config and YAML recipes:

- SIP UDP socket buffers, parse workers, parse queue capacity, and parse
  dispatch mode.
- SIP transport channel capacity, dispatch workers, and dispatch queue
  capacity.
- Transaction event capacity, dispatch workers, dispatch queue capacity,
  ACK/BYE priority burst, command channel capacity, and INVITE 2xx retransmit
  pacing.
- Dialog event capacity, dispatch workers, and dispatch queue capacity.
- Session event dispatcher workers and queue capacity.
- Server capacity, admission hard limit, soft limit, pacing delay, and
  overload `Retry-After`.
- Media mode, media session capacity, media port capacity, and signaling-only
  RTP port.
- RTP session buffer config and RTP transport buffer config.
- Media session controller config.
- Active-call no-media and media-idle watchdogs for auto-answer server
  resiliency.
- Diagnostic toggles that are useful for test evidence, while keeping noisy
  diagnostics disabled in promoted profiles.

The burst reports must include the effective values for the knobs used in a
run. If a knob affects a tuning decision but is missing from the report, add it
before using that run as evidence.

## Current Access-Edge Investigation

Recent evidence shows:

- Receiver cleanup now reaches zero retained objects after drain.
- Receiver active audio receivers drain to zero.
- Receiver RSS tail is within the gate.
- Caller-side ASR still misses threshold because of answer timeouts.
- A broader client profile candidate made the result worse and was rejected.

The next tuning target is therefore answer/setup latency under burst pressure,
not receiver cleanup retention.

### 2026-06-09 Diagnostic Run

Artifact:
`target/perf-results/perf_burst_matrix/burst_20260609_045820_79749/ae-dialog-8q24000-diagnostics/`

The best config candidate from the staged sweep, dialog dispatch `8/24000`,
still failed acceptance when rerun with timing diagnostics enabled:

- caller ASR `0.9870` with `96` failures, all answer timeouts
- no invite-send failures, media setup failures, overload rejections, or final
  retained caller objects
- receiver observed `7387` incoming calls and drained to `0` retained objects,
  `0` active audio receivers, and `0` active transactions
- receiver RSS gate remained within limit at `2.28 MB/hr`; caller RSS gate was
  `3.81 MB/hr`
- receiver admission attempted/admitted `7387` calls, rejected `0`, paced `0`,
  and never reached the soft limit: observed sessions maxed at `4757` against a
  soft limit of `5400`
- server SIP timing was not the bottleneck: INVITE-to-200 averaged `242us`,
  UDP-receive-to-200 averaged `355us`, transaction dispatch p99 was `50us`,
  dialog-to-session queue p99 was `25us`, and no timing bucket crossed `500ms`
- media setup was not the bottleneck: media start averaged `120.89us`, max
  `2.252ms`; RTP session creation averaged `111.97us`, max `2.136ms`
- caller setup latency degraded only in `burst-2` and `recovery-2`: `burst-2`
  setup p95/p99 were `15.888s` / `23.673s`; `recovery-2` setup p95/p99 were
  `23.656s` / `27.699s`

Interpretation: do not promote a recipe from this sweep. The current
Config-level server queue, dialog dispatch, media setup, and admission knobs do
not explain the failures. Admission pacing is also not active in this shape
because the server never reaches the soft threshold.

### 2026-06-09 Client Transport Diagnostics

Artifacts:

- `target/perf-results/perf_burst_matrix/burst_20260609_055448_63531/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_061202_88306/ae-dialog-8q24000-retx512-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_062514_96216/ae-dialog-8q24000-shards32-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_080354_75732/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_090007_78489/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_094745_33369/ae-dialog-8q24000-admission4500-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_100256_53195/ae-dialog-8q24000-admission4500-delay2-client-diagnostics/`

The client-side diagnostic rerun added UAC 2xx/ACK counters and SIP UDP
diagnostics. It improved observability but did not find a promotable Config
combination:

| Candidate | Delta | Caller ASR | Timeout failures | Caller RSS gate | Receiver ACKs | Receiver proactive 2xx | Decision |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| `ae-dialog-8q24000-client-diagnostics` | baseline diagnostic | `0.9941` | `44` | `1.09 MB/hr` | `5440` | `47742` | Reject: ASR below `0.999` |
| `ae-dialog-8q24000-retx512-diagnostics` | cap `sipInvite2xxRetransmitMaxDuePerTick` at `512` | `0.9872` | `95` | `0.21 MB/hr` | `5411` | `48189` | Reject: ASR regressed |
| `ae-dialog-8q24000-shards32-diagnostics` | increase `aliceShards` from `16` to `32` | `0.9911` | `66` | `26.11 MB/hr` | `5408` | `48037` | Reject: ASR and RSS gate failed |
| `ae-dialog-8q24000-client-diagnostics` | per-method UDP + per-Call-ID wire trace + host UDP delta | `0.9949` | `38` | `1.71 MB/hr` | `5529` | `47338` | Reject: ASR below `0.999`; use as protocol evidence |
| `ae-dialog-8q24000-client-diagnostics` | timestamped SIP UDP and dialog per-Call-ID traces | `0.9836` | `121` | `5.97 MB/hr` | `5527` | `48878` | Reject: ASR regressed; use as timing evidence |
| `ae-dialog-8q24000-admission4500-client-diagnostics` | admission soft limit `4500`, delay `1 ms` | `0.9862` | `102` | `0.42 MB/hr` | `5499` | `49336` | Reject: ASR below `0.999`; static pacing helped only modestly |
| `ae-dialog-8q24000-admission4500-delay2-client-diagnostics` | admission soft limit `4500`, delay `2 ms` | `0.9826` | `129` | `17.22 MB/hr` | `5430` | `49766` | Reject: ASR regressed and caller RSS gate failed |

Common findings:

- all observed failures were answer timeouts; invite send, media setup,
  overload rejection, and teardown failure counts stayed at `0`
- receiver media setup stayed clean, with `0` media start failures and `0`
  active audio receivers after drain
- receiver retained objects drained to `0`
- receiver admission did not pace with the default `5400` soft limit; lowering
  the soft limit to `4500` triggered pacing but still did not meet ASR
- SIP UDP diagnostics reported `0` worker queue full events, `0` parse
  failures, `0` transport/manager backpressure events, and `0` outbound send
  errors on both sides
- the caller processed fewer INVITE 2xx responses than the receiver sent first
  200 OKs, but every processed UAC 2xx produced a successful local ACK send
- the receiver still received only about `5400` ACKs while the caller reported
  more than `7300` ACK sends

The `512` retransmit cap did not bind: receiver
`maintenance_capped_ticks` stayed `0`, and proactive 2xx retransmits remained
near the uncapped baseline. Increasing Alice shards did not improve the
receiver ACK receipt pattern and added caller RSS growth.

Conclusion: the current Config knobs are not sufficient to promote
`access-edge-microburst`. The next investigation should move below server queue
tuning into transport/protocol evidence: per-method inbound UDP counters by
local socket and source, sampled Call-ID tracing for INVITE 2xx / ACK / BYE,
host UDP drop counters around each run, and targeted tests for ACK/BYE delivery
versus UAS 2xx retransmit pressure. If those diagnostics confirm delivery loss
or unbounded burst pressure below Rust queues, consider library changes such as
adaptive/token-based transport pacing or more granular retransmit scheduling
with counters for paced, admitted, rejected, and wait-time distribution.

The completed wire-trace run at
`burst_20260609_080354_75732/ae-dialog-8q24000-client-diagnostics/` narrows the
failure further:

- caller offered `7400` calls, succeeded `7362`, and failed `38`, all answer
  timeouts
- receiver observed `7391` incoming calls, sent first 200 OK for all `7391`,
  and drained to `0` retained objects, `0` active audio receivers, `0` media
  setup failures, and `0.52 MB/hr` receiver RSS gate growth
- server admission admitted `7391`, rejected `0`, paced `0`, and peaked at
  `4722` observed sessions below the `5400` soft limit
- Rust UDP diagnostics still showed `0` parse failures, `0` worker queue full
  events, `0` send errors, and `0` transport/manager backpressure events
- host UDP counters showed `0` additional full-socket-buffer drops during the
  run; `dropped due to no socket` increased by `3,915,308`, likely dominated by
  media traffic to released RTP sockets and requiring packet-level attribution
  before it is treated as SIP loss
- caller dialog-core processed `7374` UAC INVITE 2xx responses and reported
  `7374` ACK attempts/successes and `7374` `CallAnswered` coordination emits,
  while receiver UDP observed only `5529` ACKs
- receiver had `1862` calls with no ACK observed; this is larger than the `38`
  failed calls, so missing ACKs are a protocol pressure signal even when the
  caller eventually treats many calls as successful
- joining the `38` failed Call-IDs classified them as:
  - `9` receiver never saw the INVITE despite `10-11` caller INVITE sends
  - `17` caller received INVITE 2xx retransmits but sent no ACK before timeout
  - `5` caller sent ACK but receiver did not observe it
  - `7` caller sent ACK and receiver observed it, but the caller wait still
    timed out

The timestamped rerun at
`burst_20260609_090007_78489/ae-dialog-8q24000-client-diagnostics/` confirmed
the failure is below Config-level server queue depth:

- caller offered `7400` calls, succeeded `7279`, and failed `121`, all answer
  timeouts; receiver still drained to `0` retained objects and `0` active audio
  receivers
- caller socket-buffer overrides were unset, while the receiver/server used the
  restored `8 MiB` SIP UDP recv/send buffers
- server admission admitted `7380`, rejected `0`, paced `0`, and peaked at
  `4699` observed sessions below the `5400` soft limit
- Rust UDP diagnostics on both sides still reported `0` parse failures, `0`
  worker queue full events, `0` send errors, and `0` transport/manager
  backpressure events; caller/receiver UDP handoff p99 stayed below `1 ms`
- host UDP full-socket-buffer drops increased by `0`; host `no socket` drops
  increased by `3,939,115`, still requiring SIP/RTP attribution before it is
  treated as SIP loss
- the caller dialog ACK fast path is not the bottleneck once a 2xx reaches the
  transaction path: successful calls had UAC 2xx-to-ACK p95 `0.060 ms`, and the
  failed calls that reached the UAC ACK path had ACK after 2xx p95 `0.422 ms`
- joining the `121` failed Call-IDs classified them as:
  - `65` caller UDP saw INVITE 2xx but no UAC ACK attempt happened; first
    caller 2xx arrived `32.2-51.6 s` after first outbound INVITE, after the
    answer-timeout boundary
  - `20` receiver never saw the INVITE despite caller retransmits
  - `20` caller sent ACK but receiver never observed it
  - `16` receiver observed ACK, but caller lifecycle `CallAnswered` was not
    published before timeout/cancel

The admission-pacing follow-up tried the lower soft limit suggested by the
timestamped run:

- `softLimit=4500`, `delay=1 ms` admitted `7389` calls, paced `729`, rejected
  `0`, and improved failures from `121` to `102`, but still missed ASR
  acceptance (`0.9862`)
- `softLimit=4500`, `delay=2 ms` admitted `7369` calls, paced `777`, rejected
  `0`, and regressed to `129` failures plus a caller RSS gate failure
  (`17.22 MB/hr`)
- both runs kept receiver media setup, receiver retained objects, receiver
  active audio receivers, Rust UDP parse/queue/send errors, and host full
  socket-buffer drops clean
- the `1 ms` failed Call-IDs classified as `62` caller-saw-2xx/no-ACK-attempt,
  `13` caller ACK not seen by receiver, `11` receiver-never-saw-INVITE, and
  `16` receiver-saw-ACK/post-timeout lifecycle cases
- the `2 ms` failed Call-IDs classified as `77` caller-saw-2xx/no-ACK-attempt,
  `10` caller ACK not seen by receiver, `31` receiver-never-saw-INVITE, and
  `11` receiver-saw-ACK/post-timeout lifecycle cases
- the dominant no-ACK-attempt bucket still saw first caller 2xx only after the
  answer-timeout boundary: `32.1-45.7 s` in the `1 ms` run and `32.2-55.4 s`
  in the `2 ms` run

Interpretation: the main remaining issue is not server cleanup,
Config-level server queue depth, or a simple static admission delay. It is late
datagram delivery into the caller/receiver SIP path and post-timeout
session-event delivery under burst pressure. Do not promote a recipe from the
current sweep. Move to library work: adaptive/token-based pacing where the
sender can avoid RTP/SIP microbursts, per-socket receive-loop gap diagnostics,
queue-delay histograms around transaction/dialog/session event delivery, and
SIP/RTP packet attribution for host UDP `no socket` drops.

### 2026-06-09 Library Isolation Runs

Artifacts:

- `target/perf-results/perf_burst_matrix/burst_20260609_103932_98071/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_110812_31701/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_112609_65435/ae-dialog-8q24000-sip-only-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_114444_80163/ae-dialog-8q24000-client-diagnostics/`

The first library change added per-socket UDP receive-loop diagnostics and a
bounded receive drain after each awaited UDP receive. The diagnostics showed
large caller-side gaps before parse/transaction processing:

- pre-drain baseline ASR `0.9896`, `77` answer timeouts; caller
  `udp_receive_loop_gap` p95 `1.0 s`, p99 bucket `5.0 s`, max `37.5 s`
- bounded-drain rerun ASR `0.9916`, `62` answer timeouts; caller
  `udp_receive_loop_gap` p95 `1.0 s`, p99 bucket `5.0 s`, max `36.3 s`
- both runs had `0` Rust UDP worker queue-full events, `0` parse failures,
  `0` transport/manager backpressure events, `0` UDP send errors, and `0`
  host full-socket-buffer drop delta

Bounded drain is worth keeping as a small transport improvement, but it is not
the root fix. The per-call hop join still showed SIP datagrams arriving only
after SIP retransmission intervals: in the bounded-drain run, caller
INVITE-to-receiver INVITE p99 was `19.6 s`, receiver 2xx-to-caller 2xx p99 was
`19.9 s`, and caller INVITE-to-caller 2xx p99 was `27.4 s`.

The next two isolation runs changed media behavior:

- SIP-only media (`mediaMode: signaling-only`) eliminated answer timeouts and
  collapsed caller INVITE-to-caller 2xx p95 to `2.643 ms`, p99 to `3.894 ms`,
  max to `20.333 ms`. The perf gate failed only because the full-media burst
  harness still attempts audio setup and expects audio receivers for a
  signaling-only run.
- Full media allocation with `RVOIP_PERF_BURST_SKIP_AUDIO_SOURCE=1` also
  eliminated answer timeouts: caller setup p95 was `13.7 ms`, p99 `15.1 ms`,
  max `116.7 ms`; all `7400` INVITEs, `7400` 2xx responses, and `7400` ACKs
  were observed by the peer. The run failed later with teardown/no-media-timeout
  side effects, not setup failures. Receiver media sessions allocated and
  drained cleanly, with `0` received RTP frames and `0` retained objects.

Conclusion: media allocation alone does not reproduce the access-edge setup
tail. Generated RTP traffic during full-media calls is the dominant multiplier
for SIP datagram loss/latency in this benchmark. The next library investigation
should focus on RTP send pacing/fairness, no-media/media-idle watchdog
interaction with long holds, and SIP-vs-RTP host UDP attribution. Static SIP
queue sizing and server admission knobs should not be promoted as the answer to
this failure mode.

### 2026-06-09 RTP Scheduling Investigation

Artifacts:

- `target/perf-results/perf_burst_matrix/burst_20260609_130757_68023/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_132911_93058/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_141737_51209/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_210431_30162/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_212634_64711/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_214141_90637/ae-dialog-8q24000-client-diagnostics/`
- `target/perf-results/perf_burst_matrix/burst_20260609_215701_11703/ae-dialog-8q24000-client-diagnostics/`

The first RTP library experiment spread audio transmitter start phases across
the 20 ms packet interval and changed missed ticks to `Skip` so synchronized
timer wakeups do not bunch every generated RTP source onto the same scheduler
tick. That improved the clean full-RTP run from the bounded-drain result
(`0.9916` ASR, `62` answer timeouts) to `0.9932` ASR with `50` answer
timeouts, but it still failed the `0.999` ASR gate. Caller SIP receive-loop
gaps remained severe: p95 `1 s`, p99/p999 bucket `5 s`, max `36.08 s`, and
`3554` gaps over `500 ms`.

Endpoint media setup diagnostics are useful for focused runs but are intrusive
for acceptance measurements. With per-packet audio TX timing enabled, ASR fell
to `0.9905` with `70` timeouts. The audio TX snapshot showed `7330`
transmitter tasks, average start phase near `9.99 ms`, average tick gap near
`20 ms`, no send failures, and average send time under `1 us`, but those
per-packet timing calls changed the workload enough that the run should be
treated only as diagnostic evidence.

A later cached-tone/payload-copy experiment was rejected. In the clean run at
`burst_20260609_141737_51209`, caller ASR regressed to `0.9886` with `84`
answer timeouts, setup p95 `12.20 s`, peak pending setups `731`, and caller
SIP receive-loop max gap `50.96 s`. Receiver health stayed clean:
`7372` incoming calls observed, `7372` completed audio receivers,
`24,751,451` received audio frames, `0` active audio receivers after drain,
`0` retained objects, and post-drain RSS gate `0.42 MB/hr`. Host UDP
full-socket-buffer drops again increased by `0`; `no socket` drops increased by
`3,928,474`.

The failed Call-ID join for the rejected cached-tone/payload-copy run was:

- `36` caller saw INVITE 2xx but no ACK attempt happened before timeout
- `28` receiver never saw the INVITE
- `9` caller sent ACK but receiver did not observe it
- `11` receiver observed ACK, but caller lifecycle did not complete before
  timeout/cancel

Receiver-side SIP timing was fast in that same run: transaction dispatch p95
`25 us`, dialog event dispatch p95 `10 us`, INVITE-to-200 p95 `1 ms`, and
UDP-receive-to-200 p95 `2.5 ms`. Caller-side SIP queues were also fast once a
packet was processed: existing transaction dispatch p95 `10 us` and
dialog-to-session p95 `250 us`. The evidence now points at scheduler fairness
before SIP UDP polling while thousands of generated RTP transmitter/session
tasks are active, not at Config-level SIP queue sizes.

The next isolated media-plane control was generated-audio TX pacing. With
`RVOIP_MEDIA_AUDIO_TX_PACING=1` and target active `3000`, three repeat runs
passed all acceptance gates: ASR `1.0000`, `7400/7400` calls, `0` answer
timeouts, `0` media setup failures, `0` teardown failures, `0` retained
objects on both sides after drain, `0` receiver active audio receivers after
drain, and RSS gates under `10 MB/hr`. The two runs with pacing counters
recorded skipped about `8.34M` generated-audio TX ticks, saw active TX max near
`4830`, and never needed a divisor above `2`. Host full-socket-buffer drops
remained `0`.

A lighter target active `4000` also passed one run, but it is weaker evidence:
setup p95 regressed to `3.63 s`, setup p99 to `8.20 s`, peak pending setups to
`338`, and host `no socket` drops rose to `3,021,435`. Keep it as a probe, not
as the current candidate.

The next isolated media-plane control was a shared generated-audio TX
scheduler. Shared-only scheduling regressed and failed: ASR `0.9866`,
`7301/7400` calls, `99` answer timeouts, setup p95 `11.93 s`, setup p99
`23.64 s`, and caller CPU `119.5%`. Shared scheduling plus target-`3000`
pacing nearly passed before a stop-race guard, but still had `1` teardown
failure and `1` shared send failure. After rechecking active state around send,
three guarded shared+pacing runs passed with ASR `1.0000`, `7400/7400` calls,
`0` answer timeouts, `0` media setup failures, `0` teardown failures, `0`
retained objects, `0` receiver active audio receivers after drain, and RSS
gates under `10 MB/hr`. The three setup p99 values were `5.27 s`, `4.24 s`,
and `7.67 s`; caller CPU averaged about `95-96%`; shared send failures stayed
at `0`; shared batch max ranged from `609` to `1341`. Keep it as an opt-in
candidate, but it does not yet displace the simpler three-pass pacing-only
candidate because the tail is less stable and the implementation is more
complex.

Current decision:

- Keep bounded SIP UDP receive draining and RTP transmitter phase spreading as
  partial library improvements.
- Keep audio TX timing counters as opt-in diagnostics only; do not enable
  `mediaSetupDiagnostics` for acceptance runs.
- Do not keep or promote the cached-tone/payload-copy experiment; it regressed
  the clean run and was reverted.
- Do not promote an access-edge burst recipe from this evidence.
- Keep audio TX pacing target active `3000` as the current opt-in library
  candidate; it passed three repeat runs.
- Do not promote target active `4000` from one run because its setup tail was
  materially worse than target `3000`.
- Keep shared generated-audio TX scheduling plus target-`3000` pacing as a
  secondary opt-in candidate. Shared-only failed, and shared plus pacing did
  not show a clear tail-latency or CPU advantage over pacing-only.
- Next library work should focus on adaptive media-plane pacing or another
  small CPU reduction in the generated RTP path before changing default
  behavior.

## Tuning Method

Use one controlled change at a time. Keep the scenario, seed, host, and build
mode fixed.

Initial sweeps for `access-edge-microburst`:

1. Transport dispatch workers and queue capacity.
2. Transaction dispatch workers and queue capacity.
3. Transaction ACK/BYE priority burst.
4. INVITE 2xx retransmit max due per tick.
5. Dialog dispatch workers and queue capacity.
6. Server admission soft limit and pacing delay.
7. Alice shard count, only after server-side queues are understood.

For each candidate:

- Run `access-edge-microburst`.
- Compare caller failures, especially answer timeouts.
- Compare setup p95/p99/p99.9.
- Confirm receiver cleanup remains clean.
- Confirm RSS slope does not regress.
- Keep the smallest queues and worker counts that pass with margin.

Reject a candidate if it improves one metric by hiding overload, increasing
RSS materially, creating retained objects, or causing teardown/media failures.

## Commands

Run the required smoke:

```bash
BETA_RUN_BURST_SMOKE=1 \
BETA_RUN_BURST_MATRIX=0 \
BETA_REPORT_PACKAGE=0 \
crates/sip/rvoip-sip/scripts/beta_gate.sh --perf
```

Run only `access-edge-microburst`:

```bash
RVOIP_PERF_BURST_SCENARIOS=access-edge-microburst \
crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh
```

Run the full opt-in burst matrix:

```bash
BETA_RUN_BURST_SMOKE=1 \
BETA_RUN_BURST_MATRIX=1 \
BETA_REPORT_PACKAGE=0 \
crates/sip/rvoip-sip/scripts/beta_gate.sh --perf
```

Validate config and harness changes:

```bash
cargo test -p rvoip-sip --features perf-tests --test perf_burst_scenarios
cargo test -p rvoip-sip --features perf-tests --test config_tests
cargo test -p rvoip-sip --release --features perf-tests --test perf_burst_caller --no-run
cargo test -p rvoip-sip --release --features perf-tests --test perf_burst_receiver --no-run
cargo fmt --all -- --check
git diff --check
```

## Promotion Rules

A YAML recipe can be promoted only when:

- the scenario passes at least three times with the same recipe and seed policy
- ASR, setup latency, media, cleanup, RSS, and CPU are all stable
- caller and receiver retained objects drain to zero
- active audio receivers drain to zero
- the recipe records the scenario name, date, git revision, and artifact path
- `TUNING.md` documents the intended operating envelope and tradeoffs

Single-run winners stay as candidate artifacts under `target/perf-results/`.
They should not be committed as production recipes until repeated validation
confirms the improvement.

## Decision Rules

If answer timeouts fall and cleanup/RSS stay clean, keep the tuning change and
repeat it.

If answer timeouts do not improve, revert the candidate and test the next knob.

If a buffer or pool increases latency, CPU, or RSS without improving stability,
remove it from the hot path and from the public tuning surface.

If cleanup retention returns, stop latency tuning and fix cleanup first.

If both caller and receiver pass but host CPU or UDP drops dominate, classify
the result as host capacity evidence rather than library failure.
