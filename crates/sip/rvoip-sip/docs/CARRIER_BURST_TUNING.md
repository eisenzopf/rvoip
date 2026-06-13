# Carrier Burst Tuning Ledger

## Purpose

This document is the experiment ledger for the `access-edge-microburst`
tuning work. It records what was tried, the relevant test settings, the result,
and the decision for each run. The companion narrative plan is
`CARRIER_BURST_TUNING_PLAN.md`.

The canonical data table is
[`CARRIER_BURST_TUNING_LEDGER.csv`](CARRIER_BURST_TUNING_LEDGER.csv). The CSV
has one row per run and includes artifact path, scenario/profile files,
effective tuning knobs, result counters, RSS gates, host UDP deltas, and the
decision. The Markdown tables below are human-readable summaries of that CSV.

The current conclusion is: no `access-edge-microburst` Config recipe is ready
to promote, but media-plane pacing is now the first full-media library
candidate to pass the gates in three repeat runs. Config tuning improved
observability and found partial improvements, but every Config-only full-media
candidate missed ASR `0.999`. The remaining failure mode appears to be
generated RTP/media scheduling pressure starving SIP control-plane polling,
especially on the caller.

## Common Test Shape

Unless a row says otherwise, access-edge tuning runs used this traffic shape:

| Setting | Value |
| --- | --- |
| Seed | `2026060802` |
| Capacity | `6000` |
| Alice shards | `16` |
| Phases | `20 cps x 30s`, `120 cps x 20s`, `20 cps x 30s`, `160 cps x 20s`, `20 cps x 30s` |
| Offered calls | `7400` |
| Acceptance ASR | `>= 0.999` |
| Media setup failures | `0` |
| Teardown failures | `0` |
| Caller/receiver retained objects after drain | `0` |
| Receiver active audio receivers after drain | `0` |
| RSS gate | `<= 10 MB/hr` post-drain |

The staged tuning runs used temporary YAML under:

```bash
target/perf-experiments/access-edge/
```

The standard command shape was:

```bash
RVOIP_PERF_RECIPE_FILE=target/perf-experiments/access-edge/<recipes>.yaml \
RVOIP_PERF_BURST_SCENARIO_FILE=target/perf-experiments/access-edge/<scenarios>.yaml \
RVOIP_PERF_BURST_SCENARIOS=<candidate> \
RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS=120 \
RVOIP_PERF_MEMORY_DIAGNOSTICS=1 \
crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh
```

Some early staged runs predate the final diagnostic recipe split, but their
caller/receiver JSON reports still include the effective Config used by the
run. Use the artifact JSON as the source of truth when a row and a temporary
YAML disagree because the YAML was later reused.

## Baseline Profile Knobs

The access-edge candidate family used these common server settings unless a
row calls out a delta:

| Knob | Common value |
| --- | --- |
| SIP UDP recv/send socket buffers | `8388608` / `8388608` |
| SIP UDP parse workers | `8` |
| SIP UDP parse queue capacity | `6000` |
| SIP UDP parse dispatch | `RoundRobin` |
| Transaction dispatch workers | `4` unless varied |
| Dialog dispatch workers | `4` control, `8` for the best dialog candidate |
| Dialog dispatch queue capacity | unset control, `24000` for dialog `8/24000` |
| Session event dispatcher workers | `4` |
| Transaction command channel capacity | `512` in tuned recipes |
| Server call capacity/admission hard limit | `6000` / `6000` |
| Server admission soft limit | `5400` unless varied |
| Server admission pacing delay | `1 ms` unless varied |
| Overload Retry-After | `1 s` |
| Media mode | `enabled` unless isolation row says `signaling-only` |
| Media session capacity | `6000` |
| RTP session buffers | sender `64`, receiver `32`, event `64` in diagnostic recipes |
| RTP transport buffers | event `32`, RTP recv `1500`, RTCP recv `1500` |
| Media controller pools | audio frame pool `64/256`, RTP buffer pool `64/256` in diagnostic recipes |

## Staged Config Sweep

These runs tested Config/YAML knobs before library changes. Receiver retained
objects and receiver active audio receivers were `0` after drain for all rows
in this table.

| Artifact | Candidate and settings | Caller result | RSS gate caller / receiver | Decision |
| --- | --- | --- | --- | --- |
| `burst_20260609_014116_10963/ae-control` | Control high-density profile. Transport dispatch unset, transaction workers `4`, dialog workers `4`, soft limit `5400`, delay `1 ms`. | ASR `0.9874`, `7307/7400`, `93` failed (`92` timeouts, `1` teardown). | `0.21` / `0.42 MB/hr` | Reject. Baseline below ASR gate. |
| `burst_20260609_015402_29667/ae-transport-2q12000` | Transport dispatch workers `2`, queue `12000`. | ASR `0.9843`, `7284/7400`, `116` timeouts. | `1.72` / `0.42 MB/hr` | Reject. Regressed. |
| `burst_20260609_015402_29667/ae-transport-4q24000` | Transport dispatch workers `4`, queue `24000`. | ASR `0.9909`, `7333/7400`, `67` timeouts. | `4.80` / `0.42 MB/hr` | Reject. Better, still below ASR gate. |
| `burst_20260609_021914_47716/ae-transaction-2q12000` | Transaction workers `2`, queue `12000`. | ASR `0.9808`, `7258/7400`, `142` timeouts. | `0.31` / `0.42 MB/hr` | Reject. Regressed. |
| `burst_20260609_021914_47716/ae-transaction-4q24000` | Transaction workers `4`, queue `24000`. | ASR `0.9843`, `7284/7400`, `116` failed (`115` timeouts, `1` teardown). | `3.45` / `0.42 MB/hr` | Reject. Regressed. |
| `burst_20260609_032739_3307/ae-transaction-8q24000` | Transaction workers `8`, queue `24000`. | ASR `0.9903`, `7328/7400`, `72` timeouts. | `1.26` / `0.85 MB/hr` | Reject. Still below ASR gate. |
| `burst_20260609_032739_3307/ae-priority-burst-16` | Transaction ACK/BYE priority burst `16`. | ASR `0.9891`, `7319/7400`, `81` failed (`80` timeouts, `1` teardown). | `1.30` / `0.42 MB/hr` | Reject. Regressed. |
| `burst_20260609_032739_3307/ae-invite-2xx-512` | INVITE 2xx retransmit due-per-tick cap `512`. | ASR `0.9849`, `7288/7400`, `112` timeouts. | `1.17` / `0.42 MB/hr` | Reject. Regressed; cap did not bind in later diagnostics. |
| `burst_20260609_032739_3307/ae-dialog-2q12000` | Dialog workers `2`, queue `12000`. | ASR `0.9911`, `7334/7400`, `66` timeouts. | `1.82` / `2.78 MB/hr` | Reject. Below ASR gate. |
| `burst_20260609_032739_3307/ae-dialog-8q24000` | Dialog workers `8`, queue `24000`. | ASR `0.9943`, `7358/7400`, `42` timeouts. | `4.14` / `0.42 MB/hr` | Best Config candidate, but reject. Below ASR gate. |
| `burst_20260609_024844_70747/ae-admission-soft4500-delay1` | Admission soft limit `4500`, delay `1 ms`. | ASR `0.9909`, `7333/7400`, `67` timeouts. | `5.20` / `0.42 MB/hr` | Reject. Static pacing insufficient. |
| `burst_20260609_024844_70747/ae-admission-soft5000-delay1` | Admission soft limit `5000`, delay `1 ms`. | ASR `0.9920`, `7341/7400`, `59` failed (`58` timeouts, `1` teardown). | `5.16` / `0.42 MB/hr` | Reject. Below ASR gate. |
| `burst_20260609_031357_94602/ae-admission-soft5000-delay2` | Admission soft limit `5000`, delay `2 ms`. | ASR `0.9862`, `7298/7400`, `102` timeouts. | `3.42` / `0.42 MB/hr` | Reject. Regressed. |

## Diagnostic Config Runs

These runs added caller-side and receiver-side diagnostics to explain why the
best Config candidate still failed. Unless noted, server admission rejected
`0`, receiver retained objects after drain were `0`, and receiver active audio
receivers after drain were `0`.

| Artifact | Settings and diagnostics | Caller result | Receiver/media result | Decision |
| --- | --- | --- | --- | --- |
| `burst_20260609_045820_79749/ae-dialog-8q24000-diagnostics` | Best dialog `8/24000`; server timing diagnostics and media setup diagnostics enabled. | ASR `0.9870`, `7304/7400`, `96` timeouts, caller RSS `3.81 MB/hr`. | `7387` incoming, receiver RSS `2.28 MB/hr`; media setup max `2.252 ms`; admission paced `0`. | Reject. Server queue/media setup not bottleneck. |
| `burst_20260609_051936_18779/ae-dialog-8q24000-client-diagnostics` | Caller-side diagnostics added; media setup diagnostics enabled. | ASR `0.9918`, `7339/7400`, `61` failed (`60` timeouts, `1` teardown), caller RSS `2.25 MB/hr`. | `7364` incoming, receiver RSS `1.01 MB/hr`. | Reject. Still below ASR gate. |
| `burst_20260609_055448_63531/ae-dialog-8q24000-client-diagnostics` | Baseline client diagnostics, UAC 2xx/ACK counters. | ASR `0.9941`, `7356/7400`, `44` timeouts, caller RSS `1.09 MB/hr`. | `7390` incoming, `5440` receiver ACKs, `47742` proactive 2xx, receiver RSS `0.52 MB/hr`. | Reject. Best diagnostic run but below ASR gate. |
| `burst_20260609_061202_88306/ae-dialog-8q24000-retx512-diagnostics` | INVITE 2xx due-per-tick cap `512`. | ASR `0.9872`, `7305/7400`, `95` timeouts, caller RSS `0.21 MB/hr`. | `7384` incoming, `5411` receiver ACKs, `48189` proactive 2xx. | Reject. ASR regressed; cap did not bind. |
| `burst_20260609_062514_96216/ae-dialog-8q24000-shards32-diagnostics` | Alice shards `32`. | ASR `0.9911`, `7334/7400`, `66` timeouts, caller RSS `26.11 MB/hr`. | `7374` incoming, `5408` receiver ACKs, `48037` proactive 2xx. | Reject. ASR and RSS failed. |
| `burst_20260609_080354_75732/ae-dialog-8q24000-client-diagnostics` | Per-method UDP counters, per-Call-ID wire trace, host UDP delta. | ASR `0.9949`, `7362/7400`, `38` timeouts, caller RSS `1.71 MB/hr`. | `7391` incoming, `5529` receiver ACKs, receiver RSS `0.52 MB/hr`; host full-buffer drops delta `0`. | Reject. Use as protocol evidence. |
| `burst_20260609_090007_78489/ae-dialog-8q24000-client-diagnostics` | Timestamped SIP UDP and dialog Call-ID traces. Caller socket buffers unset; server socket buffers `8 MiB`. | ASR `0.9836`, `7279/7400`, `121` timeouts, caller RSS `5.97 MB/hr`. | `7380` incoming, receiver RSS `0.52 MB/hr`; host full-buffer drops delta `0`; admission paced `0`. | Reject. Diagnostics showed late caller 2xx/no-ACK-attempt cases. |
| `burst_20260609_094745_33369/ae-dialog-8q24000-admission4500-client-diagnostics` | Admission soft limit `4500`, delay `1 ms`; diagnostics on. | ASR `0.9862`, `7298/7400`, `102` timeouts, caller RSS `0.42 MB/hr`. | `7389` incoming; admission paced `729`, rejected `0`; host full-buffer drops delta `0`. | Reject. Static pacing helped only modestly. |
| `burst_20260609_100256_53195/ae-dialog-8q24000-admission4500-delay2-client-diagnostics` | Admission soft limit `4500`, delay `2 ms`; diagnostics on. | ASR `0.9826`, `7271/7400`, `129` timeouts, caller RSS `17.22 MB/hr`. | `7369` incoming; admission paced `777`, rejected `0`; host full-buffer drops delta `0`. | Reject. ASR and RSS regressed. |

Failed Call-ID joins from the diagnostic runs:

| Artifact | Failure buckets |
| --- | --- |
| `burst_20260609_080354_75732` | `9` receiver never saw INVITE, `17` caller saw 2xx but no ACK attempt, `5` caller ACK not seen by receiver, `7` receiver saw ACK but caller lifecycle timed out. |
| `burst_20260609_090007_78489` | `20` receiver never saw INVITE, `65` caller saw 2xx but no ACK attempt, `20` caller ACK not seen by receiver, `16` receiver saw ACK but caller lifecycle timed out. |
| `burst_20260609_094745_33369` | `11` receiver never saw INVITE, `62` caller saw 2xx but no ACK attempt, `13` caller ACK not seen by receiver, `16` receiver saw ACK but caller lifecycle timed out. |
| `burst_20260609_100256_53195` | `31` receiver never saw INVITE, `77` caller saw 2xx but no ACK attempt, `10` caller ACK not seen by receiver, `11` receiver saw ACK but caller lifecycle timed out. |

## Library Isolation Runs

These runs moved below Config tuning into transport/media behavior.

| Artifact | Settings and library state | Caller result | Receiver/media result | Decision |
| --- | --- | --- | --- | --- |
| `burst_20260609_103932_98071/ae-dialog-8q24000-client-diagnostics` | Per-socket UDP receive-loop diagnostics before bounded receive drain. | ASR `0.9896`, `7323/7400`, `77` timeouts, caller RSS `2.19 MB/hr`; caller receive-loop p95 `1 s`, p99 bucket `5 s`, max `37.5 s`. | `7382` incoming, receiver RSS `0.52 MB/hr`; host full-buffer drops delta `0`. | Reject. Baseline for library diagnostics. |
| `burst_20260609_110812_31701/ae-dialog-8q24000-client-diagnostics` | Bounded SIP UDP receive drain after each awaited receive. | ASR `0.9916`, `7338/7400`, `62` timeouts, caller RSS `0.21 MB/hr`; caller receive-loop p95 `1 s`, p99 bucket `5 s`, max `36.3 s`. | `7381` incoming, receiver RSS `0.52 MB/hr`; host full-buffer drops delta `0`. | Keep as partial improvement, but not root fix. |
| `burst_20260609_112609_65435/ae-dialog-8q24000-sip-only-diagnostics` | Server and client `mediaMode: signaling-only`. | Harness ASR `0` because media expectations failed, but SIP setup path had no answer timeouts; caller INVITE-to-2xx p95 `2.643 ms`, p99 `3.894 ms`, max `20.333 ms`. | Receiver observed `7400` incoming; no audio receivers expected; host full-buffer drops delta `0`. | Diagnostic only. SIP stack is not the root. |
| `burst_20260609_114444_80163/ae-dialog-8q24000-client-diagnostics` | Full media allocation, caller skipped RTP tone source with `RVOIP_PERF_BURST_SKIP_AUDIO_SOURCE=1`. | ASR `0.6332`, but answer timeouts `0`; failures were `2714` teardown/no-media side effects. Setup p95 `13.7 ms`, p99 `15.1 ms`, max `116.7 ms`. | Receiver observed all `7400` INVITEs/2xx/ACKs, allocated media, drained cleanly, received `0` RTP frames. | Diagnostic only. Generated RTP traffic triggers the setup failure. |

## RTP Scheduling Experiments

These runs tested library changes in the generated RTP path.

| Artifact | Settings and library state | Caller result | Receiver/media result | Decision |
| --- | --- | --- | --- | --- |
| `burst_20260609_130757_68023/ae-dialog-8q24000-client-diagnostics` | Audio transmitter start phase spread across 20 ms interval; missed ticks set to `Skip`; endpoint media diagnostics off. | ASR `0.9932`, `7350/7400`, `50` timeouts, caller RSS `0.57 MB/hr`; caller UDP gap p95 `1 s`, p99/p999 `5 s`, max `36.08 s`. | `7384` incoming, receiver RSS `2.53 MB/hr`; active audio `0`, retained `0`; host full-buffer drops delta `0`. | Keep as partial improvement. Still below ASR gate. |
| `burst_20260609_132911_93058/ae-dialog-8q24000-client-diagnostics` | Same phase spread plus endpoint `mediaSetupDiagnostics: true`, including audio TX timing. | ASR `0.9905`, `7330/7400`, `70` timeouts, caller RSS `2.87 MB/hr`. | Audio TX diagnostics: `7330` tasks, avg start phase `~9.99 ms`, avg tick gap `~20 ms`, send failures `0`, avg send under `1 us`; host full-buffer drops delta `0`. | Reject as acceptance data. Diagnostics are intrusive. |
| `burst_20260609_134929_16103/ae-dialog-8q24000-client-diagnostics` | Batched audio TX diagnostics attempt. | Aborted manually after early regression, about `91` timeouts while still running. | Host full-buffer drops delta `0`; no final caller/receiver JSON. | Non-citable aborted run. |
| `burst_20260609_140223_30886/ae-dialog-8q24000-client-diagnostics` | Recurrence tone generation plus conditional timing attempt. | Aborted manually after clear regression, about `112` caller failures and high caller CPU while still running. | Host full-buffer drops delta `0`; no final caller/receiver JSON. | Non-citable aborted run. |
| `burst_20260609_141737_51209/ae-dialog-8q24000-client-diagnostics` | Cached-tone/payload-copy experiment; endpoint media diagnostics off. | ASR `0.9886`, `7316/7400`, `84` timeouts, setup p95 `12.20 s`, peak pending setups `731`, caller RSS `5.81 MB/hr`; caller UDP max gap `50.96 s`. | `7372` incoming, `7372` completed audio receivers, `24,751,451` RTP frames, active audio `0`, retained `0`, receiver RSS `0.42 MB/hr`; host full-buffer drops delta `0`. | Reject and revert cached-tone/payload-copy change. |
| `burst_20260609_203719_87572/ae-dialog-8q24000-client-diagnostics` | Cumulative RTP hot-path bundle: destination cache, RTP/RTCP receive buffer reuse, audio TX active/timestamp lock removal, and RTP send stats batching. | ASR `0.9900`, `7326/7400`, `74` timeouts, setup p95 `13.37 s`, p99 `23.35 s`, peak pending setups `732`, caller RSS gate `0.51 MB/hr`. | `7368` incoming, `7368` completed audio receivers, `24,518,896` RTP frames, active audio `0`, retained `0`, receiver RSS gate `0.52 MB/hr`; host full-buffer drops delta `0`. | Reject. Regressed versus phase-spread best and did not reduce setup tail. |
| `burst_20260609_210431_30162/ae-dialog-8q24000-client-diagnostics` | Audio TX pacing enabled with `RVOIP_MEDIA_AUDIO_TX_PACING=1`, target active `3000`; endpoint media diagnostics off; counter patch not yet applied. | ASR `1.0000`, `7400/7400`, `0` timeouts, setup p95 `1.25 s`, p99 `3.79 s`, peak pending setups `176`, caller RSS gate `0.42 MB/hr`. | `7400` incoming, `7400` completed audio receivers, `26,683,664` RTP frames, active audio `0`, retained `0`, receiver RSS gate `2.26 MB/hr`; host full-buffer drops delta `0`. | Keep candidate. First pass, but pacing counters were not recorded. |
| `burst_20260609_212634_64711/ae-dialog-8q24000-client-diagnostics` | Same audio TX pacing target `3000`, with lightweight pacing counters recorded outside media setup diagnostics. | ASR `1.0000`, `7400/7400`, `0` timeouts, setup p95 `1.67 s`, p99 `5.27 s`, peak pending setups `238`, caller RSS gate `0.80 MB/hr`; pacing skipped `8,340,945` TX ticks, active max `4837`, divisor max `2`. | `7400` incoming, `7400` completed audio receivers, `26,736,493` RTP frames, active audio `0`, retained `0`, receiver RSS gate `0.42 MB/hr`; host full-buffer drops delta `0`. | Keep candidate. Second pass; needs one more repeat before promotion. |
| `burst_20260609_214141_90637/ae-dialog-8q24000-client-diagnostics` | Same audio TX pacing target `3000`, third repeat with pacing counters. | ASR `1.0000`, `7400/7400`, `0` timeouts, setup p95 `1.67 s`, p99 `4.22 s`, peak pending setups `191`, caller RSS gate `1.47 MB/hr`; pacing skipped `8,336,949` TX ticks, active max `4827`, divisor max `2`. | `7400` incoming, `7400` completed audio receivers, `26,268,221` RTP frames, active audio `0`, retained `0`, receiver RSS gate `0.42 MB/hr`; host full-buffer drops delta `0`. | Keep candidate. Third repeat passed all acceptance gates. |
| `burst_20260609_215701_11703/ae-dialog-8q24000-client-diagnostics` | Audio TX pacing target active `4000` probe. | ASR `1.0000`, `7400/7400`, `0` timeouts, setup p95 `3.63 s`, p99 `8.20 s`, peak pending setups `338`, caller RSS gate `0.91 MB/hr`; pacing skipped `3,967,196` TX ticks, active max `4826`, divisor max `2`. | `7400` incoming, `7400` completed audio receivers, `26,084,301` RTP frames, active audio `0`, retained `0`, receiver RSS gate `0.42 MB/hr`; host full-buffer drops delta `0`. | Keep as probe only. Passed once, but setup tail and host no-socket drops were worse than target `3000`. |
| `burst_20260610_013259_65017/ae-dialog-8q24000-client-diagnostics` | Shared generated-audio TX scheduler enabled without pacing. | ASR `0.9866`, `7301/7400`, `99` timeouts, setup p95 `11.93 s`, p99 `23.64 s`, peak pending setups `675`, caller RSS gate `1.89 MB/hr`; shared due `38,347,430`, sent `38,347,216`, fail `0`, batch max `1336`; avg caller CPU `119.5%`. | `7361` incoming, `7361` completed audio receivers, `25,514,793` RTP frames, active audio `0`, retained `0`, receiver RSS gate `0.78 MB/hr`; host full-buffer drops delta `0`. | Reject. Timer consolidation alone made the setup tail and CPU worse than pacing. |
| `burst_20260610_014610_83802/ae-dialog-8q24000-client-diagnostics` | Shared generated-audio TX scheduler plus pacing target active `3000`, before the stop-race guard. | ASR `0.9999`, `7399/7400`, `0` timeouts, `1` teardown failure, setup p95 `1.62 s`, p99 `4.22 s`, peak pending setups `179`, caller RSS gate `0.71 MB/hr`; pacing skipped `8,348,421`, shared fail `1`, batch max `795`. | `7400` incoming, `7400` completed audio receivers, `26,700,539` RTP frames, active audio `0`, retained `0`, receiver RSS gate `0.42 MB/hr`; host full-buffer drops delta `0`. | Reject. Near-pass, but teardown and shared send failure break acceptance. |
| `burst_20260610_020612_97335/ae-dialog-8q24000-client-diagnostics` | Shared generated-audio TX scheduler plus pacing target active `3000`, after rechecking active state around send. | ASR `1.0000`, `7400/7400`, `0` timeouts, setup p95 `1.66 s`, p99 `5.27 s`, peak pending setups `223`, caller RSS gate `4.33 MB/hr`; pacing skipped `8,344,158`, shared fail `0`, batch max `1341`; avg caller CPU `95.1%`. | `7400` incoming, `7400` completed audio receivers, `26,576,647` RTP frames, active audio `0`, retained `0`, receiver RSS gate `0.42 MB/hr`; host full-buffer drops delta `0`. | Keep candidate. First guarded pass. |
| `burst_20260610_022419_31492/ae-dialog-8q24000-client-diagnostics` | Same guarded shared scheduler plus pacing target active `3000`. | ASR `1.0000`, `7400/7400`, `0` timeouts, setup p95 `1.65 s`, p99 `4.24 s`, peak pending setups `214`, caller RSS gate `1.09 MB/hr`; pacing skipped `8,350,194`, shared fail `0`, batch max `609`; avg caller CPU `95.3%`. | `7400` incoming, `7400` completed audio receivers, `26,734,334` RTP frames, active audio `0`, retained `0`, receiver RSS gate `0.42 MB/hr`; host full-buffer drops delta `0`. | Keep candidate. Second guarded pass. |
| `burst_20260610_023645_41698/ae-dialog-8q24000-client-diagnostics` | Same guarded shared scheduler plus pacing target active `3000`. | ASR `1.0000`, `7400/7400`, `0` timeouts, setup p95 `1.62 s`, p99 `7.67 s`, peak pending setups `199`, caller RSS gate `0.93 MB/hr`; pacing skipped `8,342,860`, shared fail `0`, batch max `786`; avg caller CPU `96.4%`. | `7400` incoming, `7400` completed audio receivers, `26,800,350` RTP frames, active audio `0`, retained `0`, receiver RSS gate `0.42 MB/hr`; host full-buffer drops delta `0`. | Keep candidate. Third guarded pass, but not preferred over simpler pacing-only because p99 was less stable. |

Failed Call-ID join for the rejected cached-tone/payload-copy run:

| Artifact | Failure buckets |
| --- | --- |
| `burst_20260609_141737_51209` | `28` receiver never saw INVITE, `36` caller saw 2xx but no ACK attempt, `9` caller ACK not seen by receiver, `11` receiver saw ACK but caller lifecycle timed out. |

## Host UDP Counter Deltas

Host UDP counters were captured once SIP UDP diagnostics started writing
`host_udp_delta.txt`. Full-socket-buffer drops were `0` in every captured run,
including runs with restored `8 MiB` SIP UDP socket buffers. `no socket` drops
rose during full-media runs and need SIP-vs-RTP packet attribution before they
are treated as SIP loss.

| Artifact | Full socket buffer drops | No-socket drops | Datagrams received | Datagrams output |
| --- | ---: | ---: | ---: | ---: |
| `burst_20260609_080354_75732/ae-dialog-8q24000-client-diagnostics` | `0` | `3,915,308` | `29,240,255` | `35,432,374` |
| `burst_20260609_090007_78489/ae-dialog-8q24000-client-diagnostics` | `0` | `3,939,115` | `28,924,596` | `35,049,216` |
| `burst_20260609_094745_33369/ae-dialog-8q24000-admission4500-client-diagnostics` | `0` | `4,193,619` | `29,043,170` | `34,991,538` |
| `burst_20260609_100256_53195/ae-dialog-8q24000-admission4500-delay2-client-diagnostics` | `0` | `3,938,817` | `28,645,906` | `34,971,157` |
| `burst_20260609_103932_98071/ae-dialog-8q24000-client-diagnostics` | `0` | `4,310,079` | `29,112,450` | `35,143,574` |
| `burst_20260609_110812_31701/ae-dialog-8q24000-client-diagnostics` | `0` | `4,273,470` | `28,989,467` | `35,217,072` |
| `burst_20260609_112609_65435/ae-dialog-8q24000-sip-only-diagnostics` | `0` | `247,112` | `288,147` | `39,196` |
| `burst_20260609_114444_80163/ae-dialog-8q24000-client-diagnostics` | `0` | `593,082` | `659,279` | `60,091` |
| `burst_20260609_130757_68023/ae-dialog-8q24000-client-diagnostics` | `0` | `4,577,892` | `29,548,356` | `35,432,760` |
| `burst_20260609_132911_93058/ae-dialog-8q24000-client-diagnostics` | `0` | `3,645,576` | `29,015,719` | `35,265,377` |
| `burst_20260609_134929_16103/ae-dialog-8q24000-client-diagnostics` | `0` | `2,982,012` | `26,300,252` | `32,959,211` |
| `burst_20260609_140223_30886/ae-dialog-8q24000-client-diagnostics` | `0` | `2,444,711` | `24,934,737` | `31,756,054` |
| `burst_20260609_141737_51209/ae-dialog-8q24000-client-diagnostics` | `0` | `3,928,474` | `28,754,715` | `35,316,262` |
| `burst_20260609_210431_30162/ae-dialog-8q24000-client-diagnostics` | `0` | `1,986,534` | `28,735,812` | `28,128,211` |
| `burst_20260609_212634_64711/ae-dialog-8q24000-client-diagnostics` | `0` | `1,942,135` | `28,744,727` | `28,147,322` |
| `burst_20260609_214141_90637/ae-dialog-8q24000-client-diagnostics` | `0` | `2,394,587` | `28,729,990` | `28,053,692` |
| `burst_20260609_215701_11703/ae-dialog-8q24000-client-diagnostics` | `0` | `3,021,435` | `29,188,950` | `32,026,717` |
| `burst_20260610_013259_65017/ae-dialog-8q24000-client-diagnostics` | `0` | `3,698,586` | `29,284,848` | `35,544,868` |
| `burst_20260610_014610_83802/ae-dialog-8q24000-client-diagnostics` | `0` | `2,177,444` | `28,945,288` | `28,584,335` |
| `burst_20260610_020612_97335/ae-dialog-8q24000-client-diagnostics` | `0` | `2,322,696` | `28,964,840` | `28,577,634` |
| `burst_20260610_022419_31492/ae-dialog-8q24000-client-diagnostics` | `0` | `2,070,238` | `28,870,370` | `28,647,404` |
| `burst_20260610_023645_41698/ae-dialog-8q24000-client-diagnostics` | `0` | `2,039,107` | `28,903,444` | `28,576,383` |

## Current Working Decisions

- UDP socket buffer knobs are real and wired. They were restored and appeared
  in effective Config snapshots, but the host counter evidence does not show
  full-socket-buffer overflow as the current bottleneck.
- Config tuning did not find an accepted recipe.
- Static admission pacing exists and can be triggered, but the tested static
  delays did not pass and sometimes regressed.
- Bounded SIP UDP receive draining is worth keeping as a small improvement.
- RTP audio transmitter phase spreading is worth keeping as a partial
  improvement.
- The cumulative RTP hot-path bundle with destination caching, receive buffer
  reuse, audio TX lock removal, and send stats batching regressed and should
  not be promoted as a bundle.
- Audio TX pacing with target active `3000` is the first full-media candidate
  to pass acceptance gates in three repeat runs. It should remain opt-in until
  the implementation is reviewed for production-facing configuration and
  diagnostics.
- Audio TX pacing target active `4000` passed one probe run, but setup p95/p99
  and host no-socket drops were materially worse than target `3000`; do not
  promote `4000` without more evidence.
- Shared generated-audio TX scheduling without pacing regressed and should not
  be promoted.
- Shared generated-audio TX scheduling plus target-`3000` pacing passed three
  guarded runs after a stop-race fix. Keep it as an opt-in candidate, but do
  not prefer it over pacing-only yet: it adds scheduler complexity, did not
  materially reduce caller CPU, and had less stable setup p99 in the third run.
- Per-packet/media setup diagnostics are useful for targeted runs only. Do not
  enable them in acceptance measurements.
- The cached-tone/payload-copy experiment was rejected and should not be
  promoted.
- The next library experiment should focus on adaptive media-plane pacing or
  another small CPU reduction in the generated RTP path; larger SIP queues have
  not explained the failures.

## Validation Commands

Validation commands run after the current investigation:

```bash
cargo fmt --package rvoip-media-core --package rvoip-sip-transport --package rvoip-sip
cargo test -p rvoip-media-core audio_generation
cargo test -p rvoip-media-core diagnostics
cargo test -p rvoip-sip-transport diagnostics
cargo test -p rvoip-sip-transport try_receive
cargo test -p rvoip-sip --features perf-tests --test config_tests
cargo test -p rvoip-sip --features perf-tests --test perf_burst_scenarios
cargo test -p rvoip-sip --release --features perf-tests --test perf_burst_caller --no-run
cargo test -p rvoip-sip --release --features perf-tests --test perf_burst_receiver --no-run
git diff --check
```
