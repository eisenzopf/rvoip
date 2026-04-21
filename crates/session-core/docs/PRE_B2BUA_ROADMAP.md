# Pre-b2bua Roadmap

Strategic plan for what to finish in `session-core` (and adjacent crates)
before starting the `b2bua` wrapper crate, plus a parallel carrier-interop
track. Synthesised from:

- `docs/HARDENING_BEFORE_B2BUA.md` — all three Tiers complete (T1/T2/T3).
- `docs/RFC_COMPLIANCE_STATUS.md` — current RFC + carrier-interop matrix.
- `docs/UPDATE_STATUS.md` — outbound UPDATE intentionally unused from v3 API.
- `docs/TELCO_USE_CASE_ANALYSIS.md` — 10 real-world use cases vs API shapes.

## Status

- ✅ Single-session control plane hardened (leak-free, unsafe-free,
  shutdown-clean, API-symmetric, RFC 4028 §10 compliant).
- ✅ UAS-side re-INVITE/UPDATE wired; session-timer refresh-failure
  observable end-to-end.
- ✅ Audio roundtrip regression test locks in the full RTP+PCMU media
  path.
- ✅ Per-call event streams and media-core bridge primitive landed —
  both b2bua blockers cleared.
- ✅ RFC-polish items landed — `INFO` helper, early-media `AudioSource`
  wiring (with automatic switchback to `PassThrough` on
  `EarlyMedia → Active`), and 422 Session Interval Too Small with both
  observability and auto-retry (two-retry cap).
- ✅ **Post-roadmap hardening pass landed** — RFC 3261 §14.1 hold/resume
  correctness fix (see *Hardening pass — post-roadmap* below); all
  ERROR/WARN noise in the example suite resolved.
- 🟢 **b2bua crate (`crates/b2bua`) is unblocked** — `UnifiedCoordinator`
  now exposes every primitive a b2bua wrapper needs, including the
  final b2bua prerequisite: RFC 3515 §2.4.5 progress-NOTIFY wiring
  (both directions) + public `send_notify` and
  `make_transfer_leg` / `set_transferor_session`. See the *NOTIFY
  support (RFC 6665 + RFC 3515 §2.4.5)* section below.
- ⬜ Carrier-facing transport (TLS/TCP, Contact rewrite, SRV) is a
  separate multi-day track, still unchanged.

---

## Hardening pass — post-roadmap

The five-item roadmap cleared the b2bua *blockers*. A subsequent
hardening pass — driven by auditing ERROR/WARN output of the full
example suite — landed in a single session after the roadmap closed,
and is the load-bearing RFC compliance work for anything that exercises
mid-call re-INVITEs (hold, resume, session-timer refresh, SDP
renegotiation).

**Fixed — RFC 3261 §14.1 hold/resume correctness (the big one):**
- New `CallState::HoldPending` intermediate state
  (`src/types.rs`, `src/session_store/store.rs`, `src/state_table/types.rs`,
  `src/state_table/yaml_loader.rs`).
- State commit + media side-effects + `CallOnHold`/`CallResumed` publish
  are now gated on `Dialog200OK` for both hold and resume
  (`state_tables/default.yaml`).
- Failure rollback transitions added: `HoldPending → Active` and
  `Resuming → OnHold` on `Dialog4xxFailure` / `Dialog5xxFailure` /
  `Dialog6xxFailure` / `DialogTimeout` (session parameters unchanged
  per RFC 3261 §14.1).
- `Action::SendReINVITE` (`src/state_machine/actions.rs:435-`) now
  picks SDP direction from the committed target state and persists
  `session.pending_reinvite` *before* the wire-send await so concurrent
  `ReinviteGlare` handlers can read it.
- New `Action::ClearPendingReinvite` resolves simultaneous-hold glare
  by accepting the peer's re-INVITE (`HoldPending + ReinviteReceived
  → OnHold`) and cancelling our scheduled retry — breaking what would
  otherwise be a 491-loop-forever deadlock under the new RFC-strict
  state gating.
- `Action::ScheduleReinviteRetry` role-split backoff per RFC 3261 §14.1:
  UAC (Call-ID owner) 2.1–4.0 s, UAS (non-owner) 0–2.0 s. Ensures the
  non-owner retries first, breaking glare deterministically.
- Executor race fix at `src/state_machine/executor.rs:440-456` —
  Task A (hold caller) no longer clobbers Task B's concurrent
  `Dialog200OK → OnHold` commit on its post-action save.
- `handle_call_failed` (`src/adapters/session_event_handler.rs`) now
  checks `call_state == HoldPending | Resuming` before publishing
  terminal `CallFailed` + releasing the session. A non-2xx on a
  mid-call re-INVITE is not terminal for the call.

**Fixed — dialog-core RFC 3261 §17.1.1.3 false termination:**
- `crates/dialog-core/src/manager/transaction_integration.rs:491-` —
  `send_request_in_dialog` now suppresses "Transaction terminated after
  timeout" for `Method::Invite`, mirroring the pre-existing 422-retry
  and `unified.rs::make_call` suppression. Root cause: an INVITE client
  transaction auto-terminates on 2xx+ACK, and on fast loopback the
  termination races inside `send_request().await` before it returns,
  causing the generic dialog path to surface the termination as a fatal
  "transport error". Same fix applied to the auth-retry INVITE path.

**Fixed — log-noise audit:**
- `SessionHandle::hangup` (`src/api/handle.rs`) — false-positive
  "background hangup failed" WARN demoted to `trace!` when the session
  is already gone (new `SessionError::is_session_gone()` helper in
  `src/errors.rs`). `SimplePeer::hangup` given the same treatment.
- `handle_session_refresh_failed`
  (`src/adapters/session_event_handler.rs`) — leftover
  `warn!("🎯 [handle_session_refresh_failed]")` dev-print demoted to
  `debug!` with the emoji removed.
- `state_machine/helpers.rs::hangup` now early-returns
  `SessionNotFound` without dispatching to the state machine when the
  session is already gone, suppressing the upstream executor
  `ERROR: Failed to get session …` log.

**Tracking doc:** `docs/EXAMPLE_RUN_ERRORS_TRACKING.md` — per-cluster
status (A/B/C/D) and verification. All four clusters **FIXED**.

**Regression coverage:** all 27 `cargo test -p rvoip-session-core`
binaries pass; all 19 `run_all.sh` examples pass with zero ERROR/WARN
lines across every log.

**Side-effect:** cleared the `glare_retry` example's previously-expected
"ERROR lines during glare" — Cluster D is now silent as a bonus.

---

## Which API surface for which consumer

`session-core` exposes three public API surfaces today. They're not
interchangeable — each targets a different use case. Knowing which
surface a given downstream (b2bua, REST gateway, softphone, CI test)
should consume tells us where to land each new primitive.

| Consumer | API surface | Why |
|----------|-------------|-----|
| **b2bua wrapper crate** (`crates/b2bua`) | `UnifiedCoordinator` | Multi-session per process. Needs direct event access, `bridge()`, `redirect_call`, `accept_call_with_sdp` — all already on `UnifiedCoordinator`. Trait methods / sequential `wait_*` add no value for leg coordination. |
| **Internet APIs** (REST / gRPC / WebSocket gateways; phone-as-a-service; any "server" in the TELCO matrix) | `UnifiedCoordinator` | Same shape as b2bua — one process, N sessions, events fanned out to external consumers. HTTP handlers translate `POST /calls` → `UnifiedCoordinator::make_call`; WS handlers translate `events_for_session` → WS frames. |
| **Softphones, agents, voicemail, E911** | `CallbackPeer` | Single endpoint, structured event-handler methods map cleanly to UI / audit logic. Matches `TELCO_USE_CASE_ANALYSIS.md` recommendation. |
| **Scripted / test / sequential flows** (CI harnesses, mock peers, demo scripts) | `StreamPeer` | `wait_for_answered` / `wait_for_incoming` / `wait_for_ended` are exactly what linear scripts need. Already used by every integration test in this crate. |
| **IVR, call recording** | Scale-dependent. Single-stream IVR → `CallbackPeer`. Many concurrent calls or per-call DTMF collection → `UnifiedCoordinator`. |

### Wrapper discipline

- `UnifiedCoordinator` is the **core API**. Every new primitive (per-call
  event filtering, media bridging, session pairing, outbound `INFO`,
  422 retry) lands here first.
- `CallbackPeer` and `StreamPeer` are **thin ergonomic shells** over
  `UnifiedCoordinator`. They must not own unique state or branching
  logic — if a capability exists anywhere, it exists on
  `UnifiedCoordinator`, and the peer types optionally adapt it for
  their ergonomics.
- Practical consequence: if you're adding a feature and find yourself
  duplicating state or branching in a peer type, stop and move the
  primitive down to `UnifiedCoordinator`.

### What this means for the rest of this doc

All five session-core items below target `UnifiedCoordinator` as the
primary API. The peer types may later grow thin shims exposing the same
methods; that's per-item and not on the critical path for b2bua.

---

## Recommended sequencing

| # | Item | Est. | Blocks b2bua? | Blocks clients? | Status |
|---|------|------|---------------|-----------------|--------|
| 1 | Event-stream API with per-call filtering | 1–2 d | **Yes** | Makes IVR / multi-call better | ✅ Landed — `UnifiedCoordinator::events`, `events_for_session` |
| 2 | Media-core bridge primitive (RTP relay) | 1–2 d | **Yes** | No | ✅ Landed — `UnifiedCoordinator::bridge` (+ dead `MediaRelay` / `PacketForwarder` cleanup) |
| 3 | Early-media `AudioSource` wiring | ~½ d | No | IVR / voicemail ringback | ✅ Landed — `IncomingCall::send_early_media_with_source`, `UnifiedCoordinator::set_audio_source` |
| 4 | Outbound `INFO` helper | ~½ d | No | Fax / DTMF interop | ✅ Landed — `SessionHandle::send_info(content_type, body)` |
| 5 | UAC-side 422 Session Interval Too Small retry | ~½ d | No | RFC 4028 completeness | ✅ Landed — observability + auto-retry with 2-retry cap (`tests/session_422_retry.rs`) |
| 6 | **Start b2bua crate** on top of (1)+(2) | — | — | — | 🟢 Unblocked — ready to start `crates/b2bua` |
| P | Carrier track in parallel: TLS → Contact rewrite → RFC 3263 → SIP Outbound → STUN | weeks | No | **Yes, for cloud carriers** | ⬜ Unchanged |

Items 1–5 landed in `session-core`, `media-core`, `dialog-core`, and
`infra-common`. The b2bua crate (6) is now the next repo-level
milestone; its 3-peer shape is already exercised by
`tests/bridge_roundtrip_integration.rs` with
`examples/streampeer/bridge/bridge_peer.rs` as the skeleton. Carrier
track (P) runs alongside and doesn't block b2bua — LAN / Asterisk /
FreeSWITCH setups work today.

---

## 1. Event-stream API with per-call filtering

### Why

`TELCO_USE_CASE_ANALYSIS.md` identifies **per-call event isolation** as
the killer feature for B2BUA, IVR, contact-center supervisors, and call
recording. Today we have:

- `CallbackPeer` — trait-based, one method per event type, no per-call
  filtering. Great for softphones and single-call agents.
- `StreamPeer` — sequential `wait_for_answered` / `wait_for_incoming`,
  plus a coarse `subscribe_events()` that fires every event for every
  session. Good for scripted flows, poor for reactive per-call logic.

Neither cleanly supports:

```rust
// B2BUA: monitor both legs and hang up the peer when either ends.
let inbound_events  = peer.events_for_session(&inbound_id);
let outbound_events = peer.events_for_session(&outbound_id);
tokio::select! {
    Some(CallEvent::Ended { .. }) = inbound_events.next() => hangup(outbound),
    Some(CallEvent::Ended { .. }) = outbound_events.next() => hangup(inbound),
}

// IVR: collect DTMF from one call until '#'.
let digits = peer.dtmf_stream()
    .filter(|(id, _)| async move { *id == call_id })
    .map(|(_, d)| d)
    .take_while(|d| async move { *d != '#' })
    .collect::<Vec<_>>().await;
```

### Approach

Land primitives on `UnifiedCoordinator` — b2bua and internet-API
consumers use them directly; `CallbackPeer` / `StreamPeer` may later
grow thin shims if client use cases want them.

Backing: the existing per-session event broadcaster used inside
`SessionHandle`. New API shape:

```rust
impl UnifiedCoordinator {
    pub fn events_for_session(&self, id: &SessionId)
        -> impl Stream<Item = Event> + Send;
    pub fn dtmf_stream(&self)
        -> impl Stream<Item = (SessionId, char)> + Send;
    pub fn incoming_calls(&self)
        -> impl Stream<Item = IncomingCall> + Send;
    pub fn transfers(&self)
        -> impl Stream<Item = ReferRequest> + Send;
}
```

### Critical files

- `crates/session-core/src/api/unified.rs` — add the four methods.
- `crates/session-core/src/api/handle.rs` — the per-session
  broadcaster already exists for `SessionHandle::subscribe_events()`;
  reuse it so `events_for_session` is a thin pass-through.
- `crates/session-core/src/adapters/event_router.rs` — global event
  fan-out lives here; add a "filter by session ID or event kind" tap
  for the global streams (incoming_calls, dtmf_stream, transfers).

### Verification

- Unit tests for each stream method.
- Integration test that spawns a mock 2-leg call and asserts each leg's
  `events_for_session` sees only its own events.
- Extend `TELCO_USE_CASE_ANALYSIS.md` with a worked B2BUA sketch using
  the new API.

### Landed

- `UnifiedCoordinator::events() -> EventReceiver` (unfiltered) and
  `UnifiedCoordinator::events_for_session(&SessionId) -> EventReceiver`
  (pre-filtered by `call_id`), both in
  `crates/session-core/src/api/unified.rs`.
- DTMF / incoming-call / transfer streams are accessible via existing
  `EventReceiver::next_dtmf` / `next_incoming` / `next_transfer`
  helpers in `crates/session-core/src/api/stream_peer.rs`. The
  roadmap's four-method sketch collapsed to two methods on
  `UnifiedCoordinator` + reuse of the existing filter helpers — no
  new stream types were needed.
- `EventReceiver` is publicly re-exported from the crate root so any
  peer type (or b2bua) can consume it directly.
- Tests: `crates/session-core/tests/event_stream_filtering_tests.rs`
  — per-session isolation, unfiltered-sees-all, DTMF helper end-to-end.
- Used in production-shape by
  `examples/streampeer/bridge/bridge_peer.rs`, which uses
  `events_for_session` to observe the outbound leg's `CallAnswered`
  before accepting the inbound leg.

---

## 2. Media-core bridge primitive

### Why

A b2bua forwarding calls between networks shouldn't have to
`receiver.recv() → sender.send()` decoded `AudioFrame`s in app-space —
that burns CPU and adds a jitter hop. Media-core already owns both
legs' RTP sockets; it should expose a way to say "pipe inbound RTP of
session A directly to outbound RTP of session B" without app-level
sample handling (ideally without even decoding).

### Approach

In `media-core`: add a `bridge_sessions(id_a, id_b)` helper that wires
the RTP receive socket of one session into the RTP send socket of the
other (and vice versa). Two modes:

- **Transparent relay**: packet-level forwarding, zero transcoding.
  Works when both legs negotiated the same codec.
- **Transcoded bridge** (future): decode → optional resample/mix →
  re-encode. Needed when codecs differ or when a b2bua injects audio.

Start with transparent relay — it's sufficient for the 80% case and
doesn't block the b2bua crate.

Expose a thin session-core pass-through:
`UnifiedCoordinator::bridge(&session_a, &session_b) -> Result<BridgeHandle>`
where `BridgeHandle` teardown unwires the relay.

### Critical files

- `crates/media-core/src/` — identify the RTP I/O seam (already used by
  `AudioStream` for `AudioFrame` delivery).
- `crates/session-core/src/api/unified.rs` — add `bridge(...)`.

### Verification

- Extend the audio-roundtrip test pattern to a 3-peer topology: Alice
  calls B2BUA-peer, B2BUA-peer calls Carol, bridge the two legs. Assert
  Alice's tone shows up at Carol's WAV and vice versa.

### Landed

- `UnifiedCoordinator::bridge(&SessionId, &SessionId) -> Result<BridgeHandle, BridgeError>`
  in `crates/session-core/src/api/unified.rs`.
- Underlying primitive: `MediaSessionController::bridge_sessions` at
  `crates/media-core/src/relay/controller/bridge.rs` — transparent
  packet-level relay (no transcoding), `DashMap<DialogId, DialogId>`
  partner-map tracking, `BridgeHandle::drop()` flips an atomic cancel
  gate synchronously and aborts forwarder tasks asynchronously.
- Preconditions enforced at call time:
  - Both sessions must have a remote RTP address →
    `BridgeError::SessionNotActive`.
  - Negotiated payload types must match →
    `BridgeError::CodecMismatch { a_pt, b_pt }`.
  - Neither session may already be bridged →
    `BridgeError::AlreadyBridged`.
- DTMF (RFC 2833) rides the relay transparently. RTCP is not bridged —
  each leg keeps generating its own reports (RFC 3550 §7.2).
- **Dead-code cleanup bundled**: deleted
  `crates/media-core/src/relay/packet_forwarder.rs`,
  `crates/media-core/src/relay/controller/relay.rs`, and
  `crates/media-core/src/integration/session_bridge.rs` (all were
  unfinished skeletons not wired into session-core). Trimmed
  `MediaRelay`, `RelaySessionConfig`, `RelayEvent`, `RelayStats`,
  `create_relay_config`, `generate_session_id`, and the
  `relay: Option<Arc<MediaRelay>>` field on `MediaSessionController`.
  G.711 passthrough codecs preserved at their live locations.
- Tests: 6 unit tests in `bridge.rs` (preconditions, handle lifecycle,
  partner-map bookkeeping, `stop_media` cleanup) — all in-process,
  millisecond-fast. End-to-end 3-peer SIP test at
  `crates/session-core/tests/bridge_roundtrip_integration.rs` with
  new examples under `examples/streampeer/bridge/` (`alice.rs`,
  `carol.rs`, `bridge_peer.rs`, `run.sh`). Goertzel-asserts tones cross
  the relay in both directions; full run ≈42 s.

---

## 3. Early-media `AudioSource` wiring

### Why

183 Session Progress signalling, PRACK, SDP handoff — all of that is in
place. What's missing is actually *playing* a ringback tone / "please
hold" announcement during the `EarlyMedia` state. Documented explicitly
as not-yet-scope in `RFC_COMPLIANCE_STATUS.md` §Partial/aesthetic #2.

### Approach

Wire an `AudioSource` (file, generator, or live stream) into the media
session during `EarlyMedia`, and stop it automatically on transition
to `Active`. Public API:

```rust
impl IncomingCall {
    pub async fn send_early_media_with_source(
        &self, sdp: String, source: Box<dyn AudioSource>) -> Result<()>;
}
```

### Critical files

- `crates/session-core/src/api/incoming.rs` — extend
  `send_early_media` variant.
- `crates/media-core/` — `AudioSource` trait + file-playback impl.
- `crates/session-core/state_tables/default.yaml` — make sure the
  `Active` transition from `EarlyMedia` stops the source.

### Verification

- IVR example that plays a WAV during early media and asserts via the
  existing `audio_roundtrip_integration` pattern that Alice hears the
  tone *before* 200 OK.

### Landed (API level)

- `IncomingCall::send_early_media_with_source(sdp, source)` in
  `crates/session-core/src/api/incoming.rs` — wraps
  `send_early_media` + `set_audio_source`.
- `UnifiedCoordinator::set_audio_source(session_id, source)` delegates
  through `MediaAdapter::set_audio_source` to
  `MediaSessionController::set_audio_source` (new — wraps the existing
  `AudioTransmitter::set_audio_source`).
- `AudioSource` re-exported from the crate root. **Followed the
  existing enum rather than trait-ifying it**: the enum already covers
  Tone / CustomSamples / PassThrough; promoting to a trait would be a
  bigger refactor and isn't needed to unblock b2bua.
- **Follow-up deferred**: auto-switchback to `PassThrough` on the
  `EarlyMedia → Active` transition. Today the app must explicitly call
  `set_audio_source(PassThrough)` after `accept_call` if it wants
  bidirectional audio to replace the tone. Automating requires either
  a new state-table action (`SwitchToPassThroughOnActive`) or changing
  `start_audio_transmission_with_config` so it replaces an existing
  transmitter's source instead of no-oping on re-entry.
  See *Follow-ups carved off* below.

---

## 4. Outbound `INFO` helper

### Why

`RFC_COMPLIANCE_STATUS.md` row: `INFO | ⚠️ | ⚠️ | dialog-core has the
plumbing; no session-core helper yet`. Used for SIP-INFO DTMF (some
carriers prefer this over in-band RFC 2833) and some fax flows.

### Approach

Public method on `SessionHandle`:

```rust
pub async fn send_info(
    &self, content_type: &str, body: &[u8],
) -> Result<()>;
```

Wrap the existing `DialogManager::send_request(Method::Info, ...)`. Tiny.

### Critical files

- `crates/session-core/src/api/handle.rs` — add method.
- `crates/session-core/src/adapters/dialog_adapter.rs` — plumb body +
  content-type.

### Verification

- Unit test that asserts the request built carries the correct
  `Content-Type` header + body. Optional: wire into the DTMF example.

### Landed

- `SessionHandle::send_info(content_type: &str, body: &[u8])` in
  `crates/session-core/src/api/handle.rs`.
- `UnifiedCoordinator::send_info(session_id, content_type, body)` in
  `crates/session-core/src/api/unified.rs`.
- `DialogAdapter::send_info` plumbs the content-type all the way down
  through a new dialog-core entry point
  `DialogManager::send_info_with_content_type(dialog_id, content_type, body)`
  (mirrors the `send_bye_with_reason` pattern). The generic
  `send_request_in_dialog` path always stamped INFO bodies as
  `application/info`; the new path lets callers pick
  `application/dtmf-relay` (SIP-INFO DTMF), `application/sipfrag` (fax
  flow control), `application/media_control+xml` (video FIR/PLI), etc.
- Verification today is type-level and by downstream build-through;
  wire-level tests land with the first real DTMF/fax interop consumer.

---

## 5. UAC-side 422 retry (RFC 4028 §6)

### Why

Today UAS emits 422 Session Interval Too Small + `Min-SE` correctly,
but the UAC doesn't auto-retry with a bumped `Session-Expires`. Rare in
practice, but the matching branch of the RFC 4028 story is missing.

### Approach

Mirror the existing 423 REGISTER-retry pattern: on 422 to INVITE, read
`Min-SE` from the response, bump our local Session-Expires, re-issue.
Two-retry cap matching the 423 path.

### Critical files

- `crates/session-core/state_tables/default.yaml` — new transition
  for `Dialog422Response` with `Min-SE` capture.
- `crates/session-core/src/state_machine/actions.rs` —
  `SendINVITEWithBumpedSessionExpires` action (mirrors the 423 pattern).

### Verification

- `tests/session_422_retry.rs` — in-process raw-UDP mock UAS returns
  422 + Min-SE, asserts retry carries the bumped value.

### Landed (observability only)

What shipped:

- New cross-crate event
  `DialogToSessionEvent::SessionIntervalTooSmall { session_id, min_se_secs }`
  in `crates/infra-common/src/events/cross_crate.rs`.
- dialog-core emits it from the 422 arm of the UAC response translator
  at `crates/dialog-core/src/events/event_hub.rs`; parses `Min-SE:`
  from the response. Falls through to generic `CallFailed` when the
  header is missing or unparseable.
- session-core dispatches to `handle_session_interval_too_small` in
  `src/adapters/session_event_handler.rs` (checked **before**
  `CallFailed` to avoid substring collisions). The handler logs Min-SE
  at WARN, drives the existing `Dialog4xxFailure(422)` transition, and
  publishes
  `Event::CallFailed { status_code: 422, reason: "Session Interval Too Small (required Min-SE: Xs)" }`
  so apps can read the required floor out of the reason string.

What's deferred (see *Follow-ups carved off* below):

- Auto-retry with a bumped `Session-Expires`, two-retry cap, mirroring
  the 423 REGISTER pattern at `dialog_adapter.rs:722-783`.
- Blocker: dialog-core's `inject_session_timer_headers` reads
  Session-Expires / Min-SE from the global `DialogManagerConfig`, not
  per-session. A per-session override requires a new dialog-core entry
  point `DialogManager::send_invite_with_session_timer_override(dialog_id, sdp, secs, min_se)`
  parallel to `send_bye_with_reason`.
- Estimated 4–6 focused hours including an integration test modeled
  on `crates/session-core/tests/register_423_retry.rs`.

---

## 6. Start the b2bua crate

Both blockers (Items 1 + 2) have landed — b2bua is unblocked. A working
skeleton already exists at
`crates/session-core/examples/streampeer/bridge/bridge_peer.rs`
(~100 LOC). The production shape, lifted into `crates/b2bua`, looks
like:

```rust
pub struct B2bua { inner: Arc<UnifiedCoordinator>, links: DashMap<SessionId, SessionId> }

impl B2bua {
    pub async fn bridge_incoming(
        &self, inbound_id: SessionId, outbound_uri: &str,
    ) -> Result<BridgedCall> {
        let outbound_id = self.inner.make_call(self.local_uri(), outbound_uri).await?;

        // Item 1: watch the outbound leg until CallAnswered.
        let mut outbound_events = self.inner.events_for_session(&outbound_id).await?;
        loop {
            match outbound_events.next().await {
                Some(Event::CallAnswered { .. }) => break,
                Some(Event::CallEnded { .. }) | Some(Event::CallFailed { .. }) => {
                    return Err("outbound leg terminated before answering".into());
                }
                Some(_) => continue,
                None => return Err("event stream closed".into()),
            }
        }
        self.inner.accept_call(&inbound_id).await?;

        // Item 2: transparent RTP relay between the two legs.
        let handle = self.inner.bridge(&inbound_id, &outbound_id).await?;

        self.links.insert(inbound_id.clone(), outbound_id.clone());
        self.links.insert(outbound_id.clone(), inbound_id.clone());

        // Tear the partner down when either leg ends — uses per-call
        // event streams from Item 1.
        self.watch_pair(inbound_id, outbound_id, handle);
        Ok(BridgedCall { /* ... */ })
    }
}
```

The `bridge_peer` example exercises this exact sequence end-to-end in
`tests/bridge_roundtrip_integration.rs`, so lifting it into the b2bua
crate is mechanical. This is a separate crate (`crates/b2bua`) — it
doesn't modify session-core.

---

## Parallel track — Carrier / cloud interop

Separate workstream, doesn't block b2bua. LAN / IP-based / Asterisk /
FreeSWITCH work today. Production cloud carriers (Twilio, Vonage,
Bandwidth, BYOC providers) need these in order:

| Step | What | Effort | Unblocks |
|------|------|--------|----------|
| P1 | **TLS transport** — finish `sip-transport`'s rustls client-side connector, add `Config::tls_cert_path` / `tls_key_path`, flip hardcoded `enable_tls: false` at `api/unified.rs:585` | 2–3 d | Twilio / Vonage / Bandwidth production; `sips:` URIs |
| P2 | **Contact header rewrite** from discovered `received=` / `rport=` | 1 d | Long-duration registrations behind NAT |
| P3 | **RFC 3263 SRV + NAPTR** resolution (add `hickory-resolver`; handle `_sip._udp` SRV priority/weight) | 2–3 d | Carrier geo-failover; auto-select UDP/TCP/TLS per NAPTR |
| P4 | **TCP transport** — same wiring pattern as TLS; flip `enable_tcp: false` | 1–2 d | Large SDP / video / PBX fallback |
| P5 | **SIP Outbound (RFC 5626)** — flow-id + CRLF keepalive | 1–2 d | Registration keep-alive behind NAT on TLS/TCP |
| P6 | **STUN (RFC 5389)** + `public_address` config | 3–5 d | UAC behind strict NAT reaching public carriers |
| P7 | **Digest `nc` counter tracking, `auth-int`, `-sess` variants** | 1–2 d | Strict carrier auth servers |

P1 is the highest-leverage carrier work — without it, session-core
cannot talk to any of the major cloud SIP providers. Once P1+P2 land,
realistic production deployments become possible for a broad class of
apps.

---

## What we're intentionally *not* doing (yet)

- **Attended-transfer orchestration**: primitives exposed
  (`SessionHandle::transfer_attended`, `dialog_identity`), multi-session
  linkage belongs in b2bua or app code — not this crate.
- **305 / 380 proxy semantics**: treated as generic 3xx. Fix when a real
  scenario demands it.
- **PUBLISH presence flows**: dialog-core plumbing exists; no app
  scenario forcing us to exercise it.
- **Outbound OPTIONS helper**: incoming works; outbound useful mainly
  for keep-alive which will be better-served by SIP Outbound (P5).

---

## Open questions / decisions (resolved)

1. **Event-stream API (Item 1) — extend `StreamPeer` or introduce
   `EventStreamPeer`?** — **Resolved: did neither.** Landed on
   `UnifiedCoordinator` only, per the *Wrapper discipline* principle
   at the top of this doc. `EventReceiver` is publicly re-exported
   from the crate root, so any peer type can wrap it with thin shims
   later if a specific consumer asks for them.

2. **Media bridge mode (Item 2) — transparent RTP relay vs transcoded
   bridge?** — **Resolved: transparent relay only.** Codec-mismatch
   returns `BridgeError::CodecMismatch { a_pt, b_pt }` rather than
   silently transcoding. Transcoded bridge remains a future upgrade
   for when codec-mismatch use cases actually arrive.

3. **`UnifiedCoordinator::bridge` return type.** — **Resolved:
   `BridgeHandle` with RAII `Drop`.** `Drop` synchronously flips an
   atomic cancel gate (so partner-map entries disappear immediately)
   and spawns an async cleanup that aborts the forwarder tasks.

4. **Bundle Item 5 (422 retry) with Item 4 (INFO helper)?** —
   **Eventually yes.** Both landed in the same session; observability for
   422 shipped first, then the auto-retry half was completed as a
   separate hardening pass that added a per-session session-timer
   override in dialog-core.

---

## Follow-ups — all landed

Both deferred follow-ups have shipped:

- **Item 3 follow-up: auto-switchback to `PassThrough` on
  EarlyMedia → Active.** ✅ Landed. New state-machine action
  `SwitchToPassThroughOnActive` wired into the three transitions that
  lead into `Active` (UAS `Answering → DialogACK → Active`, UAC
  `Initiating → Dialog200OK → Active`, UAC `Ringing → Dialog200OK →
  Active`). Idempotent for calls that never set a source — the
  transmitter is already in `PassThrough`. Swallows the "transmitter
  not active" error so pre-negotiated-SDP flows (e.g.
  `accept_call_with_sdp`) are unaffected. State-table wiring verified
  by `tests/early_media_tests.rs::dialog_ack_auto_switches_transmitter_to_passthrough`
  and its UAC counterpart.

- **Item 5 follow-up: auto-retry on 422 with bumped `Session-Expires`.**
  ✅ Landed. New dialog-core entry point
  `UnifiedDialogApi::send_invite_with_session_timer_override(dialog_id, sdp, session_secs, min_se)`
  bypasses `DialogManagerConfig`'s global timer values and injects the
  per-call overrides (mirrors `send_bye_with_reason`). Session-core-v3
  routes 422 Min-SE through a new `SessionIntervalTooSmall` state
  event and a `SendINVITEWithBumpedSessionExpires` action with a
  2-retry cap. Malformed 422s (no Min-SE, or unparseable) fall through
  to the existing terminal `CallFailed(422, "… Min-SE: Xs")` path —
  backwards-compatible with apps that observe the reason string.
  Integration test: `tests/session_422_retry.rs` covers both the
  success-after-retry path and the 2-retry cap exhaustion.

---

## NOTIFY support (RFC 6665 + RFC 3515 §2.4.5) — landed

Closes the last b2bua prerequisite flagged in the Status section.
Covered by `docs/NOTIFY_SUPPORT_IMPLEMENTATION_PLAN.md` (now "landed"
status).

**Public API on `UnifiedCoordinator` / `SessionHandle`:**
- `SessionHandle::send_notify(event_package, body, subscription_state)`
  + `UnifiedCoordinator::send_notify(session_id, …)` — generic outbound
  NOTIFY on any event package. Bypasses the state machine; delegates
  to `DialogAdapter::send_notify` and on through
  `UnifiedDialogApi::send_notify`.
- `UnifiedCoordinator::make_transfer_leg(from, to, transferor_session_id)`
  — atomic transfer-leg creation. Pre-populates
  `SessionState.transferor_session_id` + `is_transfer_call = true`
  before the `MakeCall` event dispatches, closing the race where a
  fast loopback `Dialog180Ringing` arriving mid-dispatch would beat
  a post-creation linkage update and cause the `SendTransferNotify*`
  actions to no-op.
- `UnifiedCoordinator::set_transferor_session(leg, transferor)` — the
  lower-level primitive, for non-standard orchestration. Callers
  accept the race window.

**State-machine actions (`crates/session-core/src/state_table/types.rs`):**
- `SendRefer100Trying` — fires on `Both + Active + TransferRequested`
  alongside `SendReferAccepted`. Sends 100 Trying sipfrag NOTIFY on
  the REFER-receiver's own dialog (the implicit-subscription ack).
- `SendTransferNotifyRinging` / `SendTransferNotifySuccess` — appended
  to `UAC + Initiating + Dialog180Ringing`, `UAC + Ringing +
  Dialog200OK`, and `UAC + Initiating + Dialog200OK` (fast answer).
  Each no-ops when `session.transferor_session_id.is_none()` so
  non-transfer calls are unaffected.
- `SendTransferNotifyFailure` — action exists for parity; at runtime
  the failure-side NOTIFY actually fires from
  `session_event_handler::handle_call_failed` when the failing
  session carries `transferor_session_id`. Reason: the YAML loader's
  `Dialog4xxFailure` / `5xxFailure` names fall through to
  `MediaEvent(…)` (unmapped), so the Initiating-failure YAML
  transitions don't match the runtime-dispatched events. The adapter
  path is the reliable entry point.

**Cross-crate event (`crates/infra-common/src/events/cross_crate.rs`):**
- `DialogToSessionEvent::NotifyReceived` extended with
  `subscription_state: Option<String>` and `content_type: Option<String>`
  (on top of the pre-existing `event_package` + `body`). Dialog-core's
  `handle_notify_method` now publishes it via the existing
  `publish_cross_crate_event` path in both the SubscriptionManager and
  fallback paths. The `SessionCoordinationEvent::ReInvite` wrapper
  path (which would have filtered NOTIFY out at the conversion arm)
  is no longer used for NOTIFY routing.

**Public `Event::NotifyReceived` + sipfrag parsing:**
- `session_event_handler::handle_notify_received` always publishes
  `Event::NotifyReceived { call_id, event_package, subscription_state,
  content_type, body }`.
- For `event_package == "refer"` + `Content-Type: message/sipfrag`,
  the sipfrag status line (`SIP/2.0 NNN Reason…`) is parsed into one
  of `Event::TransferProgress` (1xx), `TransferCompleted` (2xx), or
  `TransferFailed` (3xx–6xx). Symmetric with the send-side emissions,
  so a b2bua listens uniformly regardless of direction.

**Coverage:**
- `tests/notify_send_integration.rs` — multi-binary end-to-end
  `send_notify` → `Event::NotifyReceived` round trip.
- `tests/transfer_notify_wiring_tests.rs` — 5 state-table structural
  tests verifying the YAML wiring (100-Trying ordering, progress
  actions on the right transitions, media-commit-before-NOTIFY
  ordering, non-transfer `next_state` invariance).
- `src/adapters/session_event_handler.rs::tests` — sipfrag
  status-line parser unit tests (progress / final success / malformed
  inputs).
- **Deferred to the b2bua crate**: three-peer REFER-progress-send
  fixture (Alice ↔ b2bua Bob ↔ Carol) asserting the full
  `SendTransferNotify*` → wire NOTIFY → transferor sees
  `TransferProgress` → `TransferCompleted` loop end-to-end, plus a
  sipfrag-receive mock fixture for isolated transferor-side
  verification. These need three-peer fixtures that belong with the
  b2bua crate's own CI anyway.

---

## TL;DR

The five-item pre-b2bua gate, both carved-off follow-ups, and the RFC
3515 §2.4.5 NOTIFY prerequisite are all cleared. `UnifiedCoordinator`
exposes per-call event streams, a transparent RTP bridge primitive,
early-media audio-source injection with automatic switchback to
`PassThrough` on answer, an outbound INFO helper with custom content
types, full RFC 4028 §6 422 handling (observability + auto-retry with
2-retry cap), a generic `send_notify`, and atomic
`make_transfer_leg` for RFC 3515 §2.4.5 progress reporting in both
directions. The `crates/b2bua` crate is unblocked on every known
prerequisite — a working bridge skeleton already lives at
`examples/streampeer/bridge/bridge_peer.rs` and is CI-exercised by
`tests/bridge_roundtrip_integration.rs`.

Carrier track (P1–P7) is multi-week and runs in parallel; LAN /
Asterisk / FreeSWITCH deployments work today without any of it. TLS
(P1) is the single highest-leverage carrier item — it unblocks every
major cloud provider.
