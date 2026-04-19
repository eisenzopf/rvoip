# Pre-b2bua Roadmap

Strategic plan for what to finish in `session-core-v3` (and adjacent crates)
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
- ⬜ Two gaps block b2bua cleanly: per-call event streams and a media
  bridge primitive.
- ⬜ Several small RFC items are cheap and worth closing now.
- ⬜ Carrier-facing transport (TLS/TCP, Contact rewrite, SRV) is a
  separate multi-day track.

---

## Which API surface for which consumer

`session-core-v3` exposes three public API surfaces today. They're not
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

All five session-core-v3 items below target `UnifiedCoordinator` as the
primary API. The peer types may later grow thin shims exposing the same
methods; that's per-item and not on the critical path for b2bua.

---

## Recommended sequencing

| # | Item | Est. | Blocks b2bua? | Blocks clients? |
|---|------|------|---------------|-----------------|
| 1 | Event-stream API with per-call filtering | 1–2 d | **Yes** | Makes IVR / multi-call better |
| 2 | Media-core bridge primitive (RTP relay) | 1–2 d | **Yes** | No |
| 3 | Early-media `AudioSource` wiring | ~½ d | No | IVR / voicemail ringback |
| 4 | Outbound `INFO` helper | ~½ d | No | Fax / DTMF interop |
| 5 | UAC-side 422 Session Interval Too Small retry | ~½ d | No | RFC 4028 completeness |
| 6 | **Start b2bua crate** on top of (1)+(2) | — | — | — |
| P | Carrier track in parallel: TLS → Contact rewrite → RFC 3263 → SIP Outbound → STUN | weeks | No | **Yes, for cloud carriers** |

Items 1-5 land in `session-core-v3` / `media-core`. The b2bua crate (6) is
the next repo-level milestone and consumes them. Carrier track (P) runs
alongside and doesn't block b2bua — LAN / Asterisk / FreeSWITCH setups
work today.

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

- `crates/session-core-v3/src/api/unified.rs` — add the four methods.
- `crates/session-core-v3/src/api/handle.rs` — the per-session
  broadcaster already exists for `SessionHandle::subscribe_events()`;
  reuse it so `events_for_session` is a thin pass-through.
- `crates/session-core-v3/src/adapters/event_router.rs` — global event
  fan-out lives here; add a "filter by session ID or event kind" tap
  for the global streams (incoming_calls, dtmf_stream, transfers).

### Verification

- Unit tests for each stream method.
- Integration test that spawns a mock 2-leg call and asserts each leg's
  `events_for_session` sees only its own events.
- Extend `TELCO_USE_CASE_ANALYSIS.md` with a worked B2BUA sketch using
  the new API.

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

Expose a thin session-core-v3 pass-through:
`UnifiedCoordinator::bridge(&session_a, &session_b) -> Result<BridgeHandle>`
where `BridgeHandle` teardown unwires the relay.

### Critical files

- `crates/media-core/src/` — identify the RTP I/O seam (already used by
  `AudioStream` for `AudioFrame` delivery).
- `crates/session-core-v3/src/api/unified.rs` — add `bridge(...)`.

### Verification

- Extend the audio-roundtrip test pattern to a 3-peer topology: Alice
  calls B2BUA-peer, B2BUA-peer calls Carol, bridge the two legs. Assert
  Alice's tone shows up at Carol's WAV and vice versa.

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

- `crates/session-core-v3/src/api/incoming.rs` — extend
  `send_early_media` variant.
- `crates/media-core/` — `AudioSource` trait + file-playback impl.
- `crates/session-core-v3/state_tables/default.yaml` — make sure the
  `Active` transition from `EarlyMedia` stops the source.

### Verification

- IVR example that plays a WAV during early media and asserts via the
  existing `audio_roundtrip_integration` pattern that Alice hears the
  tone *before* 200 OK.

---

## 4. Outbound `INFO` helper

### Why

`RFC_COMPLIANCE_STATUS.md` row: `INFO | ⚠️ | ⚠️ | dialog-core has the
plumbing; no session-core-v3 helper yet`. Used for SIP-INFO DTMF (some
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

- `crates/session-core-v3/src/api/handle.rs` — add method.
- `crates/session-core-v3/src/adapters/dialog_adapter.rs` — plumb body +
  content-type.

### Verification

- Unit test that asserts the request built carries the correct
  `Content-Type` header + body. Optional: wire into the DTMF example.

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

- `crates/session-core-v3/state_tables/default.yaml` — new transition
  for `Dialog422Response` with `Min-SE` capture.
- `crates/session-core-v3/src/state_machine/actions.rs` —
  `SendINVITEWithBumpedSessionExpires` action (mirrors the 423 pattern).

### Verification

- `tests/session_422_retry.rs` — in-process raw-UDP mock UAS returns
  422 + Min-SE, asserts retry carries the bumped value.

---

## 6. Start the b2bua crate

Only after (1) + (2) land. A sketch of the shape:

```rust
pub struct B2bua { inner: UnifiedCoordinator, links: DashMap<SessionId, SessionId> }

impl B2bua {
    pub async fn bridge_incoming(
        &self, inbound: IncomingCall, outbound_uri: &str,
    ) -> Result<BridgedCall> {
        let outbound = self.inner.call(outbound_uri).await?;
        let outbound_sdp = self.inner.wait_for_sdp(&outbound).await?;
        let inbound_id = inbound.accept_with_sdp(outbound_sdp).await?;
        self.inner.bridge(&inbound_id, &outbound).await?; // from Item 2
        self.links.insert(inbound_id.clone(), outbound.clone());
        self.links.insert(outbound.clone(), inbound_id.clone());
        // Use Item 1 per-session event streams to tear down partner on hangup.
        self.watch_pair(inbound_id, outbound);
        Ok(BridgedCall { /* ... */ })
    }
}
```

This is a separate crate (`crates/b2bua`) — it doesn't modify
session-core-v3.

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

P1 is the highest-leverage carrier work — without it, session-core-v3
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

## Open questions / decisions

1. **Event-stream API (Item 1) — extend `StreamPeer` or introduce
   `EventStreamPeer`?** `TELCO_USE_CASE_ANALYSIS.md` assumes two
   distinct types. Extending avoids API proliferation; a separate type
   is cleaner for users writing reactive-only code. Recommendation:
   extend `StreamPeer` with the new stream methods, since `StreamPeer`
   already implies async/stream semantics.

2. **Media bridge mode (Item 2) — transparent RTP relay vs transcoded
   bridge?** Start transparent; transcoded bridge is a future upgrade
   when codec-mismatch use cases arrive. Document the limitation.

3. **`UnifiedCoordinator::bridge` return type** — should it be a
   `BridgeHandle` whose Drop unwires the relay, or a fire-and-forget
   tied to session lifetimes? `BridgeHandle` matches the rest of the
   crate's RAII idiom. Pick that.

4. **Should Item 5 (422 retry) be bundled with Item 4 (INFO helper) as
   "small RFC wins"?** Yes — same PR.

---

## TL;DR

Five small items (1–5, roughly a week of focused work combined) close
the API+RFC gap to start the b2bua crate cleanly. Carrier track (P1–P7)
is multi-week and runs in parallel; LAN / Asterisk / FreeSWITCH
deployments work today without any of it. TLS (P1) is the single
highest-leverage carrier item — it unblocks every major cloud provider.
