# rvoip-webrtc Gap Plan (post-H1–H7)

**Deliverable location:** [`crates/rvoip-webrtc/docs/GAP_PLAN.md`](GAP_PLAN.md)

**Companion docs:**
- [`docs/IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) — original phases 0–11 (✅).
- [`docs/HARDENING_PLAN.md`](HARDENING_PLAN.md) — H1–H7 audit + remediation (✅).
- [`CHANGELOG.md`](../CHANGELOG.md) — release notes for the hardening arc.

**Audit date:** 2026-05-24. **Implementation status:** post G1–G12 (see [GAP_IMPLEMENTATION_PLAN.md](GAP_IMPLEMENTATION_PLAN.md) for the operational plan and [CHANGELOG.md](../CHANGELOG.md) for what landed).
**Build target:** `webrtc-rs 0.20.0-alpha.1` (workspace-pinned).
**Scope of this plan:** finish the journey from "production-deployable 1:1 WebRTC gateway/server" (where H7 left the crate) to "drop-in WebRTC client/server library a developer can `cargo add` and use to ship browser-talking apps without an external proxy." All work stays under `crates/rvoip-webrtc/**` unless explicitly noted.

---

## 1. Executive grading

**Updated 2026-05-24 (post G1–G12):** every 🔴 row below is now 🟢, except
the four explicitly-deferred items tracked in §3 (G7 multi-codec audio,
G3 cpal/nokhwa backends, G10 identity binding, G9b SIP media E2E).

The H1–H7 arc already closed the panic / silent-drop / no-TLS / no-trickle / no-metrics class of issues that blocked any real deployment. As of v0.1.26+H7 the crate is a **credible, narrowly-scoped 1:1 WebRTC interop adapter and signaling server** with these properties:

| Dimension | Verdict |
|---|---|
| Core ICE / DTLS-SRTP / SDP / RTP plumbing | ✅ Inherited cleanly from webrtc-rs 0.20-alpha; no wrapper gaps |
| WHIP/WHEP server (RFC 9725) | ✅ Surface compliance shipped in H4 — *except* Bearer auth and `Accept-Post` (see §3.4) |
| WS JSON signaling | ✅ Solid for first-party clients; ⚠ no schema/version negotiation |
| Trickle ICE (RFC 8838 + RFC 8840) | ✅ Bidirectional WS + WHIP PATCH |
| Codec coverage (Opus, G.711, VP8, VP9, H.264 CB) | ✅ Sufficient for Chrome/Firefox/Safari + SIP bridge |
| RTCP feedback (NACK, PLI, FIR, REMB, TWCC) | ✅ Registered on all video codecs + Opus |
| Stats / observability | ✅ Typed snapshot + Prometheus exporter |
| TLS termination | ✅ In-process WHIPS/WSS (feature `tls-rustls`) |
| Identity (DTLS fingerprint extraction) | ⚠ Partial — extracted, not bound to `IdentityAssurance` (rvoip-core blocker) |
| Real client surface | ⚠ Trait-level (`AudioSource`/`AudioSink`) only — **no microphone/camera backend** |
| Data channel configurability | ⛔ Hard-coded defaults; no API for `ordered`, `maxRetransmits`, `maxPacketLifetime`, `protocol`, `negotiated` |
| Simulcast / SVC | ⛔ Detection-only; no negotiation or per-encoding pump |
| SFU / multi-party | ⛔ Out of scope (correct — this is a 1:1 adapter) |
| Browser interop CI coverage | ⚠ Static demo pages + headless harness exist (`#[ignore]`'d) — not run nightly |
| SIP↔WebRTC media E2E | ⚠ Wiring smoke test only; blocked on rvoip-core orchestrator bridge for SIP |

**Headline rubric (against the 2026 WebRTC standards landscape):**

- **Browser-interop transport library:** ~92% of "Must" requirements satisfied. Remaining gaps: WHIP Bearer auth, data-channel options API, `Accept-Post` header, explicit `extmap-allow-mixed` advertisement check, perfect-negotiation rollback support, ICE consent-freshness exposure (handled by webrtc-rs, not asserted by tests).
- **SIP-WebRTC bridge:** DTMF wire format ✅, G.711 ✅, but the DTMF wire test is `#[ignore]`'d pending a multi-codec audio transceiver fix (§3.6); orchestrator-level SIP bridge media is wiring-only (§3.10).
- **WHIP ingest server:** Full RFC 9725 §4.x surface *except* §4.1 Bearer authentication, optional `Accept-Post` advertisement, and `Link: rel="ice-server"` is configurable but not auto-populated from `ice_servers`.
- **End-user WebRTC client:** Plumbing is there, **but a developer cannot `cargo add rvoip-webrtc` and place a call from a microphone today** — they must implement an `AudioSource` themselves on top of cpal/AVFoundation/etc.

---

## 2. Cross-reference: standards rubric ↔ implementation

Severity legend: 🔴 blocker (Must-level miss for the stated role), 🟡 gap (Should-level miss or scope creep into App layer), 🟢 met (✅ in code + tests), ⚪ deferred by design.

### 2.1 Core protocol (ICE / DTLS-SRTP / SDP)

| Rubric item | Status | Notes |
|---|---|---|
| ICE (RFC 8445) | 🟢 | webrtc-rs 0.20-alpha; full gather + trickle modes via `WebRtcConfig::trickle_ice` |
| STUN (RFC 8489 + 5389 compat) | 🟢 | Default Google STUN server in `WebRtcConfig::default()` |
| TURN UDP (RFC 8656) | 🟢 | Configurable via `IceServerConfig::turn(...)` |
| TURN TCP / TLS (RFC 6062, 7065) | 🟡 | URL parsing supported; not exercised by any in-tree test |
| DTLS-SRTP + AES-GCM (RFC 5764 / 7714) | 🟢 | Inherited from webrtc-rs; modern default profile |
| DTLS 1.3 (RFC 9147) | 🟢 | Inherited from webrtc-rs |
| SDP base (RFC 4566) + JSEP (RFC 8829) | 🟢 | Offer/answer state machine working both roles |
| BUNDLE (RFC 9143) | 🟢 | Inherited; default is bundle-all |
| rtcp-mux (RFC 5761) | 🟢 | Inherited |
| Trickle ICE (RFC 8838) | 🟢 | H2 |
| Trickle SDP fragment (RFC 8840) | 🟢 | H2; WHIP `PATCH application/trickle-ice-sdpfrag` parses `a=mid` scoped candidates |
| ICE restart (RFC 8839 §5.3) | 🟢 | `RvoipPeerConnection::restart_ice()` + WHIP PATCH SDP variant |
| Consent freshness (RFC 7675) | 🟢 | Inherited from webrtc-rs; **not asserted by any test** |
| mDNS host candidates (RFC 8839) | 🟢 | `MdnsCandidatePolicy::Drop` (safe default for hosted servers); `Pass` for LAN deployments |
| SDP rollback / perfect negotiation | 🟢 | G11 ships `RvoipPeerConnection::rollback_local()`; G3 ships `PerfectNegotiation` helper with W3C polite/impolite collision resolution |
| ICE PAC (RFC 8863) | 🟢 | Inherited |

### 2.2 Media plane (RTP / RTCP / codecs / header extensions)

| Rubric item | Status | Notes |
|---|---|---|
| RTP base (RFC 3550 / 3551) | 🟢 | webrtc-rs |
| AVPF feedback (RFC 4585) + PLI/FIR (RFC 5104) | 🟢 | Registered per codec in [`peer/builder.rs`](../src/peer/builder.rs) |
| RTX (RFC 4588) | 🟢 | Default interceptors via `register_default_interceptors` |
| TWCC (`transport-wide-cc-extensions`) | 🟢 | Registered on all video + Opus |
| REMB (legacy) | 🟢 | Registered on video for backward-compat |
| abs-send-time header ext | 🟢 | G6 — explicitly registered (audio + video) in `build_media_engine` |
| `urn:ietf:params:rtp-hdrext:sdes:mid` (RFC 9335) | 🟢 | G6 — explicitly registered (audio + video) |
| `urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id` (RFC 8852, RID) | 🟢 | G6 — explicitly registered (video); RID + repaired-RID both |
| `extmap-allow-mixed` (RFC 8285) | 🟢 | G6 — Chrome fixture asserts round-trip |
| Audio level header ext (RFC 6464) | 🟢 | G6 — explicitly registered; Safari + Chrome fixtures assert |
| Opus (RFC 6716 / 7587) | 🟢 | PT 111, stereo, `useinbandfec=1`, `minptime=10` |
| Opus FEC / DTX surface | 🟢 | G12 — `OpusSettings` config with `use_in_band_fec` / `use_dtx` / `min_ptime_ms` / `max_average_bitrate_bps` / `stereo`; threads into the engine fmtp line |
| RED for Opus (RFC 2198) | ⚪ | Not implemented; nice-to-have for lossy audio |
| Comfort Noise (RFC 3389) | ⚪ | Not registered |
| G.711 PCMU/PCMA (RFC 3551) | 🟢 | PT 0 / 8 |
| G.722 (RFC 3551) | ⚪ | Not registered; some SIP gateways expect it |
| DTMF (RFC 4733 telephone-event) | 🟡 | PT 101 registered; encoder ships correct wire format; **wire-capture test `#[ignore]`'d** due to single-codec transceiver limitation (`add_local_audio_track` only advertises Opus) |
| VP8 (RFC 7741) | 🟢 | PT 96, full feedback |
| VP9 | 🟢 | PT 98, profile-id=0 |
| H.264 Constrained Baseline | 🟢 | PT 102, `packetization-mode=1`, `profile-level-id=42e01f` (Safari path) |
| H.264 other profiles (high, baseline level mismatch) | 🟡 | Only `42e01f` offered; some SBCs require baseline `42001f` |
| AV1 | ⚪ | Not registered; webrtc-rs 0.20-alpha support unclear |
| H.265 / HEVC | ⚪ | Out of scope; Chrome 136+ ships but Firefox does not |
| Simulcast (RFC 8853 + RID) | 🔴 (for streaming use) / ⚪ (1:1 only) | Detected in SDP via `sdp_indicates_simulcast()`; not negotiated |
| SVC (webrtc-svc) | ⚪ | Inherited from webrtc-rs only if upstream supports |

### 2.3 Data channel (SCTP / DCEP)

| Rubric item | Status | Notes |
|---|---|---|
| SCTP over DTLS (RFC 4960) | 🟢 | webrtc-rs |
| Data channel framing (RFC 8831) | 🟢 | Inherited |
| DCEP (RFC 8832) | 🟢 | Inherited |
| `m=application` SDP (RFC 8841) | 🟢 | Inherited |
| Sender-side fragmentation (`>max-message-size`) | 🟢 | Handled by webrtc-rs sctp transport |
| `ordered` / `maxRetransmits` / `maxPacketLifetime` config | 🟢 | G1 — [`DataChannelOptions`](../src/peer/data_channel.rs) typed constructors (`reliable`, `unreliable`, `partial_reliable_retransmits`, `partial_reliable_lifetime`) |
| `protocol` field (DCEP) | 🟢 | G1 — `DataChannelOptions::with_protocol(...)` |
| `negotiated` / pre-agreed stream id | 🟢 | G1 — `DataChannelOptions::with_negotiated_id(...)` |
| Empty-message support (PPID 56 / 57) | 🟢 | Inherited from webrtc-rs |
| Binary vs text PPID disambiguation | 🟢 | Inherited |
| Backpressure on `send_text` | 🟡 | No explicit `bufferedAmount` accessor or `bufferedAmountLowThreshold` event on the wrapper |
| I-DATA / message interleaving (RFC 8260) | 🟢 | Inherited (modern coturn/webrtc-rs default) |
| Soak test under sustained DC traffic | 🟢 | [`tests/dc_soak.rs`](../tests/dc_soak.rs) — 20-cycle no-panic loop + 5 s ping/pong |

### 2.4 Signaling surfaces

| Rubric item | Status | Notes |
|---|---|---|
| WHIP POST `application/sdp` (RFC 9725 §4.1) | 🟢 | Content-Type validated; 415 on mismatch |
| WHIP `Location` on 201 | 🟢 | |
| WHIP `ETag` / `If-Match` | 🟢 | G2 — `ETag` emitted; `If-Match` enforced on ICE-restart PATCH (412 on mismatch, 428 when missing) |
| WHIP `Accept-Patch` | 🟢 | `application/sdp, application/trickle-ice-sdpfrag` |
| WHIP `Link: rel="ice-server"` | 🟢 | G2 — auto-populated from `WebRtcConfig::ice_servers` with `username`/`credential` when present |
| WHIP `Accept-Post: application/sdp` | 🟢 | G2 — emitted on `OPTIONS` and 4xx error responses |
| WHIP Bearer auth (RFC 9725 §4.1) | 🟢 | G2 — `WhipAuthHook` trait + `BearerStaticTokenAuth` reference impl + `serve_listener_with_auth(...)` |
| WHIP DELETE | 🟢 | |
| WHIP PATCH `application/trickle-ice-sdpfrag` (RFC 8840) | 🟢 | 204 / 400 / 404 / 415 |
| WHIP PATCH `application/sdp` (ICE restart) | 🟢 | |
| CORS preflight (`OPTIONS`) | 🟢 | `tower-http` CorsLayer when `cors_origins` configured |
| WHEP POST | 🟢 | |
| WHEP PATCH (subscriber answer) | 🟢 | |
| Multiple subscribers per WHEP source | 🟡 | Routing model is one-route-per-session-id; fan-out is SFU territory but the docs don't say so |
| `/healthz` / `/readyz` | 🟢 | |
| `/metrics` (Prometheus text) | 🟢 | |
| Per-IP rate limit | 🟢 | Token bucket on WHIP POST |
| Session cap | 🟢 | Atomic `SessionSlotGuard` (race-free) |
| Graceful shutdown w/ drain | 🟢 | `WebRtcServer::shutdown_with_deadline` |
| WS message schema validation | 🟡 | serde-parse only; no JSON Schema or versioning |
| WS subprotocol negotiation (`Sec-WebSocket-Protocol`) | 🟢 | G2 — echoed during upgrade; default `rvoip.webrtc.v1` when offered |
| WS auth (Bearer via subprotocol or query) | 🟢 | G2 — `WsAuthHook` trait + token via `token.<value>` subprotocol or `?access_token=` query |
| WS ping/pong keepalive | 🟢 | `ws_keepalive_secs` |
| WS max message size | 🟢 | `ws_max_message_size` (1 MB default) |
| WS ICE candidate scoping by `connection_id` | 🟢 | H5 |

### 2.5 Security & identity

| Rubric item | Status | Notes |
|---|---|---|
| HTTPS for WHIP / WSS for WS | 🟢 | `tls-rustls` feature; in-process termination via `axum-server` + `tokio-rustls` |
| DTLS-SRTP fingerprint extraction | 🟢 | `WebRtcAdapter::remote_dtls_fingerprint(conn)` |
| Fingerprint binding to `IdentityAssurance` | ⚪ deferred | G10 — blocked on rvoip-core adding the `DtlsFingerprint` enum variant; wrapper change is one line when upstream lands |
| Cert pinning hook | 🟡 | Out-of-band via `remote_dtls_fingerprint`; no pre-handshake reject callback |
| Bearer auth (WHIP/WS) | 🔴 | Missing |
| TURN credential rotation (RFC 7635 / draft-uberti-behave-turn-rest) | 🟢 | `turn_rest::generate_ephemeral` ships HMAC-SHA256 ephemeral credentials |
| SDP log redaction | 🟡 | No helper; full SDP can leak host IPs into traces at `debug` level |
| W3C WebRTC Identity (`setIdentityProvider`) | ⚪ | No browser ships a real IdP integration in 2026; out of scope |

### 2.6 Observability

| Rubric item | Status | Notes |
|---|---|---|
| Inbound RTP stats (packets/bytes/loss/jitter) | 🟢 | `WebRtcStatsSnapshot` |
| Outbound RTP stats | 🟢 | G4 — `OutboundStats` shipped (packets/bytes/retx/NACK/PLI/FIR); merged via `InboundStats::merge_webrtc_report` |
| Per-pair RTT | 🟡 | Available via webrtc-rs `get_stats`; not surfaced in `WebRtcStatsSnapshot` |
| Candidate-pair stats | 🟢 | G4 — `CandidatePairStats` shipped (local/remote candidate type, current/total RTT, available outgoing bitrate, responses received) |
| MOS estimate | 🟢 | Simple R-factor in `pump.rs` |
| RTCP XR (RFC 3611) | ⚪ | Not exposed; webrtc-rs support unclear |
| Prometheus exporter | 🟢 | `observability::render_prometheus` |
| `#[instrument]` spans on adapter entries | 🟢 | H7 |
| Structured event log per connection | 🟡 | Adapter events emit; no per-connection ICE/DTLS/codec timeline |
| Health / readiness endpoints | 🟢 | |

### 2.7 Client surface

| Rubric item | Status | Notes |
|---|---|---|
| `Signaler` trait + `WsSignaler` | 🟢 | H5 |
| Offerer + Answerer flows | 🟢 | H5 |
| Exponential-backoff signaling reconnect | 🟢 | `WsSignalerConfig::retry_max_attempts` |
| `SessionHandle::close()` + Drop-cleanup | 🟢 | H5 |
| `AudioSource` / `AudioSink` traits | 🟢 | H5 |
| `VideoSource` / `VideoSink` traits | ⚪ deferred | G3 — trait skeleton in plan; cpal/nokhwa-backed impls deferred (workspace dep scope) |
| Microphone backend (cpal / AVFoundation / WASAPI) | ⚪ deferred | G3 cpal-backed `AudioSource` deferred — needs workspace dep additions |
| Camera backend (nokhwa / AVFoundation / V4L2) | ⚪ deferred | Same as above |
| Signaling-connection pool (one WS, many calls) | 🟢 | G3 — `SignalingPool` keyed by base URL with idle TTL |
| Session resume after signaling blip | ⚪ deferred | `WsSignalerConfig` retry/backoff exists; full mid-handshake resume deferred |
| `perfect-negotiation` glare resolution helper | 🟢 | G3 — `PerfectNegotiation` + `NegotiationAction` typed enum |

### 2.8 Interop & tests

| Rubric item | Status | Notes |
|---|---|---|
| Two-peer loopback | 🟢 | `tests/loopback.rs` |
| Recorded-Chrome SDP fixture | 🟢 | `tests/browser_sdp_interop.rs` (Chromium 120) |
| Headless-Chromium harness | 🟡 | `tests/browser_interop.rs` exists, `#[ignore]`'d — requires Chromium on `PATH`, not run in CI |
| Recorded-Safari SDP fixture | 🟢 | G6 — `tests/browser_sdp_interop.rs::safari_audio_offer_negotiates_opus_and_echoes_audio_level` |
| Recorded-Firefox SDP fixture | 🟢 | G6 — `tests/browser_sdp_interop.rs::firefox_audio_offer_negotiates_opus_with_mid_hdrext` |
| Lossy-link RTP simulation (NACK verification) | 🟡 | G5 ships `spawn_lossy_udp_proxy` helper; drop-rate-driven NACK round-trip blocked on webrtc-rs candidate-override APIs not in 0.20-alpha |
| TURN relay full path | 🟡 | `ice_transport_policy: Relay` wired; no coturn fixture |
| SIP↔WebRTC media transcode E2E | ⚪ deferred | G9b — blocked on orchestrator SIP bridge landing in rvoip-core |
| 1-hour soak | 🟢 | `tests/soak_long.rs` (9 701 cycles validated) |
| Load test | 🟢 | `tests/whip_load.rs` (50 concurrent) — but only at WHIP HTTP layer, not at sustained media flow |

---

## 3. Gap roadmap (G-series — picks up where H7 left off)

Ordered by user-visible impact and unblock value. Each phase is self-contained and shippable. Effort estimates are in engineer-days for a single contributor familiar with the crate.

### Phase G1 — Data channel configurability + DC backpressure (1–2 d) 🔴

**Why:** Without an `ordered`/`maxRetransmits` API, the crate cannot honestly claim to support production data-channel use cases (gaming, file transfer, partial-reliable telemetry).

**Tasks:**

1. Replace [`RvoipPeerConnection::create_data_channel(label)`](../src/peer/session.rs:301) with a typed `DataChannelOptions`:
   ```rust
   pub struct DataChannelOptions {
       pub ordered: bool,                       // default true
       pub max_retransmits: Option<u16>,        // mutually exclusive with max_packet_lifetime
       pub max_packet_lifetime_ms: Option<u16>,
       pub protocol: Option<String>,
       pub negotiated_id: Option<u16>,          // pre-agreed stream id, skips DCEP
   }
   pub async fn create_data_channel(self: &Arc<Self>, label: &str, opts: DataChannelOptions) -> Result<...>
   ```
   Map to webrtc-rs `RTCDataChannelInit` (RFC 8832 fields).
2. Add `RvoipDataChannel::buffered_amount()` + `set_buffered_amount_low_threshold(u64)` + an `on_buffered_amount_low` event channel.
3. Add `RvoipDataChannel::max_message_size()` accessor (negotiated via SDP `a=max-message-size`).
4. New `tests/dc_options.rs`: open ordered + unordered + partial-reliable channels, send messages over each, assert round-trip, assert `buffered_amount` drains.

**Verification:** All existing DC tests pass; new test covers all five RFC 8832 modes.

### Phase G2 — WHIP authentication + missing headers (2–3 d) 🔴

**Why:** RFC 9725 §4.1 mandates Bearer auth interop. Production WHIP clients (OBS, GStreamer `whipclientsink`, browser SDKs) all send `Authorization: Bearer ...`. Today every POST is accepted unauthenticated.

**Tasks:**

1. New `WhipAuthHook` trait in `src/signaling/whip.rs`:
   ```rust
   #[async_trait]
   pub trait WhipAuthHook: Send + Sync {
       async fn authenticate(&self, bearer: Option<&str>, session_hint: &str, addr: SocketAddr)
           -> Result<AuthContext, AuthRejection>;
   }
   pub enum AuthRejection { Unauthorized, Forbidden, Throttled }
   ```
   `WebRtcServerBuilder::with_whip_auth(Arc<dyn WhipAuthHook>)` registers it; default = `AllowAnonymous`.
2. Emit `Accept-Post: application/sdp` on `OPTIONS /whip/*` and on every error 4xx response (browser-side feature detection).
3. Auto-populate `Link: <stun:...>; rel="ice-server"` headers from `WebRtcConfig::ice_servers` on every 201 CREATED. Today the header only fires when the application populates it explicitly.
4. Enforce `If-Match: <etag>` on PATCH for ICE restart; 412 Precondition Failed on mismatch (RFC 9725 §4.4.1).
5. Symmetric work in `signaling/websocket.rs`: pluggable `WsAuthHook` checking a token via subprotocol token or query parameter; reject before upgrade.

**Verification:** Extend `tests/whip_compliance.rs` to cover (a) anonymous POST when no auth hook → 201; (b) authenticated POST → 201; (c) anonymous POST when auth hook registered → 401; (d) PATCH with stale ETag → 412; (e) `OPTIONS` → 204 with `Accept-Post`.

### Phase G3 — Real client surfaces: mic + camera + reconnect (4–6 d) 🔴

**Why:** "Real client" was the H5 goal; H5 delivered the trait surface. The actual platform glue is still missing — a developer can't `cargo add rvoip-webrtc` and place a call from a laptop today without writing platform code themselves.

**Tasks:**

1. New `client-cpal` Cargo feature: `cpal`-backed `AudioSource`/`AudioSink` for default mic + speaker (cross-platform: WASAPI / CoreAudio / ALSA / JACK).
2. New `client-video` Cargo feature (gated on `nokhwa` or `gstreamer-rs`): `VideoSource` trait + `VideoFrame` enum (`Vp8 { payload, marker }` / `H264 { nalus }` / raw `YuvI420` with optional `openh264` software encoder under a `video-x264`/`video-openh264` sub-feature).
3. Signaling pool: `SignalingPool` keyed by base WS URL — one connection per URL, many concurrent calls multiplexed by `connection_id`.
4. Session resume helper: if signaling drops mid-handshake, `Signaler::reconnect_with_session(id)` rejoins and replays the offer/answer state.
5. Perfect-negotiation helper: `WebRtcClient::with_polite(bool)` toggle that handles offer collision by SDP rollback or ignore per W3C [Perfect Negotiation](https://developer.mozilla.org/en-US/docs/Web/API/WebRTC_API/Perfect_negotiation).
6. Example: `examples/native_call.rs` — gated on `client-cpal`, makes a real call between two processes using the default mic.

**Verification:** New `tests/client_real_audio.rs` (`client-cpal`-gated, `#[ignore]`'d in CI) drives a 5 s call with real cpal capture and asserts `outbound_packets > 0` on both legs.

### Phase G4 — Outbound stats, candidate-pair stats, XR-style metrics (2 d) 🟡

**Why:** Production observability needs *both* sides of the pipe. Today only `InboundStats` is collected.

**Tasks:**

1. Extend `WebRtcStatsSnapshot` with `outbound: OutboundStats { packets_sent, bytes_sent, retransmitted_packets, nack_count }` (from webrtc-rs `get_stats` `outbound-rtp` reports).
2. Add `selected_candidate_pair: Option<CandidatePairStats { current_round_trip_time_ms, available_outgoing_bitrate, total_round_trip_time_ms }>`.
3. Optional `xr_summary: Option<RtcpXrSummary>` from RFC 3611 blocks if webrtc-rs surfaces them; otherwise document the gap.
4. Update Prometheus exporter with new series: `rvoip_webrtc_outbound_packets_total`, `rvoip_webrtc_selected_pair_rtt_ms` (histogram).

**Verification:** Update `tests/h7_observability.rs` to assert the new fields appear after a loopback call.

### Phase G5 — Lossy-link integration test + NACK verification (2 d) 🟡

**Why:** RTCP feedback is *registered* but no test proves it does anything end-to-end. A network shim that drops 5% of UDP datagrams between two in-process peers would close the loop.

**Tasks:**

1. New `tests/support/lossy_socket.rs` helper: wrap a UDP socket with a configurable per-direction drop rate using a `tokio::net::UdpSocket` proxy.
2. New `tests/lossy_link.rs`: two peers via the lossy proxy at 5% drop; send Opus + VP8 for 3 s; assert `WebRtcStatsSnapshot.packets_lost > 0` AND `nack_count > 0` AND audible audio recovery (MOS > 3.5).
3. Mirror for H.264 to validate the H.264 NACK + PLI path independently.

**Verification:** The new test passes deterministically (seeded RNG) and would fail if the H3 feedback registration regressed.

### Phase G6 — Header extensions audit + Safari fixtures (1–2 d) 🟡

**Why:** Several browser-interop gotchas only show up under specific browsers' SDP. The `tests/browser_sdp_interop.rs` fixture covers Chromium 120; Safari and Firefox quirks are unobserved.

**Tasks:**

1. Capture and add recorded SDP fixtures for Safari 17 (H.264-only path) and Firefox 125 (rid/simulcast ordering quirk) under `tests/fixtures/sdp/`.
2. Extend `tests/browser_sdp_interop.rs` to drive each fixture through `apply_remote_offer` → assert answer carries:
   - `extmap-allow-mixed`
   - `urn:ietf:params:rtp-hdrext:sdes:mid` (RFC 9335)
   - `urn:ietf:params:rtp-hdrext:ssrc-audio-level` (RFC 6464) — register in `MediaEngine` if missing
   - `urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id` (RFC 8852) when an `a=rid` is in the offer
   - For Safari fixture: H.264 PT round-trips with `profile-level-id=42e01f` and `packetization-mode=1`
3. Document the assertion suite in `tests/browser_sdp_interop.rs` doc comment as the canonical "what a browser expects" checklist.

**Verification:** Three fixtures pass; CI fails if any of the asserted extensions regress.

### Phase G7 — Multi-codec audio transceiver (unblocks DTMF wire test) (1 d) 🟡

**Why:** The `tests/dtmf_wire.rs` test is `#[ignore]`'d today because `add_local_audio_track` advertises only Opus, so PT 101 DTMF packets are dropped by SRTP filtering.

**Tasks:**

1. Refactor [`RvoipPeerConnection::add_local_audio_track`](../src/peer/session.rs) to register both Opus (PT 111) and `telephone-event` (PT 101) on the same transceiver, splitting the outbound sender per PT.
2. Update DTMF generator in [`media/dtmf.rs`](../src/media/dtmf.rs) to use the negotiated PT 101 RTP track rather than piggybacking on the audio sender.
3. Remove the `#[ignore]` attribute from `tests/dtmf_wire.rs` and assert digit / duration / volume bytes on captured packets.

**Verification:** `cargo test -p rvoip-webrtc --test dtmf_wire` passes without `--ignored`.

### Phase G8 — Browser interop CI integration (3 d) 🟡

**Why:** `tests/browser_interop.rs` already exists but is `#[ignore]`'d because Chromium isn't on PATH in CI. Without nightly runs, the SDP / ICE / DC matrix can silently regress.

**Tasks:**

1. Add `.github/workflows/nightly-interop.yml` (or equivalent) that installs Chromium in the runner and runs `cargo test --features interop-browser --test browser_interop -- --include-ignored`.
2. Extend the harness to cover: WHIP publish, WHEP subscribe, WS bidirectional call, DC text + binary round-trip, `RTCDTMFSender` DTMF over the bridge, ICE restart mid-call, hold/resume mid-call.
3. Surface failures via a Slack/GitHub-issue webhook.

**Verification:** Green nightly badge in README.

### Phase G9 — TURN relay path E2E + SIP↔WebRTC media (4–6 d) 🟡

**Why:** Both items are documented as "wiring-only" today; they're the two missing real-world bridges.

**Tasks:**

1. `tests/support/coturn_fixture.rs`: spawn coturn via `bollard` (Docker API) or `pure-rs-turn-server` if a Rust TURN test server is acceptable; expose a `TurnServer::ice_config()` helper.
2. New `tests/turn_relay_e2e.rs`: force `IceTransportPolicy::Relay`, place a call through coturn, assert the selected candidate pair is `relay/relay`, assert media flows.
3. SIP bridge: wait for `Orchestrator::bridge_connections` SIP-leg support to land in rvoip-core (tracked as upstream blocker); once available, write `tests/sip_webrtc_media_e2e.rs` placing a SIP INVITE → WebRTC subscriber with G.711↔Opus transcoding through `rvoip-media-core`, assert decoded PCM matches the transmitted PCM within an SNR threshold.

**Verification:** Both new tests run green in CI when Docker is available; gracefully skip otherwise.

### Phase G10 — DTLS fingerprint identity binding (rvoip-core blocker, then 1 d here) 🟡

**Why:** Single line of code on this side after upstream lands.

**Tasks (upstream):**

1. In `rvoip-core`, add `IdentityAssurance::DtlsFingerprint { algorithm, value }` variant.

**Tasks (this crate):**

1. Update [`adapter.rs`](../src/adapter.rs) `verify_request_signature()` to return `IdentityAssurance::DtlsFingerprint { ... }` from the negotiated peer fingerprint.
2. New `WebRtcConfig::pinned_fingerprints: Vec<DtlsFingerprint>` — when non-empty, reject any peer whose negotiated fingerprint isn't in the list during `apply_remote_offer`.
3. Test: `tests/identity_pin.rs` — known-good fingerprint accepted, mismatched fingerprint rejected.

### Phase G11 — Perfect-negotiation rollback (2 d) 🟡

**Why:** Required for non-trivial reconfiguration patterns (track add/remove mid-call from both sides). Browsers expose `setLocalDescription(rollback)`; webrtc-rs 0.20-alpha needs to be checked for support, with a wrapper-layer fallback if missing.

**Tasks:**

1. `RvoipPeerConnection::rollback_local()` — calls webrtc-rs rollback or simulates by re-applying the previous local description.
2. `WebRtcClient::with_polite(bool)` helper that handles `offer-collision` events by either ignoring (impolite) or rolling back + applying remote (polite) per the perfect-negotiation pattern.
3. `tests/perfect_negotiation.rs` — two simultaneous offers, assert one peer rolls back and both converge.

### Phase G12 — Operational hardening tail (1–2 d) 🟢

Minor follow-ups across the surface; pick up when convenient.

1. SDP log-redaction helper (`sdp::redact_for_log(sdp) -> String` strips IPs/ufrag/pwd) — wire into the adapter's `#[instrument]` spans at non-debug log levels.
2. WS subprotocol negotiation (`Sec-WebSocket-Protocol: rvoip.webrtc.v1`) — supports schema versioning.
3. CORS allow-list per-route (so `/healthz` and `/whip` can have different policies).
4. Configurable Opus `usedtx` and `maxaveragebitrate` in `WebRtcConfig`.
5. Auto-populate `Link: rel="ice-server"` from `WebRtcConfig::ice_servers` (would be moved into Phase G2 if shipped together).
6. Document multi-subscriber WHEP semantics (today: one route per session id; explicitly out-of-scope for fan-out).

### Phase G13 — Optional / future codec coverage (deferred) ⚪

1. RED for Opus (RFC 2198) when lossy-link tests show audible benefit.
2. AV1 once webrtc-rs 0.20+ stabilizes the depacketizer.
3. G.722 if a SIP-bridge user requests it.
4. H.264 high profile / extra `profile-level-id` variants on demand.

---

## 4. Out of scope (do not pursue in this crate)

These belong elsewhere in the rvoip stack or in a separate media-server project. Listed here so future contributors can deflect feature requests with a citation.

- **SFU / MCU fan-out, simulcast layer selection, multi-party mixing.** This crate is a 1:1 gateway/server adapter. Use `mediasoup`, `Galène`, or `LiveKit` for SFU/MCU.
- **Hardware codec acceleration (VAAPI / VideoToolbox / NVENC).** Belongs in a codec layer; expose `Box<dyn Encoder>` hooks if needed, do not ship encoders.
- **End-to-end media encryption (Insertable Streams / MLS).** Application concern, orthogonal to DTLS-SRTP.
- **Audio processing (AEC / AGC / NS / VAD).** Capture layer (RNNoise, libwebrtc-audio-processing). Wire in via the `AudioSource` trait once `client-cpal` lands.
- **Recording / transcoding.** Bridge to `rvoip-media-core` at the orchestrator layer.
- **Standalone TURN server.** Use coturn / eturnal externally; we ship `turn_rest::generate_ephemeral` for credential rotation.
- **vCon emission.** Lives in rvoip-core.
- **SIP B2BUA logic (REFER, re-INVITE, transfer).** Lives in `rvoip-sip`; this crate's `transfer()` deliberately returns `NotImplemented`.
- **W3C WebRTC Identity (`setIdentityProvider`).** No browser ships a real IdP integration; consider only if customer demand emerges.

---

## 5. Phase priority for a reasonable v1.1 release

If a single engineer were to land "production WebRTC client/server library" status:

| Sprint | Phases | Effort | Outcome |
|---|---|---|---|
| 1 | G1 + G2 + G4 | ~1 wk | Configurable DC, WHIP Bearer auth, full stats. **Closes the last `Must` gaps.** |
| 2 | G3 (mic + cam + reconnect + perfect-negotiation helper) | ~1.5 wk | Drop-in client library. |
| 3 | G5 + G6 + G7 | ~1 wk | Lossy-link + Safari/Firefox SDP + DTMF wire test green. |
| 4 | G8 + G9 (TURN E2E only) | ~1 wk | Nightly browser interop + TURN relay E2E in CI. |
| 5 | G10 + G11 + G12 | ~1 wk | Identity pin, perfect-negotiation, operational tail. |

Total: ~5.5 engineer-weeks for v1.1 "deployable client and server, full standards conformance, browser-interop CI."

Phases G9 (SIP media E2E) and G13 (extra codecs) follow naturally once their upstream dependencies (rvoip-core orchestrator bridge, webrtc-rs codec support) are in place.

---

## 6. Verification rubric

For each shipped phase:

1. **Static**: `cargo clippy -p rvoip-webrtc --all-features -- -D warnings`.
2. **Tests**: `cargo test -p rvoip-webrtc --all-features --tests` plus any feature-gated suites.
3. **Soak (unchanged)**: `SOAK_SECS=3600 cargo test -p rvoip-webrtc --features soak-1h --test soak_long --release` continues to pass with zero leaked tasks.
4. **Interop (post-G8)**: nightly browser-interop suite green for 7 consecutive days before tagging a release.
5. **Documentation**: every new public type / config knob shows up in `CHANGELOG.md` under the appropriate section.

---

## 7. Quick reference: status by RFC

For people grepping for "do you support RFC X?":

| RFC | Title | Status |
|---|---|---|
| 3550, 3551 | RTP / A-V profile | ✅ |
| 3711 | SRTP | ✅ |
| 4566 | SDP | ✅ |
| 4585 | RTCP-AVPF | ✅ |
| 4588 | RTX | ✅ |
| 4733 | DTMF telephone-event | ⚠ wire format ships; full SRTP path blocked on G7 |
| 4960, 8260 | SCTP, I-DATA | ✅ |
| 5104 | PLI/FIR | ✅ |
| 5285, 8285 | RTP header extensions | ✅ |
| 5761 | rtcp-mux | ✅ |
| 5763, 5764, 7714 | DTLS-SRTP, AES-GCM | ✅ |
| 5766, 8656, 7065 | TURN | ✅ |
| 5389, 8489 | STUN | ✅ |
| 6184 | H.264 | ✅ (constrained-baseline 42e01f) |
| 6386, 7741 | VP8 | ✅ |
| 6464 | Audio level hdrext | ✅ G6 explicit register |
| 6716, 7587 | Opus | ✅ |
| 7635 | TURN REST (HMAC SHA-1) | ✅ (using HMAC-SHA-256 for modern coturn) |
| 7675 | Consent freshness | ✅ (inherited) |
| 8445 | ICE | ✅ |
| 8829 | JSEP | ✅ rollback exposed (G11) |
| 8831, 8832, 8841 | Data channels | ✅ plumbing + options API (G1) |
| 8838, 8840 | Trickle ICE | ✅ |
| 8839 | mDNS candidates + ICE in SDP | ✅ |
| 8842 | DTLS in SDP O/A | ✅ |
| 8852 | RID SDES | ✅ G6 explicit register |
| 8853 | SDP simulcast | ⚠ detection only |
| 9143 | BUNDLE | ✅ |
| 9147 | DTLS 1.3 | ✅ (inherited) |
| 9335 | MID hdrext | ✅ G6 explicit register + Firefox fixture asserts |
| 9725 | WHIP | ✅ G2 — Bearer auth + Accept-Post + If-Match (412/428) + auto Link rel=ice-server |
| draft-ietf-wish-whep | WHEP | ✅ for single subscriber |

---

## 8. Document index

- ✅ [`docs/IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) — v1 architecture + phases 0–11
- ✅ [`docs/HARDENING_PLAN.md`](HARDENING_PLAN.md) — H1–H7 audit + remediation
- ✅ [`docs/GAP_PLAN.md`](GAP_PLAN.md) — *this file* — G1–G13 from "production-deployable" to "drop-in client/server library"
- ✅ [`CHANGELOG.md`](../CHANGELOG.md)
- ✅ [`README.md`](../README.md)
