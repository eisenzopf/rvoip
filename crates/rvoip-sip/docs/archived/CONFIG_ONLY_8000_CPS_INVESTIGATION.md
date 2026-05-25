# Config-Only 8000 CPS Zero-Retransmit Investigation

## Current Status

The high-CPS fast-answer behavior has been moved into public
`rvoip-sip` `Config` APIs and the benchmark listener no longer depends
on the old `RVOIP_*` server behavior environment variables. A follow-up
wiring audit removed the remaining env-backed runtime diagnostic toggles
from the high-CPS server path. A quick-answer profile follow-up now also
disables automatic Timer 100 / `100 Trying` for the high-CPS profile so
fixed immediate-answer listeners avoid one timer task per INVITE and any
extra provisional responses.

The current branch builds and the targeted config tests pass. A fresh
Config-only `8000 CPS` media-enabled listener accepts and cleans all
`120000/120000` calls with no leaked state, no UDP full-socket drops, no
transport queue backpressure, and no duplicate cache misses.

The remaining problem is retransmits. The earlier env-driven fast path
had reached `0` SIPp retransmits at `8000 CPS`. After moving the behavior
into `Config`, an interim run still had a small retransmit count
reported by the user (`91`). After the wiring audit and listener rewrite
in this branch, the server behavior is Config-owned, but the latest
Config-only validation still has recovered INVITE/BYE retransmits:

| Run | Result | Retransmits | Notes |
| --- | ---: | ---: | --- |
| Env-driven fast path before Config migration | `120000/120000` | `0` | Achieved after suppressing automatic `180 Ringing` and using UDP burst workers/queues. |
| Config-only, diagnostics off | `120000/120000` | `5395` | No failed calls, no listener leak, UDP full-socket drop delta `0`. |
| Config-only, diagnostics on | `120000/120000` | `2990` | No failed calls, final `in_flight=0`, all duplicate retransmits recovered by indexed caches. |
| Config-audit rerun, diagnostics off | `120000/120000` | `4171` | `2026-05-23 01:57`, no failed calls, warnings `0`, dead messages `0`, response `>=500 ms` count `2576`, final listener `accepted_total=120000 cleaned_total=120000`, UDP full-socket drop delta `0`. |
| Inline fast-accept listener experiment | `119999/120000` | `3497` | `2026-05-23 02:09`, `fast_auto_accept_incoming_calls=true`; lower retransmits but failed target: one SIPp timeout/dead-call path and final listener `accepted_total=120000 cleaned_total=119999`. |
| Auto-100-disabled rerun, diagnostics off | `120000/120000` | `3257` | `2026-05-23 02:35`, no failed calls, warnings/dead messages `7/7`, response `>=500 ms` count `2013`, final listener `accepted_total=120000 cleaned_total=120000`, UDP full-socket drop delta `0`. |
| Auto-100-disabled rerun, diagnostics on | `120000/120000` | `3365` | `2026-05-23 02:37`, `resp_1xx=0`, `resp_2xx=243379`, duplicate cache misses `0`, cleanup `active_total=0`, UDP full-socket drop delta `0`. |
| Fused fast-200 path, diagnostics on | `120000/120000` | `11433` | `2026-05-23 03:02`, `fast_auto_accept_incoming_calls=true`; listener `accepted_total=120000 cleaned_total=119135`, `resp_1xx=0`, duplicate cache misses `0`, `first_invite_to_200 over_500ms=0`, max `10726 us`, UDP full-socket drop delta `0`. |
| Fused fast-200 path, diagnostics off | `120000/120000` | `71566` | `2026-05-23 03:04`, `fast_auto_accept_incoming_calls=true`; listener `accepted_total=120000 cleaned_total=117787`, warnings/dead messages `20775/20775`, response `>=500 ms` count `25714`, UDP full-socket drop delta `0`. |

| Lossless fast-200 direct dispatcher, diagnostics on | `120000/120000` | `1780` | `2026-05-23 03:36`, forced with `perf_listener --fast-auto-accept --diagnostics`; listener `accepted_total=120000 cleaned_total=120000`, `resp_1xx=0`, duplicate cache misses `0`, ACK/BYE delivered `120000/120000`, `bye_cleanup_missing=0`, `first_invite_to_200 over_500ms=0`, host UDP full-socket drop delta `0`. |
| Lossless fast-200 direct dispatcher, diagnostics off | `120000/120000` | `4295` | `2026-05-23 03:37`, forced with `perf_listener --fast-auto-accept`; warnings/dead messages `0/0`, listener `accepted_total=120000 cleaned_total=120000`, host UDP full-socket drop delta `0`. |
| Lossless dispatcher, current high-CPS profile, diagnostics off | `120000/120000` | `4545` | `2026-05-23 03:39`, `fast_auto_accept_incoming_calls=false`; warnings/dead messages `3/3`, listener `accepted_total=120000 cleaned_total=120000`, host UDP full-socket drop delta `0`. |

### Parse Fanout Timing Runs

Follow-up timing instrumentation added receive-side spans that start at
UDP read and carry through parse, transport forwarding, transaction
dispatch, dialog incoming-call emit, and BYE 200 OK send. The benchmark
listener also gained `--udp-parse-workers N` and
`--udp-parse-round-robin` so parse fanout can be tested without changing
the high-CPS profile default.

Results from
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_parse_fanout_20260523_114126`:

| Run | SIPp result | Achieved CPS | Retransmits | Warnings / Dead | Listener cleanup | UDP drops | Notes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Fast path + diagnostics baseline | `120000/120000` | `8000.0` | `3659` | `0 / 0` | `120000/120000` | `0` | ACK matched/delivered `120000/120000`, duplicate cache misses `0`, `bye_cleanup_missing=0`. |
| Round-robin parse, 2 workers | `120000/120000` | `8000.0` | `5845` | `1645 / 1645` | `120000/120000` | `0` | Worse than baseline: ACK delivered `119779`, duplicate cache misses `4`, proactive 2xx retransmits `1818`. |
| Round-robin parse, 4 workers | `120000/120000` | `8000.0` | `3178` | `4 / 4` | `120000/120000` | `0` | Best run in this matrix: ACK matched/delivered `120000/120000`, duplicate cache misses `0`, proactive 2xx retransmits `10`. |
| Round-robin parse, 8 workers | `120000/120000` | `4444.4` | `164356` | `72091 / 72091` | `114981/114981` | `0` | Failed as a useful profile: ACK delivered `94880`, listener stopped with `5019` calls not cleaned, proactive 2xx retransmits `232433`. |

The server-side timing spans did not expose a `>500 ms` server stall in
the usable runs. For the best `4`-worker round-robin run:

- `udp_read_to_worker_queue p999=500 us`, max `4586 us`.
- `udp_parse p999=100 us`, max `1422 us`.
- `parse_to_transport_manager p999=250 us`, max `1589 us`.
- `transport_manager_to_transaction p999=25000 us`, max `37906 us`.
- `udp_receive_to_incoming_call_emit p999=50000 us`, max `42847 us`.
- `bye_receive_to_200 p999=50000 us`, max `42866 us`.

The `4`-worker round-robin result is a useful candidate for repeat
testing, but it still does not restore the zero-retransmit target and it
introduced a small number of SIPp dead-call warnings. The next
high-signal test is packet capture across host egress and the
SIPp/container receive side while rerunning the baseline and `4`-worker
round-robin profiles. If host egress is timely but SIPp receives late,
the remaining latency is outside rvoip's measured receive-to-send path.

This means the public Config wiring is in place, and the fused fast-200
path now proves lifecycle correctness under the `8000 CPS` SIPp run:
ACK, BYE, and cleanup delivery all drain to `120000/120000` with no
missing cleanup. The zero-retransmit performance target is still not
restored. The next developer should not assume every retransmit is a
correctness failure: many are fast-recovered duplicates. The open
question is now response latency before the session handler's fused
action timer starts, or loss/scheduling after responses leave rvoip.

### Transaction Dispatch And Media Isolation Runs

Follow-up transaction-manager work added Config-owned keyed transaction
ingress sharding:

- `Config::with_sip_transaction_dispatch_workers(N)`
- `Config::with_sip_transaction_dispatch_queue_capacity(N)`
- `perf_listener --transaction-dispatch-workers N`
- `perf_listener --transaction-dispatch-queue-capacity N`

The default remains the original single receive/handle loop. Additional
workers are keyed by call/transaction identity rather than raw
round-robin, so INVITE, ACK, BYE, and CANCEL for one call stay ordered.
High-cardinality transaction timing diagnostics are opt-in via
`--transaction-timing-diagnostics` because timestamp and atomic histogram
work materially perturbs the `8000 CPS` run.

### Dialog Dispatch Follow-Up

The 20k signaling-only runs moved the pressure point past transaction
ingress: with `--transaction-dispatch-workers 4`,
`transport_manager_to_transaction` dropped to low p999 values, while
`udp_receive_to_incoming_call_emit` and `bye_receive_to_200` rose into
hundreds of milliseconds or worse. The next code target is therefore the
transaction-event-to-dialog path.

Follow-up instrumentation and Config-owned knobs have been added for
that layer:

- `Config::with_sip_dialog_dispatch_workers(N)`
- `Config::with_sip_dialog_dispatch_queue_capacity(N)`
- `Config::with_sip_dialog_timing_diagnostics(true)`
- `perf_listener --sip-dialog-dispatch-workers N`
- `perf_listener --sip-dialog-dispatch-queue-capacity N`
- `perf_listener --dialog-timing-diagnostics`

The default remains one dialog event processor. Values above `1` use
keyed sharding by call identity or transaction key; unkeyed events
fallback round-robin. This keeps per-call ordering for INVITE, ACK, BYE,
and CANCEL while allowing a server Config to test dialog fanout.

New diagnostic spans include dialog event dispatch queue delay, dialog
handler duration by method, dialog lookup duration, dialog-to-session
publish duration by event kind, and UDP receive-to-INVITE-200 completion.
The next perf matrix should test the current clean 18k/failed 20k profile
with `--sip-dialog-dispatch-workers 1/2/4/8` and
`--dialog-timing-diagnostics`.

The first media-enabled transaction matrix was not trustworthy as a
transaction comparison because every run was dominated by session/media
setup delay. A macOS `sample` capture under load showed hot samples in
the session action path, especially `MediaAdapter::create_session`,
`RtpSession::new`, and UDP socket `bind`, before the final INVITE 200 OK
send. A perf-only listener flag was added to isolate this:
`--signaling-only-media`, which keeps SDP signaling but skips media-core
RTP allocation.

Focused result:

| Run | SIPp result | Achieved CPS | Avg INVITE->200 | Retransmits | Listener cleanup | Notes |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Normal media, fast path | `118824/120000` | `2700.5` | `2289.6 ms` | `311507` | `105613/118824` | Media session/RTP bind path dominates. |
| Signaling-only media, fast path | `120000/120000` | `8000.0` | `1.0 ms` | `0` | `120000/120000` | Same SIP path without RTP allocation. |

Results from
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_tx_dispatch_signaling_matrix_20260523_133215`:

| Run | SIPp result | Achieved CPS | Avg INVITE->200 | Retransmits | Listener cleanup |
| --- | ---: | ---: | ---: | ---: | ---: |
| Signaling-only baseline | `120000/120000` | `8000.0` | `1.0 ms` | `0` | `120000/120000` |
| Signaling-only transaction workers 2 | `120000/120000` | `8000.0` | `0.5 ms` | `0` | `120000/120000` |
| Signaling-only transaction workers 4 | `120000/120000` | `8000.0` | `0.0 ms` | `0` | `120000/120000` |
| Signaling-only transaction workers 8 | `120000/120000` | `8000.0` | `0.0 ms` | `0` | `120000/120000` |

Combined candidate results from
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_tx_udp_combined_20260523_133412`:

| Run | SIPp result | Achieved CPS | Avg INVITE->200 | Retransmits | Listener cleanup |
| --- | ---: | ---: | ---: | ---: | ---: |
| Signaling-only, UDP RR 4 + tx workers 4 | `120000/120000` | `8000.0` | `0.0 ms` | `0` | `120000/120000` |
| Normal media, UDP RR 4 + tx workers 4 | `119118/120000` | `3054.3` | `1646.2 ms` | `261810` | `111121/119118` |

The same signaling-only profile was then tested at `10000 CPS` with
UDP parse fixed at `--udp-parse-workers 4 --udp-parse-round-robin` and
transaction workers varied. Results from
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_tx_fanout_10k_20260523_134716`:

| Run | SIPp result | Achieved CPS | Avg INVITE->200 | P99.9 INVITE->200 | Retransmits | Listener cleanup |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Signaling-only, UDP RR 4, tx workers 1 | `150000/150000` | `10000.0` | `1.0 ms` | `<75 ms` | `0` | `150000/150000` |
| Signaling-only, UDP RR 4, tx workers 2 | `150000/150000` | `10000.0` | `1.0 ms` | `<50 ms` | `0` | `150000/150000` |
| Signaling-only, UDP RR 4, tx workers 4 | `150000/150000` | `10000.0` | `1.0 ms` | `<25 ms` | `0` | `150000/150000` |
| Signaling-only, UDP RR 4, tx workers 8 | `150000/150000` | `10000.0` | `1.0 ms` | `<10 ms` | `0` | `150000/150000` |

At `10000 CPS`, transaction fanout does not change pass/fail behavior:
every worker count completed successfully with zero retransmits and full
cleanup. The only visible signal is lower p99.9 INVITE-to-200 latency as
transaction workers increase. This suggests the next useful signaling-only
test is to raise CPS until the baseline breaks, then compare worker counts
near that limit.

That follow-up sweep found the default transaction path starts bending
between `15000 CPS` and `20000 CPS` with signaling-only media and UDP
parse fixed at round-robin `4`.

Baseline default transaction dispatch results from
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_tx_breakpoint_sweep_20260523_140211`:

| Run | SIPp result | Achieved CPS | Avg INVITE->200 | P99 INVITE->200 | Retransmits | Listener cleanup |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Signaling-only, UDP RR 4, tx workers 1, 12000 CPS | `180000/180000` | `12000.0` | `1.0 ms` | `<10 ms` | `8` | `180000/180000` |
| Signaling-only, UDP RR 4, tx workers 1, 15000 CPS | `225000/225000` | `15000.0` | `2.0 ms` | `<50 ms` | `85` | `225000/225000` |
| Signaling-only, UDP RR 4, tx workers 1, 20000 CPS | `285000/285000` | `8906.2` | `1635.1 ms` | `>=2000 ms` | `534434` | listener accepted `294960`, cleaned `293797` |

At `18000 CPS`, transaction fanout reduced retransmits and latency while
keeping call/cleanup correctness intact. Results came from
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_tx_breakpoint_18k_compare_20260523_140727`,
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_tx_breakpoint_18k_workers_20260523_140836`,
and
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_tx_breakpoint_18k_tx4_repeat_20260523_140945`:

| Run | SIPp result | Achieved CPS | Avg INVITE->200 | P99 INVITE->200 | Retransmits | Listener cleanup |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 18000 CPS, tx workers 1 | `270000/270000` | `16875.0` | `27.9 ms` | `<1000 ms` | `25419` | `270000/270000` |
| 18000 CPS, tx workers 2 | `270000/270000` | `16875.0` | `4.6 ms` | `<50 ms` | `1240` | `270000/270000` |
| 18000 CPS, tx workers 4 | `270000/270000` | `16875.0` | `11.9 ms` | `<100 ms` | `6624` | `270000/270000` |
| 18000 CPS, tx workers 4 repeat | `270000/270000` | `16875.0` | `6.4 ms` | `<50 ms` | `3929` | `270000/270000` |
| 18000 CPS, tx workers 8 | `270000/270000` | `16875.0` | `4.3 ms` | `<25 ms` | `1298` | `270000/270000` |

At `20000 CPS`, transaction fanout `4` was the least bad run but did not
make the profile acceptable. Results from
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_tx_breakpoint_20k_workers_20260523_140406`:

| Run | SIPp result | Achieved CPS | Avg INVITE->200 | P99 INVITE->200 | Retransmits | Listener cleanup |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 20000 CPS, tx workers 2 | `284947/285000` | `3799.3` | `1739.9 ms` | `>=2000 ms` | `571629` | listener accepted `295882`, cleaned `294828` |
| 20000 CPS, tx workers 4 | `285000/285000` | `10961.5` | `1387.7 ms` | `>=2000 ms` | `482051` | listener accepted `297131`, cleaned `295497` |
| 20000 CPS, tx workers 8 | `284693/285000` | `6326.5` | `2481.7 ms` | `>=2000 ms` | `613551` | listener accepted `297156`, cleaned `295903` |

Conclusion from the higher-CPS signaling-only runs: keyed transaction
fanout is useful once the signaling path is pushed past the easy 10k
case, and `2` to `8` workers all improve the 18k profile over the
single-loop default. `4` workers is a reasonable conservative candidate,
but the best worker count is not settled from one matrix because counts
vary run to run. `20000 CPS` exposes a different saturation mode that
transaction fanout alone does not solve.

Follow-up experiment scaffolding now lives in
`SIGNALING_SHARDING_PERF_EXPERIMENT.md` with a repeatable runner:
`crates/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh`.
The runner only uses Config-backed listener flags, including UDP parse
fanout, SIP transport-manager forwarding workers, transaction-manager
dispatch workers, and queue capacity knobs.

Two `18000 CPS` signaling-only matrices were added there. With default
socket buffers, the best clean point was UDP RR `4`, SIP transport
workers `2`, transaction workers `1`: `270000/270000`, cleanup
`270000/270000`, host UDP drops `0`, retransmits `7601`. A repeat with
4 MB Config-owned UDP socket buffers reduced host UDP drop sensitivity
but did not improve the best clean retransmit count; its best clean
points were transport `4` / transaction `2` (`9774` retransmits) and
transport `2` / transaction `2` (`9921` retransmits).

Focused repeats kept `18000 CPS` clean but showed run-to-run variance:
transport `2` / transaction `1` repeated best at `13142` retransmits,
with transport `4` / transaction `2` close behind at `13734`. Testing
the repeat winner at `20000 CPS` failed (`284999/285000`, `456188`
retransmits, listener `296540/295430`) without host UDP drops. A
`20000 CPS` transport `2` / transaction `4` run removed most
transport-to-transaction queue delay but still failed worse (`543008`
retransmits). The remaining `20k` pressure is therefore downstream of
transaction ingress, in the transaction-to-dialog / dialog incoming-call
path measured by `udp_receive_to_incoming_call_emit` and
`bye_receive_to_200`.

This changes the next optimization target: the SIP receive, transaction,
dialog, and fused response path can hit zero retransmits at `8000 CPS`
when RTP allocation is removed from the critical path. The media-enabled
run needs a media fast path before promoting transaction or parse worker
defaults. The next changes to test should reserve or choose the SDP RTP
port quickly, send the 200 OK, then bind/start the RTP session
asynchronously after the response leaves rvoip, with cleanup able to
cancel a not-yet-started media task if BYE arrives first.

## Goal

Make the media-enabled `8000 CPS` SIPp benchmark run with no server
behavior env vars and meet:

- `120000/120000` successful calls.
- `0` SIPp retransmits.
- final listener `in_flight=0`.
- `accepted_total == cleaned_total`.
- cleanup diagnostics `active_total=0`.
- UDP full-socket-buffer drops delta `0`.
- duplicate INVITE/BYE cache misses `0`.

Load-generation env vars are still allowed for SIPp and the shell
harness. The rvoip server process must get high-CPS behavior from
`Config`, not from env.

## What Changed In This Branch

### Public Config Profile

Added a public Config profile:

```rust
Config::local("perf-listener", port)
    .with_high_cps_udp_auto_answer(20_000)
```

The profile applies:

- `auto_180_ringing = false`
- `auto_100_trying = false`
- `with_channel_capacity(capacity)`
- `sip_udp_parse_workers = Some(1)`
- `sip_udp_parse_queue_capacity = Some(capacity)`
- `media_mode = MediaMode::Enabled`
- no socket buffer enlargement
- no `server_call_capacity` inflation

The profile deliberately does not set `server_call_capacity`. During
the earlier migration, using broad call capacity for media preallocation
also inflated dialog/transaction indexes and made retransmits worse. The
current branch keeps media capacity separate. The profile also keeps
`fast_auto_accept_incoming_calls = false`; that remains an explicit
application opt-in because the fused fast-200 benchmark still missed the
cleanup/retransmit target.

Relevant files:

- `crates/rvoip-sip/src/api/unified.rs`
- `crates/rvoip-sip/src/api/builder.rs`
- `crates/rvoip-sip/src/api/stream_peer.rs`
- `crates/rvoip-sip/src/api/callback_peer.rs`

### Media Capacity Controls

Added Config controls:

```rust
.with_media_session_capacity(20_000)
.with_media_port_capacity(16_384, 49_152)
```

`with_media_port_capacity(start, capacity)` computes the media port end
from Config instead of relying on hard-coded perf env ranges. The
`16384 + 49152 - 1` range reaches `65535`.

`UnifiedCoordinator::create_media_controller` now wires:

```rust
config
    .media_session_capacity
    .or(config.server_call_capacity)
    .unwrap_or(0)
```

into:

```rust
MediaSessionController::with_port_range_and_capacity(...)
```

This keeps media preallocation independent from dialog/transaction
capacity unless an application intentionally uses `server_call_capacity`
as the fallback.

### Config-Owned Diagnostics

Added Config flags:

```rust
.with_sip_udp_diagnostics(true)
.with_media_setup_diagnostics(true)
.with_cleanup_diagnostics(true)
.with_cleanup_diagnostic_events(true)
.with_srtp_diagnostics(true)
.with_rtp_diagnostics(true)
.with_media_sdp_diagnostics(true)
```

`UnifiedCoordinator::new` now propagates them to:

- `sip_udp_diagnostics` ->
  `rvoip_sip_transport::diagnostics::set_enabled` and
  `rvoip_sip_dialog::diagnostics::set_enabled`
- `media_setup_diagnostics` ->
  `rvoip_media_core::diagnostics::set_enabled`
- `cleanup_diagnostics` -> `rvoip_sip::cleanup_diag::set_enabled`
- `cleanup_diagnostic_events` ->
  `rvoip_sip::cleanup_diag::set_event_logs_enabled`
- `srtp_diagnostics` / `media_sdp_diagnostics` ->
  `rvoip_sip::adapters::media_adapter::set_sdp_diagnostics`
- `srtp_diagnostics` / `rtp_diagnostics` ->
  `rvoip_rtp_core::transport::set_udp_diagnostics`

The old env-backed runtime diagnostic toggles were removed from the
server path:

- `RVOIP_SIP_DIAGNOSTICS`
- `RVOIP_SRTP_DIAGNOSTICS`
- `RVOIP_MEDIA_DIAGNOSTICS`
- `RVOIP_RTP_DIAGNOSTICS`
- `RVOIP_PERF_CLEANUP_DIAG`
- `RVOIP_PERF_CLEANUP_DIAG_EVENTS`

`Config::state_table_path` is now the only runtime custom state-table
override; the old `RVOIP_STATE_TABLE` fallback was removed.

Residual env reads in the audited runtime crates are not server behavior
configuration:

- `RVOIP_TEST` and `RVOIP_TEST_TRANSACTION_TIMEOUT_MS` are test hooks.
- `RUN_SOCKET_TESTS` gates socket tests.
- `USER` is used only for the RTP RTCP CNAME default.

### Perf Listener Rewrite

`crates/rvoip-sip/examples/perf_listener.rs` now builds this profile by
default:

```rust
Config::local("perf-listener", port)
    .with_high_cps_udp_auto_answer(20_000)
    .with_media_port_capacity(16_384, 49_152)
    .with_media_session_capacity(20_000)
```

It also accepts optional diagnostics flags:

- `--diagnostics` enables summary counters for SIP UDP, duplicate
  recovery, media setup, and cleanup.
- `--diagnostic-events` additionally enables per-operation cleanup event
  logs and implies `--diagnostics`.
- `--wire-diagnostics` enables noisy SRTP/RTP/SDP diagnostic logs.

The listener no longer reads these old server behavior env vars:

- `RVOIP_PERF_CHANNEL_CAPACITY`
- `RVOIP_PERF_SERVER_CAPACITY`
- `RVOIP_PERF_SIP_UDP_PARSE_*`
- `RVOIP_PERF_AUTO_180_RINGING`
- `RVOIP_PERF_SUPPRESS_AUTO_180`
- `RVOIP_PERF_MEDIA_ENABLED`
- `RVOIP_PERF_NO_MEDIA`
- `RVOIP_PERF_NO_MEDIA_RTP_PORT`
- `RVOIP_PERF_RTP_PORT_*`
- `RVOIP_PERF_MEDIA_PORT_*`
- `RVOIP_SIP_UDP_PARSE_WORKERS`
- `RVOIP_SIP_UDP_PARSE_QUEUE_CAPACITY`

Verification command used:

```bash
rg -n 'RVOIP_PERF_CHANNEL_CAPACITY|RVOIP_PERF_SERVER_CAPACITY|RVOIP_PERF_SIP_UDP_PARSE|RVOIP_PERF_AUTO_180|RVOIP_PERF_MEDIA_ENABLED|RVOIP_PERF_RTP_PORT|RVOIP_PERF_MEDIA_PORT|RVOIP_PERF_SUPPRESS_AUTO_180|RVOIP_SIP_UDP_PARSE_WORKERS|RVOIP_SIP_UDP_PARSE_QUEUE_CAPACITY|RVOIP_PERF_NO_MEDIA|RVOIP_PERF_NO_MEDIA_RTP_PORT' \
  crates/rvoip-sip/examples/perf_listener.rs \
  crates/rvoip-sip/tests/perf/perf_call_setup_cps.rs \
  crates/rvoip-sip/src \
  crates/rvoip-sip-dialog/src \
  crates/rvoip-sip-transport/src \
  crates/media-core/src \
  crates/rtp-core/src
```

It returned no matches.

### Perf Test Harness Update

`crates/rvoip-sip/tests/perf/perf_call_setup_cps.rs` now uses:

```rust
config.with_high_cps_udp_auto_answer(channel_capacity)
```

for the high-CPS server profile. Load-generator env vars remain in the
SIPp scripts and perf harness; those are not server behavior knobs.

### Tests Added Or Updated

Updated/added coverage in:

- `crates/rvoip-sip/tests/config_channel_capacity_integration.rs`
- `crates/rvoip-sip/tests/config_tests.rs`

Covered cases include:

- high-CPS profile sets fast-answer, channel capacity, UDP parse worker,
  UDP parse queue capacity, disabled automatic `180 Ringing`, disabled
  automatic `100 Trying`, and media enabled.
- high-CPS profile does not set `server_call_capacity`.
- media session capacity is independent from `server_call_capacity`.
- media port capacity computes `16384-65535` for capacity `49152`.
- media port overflow/range validation fails.
- zero media capacities are rejected.
- Config diagnostics flags are retained and runtime propagation keeps SIP
  and media setup diagnostics independent.
- Endpoint JSON maps `fastAutoAcceptIncomingCalls`.

Commands run and passing after the wiring audit:

```bash
cargo fmt --all --check
cargo test -p rvoip-sip --test config_channel_capacity_integration -- --nocapture
cargo test -p rvoip-sip --test config_tests -- --nocapture
cargo test -p rvoip-sip --test fast_auto_accept_integration -- --nocapture
cargo test -p rvoip-sip endpoint_json_config_maps_builder_fields -- --nocapture
cargo check -p rvoip-sip --examples
cargo build --release -p rvoip-sip --example perf_listener
```

## Important Prior Investigation Context

### 5000 CPS Was Clean

The media-enabled `5000 CPS` path was cleaned up before this Config
migration. Final listener state drained and cleanup diagnostics ended
with `active_total=0`.

### 6000 CPS Exposed The Next Knee

A media-enabled `6000 CPS` run still failed acceptance after the invalid
RTP flood was fixed:

- success about `99.0%`
- retransmissions about `3844`
- final in-flight about `929`
- p99 still below `150 ms`
- final cleanup `active_total=0`

That pointed away from a cleanup leak and toward pressure during setup,
socket churn, response timing, or scheduler/runtime contention.

### RTP Port Capacity Was Fixed

The RTP port range had been hard-coded/misleading in perf envs. The new
Config media port capacity API is intended to make this explicit:

```rust
with_media_port_capacity(16_384, 49_152)
```

### Invalid RTP Version Flood Was Fixed Separately

The `Invalid RTP version: 1` issue was diagnosed as non-RTP datagrams
arriving on RTP sockets, not bad SDP negotiation. The RTP receive path
was updated to classify/drop non-RTP/RTCP datagrams instead of parsing
them as RTP or converting them into media events.

That issue is not the current retransmit root cause.

### Removing Automatic 180 Ringing Got To Zero Retransmits

The high-CPS IVR/call-center-style fast-answer path improved sharply
when automatic `180 Ringing` was suppressed. In this benchmark the
server accepts every INVITE immediately, so sending `180` and then `200`
adds one extra outbound UDP response per call. At `8000 CPS`, that is
`120000` extra outbound datagrams over the 15 second steady run.

The Config default remains PBX/compliance-friendly:

```rust
auto_180_ringing = true
```

The high-CPS auto-answer profile intentionally sets:

```rust
auto_180_ringing = false
```

This is appropriate for fast-answer services where the application is
ready to send final `200 OK` immediately.

## How To Reproduce The Current Config-Only Runs

Build:

```bash
cargo build --release -p rvoip-sip --example perf_listener
```

Start the listener without any `RVOIP_*` server behavior env vars:

```bash
target/release/examples/perf_listener 35060 192.168.5.2
```

For Config-owned diagnostics:

```bash
target/release/examples/perf_listener 35060 192.168.5.2 --diagnostics
```

The second argument must be the address SIPp containers can use to reach
the host listener. On this macOS Docker setup, `host.docker.internal`
resolves inside the container to `192.168.5.2`, while it may not resolve
on the host process itself.

Run the Dockerized SIPp harness from the workspace root:

```bash
RVOIP_PERF_RESULTS=crates/rvoip-sip/tests/perf/sipp_scenarios/results/config_only_8000_$(date +%Y%m%d_%H%M%S) \
RVOIP_PERF_CPS=8000 \
RVOIP_PERF_STEADY_SECS=15 \
RVOIP_PERF_SIPP_SHARD_CPS=1000 \
RVOIP_PERF_TRACE_SCREEN=0 \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_comparison_dockerized.sh \
  192.168.5.2 35060 rvoip
```

Query listener state while the listener is still running:

```bash
curl -s http://127.0.0.1:35061/state
```

Stop the listener with SIGINT so final diagnostics print:

```bash
pkill -INT -f 'target/release/examples/perf_listener 35060'
```

## Current Config-Only Results

### Diagnostics Off

Fresh config-audit rerun after removing the remaining env-backed runtime
diagnostic toggles:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_config_audit_8000_20260523_015707
```

Aggregate:

- `TotalCallCreated=120000`
- `SuccessfulCall=120000`
- `FailedCall=0`
- `Retransmissions=4171`
- `Warnings=0`
- `DeadCallMsgs=0`
- response `>=500 ms` buckets `2576`
- host UDP full-socket-buffer drops delta `0`

Listener final state after stop:

- `accepted_total=120000`
- `cleaned_total=120000`
- `in_flight=0`

This confirms config wiring is not the remaining blocker. The residual
problem is first-response latency under the 8000 CPS burst.

### Auto 100 Disabled Rerun

After changing `Config::with_high_cps_udp_auto_answer` to set
`auto_100_trying=false`, the production-style diagnostics-off benchmark
was rerun:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_auto100_off_config_20260523_023519
```

Aggregate SIPp result:

- `TotalCallCreated=120000`
- `SuccessfulCall=120000`
- `FailedCall=0`
- `Retransmissions=3257`
- `Warnings=7`
- `DeadCallMsgs=7`
- response `>=500 ms` buckets `2013`
- host UDP full-socket-buffer drops delta `0`

Listener final state after stop:

- `accepted_total=120000`
- `cleaned_total=120000`
- `in_flight=0`

The warnings/dead messages were late duplicate `200 OK` responses on
already-successful SIPp calls, consistent with recovered retransmits
rather than failed calls.

The matching diagnostics-summary pass was rerun here:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_auto100_off_diag_20260523_023707
```

Aggregate SIPp result:

- `TotalCallCreated=120000`
- `SuccessfulCall=120000`
- `FailedCall=0`
- `Retransmissions=3365`
- `Warnings=7`
- `DeadCallMsgs=7`
- response `>=500 ms` buckets `2075`
- host UDP full-socket-buffer drops delta `0`

Final diagnostics:

- `accepted_total=120000`
- `cleaned_total=120000`
- cleanup `active_total=0`
- SIP UDP `recv=363365`, `queued=363365`, `queue_full=0`,
  `parse_ok=363365`, `parse_err=0`
- SIP UDP `transport_backpressure_events=0`,
  `manager_backpressure_events=0`
- SIP UDP `sends=243379`, `send_errors=0`, `resp_1xx=0`,
  `resp_2xx=243379`
- Duplicate recovery `dup_invite_cache_miss=0`,
  `dup_bye_tombstone_miss=0`

### Fused Fast-200 Path

`Config::with_fast_auto_accept_incoming_calls(true)` now uses an
internal `IncomingCallAutoAccept` state-table event instead of the old
two-step `IncomingCall -> AcceptCall` sequence. The UAS `Idle ->
Answering` transition creates media, stores remote SDP, generates and
negotiates the local SDP answer, and sends `200 OK` without sending
`180 Ringing` and without queueing a second `AcceptCall`.

The inbound INVITE transaction id is stored on the session during setup.
`SendSIPResponse(200)` consumes that exact transaction id when present,
so the final response no longer has to rediscover the pending server
transaction from dialog/session indexes. The handler parses and stores
`raw_request` after the fast 200 path completes, before publishing the
observational app `Event::IncomingCall`.

`sip_udp_diagnostics` also reports a
`first_invite_to_200=[count=... p50_us=... p95_us=... p99_us=... p999_us=... max_us=... over_500ms=...]`
histogram for the fused handler/action segment.

Important semantics:

- `fast_auto_accept_incoming_calls=true` is unconditional auto-answer.
  Application callbacks still run for observation, but they cannot veto the
  call.
- With default config, the normal `IncomingCall` transition still sends
  automatic `180 Ringing` first. For an immediate final answer, pair fast
  accept with `auto_180_ringing=false`.
- The 200 OK is still gated by the normal UAS answer actions:
  `GenerateLocalSDP`, `NegotiateSDPAsUAS`, then `SendSIPResponse(200)`.

The focused integration test passed:

```bash
cargo test -p rvoip-sip --test fast_auto_accept_integration -- --nocapture
```

The original inline fast-accept 8000 CPS listener experiment did not meet
the benchmark target:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_fast_accept_inline_8000_20260523_020958
```

Aggregate:

- `TotalCallCreated=120000`
- `SuccessfulCall=119999`
- `FailedCall=0`
- `CurrentCall=1`
- `Retransmissions=3497`
- `Warnings=1`
- `FatalErrors=1`
- `DeadCallMsgs=1`
- host UDP full-socket-buffer drops delta `0`

Listener final state after stop:

- `accepted_total=120000`
- `cleaned_total=119999`
- `in_flight=1`

The new fused fast-200 path was then benchmarked at 8000 CPS.

Diagnostics-on result:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_fused_fast200_diag_20260523_030156
```

- SIPp `TotalCallCreated=120000`
- SIPp `SuccessfulCall=120000`
- SIPp `FailedCall=0`
- SIPp `Retransmissions=11433`
- SIPp `Warnings=5268`
- SIPp `DeadCallMsgs=5268`
- response `>=500 ms` buckets `7022`
- listener final `accepted_total=120000 cleaned_total=119135`
- SIP UDP `resp_1xx=0`
- duplicate cache misses `0`
- `first_invite_to_200 count=120000 avg_us=187 p50_us=250 p95_us=500 p99_us=500 p999_us=1000 max_us=10726 over_500ms=0`
- host UDP full-socket-buffer drops delta `0`

Diagnostics-off result:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_fused_fast200_prod_20260523_030413
```

- SIPp `TotalCallCreated=120000`
- SIPp `SuccessfulCall=120000`
- SIPp `FailedCall=0`
- SIPp `Retransmissions=71566`
- SIPp `Warnings=20775`
- SIPp `DeadCallMsgs=20775`
- response `>=500 ms` buckets `25714`
- listener final `accepted_total=120000 cleaned_total=117787`
- host UDP full-socket-buffer drops delta `0`

This path proves the handler/action work after the session event starts is
fast, but it is not stable enough to make it the high-CPS profile default.
The remaining latency is earlier than the fused state-machine action, or
outside the server after send completion. The cleanup gap also means the
next pass must account for missing ACK/BYE observation under fast auto
answer.

### Lossless Fast 200 OK And Cleanup Wiring Rerun

After replacing session-core's internal `dialog_to_session` broadcast
subscription with a direct registered sharded dispatcher, the fast path
was benchmarked again. `perf_listener` now has an explicit
`--fast-auto-accept` flag so this path can be validated without changing
the high-CPS profile default.

Diagnostics-on result:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_lossless_fast200_diag_20260523_033611
```

- SIPp `TotalCallCreated=120000`
- SIPp `SuccessfulCall=120000`
- SIPp `FailedCall=0`
- SIPp `Retransmissions=1780`
- SIPp `Warnings=0`
- SIPp `DeadCallMsgs=0`
- listener final `accepted_total=120000 cleaned_total=120000`
- SIP UDP `resp_1xx=0`
- duplicate cache misses `0`
- ACK matched/delivered `120000/120000`
- BYE `200 OK` sent `120000`
- BYE cleanup emitted/delivered `120000/120000`
- BYE cleanup missing `0`
- `first_invite_to_200 count=120000 avg_us=137 p50_us=250 p95_us=250 p99_us=500 p999_us=500 max_us=10545 over_500ms=0`
- `dialog_to_session_queue count=360000 avg_us=11 p50_us=10 p95_us=50 p99_us=250 p999_us=500 max_us=9160 over_500ms=0`
- host UDP full-socket-buffer drops delta `0`

Diagnostics-off result with forced fast auto-accept:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_lossless_fast200_prod_20260523_033753
```

- SIPp `TotalCallCreated=120000`
- SIPp `SuccessfulCall=120000`
- SIPp `FailedCall=0`
- SIPp `Retransmissions=4295`
- SIPp `Warnings=0`
- SIPp `DeadCallMsgs=0`
- listener final `accepted_total=120000 cleaned_total=120000`
- host UDP full-socket-buffer drops delta `0`

Diagnostics-off control with the current high-CPS profile default
(`fast_auto_accept_incoming_calls=false`):

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/codex_lossless_default_prod_20260523_033927
```

- SIPp `TotalCallCreated=120000`
- SIPp `SuccessfulCall=120000`
- SIPp `FailedCall=0`
- SIPp `Retransmissions=4545`
- SIPp `Warnings=3`
- SIPp `DeadCallMsgs=3`
- listener final `accepted_total=120000 cleaned_total=120000`
- host UDP full-socket-buffer drops delta `0`

The direct dispatcher fixes the fast-path cleanup loss. It does not yet
meet the `0` retransmit target, so the high-CPS profile should remain
`fast_auto_accept_incoming_calls=false` until true request-receive to
response-send timing explains the remaining INVITE/BYE duplicate window.

Result directory:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/config_only_8000_20260523_003707
```

Aggregate SIPp result:

- `TotalCallCreated=120000`
- `SuccessfulCall=120000`
- `FailedCall=0`
- `Retransmissions=5395`
- `Warnings=0`
- `DeadCallMsgs=0`
- p99 bucket `<1000 ms`
- host UDP full-socket-buffer drops delta `0`

Listener final state before stop:

- `accepted_total=120000`
- `cleaned_total=120000`
- `in_flight=0`

### Diagnostics On

Result directory:

```text
crates/rvoip-sip/tests/perf/sipp_scenarios/results/config_only_diag_8000_20260523_004058
```

Aggregate SIPp result:

- `TotalCallCreated=120000`
- `SuccessfulCall=120000`
- `FailedCall=0`
- `Retransmissions=2990`
- `Warnings=5`
- `DeadCallMsgs=5`
- p99 bucket `<1000 ms`
- host UDP full-socket-buffer drops delta `0`

Per-shard retransmissions:

| Shard | Retransmits | Warnings/Dead |
| --- | ---: | ---: |
| s0 | `131` | `0/0` |
| s1 | `416` | `0/0` |
| s2 | `313` | `0/0` |
| s3 | `603` | `0/0` |
| s4 | `341` | `3/3` |
| s5 | `558` | `2/2` |
| s6 | `65` | `0/0` |
| s7 | `563` | `0/0` |

Final listener state:

- `accepted_total=120000`
- `cleaned_total=120000`
- `in_flight=0`

Media setup diagnostics:

- `start_avg_us=110.3`
- `start_max_us=10311`
- `rtp_port_avg_us=1.1`
- `rtp_port_max_us=234`
- `rtp_session_avg_us=106.5`
- `rtp_session_max_us=10306`
- `stop_avg_us=35.1`
- `stop_max_us=393`
- `port_release_avg_us=0.9`
- `port_release_max_us=145`

SIP UDP diagnostics:

- `recv=362990`
- `queued=362990`
- `queue_full=0`
- `parse_ok=362990`
- `parse_err=0`
- `transport_backpressure_events=0`
- `manager_backpressure_events=0`
- `sends=243038`
- `send_errors=0`
- `resp_1xx=0`
- `resp_2xx=243038`
- send latency buckets:
  - `<100us=242705`
  - `<500us=330`
  - `<1ms=2`
  - `<5ms=1`
  - `<10ms=0`
  - `>=10ms=0`

Duplicate recovery diagnostics:

- `dup_invite_existing_tx=1869`
- `dup_invite_cache_hit=1869`
- `dup_invite_cache_miss=0`
- `dup_bye_tombstone_hit=1121`
- `dup_bye_tombstone_miss=0`
- `invite_2xx_ack_removed=119998`
- `invite_2xx_ack_avg_ms=35.748`

Important observation:

```text
dup_invite_cache_hit + dup_bye_tombstone_hit = 1869 + 1121 = 2990
```

That exactly matches the SIPp retransmission count for the diagnostics
run. The retransmits are being recovered by indexed/cache paths. They
are not creating new sessions, new media, or cleanup leaks.

## What The Diagnostics Rule Out

Current data does not support these as the remaining root cause:

- automatic `180 Ringing`: `resp_1xx=0`.
- kernel UDP receive drops on the host: full-socket-buffer drop delta `0`.
- UDP parse queue saturation: `queue_full=0`.
- transport/manager backpressure: both backpressure counters `0`.
- SIP parse failures: `parse_err=0`.
- send syscall latency: nearly all sends completed under `100 us`.
- missing duplicate indexes: duplicate INVITE/BYE cache misses `0`.
- media port allocator slowness: `rtp_port_avg_us=1.1`, max `234 us`.
- media teardown leak: listener drains to `accepted_total == cleaned_total`.
- cleanup leak: final cleanup active count is `0`.

Current data still leaves these possibilities open:

- queue/scheduler delay before the first final INVITE response is built.
- queue/scheduler delay before BYE `200 OK` is sent.
- transaction/dialog manager event-loop latency not captured by current
  send-latency counters.
- app/session-event path delay before auto-answer reaches
  `send_response_for_session`.
- SIPp container receive scheduling or Docker networking effects.
- packet delivery timing where the server sends quickly, but SIPp does
  not receive/process before its `500 ms` retransmit timer.

## Why The Existing Diagnostics Are Not Enough

The current transport send latency bucket measures the outbound send
operation once the response is already being sent. It does not measure
the full time from inbound request arrival to response send completion.

For the remaining retransmits, the next needed measurement is:

```text
UDP datagram received
  -> parsed
  -> queued to transport/transaction/dialog/session
  -> application auto-answer decision
  -> final response built
  -> response send completed
```

split by method and response:

- INVITE to first `200 OK`.
- retransmitted INVITE to cached `200 OK`.
- BYE to `200 OK`.
- retransmitted BYE to tombstone/cache `200 OK`.

Without that end-to-end per-request latency histogram, we cannot tell
whether the remaining tail is inside rvoip or outside it.

## Recommended Next Work

### 1. Add Config-Gated Response Timing Diagnostics

Add counters/histograms behind `Config::with_sip_udp_diagnostics(true)`
for:

- inbound UDP receive timestamp.
- parse queue wait.
- transaction/dialog manager queue wait.
- INVITE first-response latency from receive to send completion.
- INVITE cached-response latency from receive to send completion.
- BYE first-response latency from receive to send completion.
- BYE tombstone-response latency from receive to send completion.
- p50/p95/p99/p99.9/max for each path.
- counts over `500 ms` for each path.

The key acceptance question is simple:

```text
Do any first INVITE 200 OK or BYE 200 OK responses leave rvoip after 500 ms?
```

If yes, optimize the internal path. If no, investigate SIPp/container
receive timing and network delivery.

### 2. Capture A macOS sample During The Active Window

Run one diagnostic `8000 CPS` listener and sample for about 10 seconds
during the 15 second steady phase:

```bash
target/release/examples/perf_listener 35060 192.168.5.2 --diagnostics
```

In another shell after SIPp starts:

```bash
pid=$(pgrep -f 'target/release/examples/perf_listener 35060')
sample "$pid" 10 -file /tmp/rvoip_config_only_8000.sample.txt
```

Look specifically for:

- transaction manager contention.
- dialog lookup/cache contention.
- session event-handler queueing.
- auto-answer path work before final `200 OK`.
- BYE handler work before `200 OK`.
- Tokio scheduler/runtime overhead.
- allocator pressure.
- media setup unexpectedly reappearing in hot stacks.

### 3. If Server Response Latency Exceeds 500 ms

Optimize the measured path, not the SIPp scenario. Likely candidates:

- direct fast-answer path for the high-CPS auto-answer profile.
- fewer task/queue hops before INVITE `200 OK`.
- ensure BYE `200 OK` is emitted before cleanup work, not after.
- avoid any broad session/dialog scans in the first-response path.
- keep duplicate INVITE/BYE indexed-cache behavior unchanged.

Do not reintroduce broad `server_call_capacity` inflation as a media
capacity workaround.

### 4. If Server Response Latency Stays Low

If the new histograms prove rvoip sends all first responses well below
`500 ms`, collect evidence outside the server:

- packet capture on host and container interfaces.
- SIPp per-shard receive timing.
- Docker networking scheduling/timing.
- CPU utilization and runnable-thread pressure inside the SIPp
  containers.

Only make SIPp or socket-buffer changes after proving the server sends
responses before the retransmit timer.

## Useful Code Pointers

### Config And Coordinator

- `crates/rvoip-sip/src/api/unified.rs`
  - `Config`
  - `Config::with_high_cps_udp_auto_answer`
  - `Config::with_media_port_capacity`
  - `Config::with_media_session_capacity`
  - `Config::with_sip_udp_diagnostics`
  - `Config::with_media_setup_diagnostics`
  - `UnifiedCoordinator::new`
  - `UnifiedCoordinator::create_media_controller`

### Public API Builders

- `crates/rvoip-sip/src/api/builder.rs`
- `crates/rvoip-sip/src/api/stream_peer.rs`
- `crates/rvoip-sip/src/api/callback_peer.rs`

### Listener And Perf Harness

- `crates/rvoip-sip/examples/perf_listener.rs`
- `crates/rvoip-sip/tests/perf/perf_call_setup_cps.rs`
- `crates/rvoip-sip/tests/perf/sipp_scenarios/run_comparison_dockerized.sh`
- `crates/rvoip-sip/tests/perf/sipp_scenarios/uac_perf.xml`

`uac_perf.xml` has `retrans="500"` on INVITE/BYE sends. That is why any
server-side response tail over `500 ms` appears as a retransmit.

### Diagnostics

- `crates/rvoip-sip-transport/src/diagnostics.rs`
- `crates/rvoip-sip-dialog/src/diagnostics.rs`
- `crates/media-core/src/diagnostics.rs`

### BYE Handling

- `crates/rvoip-sip/src/state_table/wiring_manifest.rs`
  - documents that dialog-core sends BYE `200 OK`; session-core only
    cleans up.
- `crates/rvoip-sip-dialog/src/manager/protocol_handlers.rs`
- `crates/rvoip-sip-dialog/src/manager/transaction_integration.rs`

The next response-latency instrumentation should confirm whether BYE
`200 OK` is actually sent before expensive cleanup under `8000 CPS`
load.

## Things Not To Change Blindly

- Do not enlarge UDP socket buffers by default. Previous large-buffer
  testing worsened results.
- Do not use `server_call_capacity` as a catch-all capacity knob for
  media. It also changes dialog/transaction index sizing.
- Do not change SIPp retransmit timers to hide the problem.
- Do not disable media for the target run. The current target is
  media-enabled `8000 CPS`.
- Do not remove duplicate caches. Current diagnostics show they are
  working and prevent retransmits from creating duplicate sessions.

## Handoff Summary

The Config migration is now real enough that the listener starts from
public Config and no longer reads the old fast-path envs. The server is
correct under load: all calls succeed, all cleanup drains, UDP queues do
not back up, media allocation is fast, and duplicate retransmits are
handled by indexed caches.

The remaining gap is performance, not correctness: restoring `0`
retransmits requires finding why a subset of first INVITE/BYE responses
still miss SIPp's `500 ms` retransmit timer after the env-to-Config
migration. The next decisive change is end-to-end request-to-response
latency diagnostics, Config-gated through `with_sip_udp_diagnostics`,
followed by a sampled `8000 CPS` diagnostic run.
