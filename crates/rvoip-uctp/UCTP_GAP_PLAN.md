# UCTP — Remaining Gap Plan

**Date written:** 2026-05-25
**Supersedes (partially):** `V0X_REMAINING.md` §1.2 (WS WebRTC media plane landed; see [Closed since V0X_REMAINING was written](#closed-since-v0x_remainingmd-was-written) below).
**Companion docs:** [`UCTP_IMPLEMENTATION_PLAN.md`](UCTP_IMPLEMENTATION_PLAN.md) (authoritative design + as-built record), [`V0X_REMAINING.md`](V0X_REMAINING.md) (the previous remaining-work pass).

---

## 1. Context

The v0 spike and v0.x production-hardening track are both complete. As of 2026-05-25 the full four-substrate end state holds end-to-end:

- **rvoip-quic** + **rvoip-webtransport** + **rvoip-websocket** + **rvoip-sip** all behind a single `Orchestrator`.
- **`Orchestrator::bridge_connections`** routes audio across any substrate pair via `frame_pump` (with optional per-direction `Transcoder` for codec mismatch).
- **WS media plane**: `WebRtcMediaBridge` under `#[cfg(feature = "media-webrtc")]` gives WS bi-directional audio over a co-located WebRTC PeerConnection. End-to-end bridge proof at [`crates/rvoip-websocket/tests/ws_bridge_flow.rs`](../rvoip-websocket/tests/ws_bridge_flow.rs).

Six categories of work remain, ordered roughly from "smallest dependency / clearest path" to "biggest scope-creep":

| § | Gap | Effort | Dependencies |
|---|---|---|---|
| [2.1](#21-update-v0x_remainingmd-to-strike-the-ws-webrtc-row) | Update `V0X_REMAINING.md` (stale entries) | ~10 min | none |
| [2.2](#22-sip-adapter-eager-stream-population-quicwt-parity) | SIP adapter eager-stream parity | ~1 h | none |
| [2.3](#23-wss-substrate-tls-on-the-ws-signaling-channel) | WSS substrate (TLS on WS signaling) | ~2 h | none |
| [2.4](#24-ws-envelope-level-sdp-interception-cleanup) | WS envelope-level SDP interception | ~4 h | none |
| [3.1](#31-spec-codes-501--505-b3) | Spec codes `501` / `505` (B3) | ~30 min impl after spec PR | external spec PR |
| [3.2](#32-playwright-browser-smoke-d5) | Playwright browser smoke (D5) | ~4 h | workspace-policy decision |
| [4.1](#41-trickle-ice--connectionice-candidate-envelope-type) | Trickle ICE / `connection.ice-candidate` envelope | ~6-8 h | spec PR |
| [4.2](#42-mid-call-codec-renegotiation-connectionupdate-codec-change) | Codec renegotiation (mid-call) | ~6 h | none |
| [4.3](#43-rfc-4733--rfc-2833-full-audio-pipeline-c2-remaining) | Full RFC 4733 audio pipeline (C2) | ~1 day | none |
| [5.1](#51-aauth-validator-c4-remaining) | AAuth validator (C4 remaining) | ~1 day | none |
| [5.2](#52-rfc-9421-http-message-signatures-c4-remaining) | RFC 9421 HTTP Message Signatures | ~2 days | spec PR first |

**Closed since V0X_REMAINING.md was written:**

- **C3 — WS WebRTC media plane** ✅ landed 2026-05-25. The "off-limits this session" status that V0X_REMAINING.md §1.2 cited no longer applies; `rvoip-webrtc` is a real workspace crate, and `WebRtcMediaBridge` in `rvoip-websocket` is wired and proven. See the [WS↔WS bridge proof](../rvoip-websocket/tests/ws_bridge_flow.rs).

---

## 2. v0.x cleanups (small, unblocked)

### 2.1 Update `V0X_REMAINING.md` to strike the WS WebRTC row

**Scope:** doc edit only.

**What to change in [`V0X_REMAINING.md`](V0X_REMAINING.md):**

- §1.2 "C3 — `rvoip-websocket` media plane" — strike the entire section, replace with a 2-line "landed 2026-05-25, see `tests/ws_bridge_flow.rs`" pointer.
- §4 summary table — remove the C3 row.
- §1 lead-in — drop "off-limits this session per direct instruction" since that context is gone.

**Test gate:** none (doc-only).

### 2.2 SIP adapter eager-stream population (QUIC/WT parity)

**Why:** [`crates/rvoip-sip/src/adapter.rs:126`](../rvoip-sip/src/adapter.rs) builds `Connection { streams: vec![], ... }` at inbound-call time and lazy-creates the `SipMediaStream` only on first `streams()` call (line 310). QUIC and WT adapters eagerly populate `Connection.streams` at `InboundInvite` time. The asymmetry isn't a correctness bug — bridges through SIP do work because `bridge_connections` polls `streams()` up to the deadline — but it's a footgun for any consumer that inspects `Connection.streams` synchronously off the `Event::ConnectionInbound` event.

**Proposed approach:**

1. In `SipAdapter::build_connection` (or just before the `let connection = self.build_connection(...)` callers at lines 138, 232 of `crates/rvoip-sip/src/adapter.rs`):
   - Construct one `SipMediaStream` with a default Opus codec (mirror what `streams()` lazy-creates).
   - Push a `MediaStreamHandle::new(stream as Arc<dyn MediaStream>)` into the `streams: Vec<_>` field.
   - Also insert into whatever per-connection lookup `streams()` uses (so the eager + lazy paths agree).
2. Delete the lazy-create branch in `streams()` — it becomes a straight `.get(&conn_id).map(...)`.

**Files touched:**

- `crates/rvoip-sip/src/adapter.rs` (the `build_connection` + `streams` methods).
- Likely a per-connection stream map similar to QUIC's `Route.streams`.

**Test gate:** existing `crates/rvoip-sip/tests/*` continue to pass; add one assertion in the adapter integration test that `connection.streams.len() == 1` immediately after `Event::ConnectionInbound`.

**Estimated effort:** ~1 hour.

### 2.3 WSS substrate (TLS on the WS signaling channel)

**Why:** `crates/rvoip-websocket/src/client.rs:24` uses `tokio_tungstenite::connect_async(url.as_str())`, which auto-selects plain `ws://` vs `wss://` based on the URL scheme. But the **server** side at [`crates/rvoip-websocket/src/server.rs:53`](../rvoip-websocket/src/server.rs) only accepts plain `tokio_tungstenite::accept_async(tcp)`. For production deployments where the WS endpoint isn't fronted by a separate reverse proxy (nginx / haproxy / Envoy doing TLS termination), the WS server itself needs to terminate TLS.

**Proposed approach:**

1. Add an optional `tls: Option<Arc<rustls::ServerConfig>>` field to `UctpWsConfig`. When present, the accept-loop wraps the raw `TcpStream` in `tokio_rustls::TlsAcceptor::accept(...)` before calling `accept_async`. When absent, the existing plain path runs unchanged.
2. Provide a `UctpWsConfig::with_tls(rustls_server_config)` builder method.
3. For client-side, `UctpWsClient::connect_with_tls(url, rustls_client_config)` that calls `tokio_tungstenite::connect_async_tls_with_config(...)` for `wss://` URLs.
4. Reuse `crates/rvoip-uctp/src/substrate/tls.rs::self_signed_for_dev` for dev/test cert generation — same pattern QUIC + WT adapters use.

**Files touched:**

- `crates/rvoip-websocket/Cargo.toml` — add `tokio-rustls = { workspace = true }` and gate behind a `wss` feature if we want it optional.
- `crates/rvoip-websocket/src/{server,client,adapter}.rs` — wrapping logic and config plumbing.
- New test `crates/rvoip-websocket/tests/wss_loopback.rs` — mirror `loopback.rs` but over `wss://` with `dev_client_config_trusting`.

**Test gate:** all existing tests (with + without `media-webrtc`) still pass; new `wss_loopback` test passes.

**Estimated effort:** ~2 hours.

### 2.4 WS envelope-level SDP interception (cleanup)

**Why:** today, WS callers who want WebRTC media must drive the SDP exchange via the `UctpWsAdapter::bridge_for(conn_id)` accessor — they fetch the bridge handle, call `local_substrate_setup` and `set_remote_substrate_setup` themselves. This works for the WS↔WS bridge proof test (which is the "application code") but it's an awkward API: applications shouldn't need to hold WebRTC handles. The wire spec carries SDP inside `connection.offer.substrate_setup` / `connection.answer.substrate_setup` — the WS server should intercept those envelopes transparently.

**Proposed approach:**

Modify [`crates/rvoip-websocket/src/server.rs`](../rvoip-websocket/src/server.rs)'s `spawn_peer_session`:

1. **Inbound pump** (the loop at line 135): for each envelope, before `in_tx.send(env)`, inspect `env.msg_type`:
   - `MessageType::ConnectionOffer`: parse the payload as `ConnectionOffer`, extract `substrate_setup` as `WebRtcSubstrateSetup` via `serde_json::from_value`. Look up the route by `env.connid` (we need a new `by_connid: DashMap<String, ConnectionId>` mapping; populate it on InboundInvite). Call `route.bridge.lock().clone().unwrap().set_remote_substrate_setup(setup).await`. If the bridge isn't ready yet, queue the offer in a per-route 1-slot pending buffer; drain when the bridge appears.
   - All other types: forward unchanged.
2. **Outbound pump** (line 166): for each envelope, before `sink.send(...)`:
   - `MessageType::ConnectionAnswer`: parse payload, fetch `bridge.local_substrate_setup().await`, mutate the payload to include `substrate_setup: WebRtcSubstrateSetup { kind, sdp }` via `serde_json::to_value`. Re-marshal. Send.
   - All other types: forward unchanged.
3. Add a `by_connid: Arc<DashMap<String, ConnectionId>>` field to `UctpWsAdapter`, mirroring `by_uctp_sid`. Populate on InboundInvite (the connid the application uses comes from the first `connection.offer` envelope's `connid` field — so populate when we first see a `connection.offer` for a known route).
4. After interception lands, the test in `tests/ws_bridge_flow.rs` should still pass *without* the direct `bridge_for` calls — i.e. an alternate flavor of the test that drives only `connection.offer` envelopes and lets the server's interception handle the SDP plumbing. Add this as a second test (keep the bridge_for variant for diagnostics).

**Risk:** envelope mutation in the outbound pump changes the WS server's "envelopes are opaque" pattern. Document the deviation prominently.

**Files touched:**

- `crates/rvoip-websocket/src/server.rs` (the dual-pump interception + new `by_connid` plumbing).
- `crates/rvoip-websocket/src/adapter.rs` (`by_connid` field on `UctpWsAdapter`).
- New test `crates/rvoip-websocket/tests/ws_envelope_sdp_bridge.rs`.

**Test gate:** existing `ws_bridge_flow.rs` continues to pass; new `ws_envelope_sdp_bridge.rs` passes (drives the full path through envelopes only, no `bridge_for` calls).

**Estimated effort:** ~4 hours. The cognitive load is the mutate-envelope pattern; the actual logic is mechanical.

---

## 3. Externally blocked (decision/PR needed elsewhere)

### 3.1 Spec codes `501` / `505` (B3)

**Status:** carried verbatim from [`V0X_REMAINING.md §1.1`](V0X_REMAINING.md). Still externally blocked on the same spec PR.

**Proposed approach when unblocked** (~30 min):

1. Land the spec PR (one-line addition to `CONVERSATION_PROTOCOL.md §11.2`'s error-code table).
2. Mechanical swap of `503 transient` → `501 not-implemented` in these specific sites (grep for `503` in `crates/rvoip-uctp/`):
   - `OrchestratorSubscriptionHandler` (the legacy reject path that today returns 503).
   - The `NotImplemented` returns from `ConnectionAdapter::hold`, `resume`, `transfer`, `renegotiate_media`, `verify_request_signature` in every adapter (`crates/rvoip-{quic,webtransport,websocket,sip}/src/adapter.rs`).
3. Add `505 version-not-supported` to the protocol-version-mismatch path. Today's mismatch silently drops; the new path should send `error` envelope with code `505`.
4. New test `crates/rvoip-uctp/tests/error_codes.rs` covers both new codes.

### 3.2 Playwright browser smoke (D5)

**Status:** carried verbatim from [`V0X_REMAINING.md §2.1`](V0X_REMAINING.md). Decision pending on adding Node.js to the workspace.

**Proposed approach when policy decision lands** (~4 hours):

1. New top-level directory `tests/browser-smoke/` (Node.js project, kept out of the Cargo workspace).
2. `package.json` pinning `@playwright/test`.
3. `smoke.mjs`:
   - Spawn `cargo run --example orchestrator_bridge` as a child process; wait on stdout for the "listening on 127.0.0.1:..." line.
   - Read the self-signed cert's SHA-256 from a sidecar file the demo writes.
   - Launch headless Chrome with `--ignore-certificate-errors-spki-list=<sha256>`.
   - Open `index.html` (via `file://` or a local HTTP server).
   - Poll `localStorage.getItem('auth.session')` for non-null with 10s deadline.
   - Repeat for `ws_smoke.html`.
   - Exit 0/1.
4. `.github/workflows/browser-smoke.yml` runs `npm ci && npx playwright install chrome && npm test` on PR.
5. `tests/browser-smoke/README.md` documents the local workflow.

**Critical decision before starting:** does the workspace policy permit Node.js? If no, this stays deferred forever as a manual smoke (which is already documented).

---

## 4. v0/v1 protocol-feature gaps (substantial)

### 4.1 Trickle ICE / `connection.ice-candidate` envelope type

**Why:** today's `WebRtcSubstrateSetup.sdp` in `crates/rvoip-uctp/src/payloads/connection.rs` carries all ICE candidates inline. The WS bridge gathers candidates fully before sending the offer/answer (`gathering_complete_promise()` semantics). On LAN this completes in ~50ms. Across a real NAT with STUN/TURN it can take 2-5 seconds, blocking session setup. Trickle ICE lets candidates be sent incrementally as they're gathered. Per `CONVERSATION_PROTOCOL.md §10.2.2` this is **v1** work and explicitly deferred.

**Proposed approach when prioritized** (~6-8 hours):

1. **Spec PR first** — `CONVERSATION_PROTOCOL.md §10.2.2` to define:
   - New envelope type `connection.ice-candidate` (bidirectional, no reply).
   - Payload shape: `{ candidate: string, sdp_m_line_index: u16, sdp_mid: string }` matching the `RTCIceCandidateInit` browser type.
   - When is it valid to send (after initial offer/answer exchange, until session.end).
   - End-of-candidates signal (canonically: empty `candidate` string).
2. **Wire type** — add `MessageType::ConnectionIceCandidate` to `crates/rvoip-uctp/src/types.rs` (both serialize + deserialize tables).
3. **Payload type** — add `IceCandidateInit` struct to `crates/rvoip-uctp/src/payloads/connection.rs`.
4. **Bridge integration** — modify `crates/rvoip-websocket/src/media_bridge.rs::WebRtcMediaBridge`:
   - On construction, register an `on_ice_candidate` callback on the underlying `RvoipPeerConnection` (needs `rvoip-webrtc` to expose this — verify API).
   - Each emitted candidate becomes a `connection.ice-candidate` envelope on the route's `out_tx`.
   - Inbound `connection.ice-candidate` calls `peer.add_ice_candidate(init).await`.
5. **Switch from `gathering_complete_promise()` semantics to trickle** — `RvoipPeerConnection::create_offer_and_gather` returns immediately with the initial offer (no candidates), and candidates flow asynchronously. May require new constructor methods on `rvoip-webrtc` (`create_offer_no_gather` etc.) or a config flag.
6. **`WebRtcConfig.trickle_ice` field** — already exists per the rvoip-webrtc inventory (line 169-ish of `config.rs`). Honor it.
7. **Test** — new `crates/rvoip-websocket/tests/trickle_ice.rs` asserts candidates flow over `connection.ice-candidate` envelopes and the bridge reaches connected without inline SDP candidates.

**Files touched:** `rvoip-uctp::{types,payloads/connection}`, `rvoip-websocket::media_bridge`, possibly `rvoip-webrtc::{config,peer/session}` (need to expose `on_ice_candidate` callback + `add_ice_candidate` method if not already there).

**Risk:** `rvoip-webrtc` API surface may not expose the trickle hooks today. Verify before estimating.

### 4.2 Mid-call codec renegotiation (`connection.update` codec change)

**Why:** all four adapters (SIP, QUIC, WT, webrtc) implement `ConnectionAdapter::renegotiate_media` as `NotImplemented`. The UCTP coordinator has no `connection.update` handler that drives a codec change. Mid-call codec change is needed for: SIP re-INVITE codec negotiation, dropping to a lower-bandwidth codec under network stress, switching from PCMU to Opus when both peers learn they support it.

**Proposed approach** (~6 hours):

1. **Add `MessageType::ConnectionUpdate` handler in the coordinator** ([`crates/rvoip-uctp/src/state/coordinator.rs`](src/state/coordinator.rs)):
   - When an inbound `connection.update` envelope arrives with `action = "renegotiate-media"` and `codec_preferences = [...]`, run the §8.1 negotiation algorithm against the current peer caps + the new preferences.
   - On success, emit `connection.update` reply with the chosen codec.
   - On failure (no overlap), emit `error` with code `488 not-acceptable`.
2. **Wire `Orchestrator::renegotiate_media` driver** ([`crates/rvoip-core/src/orchestrator.rs`](../rvoip-core/src/orchestrator.rs)):
   - Look up the adapter for the connection's transport, call `adapter.renegotiate_media(conn_id, new_caps)`.
3. **Adapter impl** for each substrate:
   - **QUIC + WT** (`crates/rvoip-quic/src/adapter.rs:507`, `crates/rvoip-webtransport/src/adapter.rs`): send a `connection.update` envelope with `action = "renegotiate-media"`; await the reply; update the connection's `negotiated_codecs` field.
   - **WS** (`crates/rvoip-websocket/src/adapter.rs:337`): same shape. The WebRTC media plane needs an additional step — call `WebRtcMediaBridge::renegotiate_codec(new_codec)` which drives an SDP renegotiation (re-create offer, re-exchange, re-apply). `rvoip-webrtc` already has a `hold_renegotiate` config field per the inventory, indicating partial support; verify the full mid-call renegotiation API.
   - **SIP** (`crates/rvoip-sip/src/adapter.rs:360`): send a re-INVITE with the new SDP. SIP layer's existing re-INVITE machinery handles the dialog state.
4. **Transcoder hot-swap** — the orchestrator's `bridge_pump` holds a per-direction `Transcoder` keyed on `(from_pt, to_pt)`. When one side renegotiates, the pump's `Transcoder` must be rebuilt. Add a control channel on `CrossBridgeHandle` for "swap transcoder."
5. **Tests** — new `crates/rvoip-uctp/tests/renegotiate.rs` exercises Opus↔Opus → Opus↔PCMU mid-call. Asserts frames continue flowing post-renegotiation with correct PT.

**Files touched:** `rvoip-uctp::state::coordinator`, `rvoip-core::orchestrator`, all four adapter crates' `renegotiate_media` methods, `rvoip-core::bridge::{cross_handle,frame_pump}`.

**Risk:** transcoder hot-swap under live traffic without dropping frames is subtle. Likely needs a "drain + swap" protocol with a small buffer window.

### 4.3 RFC 4733 / RFC 2833 full audio pipeline (C2 remaining)

**Status:** carried from [`V0X_REMAINING.md §3.3`](V0X_REMAINING.md). Partial passthrough already shipped — `frame_pump` detects 4-byte transcode failures (the RFC 4733 telephone-event size) and emits the `rvoip_bridge_dtmf_passthrough_total` metric.

**Proposed approach** (~1 day):

Mechanically follow V0X_REMAINING.md §3.3:

1. **Add `payload_type: Option<u8>` to `MediaFrame`** in `crates/rvoip-core/src/stream.rs`.
2. **Mechanical update of all 79 `MediaFrame { ... }` construction sites** (counted today; V0X_REMAINING.md said "70+"). Each gets `payload_type: None` by default; the SIP-side and WebRTC inbound pumps set it from the actual RTP header.
3. **`frame_pump`** in `crates/rvoip-core/src/bridge/frame_pump.rs` routes `payload_type == Some(101)` (or the negotiated telephone-event PT) to a separate sink that emits `UctpSessionEvent::Dtmf` on the bridged side instead of forwarding the audio frame.
4. **Reverse direction**: UCTP `dtmf.send` → synthesize RFC 4733 RTP packets on the SIP side. Add a `SipMediaStream::send_dtmf_event(digit, duration)` method that emits the packets.
5. **Tests** — `crates/rvoip-uctp/tests/dtmf_bridge.rs` exercises both directions across a SIP↔UCTP bridge.

**Files touched:** rvoip-core (the trait + many call sites), rvoip-sip (RTP reader + RFC 4733 emitter), rvoip-quic / rvoip-webtransport / rvoip-websocket / rvoip-media-core / rvoip-rtp-core (mechanical `MediaFrame` construction updates). The 79-site touch is fully mechanical; budget accordingly.

---

## 5. Standards-track auth backends (each is its own session)

### 5.1 AAuth validator (C4 remaining)

**Status:** carried verbatim from [`V0X_REMAINING.md §3.1`](V0X_REMAINING.md). Standalone work, no blockers.

**Proposed approach** (~1 day):

1. New `crates/auth-core/src/aauth.rs` defining `AAuthValidator: BearerValidator + ActorTokenValidator`.
2. `auth.response` envelope payload extended with optional `actor_token: Option<String>` field (already part of CONVERSATION_PROTOCOL.md §5.6 per V0X_REMAINING.md's read).
3. Validator parses actor + subject claims, maps to `IdentityAssurance::UserAuthorized { user_id: subject, identity: actor, scopes }`.
4. Tests — round-trip a signed AAuth token through the validator; assert `IdentityAssurance` shape.

### 5.2 RFC 9421 HTTP Message Signatures (C4 remaining)

**Status:** carried from [`V0X_REMAINING.md §3.2`](V0X_REMAINING.md). **Double-blocked**: needs spec PR defining canonical envelope-field set first.

**Proposed approach when unblocked** (~2 days):

1. **Spec PR first** — `CONVERSATION_PROTOCOL.md` PR defining:
   - Which envelope fields participate in the signature base (`v`, `msg_type`, `id`, `ts`, `cid`?, `sid`?, `connid`?, `in_reply_to`?, `payload`?).
   - Canonical serialization (JSON Canonical Form vs custom).
   - Signature header format (likely a top-level `signature: { keyid, alg, sig }` field on the envelope, or a separate `auth.sign` envelope wrapping the signed inner).
2. New `crates/auth-core/src/sig9421.rs` with key resolution, canonicalization, signature verification, replay protection (via a JTI-like cache, mirror `crates/auth-core/src/dpop.rs`'s pattern).
3. Verifier integration in the coordinator's auth gate (`crates/rvoip-uctp/src/state/coordinator.rs`).
4. Tests — round-trip a signed envelope; verify replay rejection; cross-key tampering rejection.

---

## 6. Out of scope

These items showed up in earlier audits or design docs but are **not** scheduled:

- **CRC32 / checksum on envelopes** — UCTP relies on TLS/QUIC/WSS for transport integrity; per-envelope checksums are out of scope.
- **`stream.active-speaker` envelope** — listed v1 in the design doc; not yet prioritized.
- **Multi-recording session — `recording.vcon-fetch` / `vcon-fetched` round-trip** — `recording.vcon-ready` emission is wired; full fetch round-trip needs the vCon-store-side queries. Carry forward to v0.x.
- **WebTransport over h3-datagram** vs the current QUIC-datagram fallback — performance optimization; not a correctness gap.

---

## 7. Recommended execution order

If the next session has limited budget, the recommendation order is:

1. **§2.1** (10 min) — strike stale V0X_REMAINING.md entries. Always do first; cheap and reduces confusion.
2. **§2.2** (1 h) — SIP adapter eager-stream parity. Smallest substantive code change; eliminates a footgun.
3. **§3.1** (30 min, **gated on external spec PR**) — error codes 501/505. Lands when the spec PR lands.
4. **§2.3** (2 h) — WSS substrate. Unblocks production deployments without external TLS terminators.
5. **§2.4** (4 h) — WS envelope-level SDP interception. Removes the awkward `bridge_for` accessor from the application-facing API.
6. **§4.3** (1 day) — RFC 4733 full audio pipeline. Self-contained mechanical refactor; biggest correctness win.
7. **§4.2** (6 h) — codec renegotiation. Substantial but standalone.
8. **§4.1** (6-8 h, **gated on spec PR**) — trickle ICE. Production-NAT critical but spec-gated.
9. **§5.1** (1 day) — AAuth validator. Standalone IETF work.
10. **§3.2** (4 h, **gated on workspace-policy decision**) — Playwright browser smoke.
11. **§5.2** (2 days, **gated on spec PR**) — RFC 9421. Biggest scope; do last.

§2.1, §2.2, §2.3, §2.4, §4.2, §4.3, §5.1 are unblocked and can land in parallel/any order. The rest depend on external decisions.

---

## 8. Critical files (reference)

| Purpose | Path |
|---|---|
| Authoritative design + as-built | [`UCTP_IMPLEMENTATION_PLAN.md`](UCTP_IMPLEMENTATION_PLAN.md) |
| Previous remaining-work pass (partially stale) | [`V0X_REMAINING.md`](V0X_REMAINING.md) |
| Wire spec | [`../rvoip-core/CONVERSATION_PROTOCOL.md`](../rvoip-core/CONVERSATION_PROTOCOL.md) |
| Architecture | [`../rvoip-core/INTERFACE_DESIGN.md`](../rvoip-core/INTERFACE_DESIGN.md) |
| WS WebRTC bridge (today's landing) | [`../rvoip-websocket/src/media_bridge.rs`](../rvoip-websocket/src/media_bridge.rs), [`../rvoip-websocket/tests/ws_bridge_flow.rs`](../rvoip-websocket/tests/ws_bridge_flow.rs) |
| Cross-transport frame pump | [`../rvoip-core/src/bridge/frame_pump.rs`](../rvoip-core/src/bridge/frame_pump.rs) |
| `ConnectionAdapter` trait (where `NotImplemented` stubs live) | [`../rvoip-core/src/adapter.rs`](../rvoip-core/src/adapter.rs) |
| `MediaFrame` (gets `payload_type` field in §4.3) | [`../rvoip-core/src/stream.rs`](../rvoip-core/src/stream.rs) |
| Auth gate in coordinator | [`src/state/coordinator.rs`](src/state/coordinator.rs) |

---

**Verification command at every milestone** (no regressions):

```bash
cargo test -p rvoip-uctp -p rvoip-quic -p rvoip-webtransport -p rvoip-core -p rvoip-auth-core -p rvoip-vcon
cargo test -p rvoip-websocket --features media-webrtc
```

Currently both commands pass with **0 failures** across the entire UCTP-family suite (155+ tests as of this writing).
