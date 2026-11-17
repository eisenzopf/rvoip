# rvoip-bench-uas

The Device Under Test of rvoip's RFC 7501/7502 SIP benchmark release gate
(`RFC7502_BENCHMARK.md` §3). A terminating SIP UAS and media endpoint built
on rvoip's public API. Not published (`publish = false`).

```
rvoip-bench-uas --config <dut.json>
```

The benchmark orchestrator (`scripts/bench/`) starts a fresh DUT per trial,
waits for `ready.json`, runs the caller, scrapes `/metrics` and tails
`stats.jsonl`, then creates `<run_dir>/finish`. The DUT stops taking calls,
waits for the calls it holds to end and be cleaned up, writes `summary.json`
and exits. The orchestrator archives `run_dir` with the trial evidence.

## Behaviour

- Answers every INVITE with 200 OK, on rvoip's fast auto-accept path (the
  200 OK is sent on the INVITE's own path, before any application callback;
  no 100 Trying, no 180 Ringing). The SDP answer is `RTP/AVP` for `rtp` and
  `none`, `RTP/SAVP` with one `a=crypto` (the answer key, RFC 4568) for
  `srtp`, and carries `a=rtcp-mux` when the offer does.
- `media: none` answers SDP on port 9 and allocates no RTP socket.
- With media, every inbound RTP media frame is echoed to the caller's
  (latched) RTP address: rvoip receives the packet (SRTP: authenticates and
  decrypts it), hands the G.711 payload to the echo, which writes it back;
  rvoip packetizes it and (SRTP) encrypts it with the answer key. RTCP is
  muxed on the RTP port and is rvoip's own. The caller's 440 Hz tone comes
  back intact; SSRC, sequence numbers and timestamps are the DUT's own
  stream, as from any endpoint.
- Answers BYE with 200 OK (rvoip's dialog layer); absorbs ACK.
- Admission `off` never sheds. `max-cps` is a GCRA with a one-second burst
  (`max_cps` calls back to back) on rvoip's stateless new-call gate: excess
  INVITEs get `503 Service Unavailable` with `Retry-After:
  retry_after_secs`, before any transaction, dialog or session exists (on the
  TLS connection reader for TLS, on the transaction dispatch path for UDP).
- `max_concurrent_calls` is a hard cap: INVITEs beyond it get a stateless 503
  with `Retry-After` in the transaction layer.
- Drain: when `<run_dir>/finish` appears (or on SIGINT), the gate closes
  (every new INVITE gets 503 with `Retry-After`; in-dialog ACK/BYE still
  pass), and the DUT waits until active calls, pending setups, pending
  cleanups, open dialogs and RTP port reservations are all zero, or
  `drain_timeout_secs` passes. It then writes `summary.json` and exits 0, or
  3 if the drain timed out.

## Configuration (`dut.json`)

Unknown fields are rejected at every level.

| Field | Default | Meaning |
|---|---|---|
| `run_dir` | required | Run files and the `finish` trigger; created if missing. |
| `transport` | required | `udp` or `tls`. |
| `bind` | required | SIP listen address (a real port). For `tls` the TLS listener owns this TCP port and rvoip's plain UDP/TCP SIP listeners take an ephemeral port. |
| `advertise_ip` | required | Contact and SDP `c=`/`o=` address; same family as `bind`. |
| `tls_cert`, `tls_key` | required for `tls` | PEM server chain and key. Ignored for `udp`. |
| `media` | required | `none`, `rtp` or `srtp`. |
| `rtcp_mux` | `true` | `true`: answer mux when offered. `false`: rvoip still answers `a=rtcp-mux` when offered (it has no answer-side opt-out), but sends RTCP from port + 1 to peers that decline mux. |
| `rtp_port_range` | `[10000, 64999]` | Inclusive RTP port range; must not contain the SIP port. |
| `admission` | `{"mode": "off"}` | Or `{"mode": "max-cps", "max_cps": N, "retry_after_secs": S}` (`S` default 2). |
| `max_concurrent_calls` | `60000` | Hard cap (stateless 503). |
| `metrics_listen` | `0.0.0.0:9464` | Prometheus address; `null` disables; port 0 binds an ephemeral port reported in `ready.json`. |
| `stats_interval_ms` | `1000` | `stats.jsonl` interval (min 50). |
| `drain_timeout_secs` | `120` | Longest drain. |
| `worker_threads` | `null` | Signaling (Tokio) workers; `null`: CPUs minus media lanes, at least 2. |
| `tuning` | `{}` | Optional performance knobs, below. |

`tuning` (an extension to the contract; none changes protocol behaviour):

| Field | Default | Meaning |
|---|---|---|
| `media_lanes` | half the CPUs, at least 1 | Dedicated media lanes for `rtp`/`srtp`. |
| `pin_media_lanes` | `false` | Pin lane `i` to CPU `media_lane_first_cpu + i` (Linux). |
| `media_lane_first_cpu` | `0` | First pinned CPU. |
| `queue_capacity` | `20000` | SIP stack event queue capacity (recipe `$capacity`). |
| `retained_lifecycle_capacity` | `max(8 x max_concurrent_calls, max_concurrent_calls + 640000)` | Lifecycle records kept for the 64 s SIP anti-reuse horizon (sized for 10,000 calls/s). |

## rvoip configuration

`dut::rvoip_config` builds it; `cargo test -p rvoip-bench-uas` checks it.

- Starts from rvoip's bundled recipes: `pbx-media-server` with media,
  `signaling-only-server-high-performance` without (fast auto-accept, no
  automatic 100/180, four round-robin UDP parse workers, sharded
  transaction/dialog/session dispatch, 8 MiB UDP socket buffers).
- Removes the recipes' soft-limit pacing (it delays INVITEs near capacity and
  would distort setup delay), sets `server_call_admission_limit` =
  `max_concurrent_calls` with `server_call_admission_early_reject` and the
  lock-free admission check, and turns off per-session transition history.
- `sip_new_call_admission` is always `Enforce` (no rate limit for `off`), so
  the TLS ingress hook is attached from the start and drain 503s over TLS
  are answered on the connection reader.
- With media: `MediaExecutorConfig::v2()` on dedicated media lanes. RTP is
  received, SRTP-authenticated and decoded inline on the socket's lane; the
  echo is a `SipMediaStream` inbound frame sink that the fused inbound pump
  calls on that lane, and it writes through the stream's encoded G.711 writer
  on the synchronous TX path, so an echoed packet costs no task wakeup. A
  frame the synchronous path declines (and every frame on executor v1) goes
  through a per-call task that awaits the same writer.

## Files

`ready.json`, written once the SIP listener and the metrics endpoint are up:

```json
{"pid": 4242, "started_at": 1763300000.123, "transport": "tls",
 "listen": "10.90.200.10:5061", "media": "srtp",
 "metrics_listen": "0.0.0.0:9464", "version": "0.4.0",
 "git_sha": "68037b047...", "config_sha256": "<sha256 of dut.json bytes>"}
```

`stats.jsonl`: one line per `stats_interval_ms` with every counter below plus
`"t"` (unix seconds, float). The last line is the drained state.

`summary.json`:

```json
{"started_at": 1763300000.1, "ended_at": 1763300400.9, "drain_seconds": 2.3,
 "drain_timed_out": false, "true_zero": true, "clean_shutdown": true,
 "counters": {"...": "the final snapshot"},
 "version": "0.4.0", "git_sha": "...", "config_sha256": "..."}
```

- `true_zero`: active calls, pending setups, pending cleanups, open dialogs and
  RTP port reservations are all zero (the last one is stricter than the
  contract, so a leaked port also fails it).
- `clean_shutdown`: the drain reached true zero and the SIP stack stopped
  within 10 s.

`dut.log` is the process's stderr (`RUST_LOG`, default
`warn,rvoip_bench_uas=info`).

Exit codes: 0 drained, 1 runtime error (bind, I/O), 2 bad configuration,
3 drain timed out.

## Metrics

`GET /metrics` (Prometheus text 0.0.4); every name is prefixed
`rvoip_bench_uas_`. The same names, without the prefix, are the
`stats.jsonl` / `summary.json` keys (labelled counters are objects keyed by
code).

| Metric | Type | Source |
|---|---|---|
| `invites_received_total` | counter | Initial INVITEs answered (`IncomingCall`) plus every stateless 503 rvoip counted (gate, capacity guard, draining, RTP port shortage, setup failure). Retransmissions answered from rvoip's response cache are not counted. |
| `responses_sent_total{code="200"\|"503"}` | counter | 200: answered calls. 503: the rejections above that reached the wire. |
| `calls_established_total` | counter | `CallEstablished` (the ACK arrived). |
| `byes_received_total`, `byes_answered_total{code="200"}` | counter | Calls ended by the caller's BYE; rvoip's dialog layer answers 200 before the event. A BYE for an unknown dialog (rvoip answers 481) is not visible here. |
| `active_calls` | gauge | Call table: established, not ended. |
| `pending_setups` | gauge | Call table: answered, ACK not yet in, not ended. |
| `pending_cleanups` | gauge | Dialogs rvoip still holds beyond the live calls (`open_dialogs - active_calls - pending_setups`, floored at 0) plus echo streams of ended calls still closing. |
| `open_dialogs` | gauge | `rvoip_sip_dialog::live_dialog_count()`: the dialog stores' own size. |
| `connections` | gauge | TLS: established TCP sockets on the SIP port, from `/proc/net/tcp{,6}`. 0 for UDP. |
| `udp_flows` | gauge | UDP: distinct caller signaling addresses with a live call. 0 for TLS. |
| `admission_rejected_total` | counter | 503s from `max-cps`, the drain and `max_concurrent_calls`. |
| `rtp_packets_received_total`, `rtp_packets_sent_total` | counter | RTP media frames (one per 20 ms packet) rvoip delivered to the echo, and frames rvoip accepted to send back. |
| `srtp_unprotect_failures_total` | counter | `rvoip_rtp_core::transport::srtp_unprotect_failures_total()`. |
| `process_cpu_seconds_total`, `process_resident_memory_bytes`, `process_threads` | process | `/proc/self/stat`, `/proc/self/status`. |

Extensions: `media_ports` (RTP port reservations rvoip holds),
`calls_failed_total`, `calls_cancelled_total`, `calls_ended_other_total`
(answered calls that ended without the caller's BYE, such as an ACK
timeout), `echo_frames_dropped_total`, `echo_bind_failures_total`, `draining`.

### How the gauges stay exact

rvoip dispatches each call event on its own task, so one call's events can
arrive out of order. The call table keeps three flags per call (answered,
established, ended) and derives each call's contribution to `pending_setups`
and `active_calls` from them, applying only the difference on every event.
Each call therefore adds to and removes from the gauges exactly once,
whatever the order; an end seen before the start leaves a tombstone so the
late start cannot resurrect it (tombstones are swept after 120 s).
`open_dialogs` and `media_ports` are read from rvoip's own structures, so a
leak in rvoip shows up as a drain that never reaches true zero.

## Loopback throughput check

`tests/throughput.rs` (ignored by default) is a minimal signaling-only caller:
N flows (TLS connections or UDP sockets) send INVITE at an even share of the
offered rate, ACK every 200 OK and BYE at once. It is a sanity check, not
the benchmark's load generator.

```
BENCH_TRANSPORT=tls BENCH_CPS=8000 BENCH_SECONDS=15 BENCH_CONNECTIONS=16 \
  cargo test --release -p rvoip-bench-uas --test throughput -- --ignored --nocapture
```

On the 16-vCPU build host (caller and DUT sharing it, `media: none`): TLS at
2,000 / 4,000 / 6,000 / 8,000 calls/s and UDP (sipp `uac`) at 2,000 and
5,000 calls/s completed every call with no failure and drained to true
zero; the DUT used about 0.9 ms of CPU per TLS call.

## Tests

```
cargo test -p rvoip-bench-uas
```

Unit tests cover configuration parsing and validation, the admission GCRA
on rvoip's gate, the drain wait, the call table under every event order, the
echo sink, metrics rendering and the HTTP responder. `tests/loopback.rs`
runs the binary on loopback and calls it with rvoip's own UAC: signaling-only
calls over UDP and TLS, RTP echo over UDP and SRTP echo over TLS (the tone
comes back), `max-cps` 503s with `Retry-After` from a raw UDP UAC, a drain
that refuses new calls while a call is up, and a drain timeout (exit 3).
