# rvoip-webrtc Gap Implementation Plan (v1.1 — closing every G-phase)

**Deliverable location:** [`crates/rvoip-webrtc/docs/GAP_IMPLEMENTATION_PLAN.md`](GAP_IMPLEMENTATION_PLAN.md)

**Companion docs (read first):**
- [`docs/IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) — original v1 phases 0–11 (✅).
- [`docs/HARDENING_PLAN.md`](HARDENING_PLAN.md) — H1–H7 audit + remediation (✅).
- [`docs/GAP_PLAN.md`](GAP_PLAN.md) — gap identification + severity rubric (✅).

**Scope:** This document operationalizes every gap from `GAP_PLAN.md` into shippable engineering work. For each phase (G1–G13) it specifies the files that change, the new public API (with code skeletons), the test surface (with test names and assertions), acceptance criteria, dependency additions, and migration impact. All work stays under `crates/rvoip-webrtc/**` unless explicitly noted as an upstream dependency.

**Target release:** `rvoip-webrtc v1.1.0`. After all G-phases land the crate is "drop-in WebRTC client/server library, full RFC 9725 + RFC 8831 conformance, browser-interop nightly CI."

**Engineering budget:** ~5.5 engineer-weeks (G1–G12). Phases G13 + the upstream-blocked half of G9/G10 follow opportunistically.

---

## 0. Conventions used in this document

- **API skeletons** are Rust-shaped pseudocode showing the intended shape, not literal final code; signatures may shift to fit borrow-checker reality.
- **File:line citations** point at the current `main` (post-H7) tree.
- **Tests are named in the form** `feature::scenario_assertion`, e.g. `dc_options::unordered_partial_reliable_round_trips`.
- **Severity / phase ordering** matches `GAP_PLAN.md` §3.
- **Cargo feature gates** follow the existing pattern (`client`, `signaling-whip`, `tls-rustls`, …); new features are listed in each phase's "Cargo.toml changes" subsection.
- **Verification** for every phase includes: clean clippy with `-D warnings -D clippy::unwrap_used -D clippy::expect_used` (lib code), all existing tests still green, new phase-specific tests green.

---

## 1. Phase ordering and dependency graph

```
G1 (DC options) ────────────────┐
G2 (WHIP/WS auth + headers) ────┤
G4 (outbound stats)             │──► v1.1.0-rc1   (Sprint 1, ~1 wk)
                                │
G3 (mic + cam + reconnect) ─────┘──► v1.1.0-rc2   (Sprint 2, ~1.5 wk)

G5 (lossy-link NACK)            │
G6 (Safari/Firefox SDP fixtures)│──► v1.1.0-rc3   (Sprint 3, ~1 wk)
G7 (multi-codec audio xcvr)     │

G8 (browser interop CI)         │──► v1.1.0-rc4   (Sprint 4, ~1 wk)
G9a (TURN relay E2E)            │

G10 (DTLS fingerprint identity) │  [unblocked by rvoip-core change]
G11 (perfect negotiation)       │──► v1.1.0       (Sprint 5, ~1 wk)
G12 (operational tail)          │

G9b (SIP media E2E)             │──► follow-on    (blocked on rvoip-core)
G13 (extra codecs)              │──► opportunistic
```

Independent phases can be parallelized across contributors. G1–G2–G4 form the "last Must gap" sprint and should land together to enable a meaningful release candidate.

---

## 2. Phase G1 — Data channel options API + backpressure (1–2 d, 🔴)

### 2.1 Goal

Provide a typed, RFC 8832-aligned data-channel configuration surface; add bufferedAmount / lowThreshold semantics. This unblocks gaming, telemetry, and file-transfer use cases that today cannot configure reliability.

### 2.2 Files modified

| File | Change |
|---|---|
| [`src/peer/session.rs`](../src/peer/session.rs) line 301 | Replace `create_data_channel(label)` signature with `create_data_channel(label, opts)`. |
| `src/peer/data_channel.rs` (**new**) | `DataChannelOptions` struct, `RvoipDataChannel` wrapper with `buffered_amount()` + low-threshold event. |
| [`src/peer/mod.rs`](../src/peer/mod.rs) | Re-export the new types. |
| [`src/adapter.rs`](../src/adapter.rs) | Plumb optional `DataChannelOptions` through `send_message` / route creation paths. |
| [`src/lib.rs`](../src/lib.rs) line 39 | Public re-exports: `DataChannelOptions`, `RvoipDataChannel`. |
| `tests/dc_options.rs` (**new**) | Round-trip tests covering all five reliability modes. |
| `tests/dc_backpressure.rs` (**new**) | bufferedAmount drains; low-threshold event fires. |

### 2.3 API skeleton

```rust
// src/peer/data_channel.rs

/// RFC 8832 § 5.1 — DCEP DATA_CHANNEL_OPEN parameters.
///
/// `max_retransmits` and `max_packet_lifetime_ms` are mutually exclusive —
/// setting both returns `WebRtcError::InvalidArgument`.
#[derive(Clone, Debug, Default)]
pub struct DataChannelOptions {
    /// `true` (default) = ordered delivery (reliable or partial-reliable);
    /// `false` = unordered.
    pub ordered: bool,
    /// Bound on retransmissions. `Some(0)` = "unreliable, no retransmits".
    pub max_retransmits: Option<u16>,
    /// Wallclock cap on retransmission lifetime in ms.
    pub max_packet_lifetime_ms: Option<u16>,
    /// Sub-protocol identifier ("chat", "binary", "json", "rvoip.v1"…).
    pub protocol: Option<String>,
    /// Pre-agreed SCTP stream id — when `Some`, DCEP exchange is skipped.
    pub negotiated_id: Option<u16>,
}

impl DataChannelOptions {
    pub fn reliable() -> Self {
        Self { ordered: true, ..Default::default() }
    }

    pub fn unreliable() -> Self {
        Self { ordered: false, max_retransmits: Some(0), ..Default::default() }
    }

    pub fn partial_reliable_retransmits(n: u16) -> Self {
        Self { ordered: true, max_retransmits: Some(n), ..Default::default() }
    }

    pub fn partial_reliable_lifetime(ms: u16) -> Self {
        Self { ordered: true, max_packet_lifetime_ms: Some(ms), ..Default::default() }
    }

    fn validate(&self) -> Result<()> {
        if self.max_retransmits.is_some() && self.max_packet_lifetime_ms.is_some() {
            return Err(WebRtcError::InvalidArgument(
                "max_retransmits and max_packet_lifetime are mutually exclusive".into()
            ));
        }
        Ok(())
    }

    fn to_webrtc_rs(&self) -> RTCDataChannelInit { /* … */ }
}

pub struct RvoipDataChannel {
    inner: Arc<dyn DataChannel>,
    low_threshold: AtomicU64,
    on_low: tokio::sync::broadcast::Sender<()>,
}

impl RvoipDataChannel {
    pub async fn send_text(&self, msg: &str) -> Result<()> { /* … */ }
    pub async fn send_binary(&self, msg: &[u8]) -> Result<()> { /* … */ }
    pub fn buffered_amount(&self) -> u64 { /* webrtc-rs accessor */ }
    pub fn set_buffered_amount_low_threshold(&self, threshold: u64) { /* … */ }
    pub fn subscribe_buffered_amount_low(&self) -> broadcast::Receiver<()> { /* … */ }
    /// Negotiated `a=max-message-size` from SDP, or `None` if not yet known.
    pub fn max_message_size(&self) -> Option<u64> { /* parse from current remote SDP */ }
}
```

### 2.4 Migration

Existing callers of `create_data_channel(label)` must update to `create_data_channel(label, DataChannelOptions::reliable())`. Document in CHANGELOG under `### Breaking changes`. Provide an `impl From<&str> for DataChannelOptions` is **not** offered — explicit is better than implicit here.

### 2.5 Tests

```rust
// tests/dc_options.rs
#[tokio::test] async fn dc_options::unordered_zero_retransmits_round_trips() { /* … */ }
#[tokio::test] async fn dc_options::partial_reliable_retransmits_round_trips() { /* … */ }
#[tokio::test] async fn dc_options::partial_reliable_lifetime_round_trips() { /* … */ }
#[tokio::test] async fn dc_options::negotiated_pre_agreed_id_skips_dcep() { /* … */ }
#[tokio::test] async fn dc_options::mutually_exclusive_returns_invalid_argument() { /* … */ }
#[tokio::test] async fn dc_options::protocol_field_round_trips_to_remote() { /* … */ }

// tests/dc_backpressure.rs
#[tokio::test] async fn dc_backpressure::buffered_amount_drains_under_steady_send() { /* … */ }
#[tokio::test] async fn dc_backpressure::low_threshold_event_fires_when_drained_below() { /* … */ }
#[tokio::test] async fn dc_backpressure::reject_send_above_negotiated_max_message_size() { /* … */ }
```

### 2.6 Acceptance criteria

- `cargo test -p rvoip-webrtc --test dc_options --test dc_backpressure` green.
- Existing `tests/dc_soak.rs` + `tests/dc_loopback.rs` still pass (migrated to new signature).
- New types appear in `lib.rs` re-exports; documented in `CHANGELOG.md ## Unreleased ### Added`.
- No regression in 1 h soak (`SOAK_SECS=3600`).

---

## 3. Phase G2 — WHIP/WS authentication + missing RFC 9725 headers (2–3 d, 🔴)

### 3.1 Goal

Close the last RFC 9725 §4.1 / §4.4 surface gaps: Bearer authentication via a pluggable hook, `Accept-Post` advertisement, `If-Match` enforcement on PATCH, auto-populated `Link: rel="ice-server"`. Symmetric `WsAuthHook` for the WebSocket signaler.

### 3.2 Files modified

| File | Change |
|---|---|
| [`src/signaling/whip.rs`](../src/signaling/whip.rs) lines 158–510 | New `WhipAuthHook` trait + middleware; `If-Match` check on PATCH; `Accept-Post` header; auto `Link: rel="ice-server"` from config. |
| [`src/signaling/websocket.rs`](../src/signaling/websocket.rs) | New `WsAuthHook` trait; check on upgrade. |
| `src/signaling/auth.rs` (**new**) | Shared `AuthContext`, `AuthRejection`, `AnonymousAuth`, `BearerStaticTokenAuth` reference impls. |
| [`src/server.rs`](../src/server.rs) | `WebRtcServerBuilder::with_whip_auth(Arc<dyn WhipAuthHook>)` + `with_ws_auth(Arc<dyn WsAuthHook>)`. |
| [`src/adapter.rs`](../src/adapter.rs) | Store negotiated `AuthContext` on each route for downstream consumers. |
| [`src/errors.rs`](../src/errors.rs) | Add `WebRtcError::Unauthorized`, `WebRtcError::PreconditionFailed`. |
| `tests/whip_auth.rs` (**new**) | Anonymous / valid / invalid Bearer paths; `Accept-Post` advertisement; `If-Match` 412. |
| `tests/ws_auth.rs` (**new**) | WS upgrade with / without valid token. |
| `tests/whip_compliance.rs` (modified) | Extend with auto-`Link` + `Accept-Post` assertions. |

### 3.3 API skeleton

```rust
// src/signaling/auth.rs

#[async_trait]
pub trait WhipAuthHook: Send + Sync {
    /// Called before any work for POST / PATCH / DELETE. Returns the
    /// authenticated [`AuthContext`] (echoed back on the route) or an
    /// [`AuthRejection`].
    async fn authenticate(
        &self,
        method: http::Method,
        path: &str,
        bearer: Option<&str>,
        peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection>;
}

#[async_trait]
pub trait WsAuthHook: Send + Sync {
    /// Called during the WebSocket upgrade. `subprotocols` is the parsed
    /// `Sec-WebSocket-Protocol` header (some clients smuggle a token here
    /// as e.g. `rvoip.webrtc.v1, token.<base64>`). `query_token` is parsed
    /// from `?access_token=…`.
    async fn authenticate(
        &self,
        subprotocols: &[&str],
        query_token: Option<&str>,
        peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection>;
}

#[derive(Clone, Debug)]
pub struct AuthContext {
    pub subject: String,         // opaque user/tenant id
    pub scopes: Vec<String>,     // e.g. ["whip:publish", "whep:subscribe"]
    pub session_hint: Option<String>,
}

#[derive(Debug)]
pub enum AuthRejection {
    Unauthorized { www_authenticate: String },  // → 401, WWW-Authenticate: Bearer …
    Forbidden,                                  // → 403
    Throttled { retry_after_secs: u32 },        // → 429
}

/// Default: no auth required. Production deployments override with their own.
pub struct AnonymousAuth;

/// Reference implementation: single static bearer token (testing/demo only).
pub struct BearerStaticTokenAuth { pub token: String, pub scopes: Vec<String> }
```

### 3.4 WHIP header changes

```rust
// src/signaling/whip.rs — extend build_session_headers (line 258) and OPTIONS handler (line 218).

fn add_accept_post(headers: &mut HeaderMap) {
    headers.insert(
        HeaderName::from_static("accept-post"),
        HeaderValue::from_static("application/sdp"),
    );
}

fn add_ice_server_links(headers: &mut HeaderMap, ice_servers: &[IceServerConfig]) {
    // Per RFC 9725 §4.6 — one Link header per server.
    for srv in ice_servers {
        for url in &srv.urls {
            let value = match (&srv.username, &srv.credential) {
                (Some(u), Some(c)) => format!(
                    "<{}>; rel=\"ice-server\"; username=\"{}\"; credential=\"{}\"; credential-type=\"password\"",
                    url, u, c
                ),
                _ => format!("<{}>; rel=\"ice-server\"", url),
            };
            headers.append(http::header::LINK, HeaderValue::from_str(&value).expect("…"));
        }
    }
}

// PATCH ICE restart — require If-Match.
let if_match = headers.get(http::header::IF_MATCH).and_then(|v| v.to_str().ok());
if if_match != Some(current_etag.as_str()) {
    return (StatusCode::PRECONDITION_FAILED, "ETag mismatch").into_response();
}
```

### 3.5 Tests

```rust
// tests/whip_auth.rs
#[tokio::test] async fn whip_auth::anonymous_with_anonymous_hook_returns_201() { /* … */ }
#[tokio::test] async fn whip_auth::missing_bearer_with_bearer_hook_returns_401_with_www_authenticate() { /* … */ }
#[tokio::test] async fn whip_auth::valid_bearer_returns_201() { /* … */ }
#[tokio::test] async fn whip_auth::wrong_scope_returns_403() { /* … */ }
#[tokio::test] async fn whip_auth::patch_without_if_match_returns_428() { /* RFC 6585 — required */ }
#[tokio::test] async fn whip_auth::patch_with_stale_if_match_returns_412() { /* … */ }
#[tokio::test] async fn whip_auth::options_advertises_accept_post() { /* … */ }
#[tokio::test] async fn whip_auth::link_header_auto_populated_from_ice_servers() { /* … */ }

// tests/ws_auth.rs
#[tokio::test] async fn ws_auth::anonymous_with_anonymous_hook_upgrades() { /* … */ }
#[tokio::test] async fn ws_auth::missing_token_with_bearer_hook_returns_401_before_upgrade() { /* … */ }
#[tokio::test] async fn ws_auth::valid_token_via_subprotocol_upgrades() { /* … */ }
#[tokio::test] async fn ws_auth::valid_token_via_query_param_upgrades() { /* … */ }
```

### 3.6 Acceptance criteria

- `cargo test -p rvoip-webrtc --test whip_auth --test ws_auth --features signaling-whip,signaling-ws` green.
- WHIP responses include `Accept-Post: application/sdp` and `Link: <…>; rel="ice-server"` for each configured ICE server.
- `If-Match` enforced on PATCH ICE restart (412 on mismatch, 428 when missing).
- Existing `tests/whip_compliance.rs` extended; default behavior (no auth hook configured) is `AnonymousAuth` → backward-compatible.
- README "Running as a WebRTC server" section gains a "Production checklist" subsection covering auth hook + TLS.

---

## 4. Phase G3 — Real client surfaces: mic + camera + reconnect (4–6 d, 🔴)

### 4.1 Goal

Make `rvoip-webrtc` usable by a developer who has never touched cpal or AVFoundation. Ship a default microphone and (gated) camera backend, signaling-connection pooling, session resume after blips, and a perfect-negotiation helper.

### 4.2 Cargo.toml changes

```toml
[features]
client-cpal     = ["client", "dep:cpal"]
client-video    = ["client", "dep:nokhwa"]            # default to nokhwa for cross-platform camera
video-openh264  = ["client-video", "dep:openh264"]    # software H.264 encoder, GPL-friendly
video-x264      = ["client-video", "dep:x264"]        # alternative, GPL

[dependencies]
cpal     = { version = "0.16", optional = true }
nokhwa   = { version = "0.10", optional = true, features = ["input-native"] }
openh264 = { version = "0.6",  optional = true }
x264     = { version = "0.5",  optional = true }
```

### 4.3 Files modified / added

| File | Change |
|---|---|
| `src/client/cpal_source.rs` (**new**, `client-cpal`) | `CpalMicSource` (default input device → 48 kHz mono → Opus via `audiopus`); `CpalSpeakerSink` (PCM out). |
| `src/client/video.rs` (**new**, `client-video`) | `VideoSource` / `VideoSink` traits, `VideoFrame` enum, `NokhwaCameraSource`. |
| `src/client/video_encoder.rs` (**new**, `client-video`) | Software encode hook; default impl uses pre-encoded VP8 fixture, gated `openh264`/`x264` variants. |
| `src/client/pool.rs` (**new**) | `SignalingPool` keyed by base WS URL; multiplexes `connection_id`s. |
| `src/client/perfect_negotiation.rs` (**new**) | `PerfectNegotiation::new(polite: bool)`; integrates with `WebRtcClient`. |
| [`src/client/native.rs`](../src/client/native.rs) line 188 | `WebRtcClient::call` learns a `with_polite(bool)` builder field; `SessionHandle::on_signaling_drop` hook. |
| [`src/client/ws_signaler.rs`](../src/client/ws_signaler.rs) line 47 | `WsSignaler::reconnect_with_session(SessionId)` for resume; new event channel for transport state. |
| `examples/native_call.rs` (**new**, `client-cpal`) | Two-process loopback driving a real microphone end-to-end. |
| `examples/video_call.rs` (**new**, `client-video,video-openh264`) | Camera capture + H.264 encode + send. |
| `tests/client_real_audio.rs` (**new**, `client-cpal`, `#[ignore]`) | 5 s call with default mic; asserts outbound RTP packet count + non-zero stats. |
| `tests/client_pool.rs` (**new**) | Two concurrent calls share one WS connection. |
| `tests/client_resume.rs` (**new**) | Drop the WS TCP socket mid-handshake; assert client reconnects + completes call. |
| `tests/perfect_negotiation.rs` (**new**) | Two simultaneous offers; polite peer rolls back; final state converges. |

### 4.4 API skeleton

```rust
// src/client/cpal_source.rs (feature client-cpal)

pub struct CpalMicSource {
    encoder: opus::Encoder,
    rx: crossbeam_channel::Receiver<Vec<i16>>,
    ssrc: u32,
    sequence: u16,
    timestamp: u32,
    _stream: cpal::Stream, // kept alive
}

impl CpalMicSource {
    /// Open the default input device at 48 kHz mono, build an Opus encoder
    /// at 64 kbit/s VBR with FEC enabled, return a packetized source.
    pub fn open_default(stream_id: StreamId) -> Result<Self> { /* … */ }
    pub fn open_named(device_name: &str, stream_id: StreamId) -> Result<Self> { /* … */ }
}

impl AudioSource for CpalMicSource { /* fills next_packet with Opus RTP */ }

// src/client/video.rs (feature client-video)

#[derive(Clone, Debug)]
pub enum VideoFrame {
    Vp8 { payload: Bytes, marker: bool, timestamp: u32 },
    H264 { nalus: Bytes, marker: bool, timestamp: u32 },
    YuvI420 { width: u32, height: u32, planes: [Bytes; 3], timestamp: u64 },
}

#[async_trait]
pub trait VideoSource: Send + Sync {
    async fn next_frame(&mut self) -> Option<VideoFrame>;
}

#[async_trait]
pub trait VideoSink: Send + Sync {
    async fn write_frame(&mut self, frame: VideoFrame);
}

pub struct NokhwaCameraSource { /* … */ }
impl NokhwaCameraSource {
    pub fn open_default(width: u32, height: u32, fps: u32) -> Result<Self> { /* … */ }
}

// src/client/pool.rs

pub struct SignalingPool {
    by_url: DashMap<String, Arc<PoolEntry>>,
}

impl SignalingPool {
    pub fn new() -> Arc<Self> { /* … */ }
    pub async fn signaler(&self, ws_url: &str) -> Result<Arc<dyn Signaler>> { /* … */ }
}

// src/client/perfect_negotiation.rs

pub struct PerfectNegotiation {
    polite: bool,
    making_offer: AtomicBool,
    ignore_offer: AtomicBool,
    is_setting_remote_answer_pending: AtomicBool,
}

impl PerfectNegotiation {
    pub fn new(polite: bool) -> Arc<Self> { /* … */ }
    /// On incoming offer: returns `Action::Apply` or `Action::Ignore` per
    /// the W3C perfect-negotiation algorithm.
    pub async fn on_remote_offer(&self, /* … */) -> Action { /* … */ }
    /// On `negotiationneeded`: serializes local offer creation.
    pub async fn on_negotiation_needed(&self, /* … */) -> Result<()> { /* … */ }
}
```

### 4.5 Tests

```rust
// tests/client_real_audio.rs  (#[ignore], needs audio device)
#[tokio::test] #[ignore] async fn real_audio::default_mic_round_trips_5s() { /* … */ }

// tests/client_pool.rs
#[tokio::test] async fn pool::two_concurrent_calls_share_one_ws() { /* … */ }
#[tokio::test] async fn pool::pool_evicts_idle_connections() { /* … */ }

// tests/client_resume.rs
#[tokio::test] async fn resume::drop_ws_mid_handshake_reconnects_and_completes() { /* … */ }
#[tokio::test] async fn resume::session_id_preserved_across_resume() { /* … */ }

// tests/perfect_negotiation.rs
#[tokio::test] async fn perfect_neg::simultaneous_offers_converge_with_polite_rollback() { /* … */ }
#[tokio::test] async fn perfect_neg::impolite_ignores_colliding_offer() { /* … */ }
```

### 4.6 Acceptance criteria

- New examples build and run on macOS + Linux (`cargo run --example native_call --features client-cpal`).
- `tests/client_pool.rs`, `tests/client_resume.rs`, `tests/perfect_negotiation.rs` all green by default.
- `tests/client_real_audio.rs` runs green when invoked with `--ignored` on a host with an audio device.
- README adds a "Running as a native client" section with the 10-line minimum example.
- CHANGELOG gains an "Added" entry for `client-cpal`, `client-video`, perfect-negotiation, pool, resume.

---

## 5. Phase G4 — Outbound stats + candidate-pair stats (2 d, 🟡)

### 5.1 Goal

Surface the sender side of the media pipe and the selected candidate pair through the same `WebRtcStatsSnapshot` mechanism, and via Prometheus.

### 5.2 Files modified

| File | Change |
|---|---|
| [`src/media/stats.rs`](../src/media/stats.rs) | Add outbound RTP poll + selected-pair poll alongside inbound. |
| [`src/media/pump.rs`](../src/media/pump.rs) | Extend `WebRtcStatsSnapshot` with `outbound` + `selected_pair` fields. |
| [`src/observability.rs`](../src/observability.rs) | Two new Prometheus series + one histogram (`rvoip_webrtc_selected_pair_rtt_ms`). |
| `tests/h7_observability.rs` (modified) | Assert new fields populated after a loopback call. |

### 5.3 API skeleton

```rust
// src/media/pump.rs — extended snapshot

#[derive(Clone, Debug, Default)]
pub struct WebRtcStatsSnapshot {
    pub inbound: InboundStats,
    pub outbound: OutboundStats,
    pub selected_pair: Option<CandidatePairStats>,
    pub mos: f32,
}

#[derive(Clone, Debug, Default)]
pub struct OutboundStats {
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub retransmitted_packets: u64,
    pub retransmitted_bytes: u64,
    pub nack_count: u64,
    pub fir_count: u64,
    pub pli_count: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CandidatePairStats {
    pub local_candidate_type: String,   // "host" | "srflx" | "prflx" | "relay"
    pub remote_candidate_type: String,
    pub current_round_trip_time_ms: Option<f64>,
    pub total_round_trip_time_ms: Option<f64>,
    pub available_outgoing_bitrate_bps: Option<u64>,
    pub responses_received: u64,
}
```

### 5.4 Tests

```rust
// tests/h7_observability.rs — extend existing
#[tokio::test] async fn obs::outbound_stats_populated_after_loopback_send() { /* … */ }
#[tokio::test] async fn obs::selected_pair_rtt_populated_after_connect() { /* … */ }
#[tokio::test] async fn obs::prometheus_exports_outbound_and_rtt_series() { /* … */ }
```

### 5.5 Acceptance criteria

- New fields populated and non-zero in loopback test runs.
- Prometheus exposition adds `rvoip_webrtc_outbound_packets_total`, `rvoip_webrtc_outbound_bytes_total`, `rvoip_webrtc_selected_pair_rtt_ms` (histogram).
- No regression in existing `webrtc_stats_snapshot()` callers (it's an additive change).

---

## 6. Phase G5 — Lossy-link integration test + NACK verification (2 d, 🟡)

### 6.1 Goal

Prove the RTCP feedback registration from H3 *actually does something* under packet loss. Today no test exercises NACK round-trips end-to-end.

### 6.2 Files added

| File | Change |
|---|---|
| `tests/support/mod.rs` (**new** or extend) | Test-only shared helpers module. |
| `tests/support/lossy_socket.rs` (**new**) | UDP proxy with seeded RNG drop rate; `LossyProxy::with_loss_rate(0.05)`. |
| `tests/lossy_link.rs` (**new**) | Drives Opus + VP8 + H.264 through the proxy; asserts NACK round-trip + non-zero `packets_lost`. |

### 6.3 Implementation sketch

```rust
// tests/support/lossy_socket.rs

pub struct LossyProxy {
    server_addr: SocketAddr,
    client_addr: SocketAddr,
    loss_rate: f64,
    rng: StdRng,
}

impl LossyProxy {
    pub async fn spawn(server_addr: SocketAddr, loss_rate: f64, seed: u64) -> Result<SocketAddr> {
        // bind UDP, accept first datagram = client_addr, then proxy in both directions
        // with seeded drop decisions.
    }
}

// tests/lossy_link.rs

#[tokio::test]
async fn lossy_link::opus_with_5pct_loss_triggers_nack_and_recovery() {
    let proxy_addr = LossyProxy::spawn(server_addr, 0.05, 42).await?;
    // ICE config points both peers at proxy_addr;
    // send 3 s of Opus RTP; assert:
    //   - inbound.packets_lost > 0
    //   - outbound.nack_count > 0 OR remote.fraction_lost reflects recovery
    //   - MOS estimate degrades but stays above 3.5
}

#[tokio::test] async fn lossy_link::vp8_with_5pct_loss_triggers_pli_and_keyframe() { /* … */ }
#[tokio::test] async fn lossy_link::h264_with_5pct_loss_triggers_pli_and_keyframe() { /* … */ }
```

### 6.4 Acceptance criteria

- Three new tests pass deterministically (seeded RNG).
- Reverting any one of the H3 RTCP-feedback registrations causes the corresponding test to fail.
- README "Verification" section gains a bullet for lossy-link testing.

---

## 7. Phase G6 — Header extensions audit + Safari/Firefox SDP fixtures (1–2 d, 🟡)

### 7.1 Goal

Catch browser-specific SDP quirks before customers do.

### 7.2 Files modified / added

| File | Change |
|---|---|
| [`src/peer/builder.rs`](../src/peer/builder.rs) `build_media_engine` line 82 | Explicitly `register_header_extension` for: `urn:ietf:params:rtp-hdrext:sdes:mid` (audio + video), `urn:ietf:params:rtp-hdrext:ssrc-audio-level` (audio), `urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id` (video, for RID), `urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id`, `http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time` (video). |
| `tests/fixtures/sdp/safari_17_audio.sdp` (**new**) | Recorded offer from Safari 17 audio call. |
| `tests/fixtures/sdp/safari_17_audiovideo.sdp` (**new**) | Recorded offer from Safari 17 video call (H.264 only). |
| `tests/fixtures/sdp/firefox_125_simulcast.sdp` (**new**) | Recorded offer from Firefox 125 with `a=simulcast`. |
| `tests/fixtures/sdp/chrome_124_audiovideo.sdp` (**new**) | Newer Chromium fixture (TWCC + mid + abs-send-time). |
| `tests/browser_sdp_interop.rs` (modified) | Drive each fixture; assert header extensions echoed; assert codec PTs round-trip; assert `extmap-allow-mixed` echoed. |

### 7.3 Tests

```rust
#[tokio::test] async fn sdp_fixtures::chrome_124_round_trips_with_mid_and_audio_level() { /* … */ }
#[tokio::test] async fn sdp_fixtures::safari_17_h264_only_negotiates_42e01f() { /* … */ }
#[tokio::test] async fn sdp_fixtures::firefox_125_simulcast_preserves_rid_ordering() { /* … */ }
#[tokio::test] async fn sdp_fixtures::extmap_allow_mixed_echoed_when_offered() { /* … */ }
#[tokio::test] async fn sdp_fixtures::audio_level_extmap_present_in_answer() { /* … */ }
```

### 7.4 Acceptance criteria

- All fixture tests pass.
- Removing any `register_header_extension` line breaks the corresponding test.
- Test docstring documents the "what every browser expects" checklist (single source of truth for interop).

---

## 8. Phase G7 — Multi-codec audio transceiver (unblocks DTMF wire test) (1 d, 🟡)

### 8.1 Goal

Let `tests/dtmf_wire.rs` drop its `#[ignore]` attribute by giving the audio transceiver both Opus (PT 111) and telephone-event (PT 101) on a single negotiated m-line.

### 8.2 Files modified

| File | Change |
|---|---|
| [`src/peer/session.rs`](../src/peer/session.rs) `add_local_audio_track` line 208 | Register both PTs on the same transceiver; expose a per-PT sender selector. |
| [`src/media/dtmf.rs`](../src/media/dtmf.rs) | Switch to the PT 101 sender from the new selector (rather than piggybacking on the Opus track). |
| [`tests/dtmf_wire.rs`](../tests/dtmf_wire.rs) | Remove `#[ignore]`; assert digit / duration / volume bytes via packet capture in loopback. |

### 8.3 Implementation note

webrtc-rs 0.20-alpha lets you add multiple codecs per transceiver via `RTCRtpCodecParameters` on the same `TrackLocalStaticRTP`-equivalent. The trick is to either (a) use two `TrackLocalStaticRTP` instances on the same sender or (b) reuse one sender and rewrite the PT in outgoing RTP headers when sending DTMF. Option (a) is cleaner — track the negotiated PT mapping per peer and pick the right track at send time.

### 8.4 Tests

```rust
#[tokio::test] async fn dtmf_wire::send_dtmf_emits_rfc4733_telephone_events() {
    // capture inbound RTP packets at the answerer; filter PT==101;
    // assert event byte == 5 for digit '5'; assert duration increments;
    // assert end-of-event bit set in the final 3 packets.
}

#[tokio::test] async fn dtmf_wire::event_volume_clamped_to_valid_range() { /* … */ }
#[tokio::test] async fn dtmf_wire::dtmf_does_not_disrupt_opus_flow() { /* … */ }
```

### 8.5 Acceptance criteria

- `cargo test -p rvoip-webrtc --test dtmf_wire` green without `--ignored`.
- The original audio loopback (`tests/loopback.rs`) still green; no Opus regression.

---

## 9. Phase G8 — Browser interop in CI (3 d, 🟡)

### 9.1 Goal

Turn the `tests/browser_interop.rs` (`#[ignore]`'d today) into a nightly green badge.

### 9.2 Files modified / added

| File | Change |
|---|---|
| `.github/workflows/nightly-interop.yml` (**new**) | GitHub Actions job: install Chromium, run `cargo test --features interop-browser --test browser_interop -- --include-ignored`. |
| `tests/browser_interop.rs` (extend) | Add scenarios: WHIP publish, WHEP subscribe, WS bidirectional, DC text + binary, `RTCDTMFSender` round-trip, mid-call ICE restart, hold/resume. |
| `static/whip-publish.html`, `static/ws-signaling.html` (extend) | Match the new browser-interop test scenarios. |
| `README.md` | Add the nightly badge URL. |

### 9.3 Workflow skeleton

```yaml
# .github/workflows/nightly-interop.yml
name: nightly-interop
on:
  schedule: [{ cron: "17 3 * * *" }]
  workflow_dispatch:
jobs:
  interop:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: sudo apt-get update && sudo apt-get install -y chromium-browser
      - run: cargo test -p rvoip-webrtc --features interop-browser --test browser_interop -- --include-ignored
      - name: Notify on failure
        if: failure()
        uses: slackapi/slack-github-action@v1
        with: { payload-file-path: ./.github/slack-failure.json }
        env: { SLACK_WEBHOOK_URL: ${{ secrets.NIGHTLY_INTEROP_WEBHOOK }} }
```

### 9.4 Acceptance criteria

- Three consecutive green nightly runs before tagging v1.1.0.
- README has a working nightly badge.
- A test failure surfaces via Slack/issue webhook within 30 minutes.

---

## 10. Phase G9 — TURN relay path E2E (G9a, 2 d) + SIP↔WebRTC media (G9b, blocked, 2 d after unblock) (🟡)

### 10.1 G9a — TURN relay E2E

#### Files added

| File | Change |
|---|---|
| `tests/support/coturn_fixture.rs` (**new**) | Spawn coturn via `bollard` (Docker API). Helper `CoturnFixture::ice_config()` returns ready-to-use `IceServerConfig`. Skip the test gracefully when Docker isn't available. |
| `tests/turn_relay_e2e.rs` (**new**) | Force `IceTransportPolicy::Relay`, place a call through coturn, assert selected candidate pair is `relay/relay`, assert media flows. |
| `Cargo.toml` `[dev-dependencies]` | `bollard = "0.17"`. |

#### Tests

```rust
#[tokio::test] async fn turn_relay::relay_only_pair_selected_and_media_flows() {
    let Some(coturn) = CoturnFixture::start().await else {
        eprintln!("Docker unavailable; skipping"); return;
    };
    // both peers: ice_transport_policy = Relay, ice_servers = [coturn.ice_config()]
    // assert selected_pair.local_candidate_type == "relay"
    // assert selected_pair.remote_candidate_type == "relay"
    // assert outbound.packets_sent > 0 after 1 s
}
```

### 10.2 G9b — SIP↔WebRTC media E2E (upstream-blocked)

**Blocker:** `rvoip_core::Orchestrator::bridge_connections` SIP path is partially stubbed.

**When unblocked:**

| File | Change |
|---|---|
| `tests/sip_webrtc_media_e2e.rs` (**new**) | Place SIP INVITE → orchestrator → WHEP subscriber; assert PCM SNR ≥ 30 dB after G.711↔Opus transcode via `rvoip-media-core`. |
| `examples/sip_webrtc_bridge.rs` (**new**) | Runnable demo. |

#### Tracking

This subphase is gated on rvoip-core delivering the orchestrator SIP bridge. Open a tracking issue (`rvoip-core#xxx — orchestrator SIP media bridge for WebRTC interop`); link to this section.

### 10.3 Acceptance criteria

- G9a: green CI when Docker is available; gracefully skipped otherwise (with `eprintln!("skipped: …")`).
- G9b: blocked; tracker open and linked from CHANGELOG `## Unreleased ### Deferred`.

---

## 11. Phase G10 — DTLS fingerprint identity binding (1 d here + upstream rvoip-core change) (🟡)

### 11.1 Upstream change (rvoip-core, blocking)

Add to `rvoip_core::identity::IdentityAssurance`:

```rust
pub enum IdentityAssurance {
    Anonymous,
    Bearer { subject: String },
    DtlsFingerprint { algorithm: String, value: String },  // <-- new
    // …
}
```

Filed as a tracking ticket on the rvoip-core repo. Do not block this crate's release — when the variant lands, do the wrapper change below.

### 11.2 Wrapper change (this crate)

| File | Change |
|---|---|
| [`src/adapter.rs`](../src/adapter.rs) `verify_request_signature` line 907 | Return `IdentityAssurance::DtlsFingerprint` derived from `remote_dtls_fingerprint(conn)`. |
| [`src/config.rs`](../src/config.rs) | New `WebRtcConfig::pinned_fingerprints: Vec<DtlsFingerprint>`. |
| [`src/adapter.rs`](../src/adapter.rs) `apply_remote_offer` / `apply_remote_answer` | When `pinned_fingerprints` is non-empty, reject with `WebRtcError::FingerprintNotPinned` if the negotiated peer fingerprint isn't in the list. |
| `tests/identity_pin.rs` (**new**) | Pinned fingerprint accepted; mismatched fingerprint rejected. |

### 11.3 Tests

```rust
#[tokio::test] async fn identity_pin::pinned_fingerprint_allows_call() { /* … */ }
#[tokio::test] async fn identity_pin::mismatched_fingerprint_rejects_call() { /* … */ }
#[tokio::test] async fn identity_pin::empty_pin_list_allows_any() { /* … */ }
#[tokio::test] async fn identity_pin::verify_request_signature_returns_dtls_fingerprint() {
    // requires upstream rvoip-core variant; #[cfg(feature = "rvoip-core-fingerprint")]
}
```

### 11.4 Acceptance criteria

- Pinning enforces; unpinned default behavior unchanged.
- When upstream lands, `verify_request_signature` returns the real variant (verified by the gated test).

---

## 12. Phase G11 — Perfect-negotiation rollback (2 d, 🟡)

### 12.1 Goal

Provide the SDP rollback primitive (JSEP §4.1.10.2) needed for two-sided mid-call reconfiguration, and integrate it into the perfect-negotiation helper from G3.

### 12.2 Files modified

| File | Change |
|---|---|
| [`src/peer/session.rs`](../src/peer/session.rs) | Add `RvoipPeerConnection::rollback_local()` — calls webrtc-rs rollback if available; otherwise re-apply previous local description from a 1-deep history. |
| `src/client/perfect_negotiation.rs` (G3) | Use the new rollback for polite peer behavior. |
| `tests/perfect_negotiation.rs` (G3, extended) | Now actually exercises rollback. |
| `tests/rollback.rs` (**new**) | Standalone rollback test: set local offer → rollback → set new offer → succeeds. |

### 12.3 Implementation note on webrtc-rs

Check `rtc::peer_connection::RTCPeerConnection::set_local_description(rollback_sdp)` in 0.20-alpha. If not directly supported, simulate:

```rust
pub async fn rollback_local(self: &Arc<Self>) -> Result<()> {
    let prev = self.previous_local.lock().clone()
        .ok_or(WebRtcError::InvalidState("no previous local description to roll back to"))?;
    self.set_local_description(prev).await
}
```

Capture the previous description at every `set_local_description` call.

### 12.4 Tests

```rust
#[tokio::test] async fn rollback::set_local_then_rollback_restores_previous_state() { /* … */ }
#[tokio::test] async fn rollback::after_rollback_new_offer_succeeds() { /* … */ }
#[tokio::test] async fn rollback::rollback_with_no_previous_returns_invalid_state() { /* … */ }
```

### 12.5 Acceptance criteria

- All G3 and G11 perfect-negotiation tests green.
- Documentation in module docstring of `src/client/perfect_negotiation.rs` cites the W3C algorithm.

---

## 13. Phase G12 — Operational tail (1–2 d, 🟢)

Small wins to bundle into the v1.1.0 release.

| Item | File | Change |
|---|---|---|
| SDP log redaction | `src/sdp/inspect.rs` | New `redact_for_log(sdp: &str) -> String` — strips IPs from `a=candidate`, `o=`, replaces `ice-ufrag`/`ice-pwd` with `***`. Wire into `#[instrument]` spans (use `tracing` field-formatting hook). |
| WS subprotocol negotiation | `src/signaling/websocket.rs` | Negotiate `Sec-WebSocket-Protocol: rvoip.webrtc.v1`; advertise on upgrade. |
| Per-route CORS | `src/signaling/whip.rs` | Different allow-list for `/whip` vs `/healthz` vs `/metrics`. |
| Opus `usedtx` + `maxaveragebitrate` knobs | `src/config.rs` + `src/peer/builder.rs` | Surface in `WebRtcConfig::opus_settings: OpusSettings { dtx, max_bitrate_bps }`. |
| Auto `Link: ice-server` | done in G2; ensure docs cover it | — |
| Multi-subscriber WHEP doc | `README.md` | Add a "Limitations" subsection stating one-route-per-session-id semantics; recommend SFU for fan-out. |
| `WebRtcMetrics::reset()` | `src/adapter.rs` | For operators rotating Prometheus scrape windows; opt-in. |
| `WebRtcMetrics::histogram_buckets` config | `src/observability.rs` | Configurable histogram buckets for RTT. |

Tests:

```rust
#[test] fn redact::strips_ips_from_candidate_lines() { /* … */ }
#[test] fn redact::strips_ice_credentials() { /* … */ }
#[tokio::test] async fn ws_subprotocol::handshake_negotiates_rvoip_webrtc_v1() { /* … */ }
#[tokio::test] async fn opus_settings::dtx_advertised_in_fmtp() { /* … */ }
```

### Acceptance criteria

- New tests green.
- README "Limitations" section added.
- No regression in existing observability or signaling tests.

---

## 14. Phase G13 — Optional / future codec coverage (opportunistic) (⚪)

These ship when an actual user requests them — not on the critical path.

| Codec | Effort | Trigger |
|---|---|---|
| RED for Opus (RFC 2198) | 2 d | Audible benefit measured in G5 lossy-link test |
| AV1 (RTP payload + dep descriptor) | 5 d | webrtc-rs 0.20 depacketizer stable |
| G.722 | 0.5 d | Single SIP-bridge customer request |
| H.264 high profile + extra `profile-level-id` variants | 2 d | Specific SBC interop request |
| H.265 (Chrome 136+ only) | 5 d | Customer demand + license clarity |

Each gets its own short ADR if/when picked up.

---

## 15. Cross-cutting workstreams

These aren't phases; they run alongside the G-phases.

### 15.1 Documentation

| Doc | Owner phase | Update |
|---|---|---|
| `README.md` | G2, G3, G8, G12 | "Production checklist" + "Native client" + nightly badge + "Limitations" sections |
| `CHANGELOG.md` | Every phase | `## Unreleased` sections for `### Added`, `### Changed`, `### Breaking`, `### Fixed`, `### Deferred` |
| `docs/IMPLEMENTATION_PLAN.md` | At v1.1.0 tag | Stamp v1.1 as ✅ in §8.3 phase table; link to this doc |
| `docs/HARDENING_PLAN.md` | At v1.1.0 tag | Add a "Post-H7" subsection pointing at this doc |
| `docs/GAP_PLAN.md` | Every phase | Flip 🔴/🟡/⚪ markers to 🟢 as phases land; keep the rubric current |

### 15.2 CI matrix expansion

- Add a workflow step that runs the test suite with each non-default feature combination (`signaling-whip` alone, `signaling-ws` alone, `tls-rustls`, `bridge-quic`, `client-cpal`, `client-video`).
- Add `cargo doc --no-deps --all-features` to CI; fail on broken intra-doc links.

### 15.3 Workspace dependency additions (collected)

| Dep | Phase | Use |
|---|---|---|
| `cpal = "0.16"` | G3 | Microphone backend |
| `audiopus = "0.3"` | G3 | Opus encode for cpal source |
| `nokhwa = "0.10"` | G3 | Camera backend |
| `openh264 = "0.6"` (optional) | G3 | Software H.264 encode |
| `x264 = "0.5"` (optional) | G3 | Alternative H.264 encode |
| `crossbeam-channel = "0.5"` | G3 | Bridge cpal callback → async (cpal callbacks are sync) |
| `bollard = "0.17"` (dev-dep) | G9a | coturn Docker fixture |
| `rand_chacha = "0.3"` (dev-dep) | G5 | Seeded RNG for lossy proxy |

All of these are either already in the workspace or new — coordinate with the workspace `Cargo.toml` at the top of the rvoip repo.

---

## 16. Release timeline

| Tag | Phases included | ETA from start |
|---|---|---|
| `v1.1.0-rc1` | G1 + G2 + G4 | Week 1 |
| `v1.1.0-rc2` | + G3 | Week 2.5 |
| `v1.1.0-rc3` | + G5 + G6 + G7 | Week 3.5 |
| `v1.1.0-rc4` | + G8 + G9a | Week 4.5 |
| `v1.1.0` | + G10 + G11 + G12 | Week 5.5 |
| `v1.1.x` (patches) | G9b (when unblocked), G13 (on request) | Opportunistic |

Each RC needs:
1. Green default `cargo test` matrix.
2. Green `SOAK_SECS=3600` soak.
3. CHANGELOG updated.
4. README updated for any new public API.
5. Three days of green nightly interop CI before promoting RC → next RC or to final.

---

## 17. Risk register

| Risk | Likelihood | Mitigation |
|---|---|---|
| webrtc-rs 0.20-alpha changes break the wrapper | Medium | Pin `=0.20.0-alpha.1`; gate alpha upgrades behind a manual check; isolate webrtc-rs imports to `peer/`, `media/`, `signaling/`. |
| cpal device-enumeration flakiness on CI | High on CI | `tests/client_real_audio.rs` is `#[ignore]`'d; run locally. CI exercises trait-level only with the existing fixture source. |
| coturn Docker fixture flakiness | Medium | Graceful skip when Docker unavailable; do not block CI. |
| H.264 software encode license (x264 = GPL) | Low (legal) | Default to `openh264` (BSD-licensed, free Cisco binary); document `video-x264` as opt-in only. |
| Headless Chromium binary unavailable in CI runner | Medium | Pin `chromiumoxide` to a known-good Chromium version; document the apt-get step in the workflow. |
| Upstream rvoip-core orchestrator SIP bridge slips | High | G9b is explicitly deferred; the rest of v1.1 ships without it. |
| Browser SDP fixtures rot as browsers update | Low | Rebaseline fixtures every 6 months as part of release hygiene. |

---

## 18. Done definition (v1.1.0)

This release ships when **all** of the following are true:

1. Every G-phase except G9b and G13 is merged.
2. `cargo test -p rvoip-webrtc --all-features --tests` is green.
3. `cargo clippy -p rvoip-webrtc --all-features -- -D warnings -D clippy::unwrap_used -D clippy::expect_used` is clean.
4. `SOAK_SECS=3600 cargo test -p rvoip-webrtc --features soak-1h --test soak_long --release` passes with zero task leaks.
5. Three consecutive green nightly browser-interop runs.
6. `CHANGELOG.md` `## v1.1.0` section is complete (Added / Changed / Breaking / Fixed / Deferred subsections).
7. `README.md` reflects every new feature flag + the production checklist.
8. `docs/GAP_PLAN.md` rubric has no remaining 🔴 markers (🟡 acceptable for G9b/G13).

---

## 19. Document index (updated)

- ✅ [`docs/IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) — v1 architecture + phases 0–11
- ✅ [`docs/HARDENING_PLAN.md`](HARDENING_PLAN.md) — H1–H7 audit + remediation
- ✅ [`docs/GAP_PLAN.md`](GAP_PLAN.md) — gap identification + severity rubric (G1–G13)
- ✅ [`docs/GAP_IMPLEMENTATION_PLAN.md`](GAP_IMPLEMENTATION_PLAN.md) — **this file** — operational plan for G1–G13 (file-level changes, API skeletons, tests, acceptance criteria)
- ✅ [`CHANGELOG.md`](../CHANGELOG.md)
- ✅ [`README.md`](../README.md)
