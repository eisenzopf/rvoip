# Changelog — rvoip-webrtc

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased — gap-closure arc (G1–G12)

The H1–H7 arc shipped the production-hardening baseline. A subsequent gap
analysis ([`docs/GAP_PLAN.md`](docs/GAP_PLAN.md)) identified the remaining
`Must`-level surface gaps that kept the crate from being a drop-in WebRTC
client/server library: data-channel options API, RFC 9725 §4.1 Bearer
auth, outbound RTP stats, header-extension registration, SDP rollback,
perfect-negotiation helper, signaling pool, ops-tail items. This arc
([`docs/archived/GAP_IMPLEMENTATION_PLAN.md`](docs/archived/GAP_IMPLEMENTATION_PLAN.md))
closed every G-phase except G7 (deferred upstream) and the cpal backend
half of G3 (deferred, needs workspace dep additions).

### Added

#### Fail-closed inbound signaling admission

- `WebRtcServerBuilder::with_inbound_admission_confirmation(timeout)` and
  `WebRtcAdapter::new_with_inbound_admission_confirmation(...)` opt into a
  bounded protocol hold: WHIP and new inbound WebSocket offers do not expose
  an SDP answer until the orchestrator's inbound admission gate commits the
  exact lifecycle generation.
- Secure mode requires a complete, active, non-anonymous principal and a
  principal-bound routing hint. Missing gates, rejected or stale decisions,
  local teardown, and timeouts erase the provisional route and return one
  credential-free signaling failure. Secure WebSocket attachment hints are
  moved into the first inbound lifecycle and cannot be reused on the socket.
- WHEP remains outbound and bypasses inbound admission confirmation. Direct
  adapters and servers retain their historical behavior unless secure mode is
  explicitly enabled.
- New real WHIP/WS adversarial coverage in
  `tests/inbound_admission_confirmation.rs`.

#### G1 — Data channel options API + typed wrapper

- `DataChannelOptions` (`src/peer/data_channel.rs`) — typed RFC 8832 §5.1
  constructor set: `reliable()`, `unreliable()`,
  `partial_reliable_retransmits(n)`, `partial_reliable_lifetime(ms)`,
  plus builders `with_protocol(...)` and `with_negotiated_id(id)`.
- `RvoipDataChannel` wrapper with `send_text` / `send_binary` /
  `buffered_amount` / `set_buffered_amount_low_threshold` /
  `buffered_amount_low_threshold` / `ready_state` / `poll`.
- New `RvoipPeerConnection::create_data_channel_typed(label, opts)` —
  returns the typed wrapper directly.
- New tests: `tests/dc_options.rs` (7 tests covering every RFC 8832
  reliability mode + binary + invalid-argument guard) and
  `tests/dc_backpressure.rs` (typed wrapper round-trip + threshold
  round-trip).

#### G2 — WHIP/WS authentication + RFC 9725 headers

- New `src/signaling/auth.rs` with `WhipAuthHook`, `WsAuthHook`,
  `AuthContext`, `AuthRejection`, `AnonymousAuth` (default),
  `BearerStaticTokenAuth` (reference impl), and `extract_bearer` helper.
- WHIP server: `WhipState::with_auth(...)`,
  `serve_listener_with_auth(...)`, `serve_listener_with_auth_and_shutdown(...)`.
- WHIP server emits `Accept-Post: application/sdp` on `OPTIONS`.
- WHIP server auto-populates `Link: <url>; rel="ice-server"` (with
  `username` / `credential` / `credential-type` when configured) from
  `WebRtcConfig::ice_servers` on every CREATED response.
- WHIP `PATCH application/sdp` (ICE restart) enforces `If-Match: "<etag>"`
  per RFC 9725 §4.4.1 — 428 when missing, 412 on mismatch.
- WebSocket server: `serve_listener_with_auth(...)` and the WSS equivalent
  complete the async hook before HTTP 101. Rejections remain HTTP 401, 403,
  or 429 responses with `WWW-Authenticate` / `Retry-After` where applicable.
  Tokens are accepted via `Sec-WebSocket-Protocol: token.<value>` or
  `?access_token=<value>`.
- WHIP, WHEP, WS, and WSS now share adapter-owned route authorization keyed
  by issuer + tenant + subject (`PrincipalOwnershipKey`). Complete principals
  are retained on routes, emitted through `PrincipalAuthenticated`, and
  removed atomically with route cleanup. Authenticated outbound WS/WSS routes
  can be bound before exposure with `bind_authenticated_principal(...)`.
- `WebRtcServerBuilder::with_whip_auth(...)` / `with_ws_auth(...)`.
- New `WebRtcError` variants: `InvalidArgument`, `Unauthorized`, `Forbidden`,
  `PreconditionFailed`, `InvalidState`, `FingerprintNotPinned`.
- New tests: `tests/whip_auth.rs`, `tests/ws_auth.rs`,
  `tests/signaling_ownership.rs`, and WSS pre-upgrade coverage in
  `tests/tls_termination.rs`.

#### G4 — Outbound + candidate-pair stats

- `WebRtcStatsSnapshot` extended with `outbound: OutboundStats` (packets
  sent, bytes, retransmits, NACK / PLI / FIR counts) and
  `selected_pair: Option<CandidatePairStats>` (local + remote candidate
  type labels, current/total RTT, available outgoing bitrate, responses
  received, nominated flag).
- `InboundStats::merge_webrtc_report` now also harvests `outbound-rtp`
  aggregates and the nominated candidate pair.
- `WebRtcAdapter::aggregated_stats()` rolls every live stream's snapshot
  into a single typed sample.
- New `observability::render_prometheus_with_stats(metrics, snapshot)`
  emits 10 new counter / gauge series:
  `rvoip_webrtc_{inbound_packets,inbound_bytes,packets_lost,frames_dropped,outbound_packets,outbound_bytes,retransmitted_packets,nack_count,pli_count,fir_count}_total`,
  plus gauges
  `rvoip_webrtc_{jitter_ms,packet_loss_pct,mos_estimate,selected_pair_rtt_ms,available_outgoing_bitrate_bps}`.
- WHIP `/metrics` endpoint wired to the extended exporter.
- New tests in `tests/h7_observability.rs` covering the new series.

#### G6 — Header extensions + Safari/Firefox SDP fixtures

- `register_default_header_extensions` registers MID (RFC 9335),
  audio-level (RFC 6464), RID and repaired-RID (RFC 8852),
  abs-send-time (draft), TWCC (draft) explicitly so browser-interop SDPs
  round-trip the right `extmap:` IDs. Called automatically from
  `build_media_engine`.
- New constants exported from `peer::builder`: `HDREXT_SDES_MID`,
  `HDREXT_SDES_RID`, `HDREXT_SDES_RRID`, `HDREXT_AUDIO_LEVEL`,
  `HDREXT_ABS_SEND_TIME`, `HDREXT_TWCC`.
- `tests/browser_sdp_interop.rs` extended with recorded Safari 17 and
  Firefox 125 audio offers; asserts opus negotiation + hdrext round-trip
  per fixture.

#### G11 — SDP rollback primitive

- `RvoipPeerConnection::rollback_local()` — wraps
  `setLocalDescription({type:"rollback"})` per JSEP §4.1.10.2.
- `RvoipPeerConnection::signaling_is_stable()` — true when no pending
  local/remote description.
- New `tests/rollback.rs`.

#### G3 — Perfect negotiation + signaling pool

- `PerfectNegotiation` helper (`src/client/perfect_negotiation.rs`) —
  W3C polite/impolite collision resolution; returns a typed
  `NegotiationAction { Apply, Ignore, Rollback }`.
- `SignalingPool` (`src/client/pool.rs`) — `Arc<dyn Signaler>` cache
  keyed by base URL; idle TTL eviction; integrates with `WsSignaler`.
- New `tests/perfect_negotiation.rs` (4 tests).

#### G12 — Operational tail

- `OpusSettings` config struct with knobs for `use_in_band_fec`,
  `use_dtx`, `min_ptime_ms`, `max_average_bitrate_bps`, `stereo`.
  `WebRtcConfig::opus_settings` threads through to the media engine
  fmtp line.
- `build_media_engine_with_opus(opus_settings)` variant for callers
  that want full control over the Opus fmtp line.
- `sdp::redact_for_log(sdp)` — strips IPs / ufrag / pwd / origin
  from SDP for safe logging.
- `WebRtcAdapter::reset_metrics()` — opt-in counter reset for
  operators who rotate Prometheus scrape windows manually.
- New `tests/g12_ops_tail.rs` (4 tests).

#### G-tail — NACK round-trip via lossy TURN relay

- `tests/support/lossy_turn_fixture.rs` — composes the existing
  `CoturnFixture` with a per-client lossy UDP proxy that sits in front
  of coturn's control port. Each direction drops UDP datagrams with a
  configurable, seeded probability — works around webrtc-rs 0.20-alpha
  lacking a public `SettingEngine` for UDP-port pinning.
- `tests/support/coturn_fixture.rs` — extended to expose a relay-port
  range (50000–50019) and pass `--external-ip 127.0.0.1` /
  `--min-port` / `--max-port` so peers can actually reach coturn's
  relayed transports. `TURN_USERNAME` / `TURN_PASSWORD` are now public
  so adjacent fixtures can build matching `IceServerConfig`s.
- `tests/lossy_turn_nack.rs` — two peers via lossy relay at 5 % drop;
  asserts `inbound.packets_lost > 0` AND
  `outbound.nack_count > 0` — proving the registered RTCP-NACK
  feedback round-trips end-to-end. Skips on no-Docker hosts.

#### G-tail — TURN relay two-peer media E2E

- `tests/turn_relay_e2e.rs::relay_only_two_peer_media_round_trip`
  spins up two `RvoipPeerConnection` instances against the existing
  `CoturnFixture`, both with `IceTransportPolicy::Relay`, completes a
  full offer/answer + ICE handshake, pumps Opus frames end-to-end,
  and asserts `selected_pair.local_candidate_type == "relay"`.
- Test skips cleanly when Docker isn't reachable (same contract as
  the existing `relay_policy_with_coturn_fixture_builds_peer`).

#### G-tail — DC backpressure event subscription

- `RvoipDataChannel::subscribe_buffered_amount_low()` returns a
  `tokio::sync::broadcast::Receiver<()>` that fires every time the
  underlying buffer drops below the configured low threshold (W3C
  `bufferedamountlow` semantics).
- `RvoipDataChannel::subscribe_events()` returns a
  `broadcast::Receiver<DataChannelEvent>` for callers that want the
  full event stream (OnOpen / OnMessage / OnClose / ...).
- Both subscriptions lazily spawn a single background pump task that
  owns `inner().poll()`; the pump exits cleanly when the underlying
  data channel closes. Calling raw `inner().poll()` after a
  `subscribe_*` call is no longer supported on the same wrapper.
- New test `buffered_amount_low_event_fires_after_drain` in
  [`tests/dc_backpressure.rs`](tests/dc_backpressure.rs).

#### G12 #6 — WHEP routing-model documentation

- README "Limitations" section: states the one-`PeerConnection`-per-POST
  semantics of `/whep/{tag}` and points to mediasoup / Galène / LiveKit
  for SFU fan-out.
- `src/signaling/whip.rs` module-level doc adds a "Routing model" header
  so the constraint is visible directly from `cargo doc`.
- Closes the last in-tree actionable item from the G12 operational tail
  (per-route CORS deferred until a real deployment needs it).

#### G5 — Lossy-link helper

- `tests/lossy_link.rs::spawn_lossy_udp_proxy(addr, loss_rate, seed)` —
  seeded-RNG UDP forwarding proxy as a building block for fault-injection
  tests.

#### G8 / G9a — CI + TURN

- `.github/workflows/nightly-interop.yml` at the rvoip repo root —
  active nightly workflow that installs Chromium, builds with
  `interop-browser,signaling-whip,signaling-ws`, and runs
  `tests/browser_interop.rs --include-ignored`. Optional Slack webhook
  on failure (`secrets.NIGHTLY_INTEROP_WEBHOOK`).
- `docs/ci/nightly-interop.yml` — reference copy of the workflow,
  retained for docs.
- `tests/support/coturn_fixture.rs` — shells out to the `docker` CLI to
  bring up coturn in a container; returns `IceServerConfig` ready to
  drop into `WebRtcConfig::ice_servers`; gracefully skips when Docker
  isn't reachable.
- `tests/turn_relay_e2e.rs` — exercises the fixture with
  `IceTransportPolicy::Relay`.

### Changed

- `RvoipPeerConnection::create_data_channel(label)` → `create_data_channel(label, opts)`.
  Existing callers must pass `DataChannelOptions::reliable()` to preserve
  the previous default. **Breaking** at the wrapper layer; the inner
  `peer_connection().create_data_channel(label, None)` call path on the
  raw webrtc-rs trait is unchanged.

### Deferred

- **G7 (multi-codec audio transceiver)** — naive two-encoding approach
  on the same SSRC broke `loopback_rtp_inbound_round_trip` in
  webrtc-rs 0.20-alpha; reverted. The DTMF wire test stays
  `#[ignore]`'d. Proper fix needs per-codec sender API exposure that
  upstream hasn't yet shipped.
- **G3 cpal microphone + nokhwa camera backends** — would require
  workspace `Cargo.toml` dep additions outside this crate's scope.
  The `AudioSource` / `VideoSource` trait surface is in place; a
  follow-up `client-cpal` feature can plug in.
- **G10 (DTLS fingerprint identity binding)** — one-line wrapper
  change blocked on upstream `rvoip-core` adding
  `IdentityAssurance::DtlsFingerprint` variant.
- **G9b (SIP↔WebRTC media E2E)** — blocked on
  `Orchestrator::bridge_connections` SIP path landing in `rvoip-core`.

### Verification

- `cargo build -p rvoip-webrtc --all-features --all-targets` — clean
- `cargo clippy -p rvoip-webrtc --all-features --no-deps` — clean
- `cargo test -p rvoip-webrtc --all-features --tests` — all green
  (one DTMF wire test remains `#[ignore]`'d, see G7 deferral)

---

## v0.1.26 — production-hardening arc (H1–H7)

The v1 implementation (Phases 0–11 of [`docs/archived/IMPLEMENTATION_PLAN.md`](docs/archived/IMPLEMENTATION_PLAN.md))
shipped the skeleton, peer/media/SDP layers, `WebRtcAdapter`, WHIP/WHEP/WS
signaling, client API, and the QUIC bridge. An end-to-end audit then identified
that the surface was demo-grade rather than production-grade: panics on benign
inputs, silent event drops, no real client surfaces, no metrics/CORS/rate-limit,
trickle ICE returning `NotImplemented`, etc. The H1–H7 work below closed
every gap. See [`docs/archived/HARDENING_PLAN.md`](docs/archived/HARDENING_PLAN.md) for the full
audit trail and per-task post-mortem.

### Added

#### Correctness baseline (H1)
- `WebRtcError::WrongRole { expected, actual }` and `WebRtcError::AlreadySubscribed`
  error variants.
- `WebRtcAdapter::try_subscribe_events() -> Result<Receiver, AlreadySubscribed>`
  for callers that want to detect double-subscribe; the trait-level
  `subscribe_events` now returns a closed receiver + warn instead of panicking.
- `WebRtcTransportHandle { connection_id, routes: Weak, cancel }` typed
  TransportHandle replacing the unit `Arc::new(())` placeholder.
- `WebRtcConfig::handler_channel_capacity`, `inbound_send_deadline_ms`,
  `session_idle_ttl_secs` config knobs.
- Session reaper task — periodically reclaims `Failed`-state routes after the
  configured TTL.
- `HandlerDropCounters` — per-channel atomic drop counters exposed via
  `RvoipPeerConnection::handler_drop_counters()`.
- `#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]`
  to prevent regressions.

#### Trickle ICE + renegotiation (H2)
- `WebRtcConfig::trickle_ice: bool` — when true, `create_offer_and_gather` /
  `create_answer_and_gather` return immediately without waiting for ICE
  gathering complete.
- `WebRtcConfig::hold_renegotiate: bool` (default `true`) — hold/resume now
  also produces a fresh local SDP via renegotiation, not just transceiver
  direction.
- `RvoipPeerConnection::recv_local_ice()` / `try_recv_local_ice()` /
  `drain_local_ice()` — outbound trickle candidate channel.
- `RvoipPeerConnection::restart_ice()` — wraps webrtc-rs's restart flag.
- `WebRtcAdapter::apply_trickle_candidate(conn, RTCIceCandidateInit)` —
  feed remote trickle candidates with optional mDNS filtering.
- `WebRtcAdapter::restart_ice(conn)` — produce a fresh offer/answer with
  fresh ufrag/pwd.
- WebSocket signaler: real `{type:"ice-candidate", candidate, connection_id}`
  handling for inbound; per-WS-connection forwarder task pushes locally-gathered
  candidates back to the client when `trickle_ice_enabled`.
- WHIP `PATCH application/trickle-ice-sdpfrag` per RFC 8840 — parses
  `a=candidate:` lines scoped by `a=mid:`, returns 204 / 400 / 404 / 415.

#### RTCP feedback + codecs + stats (H3)
- RTCP feedback registered on every codec in the engine:
  Opus → `transport-cc`; VP8/VP9/H.264 → `goog-remb`, `transport-cc`,
  `ccm fir`, `nack`, `nack pli`.
- VP9 (`profile-id=0`) and H.264 (constrained-baseline level 3.1,
  `packetization-mode=1`, `profile-level-id=42e01f`) codec registrations
  added to the media engine — Safari and most SIP SBCs require H.264.
- `WebRtcStatsSnapshot { packets_received, bytes_received, packets_lost,
  jitter_ms, packet_loss_pct, mos, frames_dropped }` typed snapshot with
  simple MOS estimator.
- `WebRtcMediaStream::webrtc_stats_snapshot()` accessor.

#### Server hardening (H4)
- WHIP/WHEP RFC 9725 surface compliance:
  - `Content-Type: application/sdp` enforcement on POST (415 on mismatch).
  - `Location`, `ETag`, `Accept-Patch: application/sdp,
    application/trickle-ice-sdpfrag`, `Link: <…>; rel="ice-server"` headers
    on every CREATED response.
  - Session-id path segment is now honored (was previously discarded).
  - Proper `404` on unknown routes, `405` / `415` / `400` / `503` status codes.
- `GET /healthz` and `GET /readyz` endpoints with active session count.
- `GET /metrics` Prometheus text-format exporter (6 series:
  `inbound_total`, `outbound_total`, `active_sessions`,
  `signaling_errors_total`, `sessions_rejected_over_cap`, `reaped_total`).
- Pluggable CORS via `WebRtcConfig::cors_origins` (e.g. `["*"]` or explicit
  list); `tower-http`'s `CorsLayer` with `OPTIONS` preflight.
- Per-IP token-bucket rate limit (`WebRtcConfig::whip_per_ip_per_min`) →
  `429 Too Many Requests`.
- Atomic session cap (`WebRtcConfig::max_concurrent_sessions`) using a
  reserve/commit `SessionSlotGuard` — race-free under concurrent originate /
  apply_remote_offer (verified at 20 concurrent posts against cap=10
  producing exactly 10 CREATED + 10 503).
- `WebRtcServer::shutdown_with_deadline(Duration)` — graceful drain that
  notifies listeners to stop accepting, walks routes calling
  `adapter.end(_, Normal)`, then awaits task exit.
- `WebRtcConfig::ws_max_message_size` (default 1 MB) and
  `ws_keepalive_secs` (default 15 s, loopback sets `0`) for the WebSocket
  signaler.
- `turn_rest` module: `generate_ephemeral(url, secret, ttl, hint) -> IceServerConfig`
  produces RFC 7635-style ephemeral TURN credentials using HMAC-SHA256.
- `WebRtcMetrics` typed metrics snapshot.

#### Real `WebRtcClient` (H5)
- `WsSignaler::send_answer(&Answer)` — answerer flow with `connection_id`
  scoping + best-effort ack handling.
- `WsSignaler::send_ice(&IceCandidate)` is now a real trickle send.
- `send_ice_for(signaler, connection_id, candidate_json)` helper for
  explicit candidate scoping.
- `WsSignalerConfig` with `retry_max_attempts`, `initial_backoff`,
  `max_backoff`, `request_timeout` (default = 1 attempt for backward
  compatibility).
- `SessionHandle::close()` + best-effort `Drop` impl — the last clone fires
  a detached `peer.close()` task via `Arc` strong-count check; `closed:
  Arc<AtomicBool>` guards against double-close.
- New `client::media_source` module:
  - `AudioSource` and `AudioSink` async traits.
  - `FixtureAudioSource` — paced silent Opus at 20 ms / 48 kHz.
  - `NullAudioSink` and `CountingAudioSink`.
  - `run_audio(source, sink, frames_out, frames_in, AudioPacing)` — bridges
    a source/sink pair to a `WebRtcMediaStream`'s frame channels with
    paced or unpaced delivery.

#### Interop + load (H6)
- `tests/whip_load.rs` — 50 concurrent WHIP POST load test; verifies ≥90%
  CREATED success rate and metrics consistency under concurrency. Second
  test verifies the atomic session cap truncates exactly at the configured
  limit.
- `tests/browser_sdp_interop.rs` — recorded Chromium 120 audio-only offer
  fixture; asserts the answer carries fresh ufrag/pwd, DTLS fingerprint,
  `setup:active|passive`, `mid:0`, BUNDLE, rtcp-mux, Opus PT 111 echoed +
  follow-up trickle candidate accepted. Second test: malformed SDP returns
  a diagnostic error, not a panic.
- `tests/dc_soak.rs` — open/close lifecycle no-panic loop (20 iterations) +
  5 s sustained DC ping/pong leak check.
- `tests/sip_webrtc_bridge.rs` — `SipAdapter` + `WebRtcAdapter` coexist on
  one `Orchestrator`; documents capability shape asymmetry.
- `tests/turn_relay.rs` — `WebRtcConfig::ice_transport_policy: { All,
  Relay }` propagates to `RTCConfigurationBuilder::with_ice_transport_policy`.
- `static/whip-publish.html` + `static/ws-signaling.html` static demo pages
  for manual browser testing (`getUserMedia` → WHIP POST and WS-signaled
  data channel).
- `tests/browser_interop.rs` — headless-Chromium harness via `chromiumoxide`
  (feature `interop-browser`, `#[ignore]`'d because Chromium binary on PATH
  is a prereq).
- `tests/soak_long.rs` — leak-detecting soak (feature `soak-1h`,
  `SOAK_SECS` env var, default 60 s). Asserts `num_alive_tasks` returns to
  within +50 of baseline after every cycle. **Full 1-hour run validated:
  9 701 cycles / 48 505 peer pairs / `peak_alive=0` / `final_alive=0`.**

#### Identity + observability (H7)
- `src/observability.rs` — Prometheus text-format exporter for `WebRtcMetrics`.
- `WebRtcConfig::mdns_candidate_policy: { Drop, Pass }` (default `Drop`) —
  filters RFC 8839 `.local` trickle candidates that wouldn't resolve on
  hosted servers anyway.
- `#[instrument]` spans on every adapter entry point (`apply_remote_offer`,
  `originate`, `accept`, `end`, `apply_trickle_candidate`, `restart_ice`)
  with `connection_id` field propagation.
- `src/identity.rs`: `DtlsFingerprint { algorithm, value }` + per-SDP
  `extract_fingerprints(sdp) -> Vec<DtlsFingerprint>`.
- `WebRtcAdapter::remote_dtls_fingerprint(conn)` — returns parsed remote
  fingerprints for out-of-band pinning / verification.
- `src/tls.rs` (feature `tls-rustls`): `TlsConfig::from_pem_files` /
  `from_pem_bytes` — Arc-backed cert+key holder usable from both axum-server
  (WHIPS) and tokio-rustls (WSS).
- `WebRtcServerBuilder::with_whips(addr, TlsConfig)` and `with_wss(addr,
  TlsConfig)` — in-process TLS termination on the WHIP and WS listeners.

### Changed

- Handler channel capacity default raised from 8 to 256
  (`WebRtcConfig::handler_channel_capacity`).
- `WebRtcMediaStream::enable_webrtc_stats(peer, cancel: Arc<Notify>)` — now
  takes a cancel notify; the stats poll task exits cleanly on stream close.
- `media::pump::spawn_inbound_pump` signature gained `send_deadline_ms: u64`
  and `cancel: Option<Arc<Notify>>` parameters — slow downstream consumers
  now drop frames after the deadline instead of stalling the pump forever.
- `WebRtcConfig::default()` now disables WS keepalive (`ws_keepalive_secs: 0`)
  for loopback configs so tests don't observe ping frames mid-handshake.
- Default `WebRtcFeatureSupport` now reports `trickle_ice_signaling: true`
  (was deferred in v1).

### Fixed

- **Critical DashMap deadlock** in `client/comprehensive.rs::handle_server_connection`
  — `let Some(route) = adapter.routes().get(&conn) else { return; }` held a
  `DashMap::Ref` across `peer.wait_connected(15s)`, `peer.wait_data_channel(10s)`,
  and the infinite `poll_data_channel` loop. Any concurrent writer to the
  same shard blocked indefinitely. Fix: `drop(route)` immediately after
  extracting `peer`. Root-caused via instrumentation after a 14-hypothesis
  bisect had exonerated every H1–H6 change. **All 4 H4-regressed
  comprehensive tests + the H6 DC soak unblocked.**
- Race-free `on_track` attach: streams seeded synchronously in
  `apply_remote_offer`/`originate`, eliminating the 20 ms-poll race window
  in the original code where a track arriving before `accept()` could be
  silently dropped.
- WebSocket signaler now skips Ping/Pong/Close control frames before
  attempting JSON parse — previously a server keepalive ping mid-handshake
  could crash the read loop.
- All `serde_json::to_string(...).unwrap()` panic sites in the WS handler
  replaced with `.map_err(WebRtcError::Signaling)`.
- All `assert_eq!(self.role, ...)` panic sites in `peer/session.rs`
  replaced with `require_role()?` returning `WebRtcError::WrongRole`.
- Outbound RTP pump (`spawn_outbound_pump`) no longer stalls forever on
  slow downstream consumers — bounded send with `inbound_send_deadline_ms`
  timeout and per-stream drop counter.
- Background stats collector now exits cleanly on stream close via cancel
  notify (was leaking a polling task per closed stream).
- Session reaper cancels cleanly when `WebRtcAdapter` is dropped (new
  `Drop` impl on the adapter).

### Removed

- Nothing was removed — all changes are additive or fixes.

### Test footprint

- **Before this arc**: ~15 integration tests + 3 lib unit tests.
- **After this arc**: 33 integration test files / ~90 individual tests +
  12 lib unit tests; 2 documented `#[ignore]`'d tests (DTMF wire test needs
  multi-codec audio transceiver; browser interop needs Chromium binary on
  `PATH`).
- New features added: `tls-rustls`, `interop-browser`, `soak-1h`.

### Workspace dependency additions

- `tokio-rustls = "0.26"` (used by `tls-rustls` feature)
- `axum-server = "0.7"` with `tls-rustls` feature
- `chromiumoxide = "0.7"` with default features off, `tokio-runtime` only
  (used by `interop-browser` feature)

All other deps were already in the workspace.

### Verification

- `cargo build -p rvoip-webrtc --all-features --all-targets` — clean
- `cargo clippy -p rvoip-webrtc --all-features --no-deps` — clean
- `cargo test -p rvoip-webrtc --all-features --tests` — all green
- `cargo test -p rvoip-webrtc --features bridge-quic --test webrtc_quic_bridge_e2e`
  — passes (~2 s)
- `SOAK_SECS=3600 cargo test -p rvoip-webrtc --features soak-1h --test soak_long
  --release` — passes (3600.80 s, 9 701 cycles, zero leaks)

### Migration notes

- All new `WebRtcConfig` fields have `Default` impls, so existing
  `WebRtcConfig::default()` / `WebRtcConfig::loopback()` callers are
  unchanged.
- `WebRtcAdapter::subscribe_events()` still returns a `Receiver` (trait
  contract preserved) but is now infallible on double-call (warns +
  returns closed receiver). New `try_subscribe_events() -> Result` for
  callers that need to detect.
- `SessionHandle` is still `Clone`; the new `Drop` impl is a no-op except
  on the last clone. Use the new `SessionHandle::close()` for deterministic
  teardown.
- `WsSignaler::send_answer` and `send_ice` were `NotImplemented`/no-op
  stubs before; callers relying on the stub behavior may now see real
  errors when the WS server is unreachable.
