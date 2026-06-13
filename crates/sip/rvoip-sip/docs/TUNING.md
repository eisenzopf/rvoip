# rvoip-sip Tuning Guide

## Default Policy

`Config::default()`, `Config::local(...)`, and `Config::on(...)` are the
client/default UA profile. They favor interoperability and predictable behavior:

- media allocation is enabled
- automatic `180 Ringing` is enabled
- automatic Timer `100 Trying` is enabled
- dispatch paths stay single-worker unless explicitly configured
- UDP socket buffers stay at the OS default
- ACK/BYE priority burst and INVITE 2xx resend pacing use transaction-layer
  defaults

Server and benchmark tuning is opt-in. The preferred path is a YAML-backed
`PerformanceConfig` recipe. The crate ships a default recipe book at
`config/performance-recipes.yaml`, and applications can provide their own recipe
file without changing library source. Do not treat high-CPS signaling-only
recipes as universal defaults; they are reproducible starting points for a
specific SIPp signaling-only workload.

## Profile Starting Points

| Workload | Starting point | Notes |
| --- | --- | --- |
| Client / UA | `PerformanceConfig::endpoint()` or omitted performance config | Keep default auto `180`, Timer `100`, media, queue sizes, and socket buffers. |
| Local examples/tests | `Config::local_lab(...)` | Alias for local loopback examples and integration tests. |
| Normal LAN server / PBX | `Config::lan_pbx(...)` | Directly reachable SIP and media address. Add capacity sizing only when expected concurrency requires it. |
| Asterisk TLS registered-flow client | `Config::asterisk_tls_registered_flow(...)` | TLS client mode, registered-flow reuse, mandatory SDES-SRTP. |
| FreeSWITCH internal LAN | `Config::freeswitch_internal(...)` | Direct LAN profile with strict codec matching. |
| FreeSWITCH TLS/SRTP | `Config::freeswitch_tls_srtp_reachable_contact(...)` | Directly reachable TLS Contact, mandatory SRTP, FreeSWITCH-compatible suite policy. |
| Carrier/SBC | `Config::carrier_sbc(...)` | TLS client mode, registered-flow Contact, mandatory SRTP, public signaling/media address, outbound proxy route. |
| SIP proxy plus RTPengine lab | `Config::proxy_rtpengine(...)` | Signaling route helper; RTPengine control remains above `rvoip-sip`. |
| PBX media server | `PerformanceConfig::pbx_media_server(capacity)` | UDP auto-answer server profile with media allocation enabled. |
| Signaling-only high-performance server | `PerformanceConfig::signaling_only_server_high_performance(capacity)` | SIP signaling throughput profile that intentionally skips RTP allocation. |
| Legacy high-CPS UDP auto-answer | `Config::with_high_cps_udp_auto_answer(capacity)` | Conservative low-level modifier for immediate-answer UDP servers and SIPp experiments. |

## Recipe: Client / UA Default

Use this for ordinary softphones, agents, registered clients, and app endpoints
where correctness and interop matter more than synthetic CPS.

```rust
use rvoip_sip::Config;

let config = Config::local("alice", 5060);
```

Equivalent YAML recipe name: `endpoint`.

Keep these defaults unless a deployment needs a specific change:

| Setting | Default stance |
| --- | --- |
| Media | Enabled |
| Automatic `180 Ringing` | Enabled |
| Automatic Timer `100 Trying` | Enabled |
| Dispatch workers | Single path |
| Socket buffers | OS default |
| `sip_transaction_dispatch_priority_burst_max` | Unset, transaction default `64` |
| `sip_invite_2xx_retransmit_max_due_per_tick` | Unset, transaction default `2048` |

## Recipe: Normal Server / PBX

Start with `Config::lan_pbx(...)` for a directly reachable LAN/server endpoint.
Size queues and hot indexes from expected active calls or burst-arriving calls,
but do not add worker fanout until diagnostics show a backlog.

```rust
use rvoip_sip::Config;

let bind = "0.0.0.0:5060".parse().unwrap();
let advertised = "192.168.1.50:5060".parse().unwrap();

let config = Config::lan_pbx("pbx", bind, advertised)
    .with_channel_capacity(2_000)
    .with_server_capacity(2_000);
```

Guidance:

- `with_channel_capacity(N)` sets app-facing queues to `N` and lower-level SIP
  event queues to `N * 10`.
- `with_server_capacity(N)` reserves dialog/transaction lookup capacity without
  changing queue sizes.
- Leave media enabled for real servers unless the service truly has no RTP.
- Leave transport, transaction, and dialog dispatch workers unset until
  `sip_udp_diag`, transaction diagnostics, or dialog diagnostics point to a
  specific queue.

## Recipe: PBX Media Server

Use this as the media-enabled server profile for PBX-style UDP auto-answer
services. The beta gate validates this shape rather than using a beta-only
profile.

```rust
use rvoip_sip::{Config, PerformanceConfig};

let bind = "0.0.0.0:5060".parse().unwrap();
let advertised = "192.168.1.50:5060".parse().unwrap();

let config = Config::lan_pbx("answerer", bind, advertised)
    .try_with_performance_config(PerformanceConfig::pbx_media_server(2_000))?;
```

The bundled `pbx-media-server` YAML recipe currently:

- sets channel capacity and UDP parse queue capacity from `capacity`
- disables automatic `180 Ringing`
- disables automatic Timer `100 Trying`
- enables fast auto accept
- sets UDP receive/send socket buffers to `8_388_608`
- sets UDP parse workers to `4`
- sets UDP parse dispatch to round-robin
- sets transaction dispatch workers to `2`
- sets dialog dispatch workers to `4`
- sets session event dispatcher workers to `4`
- sets per-transaction command channel capacity to `128`
- sets server lookup capacity to `capacity`
- sets server admission limit to `capacity`
- starts admission pacing at 90 percent of `capacity`
- uses a `1ms` pacing delay while above the soft threshold
- once the hard admission limit is reached, sends `503 Service Unavailable`
  with `Retry-After: 1` until retained sessions drop below the soft threshold
- sets media port capacity to `16_384..=65_535` (`49_152` ports)
- sets media session capacity to `capacity`
- keeps media enabled

If validation shows command-channel pressure at 2,000 CPS, retest with
`sipTransactionCommandChannelCapacity: 256` in a custom YAML recipe file and
record the result before changing the bundled recipe default.

### Beta Release SIPp Target

The beta gate's managed SIPp target should use the bundled `pbx-media-server`
recipe with diagnostics disabled for release latency measurements:

```bash
BETA_SIPP_PERF_PROFILE=pbx-media-server
BETA_SIPP_DIAGNOSTICS=0
```

Leave `BETA_SIPP_DIAGNOSTICS` unset, or set it explicitly to `0`, for
promotion runs. Enable `BETA_SIPP_DIAGNOSTICS=1` only for focused RCA after a
clean non-diagnostic control exists. The diagnostic path enables additional
SIP UDP, media setup, and cleanup counters on the managed `perf_listener`;
those counters are useful for attribution, but they can perturb tail latency and
must not be used as the acceptance latency baseline.

The 2026-06-12 SIPp RCA used this rule at 2,000 CPS. A beta-style run with
`BETA_SIPP_DIAGNOSTICS=0` completed `30,000/30,000` calls with p99 and p99.9
both under `10 ms`, while diagnostic controls widened the tail into the
`<50 ms` bucket in repeat runs. Treat that as evidence to keep diagnostics as
RCA tooling, not a release-performance setting.

## RTP and Media Buffer Tuning

The SIP API exposes the RTP/media memory knobs used by the full-media path.
Leave these defaults alone for normal deployments; tune them when a soak or
production profile shows a specific queue, receive buffer, or media-core pool is
oversized or undersized for the workload.

```rust
use rvoip_sip::{
    Config, MediaPoolConfig, MediaSessionControllerConfig,
    RtpSessionBufferConfig, RtpTransportBufferConfig,
};

let mut media = MediaSessionControllerConfig::default();
media.audio_frame_pool = MediaPoolConfig {
    initial_size: 16,
    max_size: 64,
    sample_rate: 8000,
    channels: 1,
    samples_per_frame: 160,
};
media.rtp_buffer_size = 480;
media.rtp_buffer_initial_count = 16;
media.rtp_buffer_max_count = 64;

let config = Config::local("answerer", 5060)
    .with_media_session_controller_config(media)
    .with_rtp_session_buffer_config(RtpSessionBufferConfig {
        sender_channel_capacity: 64,
        receiver_channel_capacity: 32,
        event_channel_capacity: 64,
    })
    .with_rtp_transport_buffer_config(RtpTransportBufferConfig {
        event_channel_capacity: 32,
        recv_buffer_size: 1500,
        rtcp_recv_buffer_size: 1500,
    });
```

Default buffer values:

| Config object | Field | Default | Notes |
| --- | --- | ---: | --- |
| `RtpSessionBufferConfig` | `sender_channel_capacity` | `64` | Bounded RTP packet send queue per session. |
| `RtpSessionBufferConfig` | `receiver_channel_capacity` | `32` | Legacy polling receive queue per session. |
| `RtpSessionBufferConfig` | `event_channel_capacity` | `64` | RTP session event broadcast ring. |
| `RtpTransportBufferConfig` | `event_channel_capacity` | `32` | RTP/RTCP transport event broadcast ring. |
| `RtpTransportBufferConfig` | `recv_buffer_size` | `1500` | UDP RTP receive buffer size. |
| `RtpTransportBufferConfig` | `rtcp_recv_buffer_size` | `1500` | UDP RTCP receive buffer size for separate RTCP sockets. |
| `MediaSessionControllerConfig` | `audio_frame_pool.initial_size` | `32` | Shared reusable decoded audio frame pool. |
| `MediaSessionControllerConfig` | `audio_frame_pool.max_size` | `128` | `0` means unlimited for the media-core pool, but bounded values are recommended. |
| `MediaSessionControllerConfig` | `audio_frame_pool.sample_rate` | `8000` | Must match the deployed audio profile. |
| `MediaSessionControllerConfig` | `audio_frame_pool.channels` | `1` | Mono for the current G.711 beta media profile. |
| `MediaSessionControllerConfig` | `audio_frame_pool.samples_per_frame` | `160` | 20 ms at 8 kHz. |
| `MediaSessionControllerConfig` | `rtp_buffer_size` | `480` | Reusable encoded RTP payload buffer size. |
| `MediaSessionControllerConfig` | `rtp_buffer_initial_count` | `32` | Initial reusable RTP payload buffer count. |
| `MediaSessionControllerConfig` | `rtp_buffer_max_count` | `128` | Maximum reusable RTP payload buffer count. |

Available API surfaces:

| Surface | RTP session method | RTP transport method | Media controller method |
| --- | --- | --- | --- |
| `Config` / `UnifiedCoordinator` | `with_rtp_session_buffer_config(...)` | `with_rtp_transport_buffer_config(...)` | `with_media_session_controller_config(...)` |
| `SessionBuilder` | `with_rtp_session_buffer_config(...)` | `with_rtp_transport_buffer_config(...)` | `with_media_session_controller_config(...)` |
| `StreamPeerBuilder` | `rtp_session_buffer_config(...)` | `rtp_transport_buffer_config(...)` | `media_session_controller_config(...)` |
| `CallbackPeerBuilder` | `rtp_session_buffer_config(...)` | `rtp_transport_buffer_config(...)` | `media_session_controller_config(...)` |

`with_media_session_controller_config(...)` copies the controller config's
nested RTP session/transport settings into the SIP-level RTP config fields.
Calling `with_rtp_session_buffer_config(...)` or
`with_rtp_transport_buffer_config(...)` afterwards overrides those copied
values.

## Recipe: Carrier Media Burst

Carrier burst evidence uses the split-process media burst harness:

```bash
RVOIP_PERF_BURST_SCENARIOS=carrier-smoke \
crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh
```

The bundled scenario book is `config/perf-burst-scenarios.yaml`. It defines a
short beta-gate smoke plus broader opt-in profiles such as
`access-edge-microburst`, `contact-center-flash`, `overload-recovery`, and
`high-density-media-burst`. Each scenario records caller and receiver JSON plus
a `_burst.md` carrier summary under `target/perf-results/perf_burst_matrix/`.
The short smoke records RSS slope but only enforces the RSS growth gate when
the post-drain window reaches the scenario's `minRssGateWindowSecs`; use a
longer `RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS` for citable RSS-gate evidence.

The bundled burst recipes are starting points:

| Recipe | Intended use |
| --- | --- |
| `carrier-burst-balanced` | Default carrier media burst profile. |
| `carrier-burst-high-density` | Larger SIP/media queues and media pools for dense bursts. |
| `carrier-burst-memory-tight` | Smaller RTP/media queues and media pools for memory-sensitive overload tests. |

Carrier burst recipes enable `activeCallNoMediaTimeoutSecs: 60` and
`activeCallMediaIdleTimeoutSecs: 60`. These are auto-answer server resiliency
guards: after an inbound call reaches `Active`, the server releases it if
media-core has not observed RTP within the no-media window, or if RTP later
stops advancing for the media-idle window. Keep them disabled for ordinary
endpoints or valid silent-call workloads.

Promote a burst recipe only after repeat runs show ASR, setup p99, RSS slope,
retention, and media receiver cleanup are stable. Record the artifact path and
git revision next to any promoted operating envelope.

Current `access-edge-microburst` evidence does not justify promoting a carrier
burst recipe. On 2026-06-09, the best dialog `8/24000` candidate still missed
ASR with answer timeouts. Receiver media setup and cleanup were clean, SIP UDP
queues did not report Rust-level backpressure, and changing
`sipInvite2xxRetransmitMaxDuePerTick` to `512`, increasing `aliceShards` to
`32`, or applying static admission pacing did not pass. Treat similar failures
as a transport/protocol investigation before growing more server queues.

The timestamped diagnostic run
`burst_20260609_090007_78489/ae-dialog-8q24000-client-diagnostics/` added
epoch-microsecond SIP UDP `call_traces` timestamps and dialog
`call_timing_traces`. It failed ASR (`0.9836`, `121` answer timeouts), while
media setup, retained objects, receiver active audio receivers, Rust UDP queue
full/parse/send errors, and host full-socket-buffer drops all stayed at `0`.
The failed Call-IDs split into late caller 2xx delivery with no UAC ACK attempt
(`65`), receiver-missing INVITEs (`20`), caller ACKs not seen by the receiver
(`20`), and receiver-saw-ACK/post-timeout lifecycle races (`16`). Do not promote
a Config recipe from this.

Admission pacing follow-up runs then lowered the server soft limit to `4500`.
With `delay=1 ms`, admission pacing triggered (`729` pacing decisions) and
improved ASR only modestly to `0.9862` (`102` answer timeouts). With
`delay=2 ms`, pacing triggered `777` times but regressed to ASR `0.9826`
(`129` answer timeouts) and failed the caller RSS gate (`17.22 MB/hr`). In both
runs receiver media setup, cleanup, Rust UDP queue/parse/send counters, and
host full-socket-buffer drops stayed clean. Do not promote static admission
pacing from this sweep.

Follow-up library isolation runs narrowed the failure further. Adding bounded
UDP receive draining improved ASR only from `0.9896` to `0.9916` and left
caller receive-loop p95 at `1 s`, so receive draining is not the root fix.
When the same burst shape ran with signaling-only media, caller setup p95 was
`13.1 ms` and there were no answer timeouts. When full media allocation stayed
enabled but the perf caller skipped installing the RTP tone source
(`RVOIP_PERF_BURST_SKIP_AUDIO_SOURCE=1`), caller setup p95 was `13.7 ms`, all
`7400` INVITEs/2xx/ACKs were observed by the peer, and receiver media sessions
allocated and drained cleanly. Treat generated RTP traffic, not SIP queue
sizing or media allocation alone, as the current access-edge bottleneck.

The first RTP library experiment spread generated-audio transmitter start
phases across the 20 ms packet interval and skipped missed ticks. That helped
but did not pass: a clean full-RTP run improved to ASR `0.9932` with `50`
answer timeouts, while caller SIP receive-loop gaps still had p95 `1 s` and
p99/p999 bucket `5 s`. A follow-up cached-tone/payload-copy experiment was
rejected after ASR regressed to `0.9886` with `84` answer timeouts. In that
rejected run, receiver SIP processing stayed fast (INVITE-to-200 p95 `1 ms`),
receiver media cleanup stayed clean (`0` active audio receivers and `0`
retained objects after drain), and host full-socket-buffer drops again stayed
at delta `0`.

Audio TX pacing with target active `3000` is the current leading full-media
library candidate. Three repeat runs passed the acceptance gates with ASR
`1.0000`, `7400/7400` calls, `0` answer timeouts, `0` media setup failures,
`0` teardown failures, `0` caller/receiver retained objects after drain, `0`
receiver active audio receivers after drain, and RSS gates below `10 MB/hr`.
The pacing path skipped about `8.3M` generated-audio TX ticks per run and kept
the caller setup tail much lower than the no-pacing library runs. A lighter
target active `4000` passed once but had worse setup p95/p99 and more host
`no socket` drops, so it is only a probe. Keep this as opt-in library evidence
until it has a production-facing Config/API decision. A follow-up shared
generated-audio TX scheduler probe did not beat this result: shared scheduling
without pacing regressed to ASR `0.9866`, and shared scheduling with target
`3000` pacing passed three guarded runs but added scheduler complexity without
a clear CPU win and had less stable setup p99 (`7.67 s` on the third guarded
run). Treat pacing-only target `3000` as the simpler leading candidate.

## Recipe: Signaling-Only High-Performance Server

Use this profile to isolate SIP signaling throughput from media allocation:

```rust
use rvoip_sip::{Config, PerformanceConfig};

let config = Config::lan_pbx("answerer", bind, advertised)
    .try_with_performance_config(
        PerformanceConfig::signaling_only_server_high_performance(20_000)
    )?;
```

The bundled `signaling-only-server-high-performance` YAML recipe applies the
same worker/socket/queue profile as `pbx-media-server`, then switches to
signaling-only SDP. Do not market high-CPS signaling-only results as the
general-user full-media baseline.

The server recipes own overload behavior through Config. At
`serverCallAdmissionSoftLimit`, rvoip-sip paces inbound INVITE admission. At
`serverCallAdmissionLimit`, rvoip-sip enters overload mode and rejects new
inbound INVITEs with SIP `503 Service Unavailable` and the configured
`Retry-After` value. It stays unavailable until retained sessions drop below the
soft threshold. Load harnesses should report this as library overload evidence,
not silently drop offered calls in the harness.

## Recipe: Legacy High-CPS UDP Auto-Answer

Use this when the service immediately answers inbound UDP INVITEs and the
application can tolerate benchmark-oriented behavior.

```rust
use rvoip_sip::Config;

let bind = "0.0.0.0:5060".parse().unwrap();
let advertised = "192.168.1.50:5060".parse().unwrap();

let config = Config::lan_pbx("answerer", bind, advertised)
    .with_high_cps_udp_auto_answer(20_000)
    .with_media_port_capacity(16_384, 49_152)
    .with_media_session_capacity(20_000)
    .with_sip_udp_recv_buffer_size(8_388_608)
    .with_sip_udp_send_buffer_size(8_388_608);
```

`with_high_cps_udp_auto_answer(capacity)` currently:

- calls `with_channel_capacity(capacity)`
- disables automatic `180 Ringing`
- disables automatic Timer `100 Trying`
- sets `sip_udp_parse_workers = Some(1)`
- sets `sip_udp_parse_queue_capacity = Some(capacity)`
- sets `sip_transaction_command_channel_capacity` to
  `(capacity / 8).clamp(128, 1000)` unless already set
- keeps media enabled
- does not enable fast auto accept
- does not set `server_call_capacity`
- does not set `server_call_admission_limit`
- does not size media port/session allocators
- does not enlarge UDP socket buffers
- does not set ACK/BYE priority burst or INVITE 2xx pacing overrides

Add `.with_fast_auto_accept_incoming_calls(true)` only for fixed immediate-answer
services where every inbound INVITE should be accepted before callbacks run.
For signaling-only benchmarks, replace media allocation with
`.with_signaling_only_media(9)`. Media-enabled high-CPS still needs separate
testing before a runtime default is promoted.

## Recipe: SIPp Signaling-Only Benchmark

Use the committed runner for reproducible matrix runs:

```bash
RVOIP_SHARDING_CPS_LEVELS="18000" \
RVOIP_SHARDING_UDP_WORKERS="4" \
RVOIP_SHARDING_TRANSPORT_WORKERS="1" \
RVOIP_SHARDING_TRANSACTION_WORKERS="2" \
RVOIP_SHARDING_DIALOG_WORKERS="4" \
RVOIP_SHARDING_CAPACITIES="20000" \
RVOIP_SHARDING_SIP_UDP_RECV_BUFFER_SIZE=8388608 \
RVOIP_SHARDING_SESSION_EVENT_WORKERS=4 \
crates/sip/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh \
  host.docker.internal 192.168.5.2 39460
```

Common fixed shape for current investigations:

| Dimension | Value |
| --- | --- |
| UDP parse workers | `4` |
| UDP parse dispatch | Round-robin |
| Transport dispatch workers | `1` |
| Transaction dispatch workers | `2` |
| Dialog dispatch workers | `4` |
| Session event dispatcher workers | `4` |
| Capacity | `20000` |
| UDP receive buffer | `8388608` |
| Media | Signaling-only SDP, RTP port `9` |

Use `RVOIP_SHARDING_SAMPLE=1` and `RVOIP_SHARDING_SAMPLY=1` only after the same
shape has a clean non-profiled control. The 20k shape is sensitive to profiler
overhead and UDP drops.

## Recipe: BYE / Termination Stress

The current BYE/termination investigation found two tunable pressure points:

- ACK/BYE requests can wait behind normal transaction dispatch work.
- Proactive cached INVITE 2xx retransmission volume can compete with teardown
  response traffic.

Start with the default transaction-layer values, then sweep:

```rust
let config = config
    .with_sip_transaction_dispatch_workers(2)
    .with_sip_dialog_dispatch_workers(4)
    .with_session_event_dispatcher_workers(4)
    .with_sip_transaction_dispatch_priority_burst_max(64)
    .with_sip_invite_2xx_retransmit_max_due_per_tick(2048);
```

Recommended sweep points:

| Knob | Values to test first | Current read |
| --- | --- | --- |
| `sip_transaction_dispatch_priority_burst_max` | `16`, `32`, `64`, `128` | `64` is the current unset default. Lower values improve fairness for normal work; higher values favor ACK/BYE latency. |
| `sip_invite_2xx_retransmit_max_due_per_tick` | `512`, `1024`, `1536`, `2048` | `512` was excellent for one clean 18k shape but hurt 20k completion, so it is not a default. |

Equivalent SIPp matrix knobs:

```bash
RVOIP_SHARDING_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX=64 \
RVOIP_SHARDING_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK=2048 \
RVOIP_SHARDING_TRANSACTION_TIMING=1 \
RVOIP_SHARDING_DIALOG_TIMING=1 \
crates/sip/rvoip-sip/tests/perf/sipp_scenarios/run_signaling_sharding_matrix.sh
```

## Recipe: 20k Experimental Frontier

The 20k CPS SIPp shape is not stable enough to promote as a default profile.
Use it as an investigation target only.

Known findings:

- `dup_invite_cache_miss=0` and `worker_mismatch=0` held in clean validation.
- BYE-only transaction priority is rejected because it starved ACK matching.
- ACK+BYE priority is the current approach because it preserves call affinity
  and avoids the ACK starvation seen with BYE-only priority.
- Prebuilt raw cached INVITE 2xx sends are useful and should stay.
- INVITE 2xx proactive resend pacing is workload-sensitive.
- `sip_invite_2xx_retransmit_max_due_per_tick = 512` cleaned one 18k run but
  caused a 20k run to complete only about 91.5 percent of expected successful
  calls, so it is unsafe as a default.
- Media-enabled high-CPS still needs separate testing. Current 18k/20k recipes
  use signaling-only media.

Promotion requires repeated same-shape controls, not one profiled run.

## Event Queue Capacities

The signaling path uses several bounded queues. When unset, app-facing queues
(`incoming_call_channel_capacity`, `state_event_channel_capacity`) default to `1000`; the
lower-level SIP queues (`sip_transport_channel_capacity`, `transaction_event_channel_capacity`)
default to `10000`; and the cross-crate event-bus and app-session-dispatcher queues
(`global_event_channel_capacity`, `session_event_dispatcher_channel_capacity`) default to `256`
(`Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY`). The cross-crate bus's own native default in
`infra-common` is `10000`; the unified `Config` layer overrides it down to `256`, so high-CPS
servers should raise it explicitly.

`with_channel_capacity(N)` is the master knob: it sets the app-facing queues to `N` and the
lower-level (transport / transaction / global / session) queues to `N * 10`. Use the individual
`with_*_channel_capacity` setters when one queue needs a different size. The cross-crate event
bus drops messages for receivers that lag past its capacity, so size
`global_event_channel_capacity` for the server's active-call / event burst.

## Config Knob Matrix

| Config API | `perf_listener` flag | SIPp matrix env | Default when unset | When to use |
| --- | --- | --- | --- | --- |
| `PerformanceConfig::endpoint()` / `profile: endpoint` | `--perf-profile endpoint` | `RVOIP_SHARDING_PERF_PROFILE=endpoint` | Omitted endpoint performance config is equivalent | Normal app endpoint/client/server defaults. |
| `PerformanceConfig::pbx_media_server(N)` / `profile: pbx-media-server` | `--perf-profile pbx-media-server --high-cps-capacity N` | `RVOIP_SHARDING_PERF_PROFILE=pbx-media-server` | Profile off | PBX-style UDP auto-answer server with media allocation enabled. |
| `PerformanceConfig::signaling_only_server_high_performance(N)` / `profile: signaling-only-server-high-performance` | `--perf-profile signaling-only-server-high-performance --high-cps-capacity N` | `RVOIP_SHARDING_PERF_PROFILE=signaling-only-server-high-performance` | Profile off | SIP signaling-only high-CPS profile; not a full-media claim. |
| Custom YAML recipe | `--recipe-file path --perf-profile name` | `RVOIP_SHARDING_EXTRA_LISTENER_ARGS='--recipe-file path'` | Bundled recipe book | Deployment-specific tuning without source changes. |
| `Config::with_high_cps_udp_auto_answer(N)` | `--high-cps-capacity N` | `RVOIP_SHARDING_CAPACITIES` | `perf_listener` uses `20000`; normal Config leaves profile off | Immediate-answer UDP server or SIPp matrix baseline. |
| `Config::with_fast_auto_accept_incoming_calls(true)` | `--fast-auto-accept` | Always enabled by sharding runner | `false` | Benchmark or fixed service that accepts every INVITE immediately. |
| `Config::with_signaling_only_media(9)` | `--signaling-only-media` | Always enabled by sharding runner | `MediaMode::Enabled` | Isolate SIP signaling from RTP allocation/socket bind cost. |
| `Config::with_channel_capacity(N)` | Indirect through `--high-cps-capacity N` | `RVOIP_SHARDING_CAPACITIES` | Incoming/state `1000`; transport/transaction `10000`; global/session `256` | Master queue knob: sets incoming/state to `N` and transport/transaction/global/session to `N * 10`. |
| `Config::with_global_event_channel_capacity(N)` | Indirect via `--high-cps-capacity` (`* 10`) | `RVOIP_SHARDING_CAPACITIES` (`* 10`) | `256` | Cross-crate event-bus (GlobalEventCoordinator broadcast + bridge) depth. Raise for high-CPS servers — the unified default sits below the bus's native `10000`. |
| `Config::with_app_event_channel_capacity(N)` | None | None | `256` (both) | One knob that sets global event **and** session-dispatcher capacity together; use the two lower-level setters when they need different values. |
| `Config::with_incoming_call_channel_capacity(N)` | Indirect via `--high-cps-capacity` | `RVOIP_SHARDING_CAPACITIES` | `1000` | Inbound-call notification queue (set to `N` directly by `with_channel_capacity`). |
| `Config::with_state_event_channel_capacity(N)` | Indirect via `--high-cps-capacity` | `RVOIP_SHARDING_CAPACITIES` | `1000` | Call state-change event queue (set to `N` directly by `with_channel_capacity`). |
| `Config::with_sip_transaction_command_channel_capacity(N)` | None (set by `with_high_cps_udp_auto_answer`) | None | `None` → high-CPS derives `(capacity / 8).clamp(128, 1000)` | Per-transaction command channel; lower it under command-channel pressure (recipe note: try `256` at 2k CPS). |
| `Config::with_media_session_capacity(N)` | Via `--high-cps-capacity` (capped) | `RVOIP_SHARDING_CAPACITIES` (capped) | `None` | Pre-size media-core session and RTP allocator indexes for expected active media sessions. |
| `Config::with_media_port_capacity(start, count)` / `with_media_ports(start, end)` | Via high-CPS profile | None | OS/default range | Bound the RTP media port range for the expected concurrent media sessions. |
| `Config::with_server_capacity(N)` | None | None | `None` | Reserve server dialog/transaction lookup capacity without changing queues. |
| `Config::with_server_call_admission_limit(N)` | YAML recipe / app Config | YAML recipe / app Config | `None` | Enforce hard server overload admission. At capacity, new inbound INVITEs receive SIP `503` until the server drops below the soft threshold. |
| `Config::with_server_call_admission_soft_limit(N)` | YAML recipe / app Config | YAML recipe / app Config | `None` | Start pacing inbound admission before the hard overload limit. |
| `Config::with_server_call_admission_pacing_delay_ms(N)` | YAML recipe / app Config | YAML recipe / app Config | `None` | Delay applied to each inbound INVITE while above the soft threshold and below hard overload. |
| `Config::with_server_overload_retry_after_secs(N)` | YAML recipe / app Config | YAML recipe / app Config | `Some(1)` | Set the `Retry-After` value on Config-owned overload rejections. |
| `Config::with_active_call_no_media_timeout_secs(N)` | YAML recipe `activeCallNoMediaTimeoutSecs` | None | `0` disabled | Auto-answer server guard that releases inbound `Active` calls when no RTP packets arrive after answer. |
| `Config::with_active_call_media_idle_timeout_secs(N)` | YAML recipe `activeCallMediaIdleTimeoutSecs` | None | `0` disabled | Auto-answer server guard that releases inbound `Active` calls when RTP stops advancing and remote BYE never arrives. |
| `Config::with_sip_udp_parse_workers(N)` | `--udp-parse-workers N` | `RVOIP_SHARDING_UDP_WORKERS` | Transport default | Add UDP parse parallelism when parse/dispatch work backs up. |
| `Config::with_sip_udp_parse_queue_capacity(N)` | `--udp-parse-queue-capacity N` | `RVOIP_SHARDING_UDP_QUEUE_CAPACITY` | SIP transport channel capacity | Bound per-worker UDP parse queue for bursty tests. |
| `Config::with_sip_udp_parse_dispatch(UdpParseDispatch::RoundRobin)` | `--udp-parse-round-robin` | Always enabled by sharding runner | Source-hash | SIPp sidecar tests where all calls share one source socket. |
| `Config::with_sip_udp_recv_buffer_size(N)` | `--sip-udp-recv-buffer-size N` | `RVOIP_SHARDING_SIP_UDP_RECV_BUFFER_SIZE` | OS default | Host UDP receive drops or bursty ingress. |
| `Config::with_sip_udp_send_buffer_size(N)` | `--sip-udp-send-buffer-size N` | `RVOIP_SHARDING_SIP_UDP_SEND_BUFFER_SIZE` | OS default | Large response bursts or send-side UDP pressure. |
| `Config::with_sip_transport_channel_capacity(N)` | `--sip-transport-channel-capacity N` | `RVOIP_SHARDING_SIP_TRANSPORT_CHANNEL_CAPACITY` | `10000` | Transport-manager queue pressure. |
| `Config::with_sip_transport_dispatch_workers(N)` | `--sip-transport-dispatch-workers N` | `RVOIP_SHARDING_TRANSPORT_WORKERS` | Single bridge | Queue delay between transport receive/parse and transaction ingress. |
| `Config::with_sip_transport_dispatch_queue_capacity(N)` | `--sip-transport-dispatch-queue-capacity N` | `RVOIP_SHARDING_SIP_TRANSPORT_DISPATCH_QUEUE_CAPACITY` | SIP transport channel capacity | Increase per-worker transport dispatch backlog only when measured. |
| `Config::with_transaction_event_channel_capacity(N)` | `--transaction-event-channel-capacity N` | `RVOIP_SHARDING_TRANSACTION_EVENT_CHANNEL_CAPACITY` | `10000` | Transaction-to-dialog event queue pressure. |
| `Config::with_sip_transaction_dispatch_workers(N)` | `--transaction-dispatch-workers N` | `RVOIP_SHARDING_TRANSACTION_WORKERS` | Single receive/handle loop | Fan out transaction ingress by stable call/transaction key. |
| `Config::with_sip_transaction_dispatch_queue_capacity(N)` | `--transaction-dispatch-queue-capacity N` | `RVOIP_SHARDING_TRANSACTION_DISPATCH_QUEUE_CAPACITY` | Transaction event channel capacity | Queue pressure inside transaction dispatch workers. |
| `Config::with_sip_transaction_dispatch_priority_burst_max(N)` | `--transaction-dispatch-priority-burst-max N` | `RVOIP_SHARDING_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX` | Transaction default `64` | Tune ACK/BYE priority fairness in multi-worker transaction dispatch. |
| `Config::with_sip_invite_2xx_retransmit_max_due_per_tick(N)` | `--invite-2xx-retransmit-max-due-per-tick N` | `RVOIP_SHARDING_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK` | Transaction default `2048` | Pace proactive cached INVITE 2xx retransmission send bursts. |
| `Config::with_sip_dialog_dispatch_workers(N)` | `--sip-dialog-dispatch-workers N` | `RVOIP_SHARDING_DIALOG_WORKERS` | Single dialog event processor | Fan out dialog transaction events by stable call key. |
| `Config::with_sip_dialog_dispatch_queue_capacity(N)` | `--sip-dialog-dispatch-queue-capacity N` | `RVOIP_SHARDING_DIALOG_DISPATCH_QUEUE_CAPACITY` | Dialog capacity hint | Dialog dispatch queue pressure. |
| `Config::with_session_event_dispatcher_workers(N)` | `--session-event-dispatcher-workers N` | `RVOIP_SHARDING_SESSION_EVENT_WORKERS` | Logical CPU count capped at `16` | App-session event publication backlog. |
| `Config::with_session_event_dispatcher_channel_capacity(N)` | `--session-event-dispatcher-queue-capacity N` | `RVOIP_SHARDING_SESSION_EVENT_QUEUE_CAPACITY` | `256` | Per-worker app-session event publication queue pressure. |
| `Config::with_sip_udp_diagnostics(true)` | `--diagnostics` | Always enabled by sharding runner; beta managed SIPp target only with `BETA_SIPP_DIAGNOSTICS=1` | `false` | UDP + duplicate-recovery summary counters. (`--diagnostics` also flips the two toggles below; in `Config` they are independent.) |
| `Config::with_media_setup_diagnostics(true)` | `--diagnostics` (shared) | Always enabled by sharding runner; beta managed SIPp target only with `BETA_SIPP_DIAGNOSTICS=1` | `false` | Media-setup summary counters; independently settable in `Config`. |
| `Config::with_cleanup_diagnostics(true)` | `--diagnostics` (shared) | Always enabled by sharding runner; beta managed SIPp target only with `BETA_SIPP_DIAGNOSTICS=1` | `false` | Session-cleanup summary counters. Distinct from `with_cleanup_diagnostic_events`, which emits per-event logs. |
| `Config::with_perf_max_rss_growth_mb_per_hr(N)` | None | None | `None` (feature `perf-tests`) | RSS-growth ceiling for perf soak tests; aborts if exceeded. |
| `Config::with_cleanup_diagnostic_events(true)` | `--diagnostic-events` | Use `RVOIP_SHARDING_EXTRA_LISTENER_ARGS` | `false` | Noisy cleanup event logs for focused investigations. |
| `Config::with_srtp_diagnostics(true)`, `with_rtp_diagnostics(true)`, `with_media_sdp_diagnostics(true)` | `--wire-diagnostics` | Use `RVOIP_SHARDING_EXTRA_LISTENER_ARGS` | `false` | Noisy wire/media logs, not for high-CPS controls. |
| `Config::with_sip_transaction_timing_diagnostics(true)` | `--transaction-timing-diagnostics` | `RVOIP_SHARDING_TRANSACTION_TIMING` | `false` | Transaction queue, dispatch, retransmit, and duplicate timing histograms. |
| `Config::with_sip_dialog_timing_diagnostics(true)` | `--dialog-timing-diagnostics` | `RVOIP_SHARDING_DIALOG_TIMING` | `false` | Dialog ingress, BYE path, cleanup, publish timing histograms, and per-Call-ID answer/ACK timing traces when dialog diagnostics are enabled. |

`Config::with_pbx_media_server_performance(N)` and
`Config::with_signaling_only_server_high_performance(N)` are Config-level equivalents of the
`PerformanceConfig::pbx_media_server` / `PerformanceConfig::signaling_only_server_high_performance`
presets above. The bundle setters `with_sip_udp_parse_config`,
`with_sip_transport_dispatch_config`, `with_sip_transaction_dispatch_config`,
`with_sip_dialog_dispatch_config`, and `with_sip_udp_socket_buffers` set several of the
individual knobs above in a single call.

## Promotion Rules

Promote a recipe only after repeated clean non-profiled controls. Profiled runs
are evidence for where CPU time goes, not acceptance evidence by themselves.

Required for promoted high-CPS signaling recipes:

- `dup_invite_cache_miss=0`
- `worker_mismatch=0`
- clean-shape `ack_unmatched=0`
- host UDP drops `0`
- listener cleanup drains before shutdown
- same-shape profiled and non-profiled controls are preserved as artifacts
- final summary includes SIPp rc, achieved CPS, retransmits, host UDP drop
  delta, dead-call `200 OK` attribution, final `sip_udp_diag`, final
  `sip_retrans_diag`, BYE/dispatch diagnostics, and `sample`/`samply` artifact
  paths when profiles are enabled

## Reading Results

Use the diagnostics to decide the next knob. Avoid changing multiple unrelated
knobs in a promoted candidate.

| Signal | First response |
| --- | --- |
| Host UDP drops increase | Increase socket buffers or reduce send/receive pressure before interpreting app queues. |
| `transport_manager_to_transaction` queue delay dominates | Test transport dispatch workers or transport dispatch queue capacity. |
| Transaction dispatch BYE/ACK delay dominates | Sweep transaction dispatch workers and `sip_transaction_dispatch_priority_burst_max`. |
| Dialog dispatch queue delay dominates | Sweep dialog dispatch workers and queue capacity. |
| BYE `tx_received_to_handler` dominates `receive_to_200` | Transaction dispatch ordering/backlog is still the likely bottleneck. |
| BYE `send_response` dominates | Investigate response serialization/send path rather than cleanup. |
| INVITE 2xx duplicate/proactive counters dominate | Sweep `sip_invite_2xx_retransmit_max_due_per_tick` with same-shape controls; confirm `maintenance_capped_ticks` changes before interpreting the result as a cap effect. |
| Caller reports ACK sends but receiver ACK count stays low | Capture host UDP drops and add per-method inbound socket/source counters before changing more server worker or queue knobs. |
| Receiver retains `Active` media calls with zero RTP after burst | Enable or lower `activeCallNoMediaTimeoutSecs` for auto-answer profiles, then confirm real RTP-bearing long calls are not released. |
| Receiver retains `Active` media calls after RTP stops and no BYE arrives | Enable or lower `activeCallMediaIdleTimeoutSecs` for auto-answer profiles, then confirm valid silent-call workloads keep it disabled or use a larger value. |
| Cleanup drains late but response tails are clean | Isolate cleanup publication/removal from the response path. |

## Related Docs

- [`BENCHMARKING.md`](BENCHMARKING.md) covers publishable benchmark suite runs.
- [`PROFILING.md`](PROFILING.md) covers Criterion, samply, flamegraph, and dhat
  workflows.
- [`archived/SIGNALING_SHARDING_PERF_EXPERIMENT.md`](archived/SIGNALING_SHARDING_PERF_EXPERIMENT.md)
  records the archived signaling-only SIPp matrix history.
- [`archived/DIALOG_CORE_BYE_TERMINATION_HOT_PATH_PLAN.md`](archived/DIALOG_CORE_BYE_TERMINATION_HOT_PATH_PLAN.md)
  records the BYE/termination investigation, priority-lane decision, and
  archived 18k/20k findings.
