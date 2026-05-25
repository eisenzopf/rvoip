# Signaling Sharding Performance Experiment

## Goal

Measure which Config-owned sharding and queue knobs move the
signaling-only SIP path after the clean `8000 CPS` baseline. The current
interesting range is `18000 CPS` to `20000 CPS`: `10000 CPS` is clean
across transaction worker counts, `18000 CPS` shows transaction fanout
benefit, and `20000 CPS` exposes another saturation mode.

This experiment intentionally keeps media in signaling-only mode. The
media-enabled path is dominated by RTP allocation/socket bind work and
needs a separate media fast-path investigation.

## Config-Owned Knobs

Every server knob used by the runner maps to a public `Config` field and
builder method. The SIPp harness still uses environment variables for
load generation, but the rvoip server process does not rely on hidden
perf-only behavior.
See [`TUNING.md`](TUNING.md) for the canonical workload recipes,
default policy, and crosswalk between `Config`, `perf_listener` flags,
and SIPp matrix environment variables.

| Runner flag | Config API |
| --- | --- |
| `--high-cps-capacity N` | `Config::with_high_cps_udp_auto_answer(N)` |
| `--fast-auto-accept` | `Config::with_fast_auto_accept_incoming_calls(true)` |
| `--signaling-only-media` | `Config::with_signaling_only_media(9)` |
| `--udp-parse-workers N` | `Config::with_sip_udp_parse_workers(N)` |
| `--udp-parse-queue-capacity N` | `Config::with_sip_udp_parse_queue_capacity(N)` |
| `--udp-parse-round-robin` | `Config::with_sip_udp_parse_dispatch(UdpParseDispatch::RoundRobin)` |
| `--sip-udp-recv-buffer-size N` | `Config::with_sip_udp_recv_buffer_size(N)` |
| `--sip-udp-send-buffer-size N` | `Config::with_sip_udp_send_buffer_size(N)` |
| `--sip-transport-channel-capacity N` | `Config::with_sip_transport_channel_capacity(N)` |
| `--sip-transport-dispatch-workers N` | `Config::with_sip_transport_dispatch_workers(N)` |
| `--sip-transport-dispatch-queue-capacity N` | `Config::with_sip_transport_dispatch_queue_capacity(N)` |
| `--transaction-event-channel-capacity N` | `Config::with_transaction_event_channel_capacity(N)` |
| `--transaction-dispatch-workers N` | `Config::with_sip_transaction_dispatch_workers(N)` |
| `--transaction-dispatch-queue-capacity N` | `Config::with_sip_transaction_dispatch_queue_capacity(N)` |
| `--transaction-dispatch-priority-burst-max N` | `Config::with_sip_transaction_dispatch_priority_burst_max(N)` |
| `--invite-2xx-retransmit-max-due-per-tick N` | `Config::with_sip_invite_2xx_retransmit_max_due_per_tick(N)` |
| `--sip-dialog-dispatch-workers N` | `Config::with_sip_dialog_dispatch_workers(N)` |
| `--sip-dialog-dispatch-queue-capacity N` | `Config::with_sip_dialog_dispatch_queue_capacity(N)` |
| `--session-event-dispatcher-workers N` | `Config::with_session_event_dispatcher_workers(N)` |
| `--session-event-dispatcher-queue-capacity N` | `Config::with_session_event_dispatcher_channel_capacity(N)` |
| `--transaction-timing-diagnostics` | `Config::with_sip_transaction_timing_diagnostics(true)` |
| `--dialog-timing-diagnostics` | `Config::with_sip_dialog_timing_diagnostics(true)` |

Defaults remain client-safe: UDP parse dispatch is source-hash, SIP
transport dispatch is a single bridge, and transaction dispatch is the
original single receive/handle loop unless a server Config opts in.
Dialog dispatch is also a single processor by default. Values above `1`
use keyed sharding by call identity/transaction key; raw round-robin is
still limited to UDP parse experiments.

## Runner

Use:

```bash
crates/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh \
  host.docker.internal 192.168.5.2 39460
```

The runner starts one `target/release/examples/perf_listener` process
per matrix point, drives the existing Docker SIPp harness, stops the
listener, and writes a combined `summary.md` with `analyze.py`.
When the advertised address is omitted, the runner attempts to resolve
`host.docker.internal` from inside the SIPp Docker network and uses that
IP for the listener's advertised SIP/SDP address.

Default matrix:

```bash
RVOIP_SHARDING_CPS_LEVELS="18000"
RVOIP_SHARDING_UDP_WORKERS="4"
RVOIP_SHARDING_TRANSPORT_WORKERS="1"
RVOIP_SHARDING_TRANSACTION_WORKERS="1 2 4 8"
RVOIP_SHARDING_DIALOG_WORKERS="1"
RVOIP_SHARDING_CAPACITIES="20000"
```

Useful expansions:

```bash
# Recheck the 18k breakpoint and the 20k saturation point.
RVOIP_SHARDING_CPS_LEVELS="18000 20000" \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh

# Add the new SIP transport-manager forwarding pool.
RVOIP_SHARDING_TRANSPORT_WORKERS="1 2 4" \
RVOIP_SHARDING_TRANSACTION_WORKERS="2 4 8" \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh

# Add dialog-core transaction-event fanout after transaction ingress is clean.
RVOIP_SHARDING_TRANSACTION_WORKERS="4" \
RVOIP_SHARDING_DIALOG_WORKERS="1 2 4 8" \
RVOIP_SHARDING_DIALOG_TIMING=1 \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh

# Capacity sweep with all channel defaults derived from the high-CPS profile.
RVOIP_SHARDING_CAPACITIES="20000 30000 40000" \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh

# Pin specific queues when testing whether one queue is the pressure point.
RVOIP_SHARDING_SIP_TRANSPORT_CHANNEL_CAPACITY=300000 \
RVOIP_SHARDING_TRANSACTION_EVENT_CHANNEL_CAPACITY=300000 \
RVOIP_SHARDING_TRANSACTION_DISPATCH_QUEUE_CAPACITY=300000 \
RVOIP_SHARDING_DIALOG_DISPATCH_QUEUE_CAPACITY=300000 \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh

# Raise kernel UDP socket buffers when host full-socket drops appear.
RVOIP_SHARDING_SIP_UDP_RECV_BUFFER_SIZE=8388608 \
RVOIP_SHARDING_SIP_UDP_SEND_BUFFER_SIZE=8388608 \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh

# Sweep ACK/BYE priority fairness and INVITE 2xx proactive resend pacing.
RVOIP_SHARDING_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX=64 \
RVOIP_SHARDING_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK=2048 \
RVOIP_SHARDING_TRANSACTION_TIMING=1 \
RVOIP_SHARDING_DIALOG_TIMING=1 \
crates/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh
```

If `target/release/examples/perf_listener` is missing, the runner builds
it. Set `RVOIP_SHARDING_BUILD=1` to force a rebuild.

## Current Baseline

Known signaling-only results from
`CONFIG_ONLY_8000_CPS_INVESTIGATION.md`:

| Profile | SIPp result | Achieved CPS | Retransmits | Cleanup |
| --- | ---: | ---: | ---: | ---: |
| `8000 CPS`, UDP RR 4 + tx 4 | `120000/120000` | `8000.0` | `0` | `120000/120000` |
| `10000 CPS`, UDP RR 4 + tx 1/2/4/8 | `150000/150000` | `10000.0` | `0` | `150000/150000` |
| `18000 CPS`, UDP RR 4 + tx 1 | `270000/270000` | `16875.0` | `25419` | `270000/270000` |
| `18000 CPS`, UDP RR 4 + tx 2 | `270000/270000` | `16875.0` | `1240` | `270000/270000` |
| `18000 CPS`, UDP RR 4 + tx 4 repeat | `270000/270000` | `16875.0` | `3929` | `270000/270000` |
| `18000 CPS`, UDP RR 4 + tx 8 | `270000/270000` | `16875.0` | `1298` | `270000/270000` |
| `20000 CPS`, UDP RR 4 + tx 4 | `285000/285000` | `10961.5` | `482051` | listener cleaned `295497` |

Interpretation: transaction sharding is real signal at `18000 CPS`, but
`20000 CPS` is not fixed by transaction fanout alone. The next matrix
should test whether the transport-manager forwarding pool and queue
capacity changes remove pressure before transaction ingress.

## 2026-05-23 18k Transport/Transaction Matrix

Results:
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/signaling_sharding_18k_20260523_143944`.

Config:

- `--fast-auto-accept --diagnostics --signaling-only-media`
- `--high-cps-capacity 20000`
- `--udp-parse-workers 4 --udp-parse-round-robin`
- `--sip-transport-dispatch-workers 1/2/4`
- `--transaction-dispatch-workers 1/2/4/8`

| Transport workers | Transaction workers | SIPp result | Achieved CPS | Avg INVITE->200 | Retrans | Host UDP drops | Cleanup |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | `270000/270000` | `16875.0` | `47.4 ms` | `8030` | `0` | `270000/270000` |
| 1 | 2 | `270000/270000` | `15882.4` | `157.8 ms` | `59450` | `0` | `270000/270000` |
| 1 | 4 | `270000/270000` | `16875.0` | `52.4 ms` | `11968` | `0` | `270000/270000` |
| 1 | 8 | `269998/270000` | `3600.0` | `180.9 ms` | `78418` | `8513` | `270000/269998` |
| 2 | 1 | `270000/270000` | `16875.0` | `33.0 ms` | `7601` | `0` | `270000/270000` |
| 2 | 2 | `270000/270000` | `15882.4` | `85.2 ms` | `16994` | `0` | `270000/270000` |
| 2 | 4 | `270000/270000` | `15882.4` | `42.9 ms` | `11707` | `0` | `270000/270000` |
| 2 | 8 | `270000/270000` | `15882.4` | `157.1 ms` | `66866` | `6358` | `270000/269993` |
| 4 | 1 | `270000/270000` | `15882.4` | `38.4 ms` | `12552` | `0` | `270000/270000` |
| 4 | 2 | `270000/270000` | `16875.0` | `32.1 ms` | `10812` | `0` | `270000/270000` |
| 4 | 4 | `270000/270000` | `15000.0` | `148.7 ms` | `58146` | `0` | `270000/270000` |
| 4 | 8 | `269998/270000` | `3600.0` | `149.8 ms` | `64470` | `9984` | `270000/269996` |

Best clean point in this run was transport `2`, transaction `1`:
`270000/270000`, cleanup `270000/270000`, host UDP drops `0`,
retransmits `7601`. Transport fanout `2` helped slightly over the
single transport bridge at the same transaction worker count, but raw
transaction worker count did not monotonically improve this run.

The `tx=8` points were not candidates: two failed SIPp completion and
all showed either host UDP full-socket drops or cleanup lag.

## 2026-05-23 18k Matrix With 4 MB UDP Socket Buffers

Results:
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/signaling_sharding_18k_sockbuf_20260523_144905`.

Additional Config-backed flags:

- `--sip-udp-recv-buffer-size 4194304`
- `--sip-udp-send-buffer-size 4194304`

| Transport workers | Transaction workers | SIPp result | Achieved CPS | Avg INVITE->200 | Retrans | Host UDP drops | Cleanup |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | `270000/270000` | `15000.0` | `207.2 ms` | `78801` | `0` | `270000/270000` |
| 1 | 2 | `269997/270000` | `3600.0` | `143.9 ms` | `51308` | `0` | `270000/269997` |
| 1 | 4 | `270000/270000` | `16875.0` | `45.9 ms` | `12259` | `0` | `270000/270000` |
| 1 | 8 | `270000/270000` | `13500.0` | `193.3 ms` | `74450` | `294` | `270000/270000` |
| 2 | 1 | `269999/270000` | `3600.0` | `80.7 ms` | `17187` | `0` | `270000/269999` |
| 2 | 2 | `270000/270000` | `16875.0` | `42.7 ms` | `9921` | `0` | `270000/270000` |
| 2 | 4 | `269999/270000` | `3600.0` | `163.4 ms` | `66118` | `0` | `270000/269999` |
| 2 | 8 | `270000/270000` | `15000.0` | `192.5 ms` | `76007` | `169` | `270000/270000` |
| 4 | 1 | `270000/270000` | `15882.4` | `33.0 ms` | `11187` | `0` | `270000/270000` |
| 4 | 2 | `270000/270000` | `16875.0` | `31.7 ms` | `9774` | `0` | `270000/270000` |
| 4 | 4 | `269995/270000` | `3599.9` | `91.6 ms` | `17639` | `0` | `270000/269995` |
| 4 | 8 | `269996/270000` | `3599.9` | `148.2 ms` | `59700` | `0` | `270000/269996` |

Socket buffers reduced or eliminated host UDP drops, but did not make
the 18k matrix consistently better. The best clean points were
transport `4`, transaction `2` (`9774` retransmits) and transport `2`,
transaction `2` (`9921` retransmits). This suggests kernel UDP receive
space is a useful safety knob, but the remaining retransmits are mostly
application scheduling/timing or load-generator pacing effects.

## 2026-05-23 Focused Repeats And 20k Candidate Runs

Focused `18000 CPS` repeats were run to avoid overfitting one noisy
matrix:

| Profile | SIPp result | Achieved CPS | Avg INVITE->200 | Retrans | Host UDP drops | Cleanup |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| UDP RR 4, transport 1, transaction 1 | `270000/270000` | `16875.0` | `50.8 ms` | `15451` | `0` | `270000/270000` |
| UDP RR 4, transport 2, transaction 1 | `270000/270000` | `16875.0` | `50.6 ms` | `13142` | `0` | `270000/270000` |
| UDP RR 4, transport 4, transaction 2 | `270000/270000` | `16875.0` | `48.7 ms` | `13734` | `0` | `270000/270000` |

The repeat winner was transport `2`, transaction `1`, but the margin was
small and retransmits were higher than the original `7601` result. The
main conclusion is still conservative: `18000 CPS` can be kept clean,
but this setup is noisy enough that no new default should be promoted
from one matrix.

The repeat winner was then tested at `20000 CPS`:

| Profile | SIPp result | Achieved CPS | Avg INVITE->200 | Retrans | Host UDP drops | Cleanup |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| UDP RR 4, transport 2, transaction 1 | `284999/285000` | `3800.0` | `1549.7 ms` | `456188` | `0` | listener `296540/295430` |
| UDP RR 4, transport 2, transaction 4 | `284994/285000` | `3799.9` | `2214.5 ms` | `543008` | `0` | listener `296994/295761` |

Both `20000 CPS` runs failed without host UDP drops. For transaction
`1`, `transport_manager_to_transaction` still showed queue delay
(`p99=100000 us`, `p999=250000 us`). For transaction `4`, that delay
dropped to `p99=250 us`, `p999=1000 us`, but SIPp performance got
worse. In the transaction `4` run, the wider receive-to-dialog spans
were the signal:

- `udp_receive_to_incoming_call_emit avg_us=786974`, `p50=1000000 us`,
  `over_500ms=161341`.
- `bye_receive_to_200 avg_us=303955`, `p50=250000 us`,
  `over_500ms=110030`.
- `first_invite_to_200` remained tiny (`avg_us=83`, `p99=500`), which
  confirms that timer starts too late to represent true SIPp PDD at this
  scale.

Conclusion: transaction sharding helps remove one queue, but at `20000
CPS` the bottleneck shifts downstream into the transaction-to-dialog /
dialog incoming-call path. Dialog-core now has the next instrumentation
and Config-owned keyed dispatch knobs, so the next matrix should compare
`--sip-dialog-dispatch-workers 1/2/4/8` with
`--dialog-timing-diagnostics`. Do not add raw round-robin at the session
layer; dialog fanout must preserve per-call ordering.

## 2026-05-23 18k Dialog Dispatch Matrix

Results:
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/dialog_sharding_18k_session4_20260523_162910`.

Config:

- `--fast-auto-accept --diagnostics --signaling-only-media`
- `--high-cps-capacity 20000`
- `--udp-parse-workers 4 --udp-parse-round-robin`
- `--sip-transport-dispatch-workers 1/2/4`
- `--transaction-dispatch-workers 1/2/4`
- `--sip-dialog-dispatch-workers 1/2/4/8`
- `--transaction-timing-diagnostics --dialog-timing-diagnostics`
- `--session-event-dispatcher-workers 4`
- `--session-event-dispatcher-queue-capacity 300000`

Best clean points from the 36-point matrix:

| Transport workers | Transaction workers | Dialog workers | SIPp result | Achieved CPS | Avg INVITE->200 | Retrans | Host UDP drops | Cleanup |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 2 | 1 | `270000/270000` | `16875.0` | `39.4 ms` | `6054` | `0` | `270000/270000` |
| 1 | 4 | 1 | `270000/270000` | `16875.0` | `34.1 ms` | `6354` | `0` | `270000/270000` |
| 1 | 1 | 8 | `270000/270000` | `18000.0` | `10.1 ms` | `6695` | `0` | `270000/270000` |
| 2 | 1 | 1 | `270000/270000` | `16875.0` | `46.6 ms` | `10966` | `0` | `270000/270000` |
| 4 | 1 | 4 | `270000/270000` | `16875.0` | `14.3 ms` | `11177` | `0` | `270000/270000` |

The clean definition here includes SIPp completion, listener accepted and
cleanup counts, duplicate cache misses `0`, UDP queue full `0`, and host
UDP full-socket drops `0`.

Dialog fanout helped one specific shape: transport `1`, transaction `1`,
dialog `8` reached the full requested `18000 CPS` with `6695`
retransmits, compared with `55107` retransmits for the same
transport/transaction shape at dialog `1`. It did not compose reliably
with transaction fanout. Several transaction+dialog fanout combinations
had `dup_invite_cache_miss > 0`, and the over-fanned transport `2` /
transaction `2` / dialog `8` and transport `2` / transaction `4` /
dialog `4` points produced host UDP full-socket drops.

A four-point no-timing repeat was then run to check whether the timing
instrumentation changed the ordering:
`crates/rvoip-sip/tests/perf/sipp_scenarios/results/dialog_sharding_18k_session4_notiming_20260523_164557`.

| Transport workers | Transaction workers | Dialog workers | SIPp result | Achieved CPS | Avg INVITE->200 | Retrans | Host UDP drops | Cleanup | Notes |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 1 | 1 | 1 | `269999/270000` | `3600.0` | `101.8 ms` | `28391` | `0` | `270000/269999` | stalled one call |
| 1 | 1 | 8 | `270000/270000` | `14210.5` | `38.1 ms` | `30571` | `0` | `270000/270000` | clean but not repeat-best |
| 1 | 2 | 1 | `270000/270000` | `16875.0` | `22.8 ms` | `7488` | `0` | `270000/270000` | repeat winner |
| 1 | 2 | 8 | `270000/270000` | `15882.4` | `59.1 ms` | `41239` | `0` | `270000/270000` | duplicate cache miss `1` |

Conclusion: keep dialog dispatch at `1` for now. The best candidate in
this session-dispatcher-4 run is UDP parse round-robin `4`, transport
dispatch `1`, transaction dispatch `2`, and dialog dispatch `1`. Dialog
dispatch `8` is worth keeping as an experimental knob because it can
eliminate the dialog bottleneck in the transaction-1 shape, but it is not
stable enough to promote. The duplicate cache misses when transaction
and dialog fanout are both enabled need investigation before dialog
dispatch is treated as a production tuning knob.

The follow-up code target is documented in
`DIALOG_CORE_HOT_PATH_INVESTIGATION_PLAN.md`. The short version: measure
dialog route-affinity splits, transaction termination cleanup churn, and
INVITE 2xx cache scan cost before adding more worker combinations.

## Acceptance Criteria

For a candidate profile:

- SIPp calls complete at the target count.
- Listener cleanup matches accepted calls.
- warnings/dead messages stay `0/0`.
- ACK matched/delivered matches total calls.
- BYE 200 and BYE cleanup delivered match total calls.
- duplicate cache misses stay `0`.
- UDP queue-full and host UDP full-socket drops stay `0`.
- Retransmits improve over the previous best clean run at the same CPS.

Do not promote new defaults from one run. Repeat the best candidate and
compare against the default single transport/transaction dispatch profile
on the same machine and Docker network.
