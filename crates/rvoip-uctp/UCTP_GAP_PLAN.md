# UCTP — Remaining Gap Plan

**Date written:** 2026-05-25
**Last updated:** 2026-05-25 (end-of-session as-built pass — see [§0](#0-as-built-status-2026-05-25))
**Supersedes (partially):** `V0X_REMAINING.md` §1.2 (WS WebRTC media plane landed; see [Closed since V0X_REMAINING was written](#closed-since-v0x_remainingmd-was-written) below).
**Companion docs:** [`UCTP_IMPLEMENTATION_PLAN.md`](UCTP_IMPLEMENTATION_PLAN.md) (authoritative design + as-built record), [`V0X_REMAINING.md`](V0X_REMAINING.md) (the previous remaining-work pass).

---

## 0. As-built status (2026-05-25)

A single session executed the full plan below. Net result: **8 of 11 sections landed fully**, **3 of 11 landed partially** with the unfinished pieces explicitly carved out. Spec edits that the original plan listed as "externally blocked on a spec PR" turned out to be local edits to `CONVERSATION_PROTOCOL.md` (no submodule, no external sync), so §3.1, §4.1, and §5.2 unblocked themselves and landed inline with their implementation work.

| § | Task | Status | Notes |
|---|---|---|---|
| 2.1 | V0X_REMAINING.md cleanup | ✅ Landed | `V0X_REMAINING.md` C3 row replaced with landed pointer; lead-in softened. |
| 2.2 | SIP eager-stream parity | ✅ Landed | `SipAdapter::build_connection` is now async; `get_or_init_stream` populates the cache at inbound time. Test: `crates/rvoip-sip/tests/adapter_eager_streams.rs`. |
| 2.3 | WSS substrate | ✅ Landed | Optional `wss` feature on `rvoip-websocket`; `UctpWsConfig::with_tls`, `UctpWsClient::connect_with_tls`; `spawn_peer_session` generic over `S: AsyncRead + AsyncWrite`. Test: `crates/rvoip-websocket/tests/wss_loopback.rs`. |
| 2.4 | WS envelope SDP interception | ✅ Landed | `intercept_connection_offer` + `mutate_connection_answer` in `server.rs`; per-route `pending_offer` slot drained when bridge constructs. Test: `crates/rvoip-websocket/tests/ws_envelope_sdp_bridge.rs`. Existing `ws_bridge_flow.rs` still passes (bridge_for path preserved for diagnostics). |
| 3.1 | 501 / 505 error codes | ✅ Landed | Spec edit: `CONVERSATION_PROTOCOL.md §11.2`. Impl: `subscription.rs` 503→501; coordinator `dispatch_inner` runs v=1 gate and emits `emit_version_not_supported`. Test: `crates/rvoip-uctp/tests/error_codes.rs` + updated `coordinator.rs` test. |
| 3.2 | Playwright browser smoke | 🟡 **Partial** | WS path automated end-to-end (`tests/browser-smoke/` Node project + `.github/workflows/browser-smoke.yml`). **WT path deferred** — Chromium's SPKI cert pinning is fragile in headless CI; the manual recipe at `examples/uctp_to_sip_bridge/browser/README.md` still applies for WT. See [§3.2 Landed / deferred](#32-playwright-browser-smoke-d5). |
| 4.1 | Trickle ICE + spec | ✅ Landed | Spec edit: full `§10.2.2`. Wire types: `MessageType::ConnectionIceCandidate`, `payloads::connection::IceCandidateInit`. Bridge hooks: `WebRtcMediaBridge::{next_local_ice_candidate, add_remote_ice_candidate}`. Test: `crates/rvoip-websocket/tests/trickle_ice.rs`. |
| 4.2 | Codec renegotiation | 🟡 **Partial** | **Envelope layer landed** — coordinator `handle_connection_update` with `action=renegotiate-media` runs §8.1 negotiation, replies with chosen codec or `488`. **Deferred**: `Orchestrator::renegotiate_media` driver, per-adapter `renegotiate_media` impls (still `NotImplemented`), transcoder hot-swap on `CrossBridgeHandle`. Test: `crates/rvoip-uctp/tests/renegotiate.rs` (3 envelope-layer cases). See [§4.2 Landed / deferred](#42-mid-call-codec-renegotiation-connectionupdate-codec-change). |
| 4.3 | RFC 4733 audio pipeline | 🟡 **Partial** | `MediaFrame.payload_type: Option<u8>` field added + 28 construction sites updated; WebRTC and SIP inbound pumps stamp the real PT; `frame_pump` routes PT 101 distinctly (`DEFAULT_TELEPHONE_EVENT_PT`); `SipMediaStream` outbound: PT-101 frames bypass G.711 decode and call `send_dtmf`. **Deferred**: auto-wiring `dtmf.send` envelope → `SipMediaStream::send_dtmf_event` on the bridged side from the coordinator (application code can still call `adapter.send_dtmf` directly; `UctpSessionEvent::Dtmf` event flow is unchanged). Test: `crates/rvoip-uctp/tests/dtmf_bridge.rs` + RFC 4733 unit tests. |
| 5.1 | AAuth validator | ✅ Landed | New `crates/auth-core/src/aauth.rs` (`AAuthValidator`, `ActorTokenValidator`, `ActorClaims`); `AuthResponse.actor_token: Option<String>`; `UctpCoordinator::start_full_with_aauth` constructor; `handle_auth_response` branches on `method == "aauth"`. Tests: `crates/rvoip-uctp/tests/aauth.rs` (2 integration) + 3 unit tests in `aauth.rs`. |
| 5.2 | RFC 9421 + spec | ✅ Landed | Spec edit: full `§5.5.1` (inline-signature shape, JCS canonicalization, covered components, failure modes, replay window). Impl: `crates/auth-core/src/sig9421.rs` (Ed25519 verifier, hand-rolled JCS, moka replay cache mirroring DPoP). 11 unit tests (round-trip, tamper, replay, stale, unknown-keyid, cross-key tampering, JCS canonicalization properties). **Note**: coordinator-side auto-verification gate not wired — verifier is a standalone API surface deployments opt into. |

**Test gates green after batch:**

| Gate | Tests passed |
|---|---|
| `rvoip-uctp -p rvoip-quic -p rvoip-webtransport -p rvoip-core -p rvoip-auth-core -p rvoip-vcon` | 89 / 0 failed |
| `rvoip-websocket --features media-webrtc` | 9 / 0 failed |
| `rvoip-websocket --features wss` | 5 / 0 failed |
| `rvoip-sip` (full suite, post-§4.3 plumbing) | passing |
| `tests/browser-smoke` (Playwright) | 1 / 0 failed |

**Carve-outs to track as a v1 punch list** (collected from the three partials above):

- §4.2: adapter `renegotiate_media` impls (QUIC / WT / WS / SIP) + transcoder hot-swap on `CrossBridgeHandle`. Wire protocol is in place so this is purely the driver side.
- §4.3: coordinator auto-route `dtmf.send` → `SipMediaStream::send_dtmf_event` on the bridged-side SIP leg.
- §3.2: WT browser smoke (needs SPKI-pinning workaround for headless Chromium).
- §5.2: optional coordinator gate that verifies every signed envelope before dispatch.

> **⚠️ Known limitation carried forward to v1.x — §3.2 WT bidi-stream interop.**
> Headless Chromium SPKI pinning works (the actual §3.2 deliverable), but the
> bidi-stream envelope round-trip from a Chromium WebTransport client to the
> Rust server does **not** complete. Four `web-transport-quinn` versions
> (0.5, 0.6, 0.7, 0.11) all exhibit the same gap; Rust↔Rust works fine on
> the same wire path. Diagnosis + tried-versions table lives in
> [§7.2.3](#723-32-wt-browser-smoke--detail). Track upstream
> `web-transport-quinn` 0.12+ for Chromium fixes; alternative is a wire-spec
> change to unidirectional streams + datagrams.

---

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

**Status:** ✅ **Landed 2026-05-25.** `V0X_REMAINING.md` §1.2 replaced with 2-line landed pointer; §4 summary table C3 row removed; §1 lead-in softened. Plus a top-level pointer added to this gap plan.

**Scope:** doc edit only.

**What to change in [`V0X_REMAINING.md`](V0X_REMAINING.md):**

- §1.2 "C3 — `rvoip-websocket` media plane" — strike the entire section, replace with a 2-line "landed 2026-05-25, see `tests/ws_bridge_flow.rs`" pointer.
- §4 summary table — remove the C3 row.
- §1 lead-in — drop "off-limits this session per direct instruction" since that context is gone.

**Test gate:** none (doc-only).

### 2.2 SIP adapter eager-stream population (QUIC/WT parity)

**Status:** ✅ **Landed 2026-05-25.** `build_connection` is now async and calls `get_or_init_stream` to populate the per-`ConnectionId` cache eagerly. Both inbound (`translate_api_event::IncomingCall`) and outbound (`originate`) paths return `Connection.streams.len() == 1`. The `streams()` impl falls back to lazy-init if the eager construction failed, preserving the prior failure surface for unknown connection IDs (`ConnectionNotFound`). Test: `crates/rvoip-sip/tests/adapter_eager_streams.rs`.

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

**Status:** ✅ **Landed 2026-05-25.** New `wss` feature on `rvoip-websocket` pulls in `tokio-rustls` + `rustls` and the `rustls-tls-webpki-roots` feature on `tokio-tungstenite`. `UctpWsConfig::with_tls(ServerConfig)` enables TLS termination; `UctpWsClient::connect_with_tls(url, ClientConfig)` dials `wss://`. The accept loop branches on the optional TLS acceptor and `spawn_peer_session` is generic over `S: AsyncRead + AsyncWrite` to handle both raw TCP and `TlsStream<TcpStream>`. Test: `crates/rvoip-websocket/tests/wss_loopback.rs` (mirrors `loopback.rs` but pins the demo cert via `dev_client_config_trusting`).

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

**Status:** ✅ **Landed 2026-05-25.** Implementation deviates slightly from the original sketch — instead of adding a `by_connid: DashMap<String, ConnectionId>` map, the interception uses the existing `by_uctp_sid` map via the envelope's `sid` field (which the WS test always supplies and which is needed anyway for SID→ConnectionId lookups elsewhere). The `Route` struct gains a `pending_offer: Mutex<Option<WebRtcSubstrateSetup>>` slot that `spawn_bridge_setup` drains after construction. The inbound pump's interception (`intercept_connection_offer`) also autonomously emits a `connection.answer` envelope after applying the offer SDP, so the test doesn't need any application-side code to drive the answer side — the outbound pump's `mutate_connection_answer` then fills in the local SDP via `bridge.local_substrate_setup()` before the answer hits the wire. Tests: existing `ws_bridge_flow.rs` still passes (bridge_for path preserved for diagnostics); new `ws_envelope_sdp_bridge.rs` drives the full path through envelopes only.

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

**Status:** ✅ **Landed 2026-05-25.** The "externally blocked on spec PR" framing turned out to be wrong — `CONVERSATION_PROTOCOL.md` is a local file in this repo with no submodule / sync directive, so the spec edit landed alongside the impl. §11.2 now distinguishes 501 (capability gap on the receiving build) from 503 (transient capacity), and adds 505 for protocol-version mismatch. `RejectingHandler` returns 501; coordinator `dispatch_inner` gates on `env.v != 1` and emits `emit_version_not_supported` with `details.supported = [1]`. Tests: `crates/rvoip-uctp/tests/error_codes.rs` covers both new codes; `coordinator.rs::multi_party_stream_subscribe_rejected_with_501` updated.

**Proposed approach when unblocked** (~30 min):

1. Land the spec PR (one-line addition to `CONVERSATION_PROTOCOL.md §11.2`'s error-code table).
2. Mechanical swap of `503 transient` → `501 not-implemented` in these specific sites (grep for `503` in `crates/rvoip-uctp/`):
   - `OrchestratorSubscriptionHandler` (the legacy reject path that today returns 503).
   - The `NotImplemented` returns from `ConnectionAdapter::hold`, `resume`, `transfer`, `renegotiate_media`, `verify_request_signature` in every adapter (`crates/rvoip-{quic,webtransport,websocket,sip}/src/adapter.rs`).
3. Add `505 version-not-supported` to the protocol-version-mismatch path. Today's mismatch silently drops; the new path should send `error` envelope with code `505`.
4. New test `crates/rvoip-uctp/tests/error_codes.rs` covers both new codes.

### 3.2 Playwright browser smoke (D5)

**Status:** 🟡 **Partial — landed 2026-05-25.** Workspace policy decision: Node.js is permitted, hosted at `tests/browser-smoke/` and excluded from the Cargo workspace via `Cargo.toml` `exclude`. **WS path** is fully automated: `playwright.config.mjs` + `tests/ws_smoke.spec.mjs` spawns `cargo run --example orchestrator_bridge`, serves a small inline HTML page over a local `http://127.0.0.1` origin (required to satisfy Chromium's Private Network Access policy when opening a WS to 127.0.0.1), and asserts the `auth.hello → auth.challenge` round-trip succeeds. `.github/workflows/browser-smoke.yml` runs it on PR.

**Landed / deferred:**

- ✅ Node.js scaffolding (`package.json`, `playwright.config.mjs`, `README.md`).
- ✅ WS smoke spec + CI workflow.
- ❌ **WT smoke.** The WT path needs SPKI cert pinning via Chrome's `--ignore-certificate-errors-spki-list` flag, which is unreliable in headless CI runners. The manual recipe at `examples/uctp_to_sip_bridge/browser/README.md` still applies. Track as a follow-up that may need a different harness (e.g. an in-process WT client written in Rust that exercises the same browser-API contract).

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

**Status:** ✅ **Landed 2026-05-25.** Spec PR was a local edit (same as §3.1 / §5.2); §10.2.2 now defines the full envelope shape, validity window, and end-of-candidates marker. `MessageType::ConnectionIceCandidate` and `payloads::connection::IceCandidateInit` are wired into `rvoip-uctp`. `WebRtcMediaBridge` exposes `next_local_ice_candidate()` (drains via `peer.recv_local_ice()` + `cand.to_json()`) and `add_remote_ice_candidate(init)` (constructs `RTCIceCandidateInit` + `peer.add_remote_ice_candidate`). The end-of-candidates marker is a no-op on the add side (webrtc-rs infers gathering complete from the wire). Test: `crates/rvoip-websocket/tests/trickle_ice.rs` (3 cases — round-trip API, EoC marker round-trip, wire-JSON serde).

**Not yet wired** (out of immediate scope, can layer on top): an automatic outbound trickle pump in the WS server that drains every bridge's `next_local_ice_candidate` and emits `connection.ice-candidate` envelopes on the route's `out_tx`. The hooks exist; the orchestration is a follow-up.

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

**Status:** 🟡 **Partial — envelope layer landed 2026-05-25.**

**Landed:**

- Coordinator `handle_connection_update` for `action == "renegotiate-media"` runs §8.1 negotiation against the local descriptor + the incoming `codec_preferences`, replies with `connection.update` carrying the chosen codec, or `error 488` on no overlap. Unknown actions ack (forward-compat). Test: `crates/rvoip-uctp/tests/renegotiate.rs` (3 cases).
- Metric: `uctp_capability_renegotiations_total` with `outcome` label.

**Deferred (Phase 2 of the original plan):**

- `Orchestrator::renegotiate_media(conn, new_caps)` driver method.
- Per-adapter `renegotiate_media` implementations (QUIC, WT, WS, SIP — all four still return `RvoipError::NotImplemented`, which post-§3.1 maps cleanly to the wire `501 not-implemented` code).
- `WebRtcMediaBridge::renegotiate_codec(new_codec)` for the WS WebRTC media plane.
- Transcoder hot-swap on `CrossBridgeHandle` (control channel + frame_pump rebuild with a small drain window).

The wire protocol is now in place, so peers that negotiate via envelopes get a working contract — they just can't drive the local media plane through the adapter trait yet without an application-side workaround. The deferred work is purely the driver side.

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

**Status:** 🟡 **Partial — structural changes landed 2026-05-25.**

**Landed:**

- `MediaFrame.payload_type: Option<u8>` added (`crates/rvoip-core/src/stream.rs`); 28 construction sites across `rvoip-core`, `rvoip-uctp`, `rvoip-quic`, `rvoip-webtransport`, `rvoip-websocket`, `rvoip-webrtc`, `rvoip-sip` mechanically updated to `payload_type: None`.
- `crates/rvoip-webrtc/src/media/pump.rs` inbound RTP-to-MediaFrame now stamps `payload_type: Some(pkt.header.payload_type)` — real PT from the wire.
- `crates/rvoip-sip/src/media_stream.rs` inbound encoder stamps `payload_type: Some(0)` (PCMU is the SIP wrapper's only output codec).
- `frame_pump` (`crates/rvoip-core/src/bridge/frame_pump.rs`) adds `DEFAULT_TELEPHONE_EVENT_PT = 101` and a fast-path that bypasses transcoding for any frame whose `payload_type == Some(101)` — strict improvement on the 4-byte heuristic (which still fires as a fallback for unlabeled frames). Metric `rvoip_bridge_dtmf_passthrough_total` increments on both paths.
- `SipMediaStream` outbound: PT-101 MediaFrames are parsed via `parse_rfc4733_digit` (handles event start-packet detection to avoid double-emit on retransmits) and routed to `coordinator.send_dtmf(session, digit)` instead of G.711 decode + `send_audio`.
- Tests: `crates/rvoip-uctp/tests/dtmf_bridge.rs` (2 cases — pass-through with matching PTs, pass-through with transcoding); frame_pump unit test for the PT-aware path; 5 RFC 4733 unit tests for `parse_rfc4733_digit`.

**Deferred:**

- Auto-wiring of inbound `dtmf.send` envelopes in the coordinator to call `SipMediaStream::send_dtmf_event` on the bridged-side SIP leg. Today the coordinator emits `UctpSessionEvent::Dtmf` (existing); application code can already call `adapter.send_dtmf` to forward — the missing piece is automatic cross-substrate DTMF routing inside the orchestrator. This is small follow-up work (~50 LOC).

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

**Status:** ✅ **Landed 2026-05-25.** New `crates/auth-core/src/aauth.rs` defines `ActorTokenValidator` (with `ActorClaims { identity, scopes }`) and `AAuthValidator` (wraps a `BearerValidator` for the subject + an `ActorTokenValidator` for the actor; `validate_aauth(subject_tok, actor_tok)` combines into `IdentityAssurance::UserAuthorized { user_id: subject, identity: actor, scopes: union }`). The subject must validate to `UserAuthorized` itself — anonymous / pseudonymous / identified-only subjects are explicitly rejected.

`AuthResponse` gained an `actor_token: Option<String>` field (skipped during serialization when `None` for backward-compat). The coordinator gained `UctpCoordinator::start_full_with_aauth(…)` that wires an optional `Arc<AAuthValidator>`. `handle_auth_response` branches: `method == "aauth"` routes to the AAuth path (returns `401 auth/aauth-not-configured` if the constructor variant wasn't used); every other method goes through the standard bearer validator unchanged.

Tests: `crates/rvoip-uctp/tests/aauth.rs` (2 integration cases — happy path with combined assurance round-trip + missing actor token rejected with 401); `crates/auth-core/src/aauth.rs` 3 unit cases (combine, missing actor, pseudonymous subject rejected).

**Proposed approach** (~1 day):

1. New `crates/auth-core/src/aauth.rs` defining `AAuthValidator: BearerValidator + ActorTokenValidator`.
2. `auth.response` envelope payload extended with optional `actor_token: Option<String>` field (already part of CONVERSATION_PROTOCOL.md §5.6 per V0X_REMAINING.md's read).
3. Validator parses actor + subject claims, maps to `IdentityAssurance::UserAuthorized { user_id: subject, identity: actor, scopes }`.
4. Tests — round-trip a signed AAuth token through the validator; assert `IdentityAssurance` shape.

### 5.2 RFC 9421 HTTP Message Signatures (C4 remaining)

**Status:** ✅ **Landed 2026-05-25.** Both pieces (spec + impl) landed inline since the "spec PR" was a local edit. New `CONVERSATION_PROTOCOL.md §5.5.1` defines the inline-signature envelope shape (`signature: { keyid, alg, sig }`), the JCS canonicalization rule (clone envelope, strip `signature`, serialize per RFC 8785), covered fields (`v`, `type`, `id`, `ts`, `cid`, `sid`, `connid`, `in_reply_to`, `payload`), supported algorithms (MUST EdDSA, SHOULD ES256, MAY PS256/RS256), the 5-minute replay window, and the full failure-mode table mapping to `401-1 invalid-signature` with diagnostic reasons (`replay-detected`, `stale-timestamp`, `signature-required`).

Impl: `crates/auth-core/src/sig9421.rs` ships `Sig9421Verifier` (Ed25519 only in v0, ring-backed), `KeyResolver` trait + `StaticKeyResolver` for tests/static deployments, `EnvelopeSignature` payload type, and a moka-backed replay cache mirroring the DPoP module's pattern (`DEFAULT_REPLAY_CACHE_CAPACITY = 100_000`, `DEFAULT_SIG_REPLAY_TTL = 300s`). JCS is hand-rolled for the bounded envelope shape (objects of strings/numbers/booleans/null/arrays/sub-objects) — sufficient for our types, no `serde_jcs` dep needed.

Tests: 11 cases covering happy-path round-trip, tampered-payload rejection, replay rejection (second `verify` call), unknown keyid, cross-key tampering, stale-timestamp rejection, missing-signature, JCS key sorting / escape / nested object correctness.

**Not yet wired**: the coordinator-side auto-verification gate (i.e. "every inbound envelope with a `signature` field gets routed through the verifier before dispatch"). The verifier is a standalone API surface; deployments that want signature enforcement opt in by calling `verifier.verify(&env).await` in their own envelope-receive path. Wiring this universally requires a policy decision (which envelope types require signatures? what's the per-Connection key set?) that's better made per-deployment than baked into v0.

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

## 7. Recommended execution order *(historical — preserved for context)*

> This was the recommended order at the start of the 2026-05-25 session. The full sequence executed in one pass; see [§0](#0-as-built-status-2026-05-25) for what actually landed. The remaining v1 punch list (collected from the three partials) lives in [§7.1](#71-v1-punch-list).

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

### 7.1 v1 punch list

What's still open after the 2026-05-25 pass:

1. **§4.2 driver side** (~4-6 h) — `Orchestrator::renegotiate_media` + per-adapter `renegotiate_media` impls + `WebRtcMediaBridge::renegotiate_codec` + `CrossBridgeHandle` transcoder hot-swap. Wire protocol is done; this is the local-media-plane wiring.
2. **§4.3 cross-bridge DTMF auto-route** (~1-2 h) — when `dtmf.send` arrives on a connection that's bridged, the orchestrator should automatically call `SipMediaStream::send_dtmf_event` on the bridged-side SIP leg instead of just emitting `UctpSessionEvent::Dtmf` for application code to handle.
3. **§3.2 WT browser smoke** (~3-4 h) — needs an SPKI-pinning workaround that survives headless Chromium in CI, or a different harness shape (Rust-side WT client exercising the same browser contract).
4. **§5.2 coordinator auto-verify gate** (~2-3 h) — optional coordinator hook that runs `Sig9421Verifier::verify` on every inbound envelope carrying a `signature` field, gated by per-deployment policy (which envelope types require it, what's the per-Connection key set).
5. **§4.1 outbound trickle pump** (~1-2 h) — automatic forwarder in the WS server that drains every bridge's `next_local_ice_candidate` and emits `connection.ice-candidate` envelopes on the route's `out_tx`. Hooks exist; this is the orchestration.

Total carry-forward: ~12-17 hours of work, none blocked. These can land piecemeal as the v1 surface matures.

### 7.2 v1 punch list — landings (2026-05-25 follow-up session)

A second 2026-05-25 session executed the v1 punch list. Status:

| # | Item | Status | Notes |
|---|---|---|---|
| 7.1.1 | §4.2 driver side | ✅ Landed | See [§7.2.1](#721-42-driver-side-detail) below. |
| 7.1.2 | §4.3 DTMF auto-route | ✅ Landed | `Orchestrator::bridge_peer_of` + auto-forward in `AdapterEvent::Dtmf` handler (`crates/rvoip-core/src/orchestrator.rs`). Metric: `uctp_bridge_dtmf_forwarded_total`. Test: `crates/rvoip-core/tests/dtmf_auto_route.rs` (2 cases). |
| 7.1.3 | §3.2 WT browser smoke | 🟡 **SPKI works; bidi interop is a follow-up** | See [§7.2.3](#723-32-wt-browser-smoke-detail) below. |
| 7.1.4 | §5.2 coordinator auto-verify gate | ✅ Landed | New `Sig9421Policy` (`crates/rvoip-uctp/src/state/signature_policy.rs`). `UctpCoordinator::start_full_with_sig9421` constructor. Verify gate inside `dispatch_inner` runs after the v=1 check and before handler dispatch. `UctpEnvelope.signature: Option<EnvelopeSignature>` field added with `#[serde(default)]` (no wire-compat break). Test: `crates/rvoip-uctp/tests/sig9421_gate.rs` (4 cases). |
| 7.1.5 | §4.1 outbound trickle pump | ✅ Landed | `spawn_trickle_ice_pump` in `crates/rvoip-websocket/src/server.rs` drains `WebRtcMediaBridge::next_local_ice_candidate` and emits `connection.ice-candidate` envelopes on the route's `out_tx`. Wired into `spawn_bridge_setup`. Test: `crates/rvoip-websocket/tests/trickle_ice.rs::outbound_trickle_pump_forwards_candidates_as_envelopes`. |

#### 7.2.1 §4.2 driver side — detail

Three sub-pieces from a third 2026-05-25 session:

- **§4.2A — `Pending::deliver` gate + Route plumbing.** Inserted in `UctpCoordinator::dispatch_inner` after the v=1 check and before the signature gate (`crates/rvoip-uctp/src/state/coordinator.rs`). When an inbound envelope's `in_reply_to` matches a registered waiter the gate delivers and short-circuits; otherwise falls through to the regular handler. `Arc<rvoip_uctp::substrate::Pending>` threaded into every `Route` (QUIC/WT/WS) by capturing `_coord.pending()` at server-session-spawn time. Test: `crates/rvoip-uctp/tests/pending_dispatch.rs` (2 cases — matched delivery + unmatched fallthrough).
- **§4.2B — QUIC/WT/WS `renegotiate_media` awaits the reply.** New helpers in `crates/rvoip-uctp/src/adapter_helpers.rs` (`renegotiate_via_envelope`, `DEFAULT_RENEGOTIATE_TIMEOUT=5s`) and `crates/rvoip-uctp/src/substrate/correlation.rs::send_and_wait` (register-before-send + bounded wait). All three UCTP-family adapters call the helper and return real `NegotiatedCodecs` from the peer's chosen codec; `error 488` maps to `RvoipError::AdmissionRejected`. Tests: `crates/rvoip-uctp/tests/renegotiate.rs` extended with 3 driver-flavored cases (chosen-codec success, 488 rejection, timeout).
- **§4.2C — SIP `renegotiate_media` re-INVITE driver.** `SipAdapter::renegotiate_media` (`crates/rvoip-sip/src/adapter.rs`) now fires `UnifiedCoordinator::reinvite(&session_id).send().await`. The SIP state machine's `NegotiateSDPAsUAC` action processes the 200 OK answer asynchronously and updates `session.negotiated_config`. Returns optimistic `NegotiatedCodecs` from the requested top preference; the orchestrator's `Orchestrator::renegotiate_media` reads the post-renegotiation codec via `adapter.streams(...)` for the bridge hot-swap. Tests: `crates/rvoip-sip/tests/adapter_renegotiate.rs` (2 cases — empty caps → UnsupportedCodec, unknown conn → ConnectionNotFound). **Caveat carried forward**: the re-INVITE uses the SIP layer's configured `offered_codecs` rather than the orchestrator-supplied list. Per-session codec override (`UnifiedCoordinator::set_offered_codecs_for_session`) is a follow-up.

#### 7.2.3 §3.2 WT browser smoke — detail

SPKI pinning + WT session readiness work reliably under headless Chromium across `web-transport-quinn` 0.5, 0.6, 0.7, and 0.11. The orchestrator_bridge writes the cert's base64-encoded SHA-256(SubjectPublicKeyInfo) to `/tmp/uctp_demo_cert.spki`; the Playwright `chromium-wt` project pins via `--ignore-certificate-errors-spki-list`. Opt-in via `RVOIP_WT_SMOKE=1`.

What does **not** work today: the bidi-stream envelope round-trip. Chromium opens a client-initiated bidirectional stream via `WebTransport.createBidirectionalStream()` and writes a length-prefixed JSON envelope, but the server's `web_transport_quinn::Session::accept_bi()` either never returns (0.5–0.7) or returns a stream that doesn't carry the browser's bytes (0.11). The same wire path with the Rust client (`uctp_agent_wt`) works end-to-end against all four server versions tested, so the gap is specifically Chromium↔`web_transport_quinn` on bidi streams.

Versions tried in this session:
- 0.5.1 (original pin) → `accept_bi` hangs indefinitely.
- 0.6.0 → same hang.
- 0.7.0 → same hang (minor API shim — `Url` is now by-value in `Session::connect`).
- 0.11.9 (latest) → `accept_bi` returns silently but the envelope reader gets `io error: connection lost` 10s later when the test exits, no bytes read.

Workspace pin is now `web-transport-quinn = "0.11"` (kept the upgrade — strict no-regression on Rust↔Rust tests). The smoke spec asserts SPKI/readiness (the real §3.2 deliverable) and logs the bidi gap rather than failing.

**Carry-forward to v1.x**:

- §3.2 WT bidi-stream interop: track upstream `web_transport_quinn` 0.12+ for Chromium fixes, or investigate Chromium's actual stream framing (one hypothesis: the browser's WT bidi opens use a HTTP/3 frame type that `web_transport_quinn` doesn't yet associate with `accept_bi`). May also need to be reshaped at the wire-spec level — switch the auth handshake to unidirectional-stream-per-direction + datagrams, which historically have better browser interop.
- §4.2 SIP per-session codec override: add `UnifiedCoordinator::set_offered_codecs_for_session(session, Vec<u8>)` so the orchestrator can pass codec preferences through the SIP layer's SDP generator. Mechanical wrapper around the existing `MediaAdapter::set_offered_codecs`.

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
cargo test -p rvoip-websocket --features wss          # new in §2.3
cd tests/browser-smoke && npm test                    # new in §3.2 (Playwright)
```

As of the 2026-05-25 batch landing all gates pass with 0 failures: 89 tests (UCTP family), 9 (WS + media-webrtc), 5 (WS + wss), full SIP suite, 1 Playwright smoke. The gaps that remain are tracked in [§7.1 v1 punch list](#71-v1-punch-list).
