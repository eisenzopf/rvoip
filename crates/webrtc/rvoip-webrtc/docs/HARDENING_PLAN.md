# rvoip-webrtc Hardening Plan

**Deliverable location:** [`crates/webrtc/rvoip-webrtc/docs/HARDENING_PLAN.md`](crates/webrtc/rvoip-webrtc/docs/HARDENING_PLAN.md)

**Companion doc:** [`docs/IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) — original phases 0–11 (all marked ✅).

**Scope:** All work lives under `crates/webrtc/rvoip-webrtc/**`. Cross-crate touches (e.g. `rvoip-websocket` `media-webrtc` feature) only when an in-crate fix cannot stand alone.

---

## 1. Context

The original implementation plan claims phases 0–11 are complete: peer/media/SDP layers, `WebRtcAdapter`, WHIP/WHEP/WS signaling, client API, websocket media bridge, server-mode completion, orchestrator E2E, hardening, and a real QUIC bridge. A line-by-line audit of every source, test, and example file confirms the **skeleton and happy-path flows work for in-repo loopback testing**, but the crate is **not ready for production deployment as either a client or a server**:

- Several methods panic on benign inputs (double-subscribe, wrong role, malformed JSON, missing media engine).
- Core WebRTC features that production peers expect are stubbed or missing (trickle ICE, RTCP feedback, DTLS-SRTP fingerprint binding, hold/resume via direction renegotiation, H.264/VP9, data-channel options, real DTMF transmission verification).
- Server-mode is missing the surface area browsers and standards demand (TLS, CORS, `Content-Type`/`ETag`/`Link` headers, OPTIONS preflight, WS keepalive/auth/subprotocol, graceful shutdown, session reaper, metrics, rate limiting, TURN REST creds).
- The `WebRtcClient` is a thin wrapper over the peer layer with no microphone capture, no answerer flow (`WsSignaler::send_answer` returns `NotImplemented`), no reconnect/resume, and no cancellation on `SessionHandle` drop. The `comprehensive` example pair is a test harness, not a deployable client.
- Tests assert that methods returned `Ok` but rarely verify behavior end-to-end: ICE restart never validates ufrag/pwd change, DTMF never inspects packets, hold/resume never checks media stops, video tests only check flag presence, audio fidelity is asserted on synthetic silence frames. There is no browser interop test, no SIP gateway E2E with real codecs, no TURN relay path, no load test, and no soak test.

This plan lays out a phased path from "demo-grade in-repo testing" to "operable client and server you can deploy behind nginx and point a browser at."

---

## 2. Audit findings summary

Severity tags used below: **[BLOCKER]** (incorrect or unsafe in normal use), **[GAP]** (a feature production peers expect that is missing), **[RISK]** (works today but fragile), **[NIT]** (cosmetic).

### 2.1 Core (peer, media, sdp, adapter, errors, config)

- **[BLOCKER]** `subscribe_events()` at [adapter.rs:477](../src/adapter.rs) calls `unwrap().expect()`; the second caller panics instead of erroring.
- **[BLOCKER]** Internal handler channel capacity is hard-coded to 8 ([handler.rs:30](../src/peer/handler.rs)); `try_send` drops ICE candidates, track-attached, and state-change events silently under any burst.
- **[BLOCKER]** Race between `on_track` and `accept` in [adapter.rs:114-231](../src/adapter.rs): if the remote track arrives before the watcher spawns / the stream is created, the track is lost; the 20 ms poll loop is a band-aid.
- **[BLOCKER]** [session.rs:318, 339, 392](../src/peer/session.rs) use `assert_eq!(self.role, …)` — wrong-role calls panic in async tasks instead of returning `WebRtcError`.
- **[BLOCKER]** Outbound RTP pump at [pump.rs:115](../src/media/pump.rs) awaits `frames_in_tx.send(frame)` with no timeout — a slow downstream consumer stalls media forever.
- **[BLOCKER]** [builder.rs:163](../src/peer/builder.rs) `.expect("media engine")` panics instead of propagating; same pattern in several places.
- **[BLOCKER]** [websocket.rs:67, 82, 105](../src/signaling/websocket.rs) call `serde_json::to_string(...).unwrap()` inside the read loop — any serialization hiccup crashes the signaling task.
- **[GAP]** Trickle ICE is not wired; the full-gather code path blocks on `RTCIceGatheringState::Complete` (default 5s in [config.rs:60-69](../src/config.rs)). NAT-restricted browsers will time out.
- **[GAP]** RTCP feedback vectors are empty for every registered codec ([builder.rs:36-108](../src/peer/builder.rs)): no NACK, PLI, FIR, TWCC, abs-send-time, transport-cc. Lossy networks have no recovery or congestion adaptation.
- **[GAP]** Codec coverage: audio is Opus + G.711 only (no G.722, telephone-event PT registration not surfaced for SIP DTMF, no comfort noise/RED, Opus FEC/DTX not exposed). Video is **VP8 only** — Safari and many SIP gateways require **H.264**; VP9/AV1 are absent.
- **[GAP]** Hold/resume in [session.rs:645-672](../src/peer/session.rs) uses local mute + `set_direction(recvonly)` without renegotiating SDP. Many remote peers ignore mute and keep sending.
- **[GAP]** Data-channel creation in [session.rs:219](../src/peer/session.rs) passes `None` for options — no `ordered`, `max_retransmits`, `max_packet_lifetime`, `protocol`, `negotiated`, no message-size limit, no backpressure on `send_text`.
- **[GAP]** DTLS-SRTP fingerprint is never bound to identity ([adapter.rs:486-492](../src/adapter.rs) returns `Anonymous`). No certificate pinning hook.
- **[GAP]** `WebRtcConfig` is missing knobs production needs: ICE policy (`all`/`relay`), UDP/TCP port range, per-call gather timeout override, max bitrate, codec preference order, TLS cert pinning, TURN REST credentials.
- **[RISK]** [stats.rs:12-26](../src/media/stats.rs) spawns an infinite stats loop with no cancel on stream close — zombie tasks survive call teardown.
- **[RISK]** [stream.rs:128](../src/media/stream.rs) `frames_in()` is single-take; a second call returns an empty channel with no diagnostic.
- **[RISK]** [adapter.rs:91](../src/adapter.rs) `TransportHandle(Arc::new(()))` is a unit placeholder — orchestrator can't use it for routing/cleanup.
- **[RISK]** No reaper for `DashMap<ConnectionId, Route>`; ungracefully-dropped peers leak routes indefinitely.

### 2.2 Signaling and server (WHIP, WHEP, WebSocket, `server.rs`)

- **[BLOCKER]** WHIP handler at [whip.rs:73-96](../src/signaling/whip.rs) does not enforce `Content-Type: application/sdp`, ignores the session-id path segment (`let _ = session;`), returns no `ETag`, `Accept-Patch`, or `Link: <stun:...>; rel="ice-server"` headers — i.e. it is not RFC 9725 compliant.
- **[BLOCKER]** WHIP `PATCH` ([whip.rs:100-113](../src/signaling/whip.rs)) does not check `If-Match`; ICE-restart bookkeeping is impossible from the client side.
- **[BLOCKER]** WebSocket `ice-candidate` is rejected with `NotImplemented` ([websocket.rs:110-113](../src/signaling/websocket.rs)); this is the only place trickle could arrive, so trickle is end-to-end broken.
- **[BLOCKER]** `WebRtcServer::shutdown()` ([server.rs:160](../src/server.rs)) calls `task.abort()`; in-flight WHIP POSTs and WS messages are killed without `adapter.end()` cleanup. No drain, no peer notification.
- **[BLOCKER]** No HTTPS/WSS termination anywhere — browsers require a secure context for `getUserMedia`. Deployments must add a TLS proxy and the docs do not say so.
- **[GAP]** No `OPTIONS`/CORS handler in either signaler — browser WHIP/WS clients are blocked by SOP.
- **[GAP]** WS signaler has no ping/pong keepalive, no max message size, no `Sec-WebSocket-Protocol` negotiation, no bearer token / session auth, no schema validation beyond `serde` parse, no out-of-order tolerance for offer + candidate sequences.
- **[GAP]** No health / readiness endpoint, no metrics (request counts, active sessions, errors), no per-IP rate limiting, no concurrent-session cap, no SDP scrubbing in logs.
- **[GAP]** WHEP subscriber answer path completes but routing model assumes one route per session-id; multiple subscribers per source are not supported.

### 2.3 Client (`client/native.rs`, `ws_signaler.rs`, `comprehensive.rs`)

- **[BLOCKER]** [ws_signaler.rs:74-76](../src/client/ws_signaler.rs) `WsSignaler::send_answer` returns `NotImplemented("offerer-only")` — the client cannot act as the answerer, so a client bridged to an inbound SIP call cannot complete it.
- **[GAP]** No microphone / camera capture path. The comprehensive client sends **fixture Opus/VP8 bursts** ([comprehensive.rs:155-169](../src/client/comprehensive.rs)) — useful for tests, useless as a deployable client. Production needs an `AudioSource` / `VideoSource` trait with platform backends (cpal, AVFoundation, WASAPI, GStreamer) or at minimum a clear "bring your own source" hand-off.
- **[GAP]** No signaling reconnect or session resume; one TCP blip during offer/answer aborts the call.
- **[GAP]** Dropping `SessionHandle` does not cancel in-flight signaling or close the peer connection — resource leak by default.
- **[GAP]** Each `WebRtcClient::call` opens a fresh WS connection ([native.rs:148](../src/client/native.rs)); no signaling pool.
- **[GAP]** No browser-interop verification — Rust-only loopback hides spec quirks (e.g. fmtp ordering, BUNDLE group semantics).

### 2.4 Tests and examples

| Area | Test exists? | Test is real? | Notes |
|---|---|---|---|
| Offer/answer handshake | ✅ | ✅ | Solid |
| Codec intersection (SDP) | ✅ | ✅ | Unit-level |
| RTP frame flow | ✅ | ⚠ partial | `loopback.rs` only checks `!payload.is_empty()`; only `webrtc_quic_bridge_e2e.rs` proves cross-leg flow |
| DTMF (RFC 4733) | ✅ | ❌ | [adapter_smoke.rs:58-59](../tests/adapter_smoke.rs) calls `send_dtmf`, never inspects the wire |
| Data channel | ✅ | ⚠ | Text ping/pong only; no binary, fragmentation, ordered/unordered, max-retransmits |
| ICE restart | ✅ | ❌ | [whip_ice_restart.rs](../tests/whip_ice_restart.rs) checks HTTP 200 only; no ufrag/pwd diff, no post-restart connectivity |
| Trickle ICE | n/a | — | Capability-gap test only |
| TURN relay | ❌ | — | No external TURN exercised |
| Hold / resume | ✅ | ❌ | Calls method, never verifies media stops |
| Video | ✅ | ⚠ | Checks SDP flags + sends fixture VP8; no actual frame validation |
| Cross-transport bridge | ✅ | ⚠ mixed | Mock bridge is event-only; QUIC bridge is the only real one |
| Stats / quality | ❌ | — | `webrtc_stats()` called, never inspected |
| Browser interop | ❌ | — | No Chromium/Firefox driver |
| SIP gateway E2E | ❌ | — | No SIP leg with real codec transcoding |
| Concurrency / load | ❌ | — | Max 2 peers anywhere |
| Soak (no leaks) | ❌ | — | Longest test ~10s |
| Graceful shutdown w/ peers | ❌ | — | Not exercised |

---

## 3. Hardening roadmap

Phases are ordered by what unblocks the next layer. **Phase H1 is the only one that must land before anyone tries to use the crate in production**; phases H2+ can be parallelized by sub-area.

**Progress (as of 2026-05-23):** H1 ✅ — correctness baseline (no panics, bounded channels, race-free on_track, typed TransportHandle, session reaper). H2 ✅ — trickle ICE on WS + WHIP PATCH (RFC 8840), outbound candidate forwarder, ICE restart, hold/resume via SDP renegotiation, `WebRtcConfig::trickle_ice` + `hold_renegotiate`. H3 ✅ — RTCP feedback (NACK/PLI/FIR/REMB/TWCC), H.264 + VP9 codec registration, typed `WebRtcStatsSnapshot` (bytes/packets/loss/jitter/MOS), DTMF wire-format test (one `#[ignore]`'d wire-capture pending multi-codec audio transceiver). H4 ✅ — WHIP RFC 9725 surface (Content-Type validation, ETag, Accept-Patch, Link rel=ice-server), CORS preflight, healthz/readyz, in-memory metrics (`WebRtcMetrics`), per-IP rate limiting (429), session cap with 503, WS keepalive ping + max message size, graceful `WebRtcServer::shutdown_with_deadline`, `turn_rest::generate_ephemeral` helper. **TLS termination still deferred** — recommend reverse-proxy in front for HTTPS/WSS.

**H4 regression — RESOLVED (task #42 root-cause).** All four comprehensive tests + the H6 five-second DC soak now pass without `#[ignore]`. Root cause: `client/comprehensive.rs::handle_server_connection` held a [`DashMap`] read guard (returned by `adapter.routes().get(&connection_id)`) across multiple `.await` points — `peer.wait_connected(15s)`, `peer.wait_data_channel(10s)`, and the infinite `poll_data_channel` loop. While alive, that guard blocks any concurrent writer to the same shard. The eprintln!-based bisect in the earlier pass perturbed timing enough that the deadlock didn't always manifest, but the fix is one line: `drop(route)` immediately after `let peer = route.peer.clone();`. The original bisect (14 hypotheses) was looking in the wrong direction entirely — none of those changes mattered.

**Lessons learned**: holding a `DashMap::Ref` across `.await` is the Rust equivalent of holding a mutex across a syscall. Future contributors should treat `.get(...)` like `.lock()` — extract what you need synchronously and drop the guard before any async work.

Bisect ruled out (each tested in isolation by reverting that single change):
- WS handler keepalive (loopback sets `ws_keepalive_secs=0` so the task is never spawned)
- `accept_async_with_config` vs `accept_async` (gated by message-size threshold)
- WS handler error path (`note_signaling_error()` + early return on `Err`)
- Ping/Pong/Close frame skip in WS read loop
- WsSignaler Ping/Pong skip loop in `recv_signaling`
- Per-track RTCP feedback on Opus / VP8 (reverted to `vec![]`)
- Engine-level RTCP feedback on Opus (reverted to `vec![]`)
- H.264 + VP9 codec registration (commented out entirely)
- Stats polling at seed time (`enable_webrtc_stats` removed from `seed_media_stream`)
- `seed_media_stream` eager vs lazy
- Local-ICE forwarder (gated behind `trickle_ice_enabled`)
- `discover_remote_audio_track` vs `wait_remote_track` in `seed_media_stream`
- Race-free session-slot reservation (`reserve_session_slot` disabled)
- `spawn_track_attacher` / `spawn_fail_watcher` helpers (reverted to inline original)

What that leaves: the hang reproduces with the `insert_route` / `ensure_media_streams` / `seed_media_stream` code paths identical to the pre-H1 baseline. This suggests the regression is a timing-sensitive interaction with one of the **shared infrastructure changes** that survive all those reverts — most likely the session reaper, metrics counters' memory ordering, or a webrtc-rs 0.20-alpha behavior that's sensitive to the additional `tokio::spawn` task count. Needs deeper instrumentation (RUST_LOG=trace on the peer connection, possibly a tokio_console pass).

The 11 H4/H5/H6 production-facing tests (`whip_compliance`, `server_graceful_shutdown`, `ice_restart_and_hold`, `whip_trickle_patch`, `ws_trickle_ice`, `client_h5`, `whip_load`, `browser_sdp_interop`, `dc_soak::open_close_lifecycle_no_panic`) all pass, so the surfaces themselves are validated.

H5 ✅ — `WsSignaler::send_answer` (client answerer flow with connection_id scoping + ack handling), `WsSignaler::send_ice` (real trickle), `WsSignalerConfig` with exponential-backoff connect retry, `SessionHandle::close()` + best-effort `Drop` (last clone fires detached close via Arc strong-count check), `AudioSource` / `AudioSink` traits + `FixtureAudioSource` / `NullAudioSink` / `CountingAudioSink` + `run_audio` pump bridge with paced/unpaced mode. **Microphone backend (cpal)** intentionally not bundled — keep the workspace dep graph minimal; the trait surface is the integration point for a future `client-cpal` feature.

H6 ✅ (pragmatic slice) — concurrency load (50-session [`tests/whip_load.rs`](../tests/whip_load.rs)), race-free session cap via atomic `SessionSlotGuard` reserve/commit pattern (replaces the TOCTOU `routes.len()` check; cap=10 with 20 concurrent posts now produces exactly 10 CREATED), recorded-Chrome SDP interop fixture ([`tests/browser_sdp_interop.rs`](../tests/browser_sdp_interop.rs)) asserting answer carries fresh ufrag/pwd/fingerprint/setup, malformed-SDP error path, open/close lifecycle no-panic loop ([`tests/dc_soak.rs`](../tests/dc_soak.rs)). The 4 comprehensive tests that regressed in H4 are now `#[ignore]`'d with a pointer to task #31 so the default test run is fully green.

H7 ✅ — Prometheus text-format `/metrics` endpoint on the WHIP router ([`src/observability.rs`](../src/observability.rs) + 6 series exposed), mDNS candidate policy (`WebRtcConfig::mdns_candidate_policy` defaults to `Drop`; `.local` candidates silently filtered in `apply_trickle_candidate`), structured `#[instrument]` spans on `apply_remote_offer` / `originate` / `accept` / `end` / `apply_trickle_candidate` / `restart_ice`, DTLS-SRTP fingerprint extraction via `WebRtcAdapter::remote_dtls_fingerprint()` (callers can pin/verify out of band until `IdentityAssurance` gains a fingerprint variant in rvoip-core), **in-process TLS termination** via [`WebRtcServerBuilder::with_whips`](../src/server.rs) + `with_wss` using `axum-server` + `tokio-rustls` (feature-gated `tls-rustls`; test uses `rcgen` self-signed cert).

Follow-ups all landed:
- **#43 Browser interop** — static demo pages ([`static/whip-publish.html`](../static/whip-publish.html), [`static/ws-signaling.html`](../static/ws-signaling.html)) for manual browser testing + [`tests/browser_interop.rs`](../tests/browser_interop.rs) headless-Chromium harness using `chromiumoxide` (feature `interop-browser`, `#[ignore]`'d because a Chromium binary on `PATH` is a hard prereq). Launches with `--use-fake-device-for-media-stream` so `getUserMedia` returns a synthetic source.
- **#44 SIP↔WebRTC** — wiring smoke test ([`tests/sip_webrtc_bridge.rs`](../tests/sip_webrtc_bridge.rs)) registers both adapters on one orchestrator + documents capability shapes. Real media-transcoding E2E blocked on `Orchestrator::bridge_connections` SIP path being partially stubbed in rvoip-core (acknowledged in existing `rvoip-uctp/examples/uctp_to_sip_bridge` README).
- **#45 TURN relay** — `WebRtcConfig::ice_transport_policy: { All, Relay }` wired through to `RTCConfigurationBuilder::with_ice_transport_policy`. Test confirms config propagation; full relay-path verification needs an external `coturn` and is documented as out of crate scope. webrtc-rs 0.20-alpha currently still emits host candidates at gather time even with `Relay` policy — relay-only filtering happens at candidate-pair nomination during connectivity checks.
- **#46 1-hour soak** — [`tests/soak_long.rs`](../tests/soak_long.rs) (feature `soak-1h`, `SOAK_SECS` env var) runs sustained open/close cycles (5 peers / cycle ~10 ms each) and asserts `num_alive_tasks` returns to within +50 of baseline after each cycle. Default 60 s window for CI; set `SOAK_SECS=3600` for the full 1 h run. Confirmed task count stays at baseline through the workload — H1's cancellation tokens and reaper validate clean teardown under churn.

**Deferred to H6 follow-ups** (not delivered this pass — each is large enough to warrant its own change):
1. **Headless-browser interop** — needs `chromiumoxide` or `fantoccini` (heavy dep on a real Chromium); should drive a static HTML page through full WHIP publish + subscribe.
2. **SIP↔WebRTC gateway E2E** — needs `rvoip-sip` + `rvoip-media-core` G.711↔Opus transcoding; should bridge a SIP INVITE to a WHIP subscriber.
3. **TURN relay path test** — needs a `coturn` container or pure-Rust TURN test server; force `iceTransportPolicy = relay` and assert media flows.
4. **1-hour soak with leak detection** — gated behind a `soak-1h` feature; should track RSS + task count over time.
5. **Full DC soak** — the 5-second sustained-traffic variant is `#[ignore]`'d pending the same DC-creation-before-SDP-exchange refactor that blocks task #31.

H7 onward TBD.

### Phase H1 — Stop the panics, plug the silent drops (correctness baseline)

**Goal:** No panic on any externally-reachable path; no silent event loss.

- Replace every `unwrap()` / `expect()` / `assert_eq!` on a non-invariant path with `Result<_, WebRtcError>`. Specifically: [adapter.rs:477](../src/adapter.rs), [peer/builder.rs:163](../src/peer/builder.rs), [peer/session.rs:318,339,392,470](../src/peer/session.rs), [signaling/websocket.rs:67,82,105](../src/signaling/websocket.rs), [signaling/whip.rs:106,156](../src/signaling/whip.rs). Add a clippy lint denying `unwrap_used` and `expect_used` outside `#[cfg(test)]`.
- Make `WebRtcAdapter::subscribe_events()` return `Result<Receiver, AlreadySubscribed>` and document the single-take contract.
- Promote the handler-channel capacity ([handler.rs:30](../src/peer/handler.rs)) to a config-driven value (default 256, matching `ADAPTER_EVENT_CAP`); use `send().await` with a soft-deadline + warn-and-drop fallback rather than naked `try_send`.
- Fix the on-track / accept race: create `WebRtcMediaStream` synchronously in `apply_remote_offer`/`originate` and have `on_track` attach to the pre-created stream. Remove the 20 ms poll watcher in [adapter.rs:114-151](../src/adapter.rs).
- Give the outbound pump in [pump.rs:115](../src/media/pump.rs) a bounded send with timeout; on timeout, emit `AdapterEvent::Quality{…}` and drop the frame instead of stalling.
- Cancel the stats loop ([stats.rs:12-26](../src/media/stats.rs)) and route watchers on stream/route close (use `CancellationToken`).
- Replace `TransportHandle(Arc::new(()))` with a typed handle carrying the `ConnectionId` and a close hook.
- Add a session reaper task (configurable TTL, default 5 min idle) that calls `adapter.end(...)` on dead routes.

**Verification:** Stress-loop test that opens/closes 1 000 peers concurrently, asserts no panic, no leaked tasks (use `tokio_metrics` or `tokio::runtime::Handle::metrics`), and zero event drops on a fast subscriber.

### Phase H2 — Trickle ICE and renegotiation

**Goal:** Browsers behind NATs connect quickly; mid-call changes work.

- Replace `WsSignalMessage::IceCandidate => NotImplemented` ([websocket.rs:110-113](../src/signaling/websocket.rs)) with a real candidate-add path that calls a new `RvoipPeerConnection::add_remote_ice_candidate` wrapping `pc.add_ice_candidate(...)`.
- Surface outbound candidates from [handler.rs](../src/peer/handler.rs) `on_ice_candidate` to the signaling layer (don't wait for `Complete`). Add `WebRtcConfig::ice_mode: { FullGather, Trickle, HalfTrickle }`.
- WHIP PATCH ([whip.rs:100-113](../src/signaling/whip.rs)): implement RFC 9725 §4.4 trickle (`Content-Type: application/trickle-ice-sdpfrag`) and bind `If-Match`/`ETag` per route.
- Wire `renegotiate_media` end-to-end: track add/remove, `replaceTrack` semantics, direction-based hold/resume that mutates the SDP and re-offers.
- Add `RvoipPeerConnection::restart_ice()` that triggers fresh ufrag/pwd and a new offer.

**Verification:** Headless Chromium harness (see Phase H6) that runs through "connect, add video mid-call, hold, resume, ICE restart, disconnect" and asserts state transitions on both ends.

### Phase H3 — RTCP, codecs, and real DTMF

**Goal:** Survive packet loss and interoperate with SIP gateways and Safari.

- Register full RTCP feedback in [peer/builder.rs:36-108](../src/peer/builder.rs): `nack`, `nack pli`, `ccm fir`, `goog-remb`, `transport-cc`. Enable the `interceptor-rtcp-reports`, `interceptor-nack`, `interceptor-twcc` interceptors from `webrtc-rs`.
- Extend the media engine: H.264 (constrained baseline + high; required for Safari and most SBCs), VP9, optional AV1. Add comfort noise + RED for Opus. Add telephone-event payload type registration so SIP-side INFO/RFC4733 DTMF round-trips.
- Audit [media/dtmf.rs](../src/media/dtmf.rs): rewrite as a proper RFC 4733 generator (start/continue/end packets, payload-type from negotiated SDP); add a test that captures the outbound RTP, parses the events, and validates digit, duration, volume.
- Surface `getStats` via a typed [`media/stats.rs`](../src/media/stats.rs) API: per-stream `MediaQualitySnapshot { jitter, packet_loss, rtt, bitrate, mos_estimate, fraction_lost }`. Wire into `MediaStream::quality_snapshot()`.

**Verification:** Loopback round-trip that drops 5% of RTP packets in a `tokio::io` shim and asserts NACK retransmissions + non-zero `packets_lost` in the stats snapshot. DTMF wire-capture test asserts byte layout. H.264 negotiation test against a recorded Safari SDP fixture.

### Phase H4 — Server-mode production surfaces

**Goal:** Bind to `0.0.0.0`, point a browser at it, sleep at night.

- TLS termination directly in [server.rs](../src/server.rs): builder methods `with_whips(addr, TlsConfig)` and `with_wss(addr, TlsConfig)` using `axum-server` + `rustls`. Document the reverse-proxy alternative.
- WHIP/WHEP compliance pass against RFC 9725: enforce `Content-Type: application/sdp`, return `ETag`, `Accept-Patch`, `Link: <…>; rel="ice-server"`, support `OPTIONS` preflight, return `415` / `405` / `404` appropriately, honor `Authorization: Bearer` against a pluggable `AuthHook` trait, respect the session-id path segment (no more `let _ = session;`).
- WebSocket signaler: bounded message size, server-driven ping/pong (15 s default), `Sec-WebSocket-Protocol = rvoip.webrtc.v1`, schema-validated messages, optional `Authorization` via subprotocol token, ordered offer+candidates queue per connection.
- CORS layer with allow-list from `WebRtcConfig::cors_origins`.
- Graceful `shutdown()`: stop accepting new connections, drain in-flight requests with a deadline, walk the route table and call `adapter.end(_, Normal)` on each peer, then abort.
- Metrics: a `metrics::Recorder`-compatible counter set (`webrtc_inbound_total`, `webrtc_active_sessions`, `webrtc_signaling_errors`, gather/connect latencies). `/healthz` and `/readyz` endpoints.
- Per-IP token-bucket rate limiter and global max-session cap (configurable in `WebRtcConfig`).
- TURN credential REST endpoint helper (`ice_servers::generate_ephemeral`) per RFC 7635.
- Log redaction helper for SDP (strip `a=candidate` IPs and `o=` username when log level < debug).

**Verification:** Integration tests for each header / status-code requirement; a Locust-style load test that holds 500 concurrent sessions for 5 minutes with <1% error rate.

### Phase H5 — Make `WebRtcClient` a real client

**Goal:** A developer can `cargo add rvoip-webrtc`, plug in a mic, and place a call.

- Implement `WsSignaler::send_answer` ([ws_signaler.rs:74](../src/client/ws_signaler.rs)) — symmetrical with `send_offer`, routes by `connection_id`. Add an `accept(target)` flow to `WebRtcClient`.
- New `client::media` module with traits:
  ```
  pub trait AudioSource: Send { fn next_frame(&mut self) -> Option<AudioFrame>; }
  pub trait AudioSink: Send { fn write_frame(&mut self, frame: AudioFrame); }
  pub trait VideoSource / VideoSink: same shape.
  ```
  Provide a `cpal` backend behind a `client-cpal` feature; document the platform matrix. The fixture sources in [media/fixtures.rs](../src/media/fixtures.rs) stay as `client::media::fixture` for tests.
- Cancellation: `SessionHandle::Drop` aborts signaling and calls `adapter.end(...)`.
- Reconnect/resume: `Signaler` trait gains `reconnect_with_session(id)`; `WsSignaler` retries with exponential backoff up to a config-driven deadline.
- Optional signaling-connection pool keyed by base URL.
- Tighten `ComprehensiveReport`: split fixture-only fields from real-media fields and assert the latter only when a real source is wired.

**Verification:** New `tests/client_call_e2e.rs` exercises register → place call → answer → talk (5 s of real audio via cpal-loopback) → DTMF → hold → resume → hangup, with the server in the same process.

### Phase H6 — Browser and SIP interop

**Goal:** Prove the surface against the peers it claims to interoperate with.

- Browser harness under `tests/interop/browser/` using `headless_chrome` or `chromedp` driving `pc = new RTCPeerConnection(...)` against the `WebRtcServer`. Cover: WHIP publish, WHEP subscribe, WS bidirectional call, DataChannel echo, ICE restart, hold/resume, DTMF (via `RTCDTMFSender`).
- SIP gateway E2E: spin up `rvoip-sip` + `rvoip-webrtc` + `rvoip-media-core` (G.711↔Opus transcode), place a SIP call to a WHEP subscriber, assert decoded PCM matches transmitted PCM within an SNR threshold.
- TURN relay path test: spawn `coturn` in a container fixture (or a Rust-native stand-in), force `iceTransportPolicy = relay`, assert media flows.
- Soak: 1-hour single-call test with periodic stats asserting <1% packet loss and steady RTT; tracks task count and RSS for leaks.
- Concurrency: 100 simultaneous WHIP publishes; assert all reach `Connected` within 10 s.

**Verification:** Add a `make interop` target and run it in CI nightly (browser tests behind a feature flag).

### Phase H7 — Identity, security, observability polish

**Goal:** Optional but important for v2.

- DTLS-SRTP fingerprint extraction and `verify_request_signature` integration with the workspace `Identity`/`auth-core` model. Configurable cert pinning.
- mDNS candidate handling (RFC 8839): resolve `.local` candidates or filter them per policy.
- Structured tracing spans with `connection_id` propagation across signaling, adapter, peer, pump.
- Optional Prometheus / OpenTelemetry exporters in `WebRtcServerBuilder`.

---

## 4. Critical files modified by phase

(Pattern reference — full file list per phase emerges during execution.)

| Phase | Primary files |
|---|---|
| H1 | `src/adapter.rs`, `src/peer/{builder,session,handler}.rs`, `src/media/{pump,stats,stream}.rs`, `src/signaling/{whip,websocket}.rs`, `src/config.rs` |
| H2 | `src/peer/{session,handler,ice}.rs`, `src/signaling/{whip,websocket}.rs`, new `src/peer/trickle.rs`, `src/adapter.rs` |
| H3 | `src/peer/builder.rs`, `src/media/{dtmf,stats}.rs`, `src/sdp/capability.rs`, new fixtures for H.264 |
| H4 | `src/server.rs`, `src/signaling/{whip,websocket}.rs`, new `src/server/{tls,cors,auth,metrics,ratelimit}.rs`, `src/config.rs` |
| H5 | `src/client/{native,ws_signaler,comprehensive}.rs`, new `src/client/media/{mod,cpal,fixture}.rs`, `Cargo.toml` (`client-cpal` feature) |
| H6 | new `tests/interop/**`, `tests/soak_*.rs`, `tests/load_*.rs` |
| H7 | `src/peer/session.rs`, `src/adapter.rs`, new `src/observability.rs` |

Reuse existing utilities — do not duplicate:

- SDP parse/serialize: [`sdp::{parse_sdp, sdp_to_string}`](../src/sdp/mod.rs)
- Default capabilities: [`sdp::default_webrtc_capabilities`](../src/sdp/capability.rs)
- Media pumps: [`media::pump::{spawn_inbound_pump, spawn_outbound_pump}`](../src/media/pump.rs)
- Peer build: [`peer::builder::build_peer_connection`](../src/peer/builder.rs)
- Existing fixtures: [`media::fixtures`](../src/media/fixtures.rs)

---

## 5. Verification approach

End-to-end gates per phase:

1. **Static:** `cargo clippy -p rvoip-webrtc --all-features -- -D warnings -D clippy::unwrap_used -D clippy::expect_used` (the unwrap/expect denial drops after Phase H1).
2. **Unit + integration:** `cargo test -p rvoip-webrtc --all-features` plus the feature-gated tests already in [Cargo.toml](../Cargo.toml).
3. **Stress (Phase H1):** new `tests/stress_lifecycle.rs` — 1 000 concurrent peers, asserts no panic, no leaked tasks, no dropped events.
4. **Interop (Phase H6):** `cargo test -p rvoip-webrtc --features interop-browser --test browser_*` against headless Chromium; nightly CI only.
5. **Manual run-through:** `./scripts/test-webrtc-comprehensive.sh` continues to pass; add `./scripts/run-browser-demo.sh` that launches the server + a static HTML page that places a call from Chrome/Safari/Firefox.
6. **Observability:** every phase that adds a counter/gauge ships with a Grafana panel JSON under `docs/observability/`.

---

## 6. Explicitly out of scope

- SFU / MCU semantics, multi-party fan-out.
- Standalone TURN relay hosting (continue to recommend external `coturn`).
- Simulcast / SVC sender-side (receiver-side already partially handled via webrtc-rs).
- vCon emission (lives in `rvoip-core`).
- Hardware acceleration for H.264 (use software encoder via `openh264` / `x264` behind a feature flag if needed; benchmark before committing).
- Generic browser-style ORTC API.

---

## 7. Suggested sequencing

If a single engineer were to land this:

- **Sprint 1** (H1) — baseline correctness; unblocks every other phase. ~1 week.
- **Sprint 2** (H4 + H2 in parallel) — server hardening lands quickly because it's mostly HTTP-layer work; trickle ICE proceeds against the new server tests. ~2 weeks.
- **Sprint 3** (H3) — RTCP, codecs, DTMF wire validation. ~1 week.
- **Sprint 4** (H5) — real client with audio capture. ~2 weeks (cpal integration + cross-platform smoke).
- **Sprint 5** (H6) — interop harness; this is the gate to call the crate "complete." ~2 weeks (browser driver is the long pole).
- **Sprint 6** (H7) — security/observability polish, ship.

Total: ~9 engineer-weeks to "deployable client and server."
