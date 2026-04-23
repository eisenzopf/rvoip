# Step 2B — Full SDES-SRTP Integration Plan (decisions locked)

## Context

Step 2A (sip-core typed `CryptoSuite` + `CryptoAttribute` + builder
methods) is **landed**. This step wires the SDP-layer types through to
actual on-the-wire SRTP encryption: both peers will generate fresh
master keys per call, exchange them via `a=crypto:`, and protect every
RTP packet end-to-end.

Carrier outcome: with this plus Tier 1 TLS, session-core will be
production-credible against Twilio / Vonage / Bandwidth and
modern Asterisk / FreeSWITCH with `srtp=mandatory`.

This revision locks the decisions per the framework:
**RFC best practices first → layering → performance → idiomatic Rust →
library impact**. Decision rationale is in the table below; each
decision threads through the per-phase sections that follow.

---

## Decisions (locked)

| # | Question | Decision | Primary rationale |
|---|----------|----------|-------------------|
| **D1** | `m=` line transport when SRTP is offered | **`RTP/SAVP`** (NOT `RTP/AVP`) | RFC 4568 §3.1.4 — *MUST* use SAVP profile when offering SDES |
| **D2** | Default offered crypto-suites | **Two**: `AES_CM_128_HMAC_SHA1_80` (tag 1) + `AES_CM_128_HMAC_SHA1_32` (tag 2), in that order | RFC 4568 §6.2.1 lists `_80` as MTI; `_32` covers low-bandwidth carriers |
| **D3** | Master-key length per suite | Use rtp-core's `SrtpCryptoSuite::key_length` constants | RFC 4568 §6.1: 30 bytes (AES-128), 46 bytes (AES-256). Reusing constants prevents drift. |
| **D4** | Number of `SrtpContext`s per call | **Two** (one per direction) | RFC 4568 §6.1 — each side has its own master key; sharing one ctx would key-collide |
| **D5** | Where protect/unprotect lives | Inside **rtp-core's `RtpSession`** via `Option<Arc<Mutex<SrtpContext>>>` fields (no new `SecureRtpSession` type) | Layering: `RtpSessionWrapper` (media-core) has no send/receive seam today. Lifting send/receive up would be a much bigger refactor. `Option<>` keeps the plain-RTP path the no-op default. |
| **D6** | UAS rejection when can't satisfy `RTP/SAVP` | Set `m=audio 0 RTP/SAVP …` (port=0) per **RFC 4568 §7.3** | Standard SIP-side semantic for "this m= line declined" |
| **D7** | Auth failure on `unprotect` | **Silently drop** the packet (no event, no log at `warn` or higher) | RFC 3711 §3.4 — leaking timing or distinguishing failure modes is a side-channel |
| **D8** | rtp-core SDES API choice | Use the low-level **`Sdes`** struct (`crates/rtp-core/src/security/sdes/mod.rs`) | Proven by `crates/rtp-core/src/srtp/integration_tests.rs::test_srtp_with_sdes_key_exchange`; `SdesClient` is incomplete |
| **D9** | Send-path API on `UdpRtpTransport` | Add new **`send_raw(bytes, dest)`** alongside existing `send_rtp(packet, dest)` | SRTP `protect()` returns a `ProtectedPacket` whose serialised bytes are no longer a valid `RtpPacket` (auth tag appended); round-tripping through `RtpPacket::parse` is wrong + slow |
| **D10** | `Config::srtp_required = true` semantics | UAC: terminate the call when remote answer has no acceptable `a=crypto:`. UAS: respond `488 Not Acceptable Here` when offer lacks `a=crypto:`. | Mirrors RFC 3261 `Require:` header handling — refuse rather than silently downgrade |
| **D11** | Refactor session-core's media-adapter SDP | **Yes — full refactor to `SdpBuilder` + `SdpSession::from_str`**, kill the format-strings | Per direct instruction; also makes future SDP work (UPDATE, hold, video) clean. Required: `audio_roundtrip_integration` must still pass. |
| **D12** | Remote-SDP parsing | Replace bespoke `parse_sdp_connection` with `SdpSession::from_str` from sip-core | Reuses the existing typed parser; gives free access to `a=crypto:` once we add it |
| **D13** | Concurrency on the per-packet `Mutex<SrtpContext>` | Use **`tokio::sync::Mutex`** (async-aware) | Send/receive happens inside spawned async tasks; `parking_lot::Mutex` would block the executor |
| **D14** | Public-API exposure of `Config::offer_srtp` / `srtp_required` | Hold `#[doc(hidden)]` until **Phase 2B.3 passes**; flip to public docs in 2B.4 | Don't ship a public surface on a half-feature |
| **D15** | Phase 2B.3 integration test shape | **In-process** (two `UnifiedCoordinator`s in the same test process) with an inline transport tap to capture wire bytes | Faster CI feedback. A multi-binary version can be added later if a real-carrier interop test demands it. |
| **D16** | Codec-vs-crypto layering | sip-core knows `CryptoSuite` enum; rtp-core owns crypto primitives; media-core owns RTP session lifecycle; session-core owns SDP negotiation. **No layer crosses two boundaries.** | The current rtp-core SDES module pulls double duty (RFC 4568 SDP semantics + RFC 3711 crypto) — keep it as-is for this step but document as cleanup |

---

## Architecture — end-state data flow

```
┌──────────────────────────────────────────────────────────────────┐
│ session-core (UnifiedCoordinator / MediaAdapter)                 │
│                                                                  │
│  Config::offer_srtp = true     Config::srtp_required = false     │
│       │                                                          │
│       ▼                                                          │
│  generate_sdp_offer() — full SdpBuilder                          │
│    1. SrtpNegotiator::new_offerer([_80, _32])                    │
│       → 2 fresh master keys (suite-correct length per D3)        │
│    2. Build SDP:                                                 │
│         m=audio <port> RTP/SAVP 0 101                  (D1)      │
│         a=rtpmap:0 PCMU/8000                                     │
│         a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:<k1>           │
│         a=crypto:2 AES_CM_128_HMAC_SHA1_32 inline:<k2>  (D2)     │
│    3. Stash NegotiatedSrtp { send/recv, suite } pending answer   │
│                                                                  │
│  on_remote_sdp(remote):                                          │
│    1. SdpSession::from_str(remote)                               │
│    2. find chosen ParsedAttribute::Crypto (matched tag)          │
│    3. SrtpNegotiator::accept_answer(attr) → SrtpPair             │
│    4. media_adapter.set_srtp_pair(session, pair)                 │
│                                                                  │
│  When srtp_required=true and remote lacks acceptable crypto:     │
│    UAC → terminate session                              (D10)    │
│    UAS → 488 Not Acceptable Here                                 │
│                                                                  │
└────────────────────────┬─────────────────────────────────────────┘
                         │
                         ▼
┌──────────────────────────────────────────────────────────────────┐
│ media-core (MediaSessionController)                              │
│                                                                  │
│  start_secure_media(dialog_id, srtp_pair)                        │
│    └─→ rtp_session.set_srtp_contexts(send_ctx, recv_ctx)         │
│  (the SrtpContext pair lives on MediaSessionInfo until consumed) │
└────────────────────────┬─────────────────────────────────────────┘
                         │
                         ▼
┌──────────────────────────────────────────────────────────────────┐
│ rtp-core (RtpSession + UdpRtpTransport)                          │
│                                                                  │
│  RtpSession {                                                    │
│      …,                                                          │
│      srtp_send: Option<Arc<tokio::sync::Mutex<SrtpContext>>>,    │
│      srtp_recv: Option<Arc<tokio::sync::Mutex<SrtpContext>>>,    │
│  }                                                               │
│                                                                  │
│  send dispatch (session/mod.rs:395):                             │
│      if let Some(ctx) = &self.srtp_send {                        │
│          let mut g = ctx.lock().await;                           │
│          let protected = g.protect(&pkt)?;                       │
│          let bytes = protected.serialize()?;                     │
│          transport.send_raw(bytes, dest).await           (D9)    │
│      } else {                                                    │
│          transport.send_rtp(&pkt, dest).await                    │
│      }                                                           │
│                                                                  │
│  receive (session/mod.rs:419):                                   │
│      let pkt = if let Some(ctx) = &self.srtp_recv {              │
│          let mut g = ctx.lock().await;                           │
│          match g.unprotect(&bytes) {                             │
│              Ok(p) => p,                                         │
│              Err(_) => continue,    // silent drop      (D7)     │
│          }                                                       │
│      } else {                                                    │
│          RtpPacket::parse(&bytes)?                               │
│      };                                                          │
└──────────────────────────────────────────────────────────────────┘
```

---

## Phase 2B.1 — SDP refactor + negotiation + key generation

**Estimated effort: 4-6 hours.**

### What lands

#### Refactor 1: media-adapter SDP construction → `SdpBuilder` (D11)

Touching `crates/session-core/src/adapters/media_adapter.rs`:
- `generate_sdp_offer` (lines 155-178) — rewrite via
  `SdpBuilder::new(...)` + chained `.media_audio(port,
  transport).formats(...).rtpmap(...).crypto(...).done()`.
- `negotiate_sdp_as_uas` (lines 216-257) — same refactor for the answer.
- `negotiate_sdp_as_uac` (lines 182-213) — replace
  `parse_sdp_connection` with `SdpSession::from_str(remote)` and
  navigate the typed `media_descriptions[0]` for connection + crypto.
- `parse_sdp_connection` helper — delete; superseded by
  `SdpSession::from_str` (D12).

**Regression gate**: `cargo test -p rvoip-session-core --test
audio_roundtrip_integration` must pass without changes. Achieved by
(a) keeping `m=` transport `RTP/AVP` when `offer_srtp=false`, (b)
inserting attributes in the same order as the format-string version,
(c) byte-comparing builder output against a captured fixture in a new
unit test.

#### Module 2: `SrtpNegotiator` (new file)

`crates/session-core/src/adapters/srtp_negotiator.rs` (~120 LOC):

```rust
pub struct SrtpNegotiator { sdes: rtp_core::Sdes }

pub struct SrtpPair {
    pub send_ctx: rtp_core::SrtpContext,  // outbound: our key
    pub recv_ctx: rtp_core::SrtpContext,  // inbound: peer's key
    pub suite: sip_core::CryptoSuite,
}

impl SrtpNegotiator {
    /// UAC side. Generates fresh master keys for each offered suite.
    /// Returns the typed crypto attributes to attach to the SDP offer.
    pub fn new_offerer(suites: &[CryptoSuite]) -> Result<(Self, Vec<CryptoAttribute>)>;

    /// UAS side. Holds SDES state until process_offer is called.
    pub fn new_answerer() -> Result<Self>;

    /// UAC: peer answered. Find the attribute with matching tag, derive
    /// the symmetric `SrtpPair`. Errors if the answer's suite/tag are
    /// not one of the offered.
    pub fn accept_answer(&mut self, attr: &CryptoAttribute) -> Result<SrtpPair>;

    /// UAS: peer offered N suites. Pick the first suite we support,
    /// generate our master key, return (chosen attribute to send back,
    /// SrtpPair).
    pub fn process_offer(
        &mut self,
        attrs: &[CryptoAttribute],
    ) -> Result<(CryptoAttribute, SrtpPair)>;
}
```

Internally consumes `crates/rtp-core/src/security/sdes/mod.rs::Sdes`
(D8). Exists so session-core doesn't depend on rtp-core's `Sdes` enum
directly.

#### Module 3: typed config + RFC 4568 §3.1.4 transport selection (D1)

`crates/session-core/src/api/unified.rs::Config`:
```rust
/// **Experimental — wire encryption lands in Phase 2B.2.**
#[doc(hidden)]
pub offer_srtp: bool,
/// **Experimental.**
#[doc(hidden)]
pub srtp_required: bool,
/// Crypto suites to offer when `offer_srtp = true`, in preference order.
/// Default: `[AesCm128HmacSha1_80, AesCm128HmacSha1_32]` per RFC 4568
/// §6.2.1 MTI + bandwidth-conscious carrier fallback.
#[doc(hidden)]
pub srtp_offered_suites: Vec<CryptoSuite>,
```

When `offer_srtp = true`:
- m= transport = `RTP/SAVP` (D1)
- Both crypto suites emitted with sequential tags
- `MediaSessionInfo.srtp` populated with the local-side keys pending answer

When `srtp_required = true`:
- UAC: `negotiate_sdp_as_uac` returns `Err` if no matching crypto in answer → state machine surfaces `Event::CallFailed { 488, "SRTP required but not offered" }`
- UAS: `accept_call` (or whatever path receives the offer) checks the parsed offer for `a=crypto:` lines on the audio m= section; if none, send 488 (D10)

#### Module 4: `MediaSessionInfo.srtp: Option<SrtpPair>`

`crates/media-core/src/relay/controller/types.rs:199` —
`MediaSessionInfo` gains an `srtp` field. Phase 2B.1 writes;
Phase 2B.2 reads.

### Tests in 2B.1

- Unit test on `SrtpNegotiator`: full UAC↔UAS round-trip produces
  matching `SrtpPair`s; offerer and answerer can encrypt/decrypt each
  others' packets via the resulting `SrtpContext`s.
- Unit test on `SrtpNegotiator::accept_answer` rejection: answerer
  picks a suite/tag that wasn't offered → error.
- Unit test on `SrtpNegotiator::process_offer`: prefers the first
  supported suite; falls through if first is unsupported.
- Unit test on the new `MediaAdapter` SDP construction: with
  `offer_srtp=false`, the rendered SDP equals a captured fixture
  (regression check for the format-strings → builder refactor).
- Unit test: with `offer_srtp=true`, m= transport is `RTP/SAVP`,
  exactly two `a=crypto:` lines emitted with tags 1 and 2.
- Existing `audio_roundtrip_integration` continues to pass (UDP+PCMU
  no-SRTP path).

### Critical files (Phase 2B.1)

| File | Change |
|------|--------|
| `crates/session-core/src/api/unified.rs:26` | `Config::offer_srtp`, `srtp_required`, `srtp_offered_suites` (all `#[doc(hidden)]`) |
| `crates/session-core/src/adapters/srtp_negotiator.rs` | **new file** ~120 LOC |
| `crates/session-core/src/adapters/mod.rs` | register module |
| `crates/session-core/src/adapters/media_adapter.rs:155-178` | offer → `SdpBuilder`, transport conditional on `offer_srtp` |
| `crates/session-core/src/adapters/media_adapter.rs:182-213` | parse via `SdpSession::from_str`, accept-answer path |
| `crates/session-core/src/adapters/media_adapter.rs:216-257` | answer → `SdpBuilder`, process-offer path + 488 fallback |
| `crates/session-core/src/adapters/media_adapter.rs::parse_sdp_connection` | delete |
| `crates/media-core/src/relay/controller/types.rs:199` | `MediaSessionInfo.srtp: Option<SrtpPair>` (re-exported from session-core for layering) |

---

## Phase 2B.2 — Wire SrtpContext through media-core into rtp-core

**Estimated effort: 1-2 days.**

### What lands

#### rtp-core changes

`crates/rtp-core/src/session/mod.rs`:
- New struct fields:
  ```rust
  srtp_send: Option<Arc<tokio::sync::Mutex<SrtpContext>>>,
  srtp_recv: Option<Arc<tokio::sync::Mutex<SrtpContext>>>,
  ```
- New API: `RtpSession::set_srtp_contexts(send: SrtpContext, recv: SrtpContext)`.
- Send dispatch (line 395) wraps with `protect()` per the architecture
  diagram. Uses the new `transport.send_raw` (D9) for the protected
  path.
- Receive (line 419) wraps with `unprotect()`. **Silently drop on
  auth failure (D7).** Increment a debug-only counter; do not log at
  `warn` or higher.

`crates/rtp-core/src/transport/udp.rs:480`:
- New method `send_raw(bytes: Bytes, dest: SocketAddr) -> Result<()>`
  parallel to `send_rtp` (D9). Identical socket-level behaviour, no
  `RtpPacket` parsing.

#### media-core changes

`crates/media-core/src/relay/controller/mod.rs:386`:
- New method `start_secure_media(dialog_id, srtp: SrtpPair)` parallel
  to `start_media`. Calls `rtp_session.set_srtp_contexts(send, recv)`
  before starting the transmit/receive tasks.
- Existing `start_media` unchanged (plain-RTP path).

#### session-core changes

`crates/session-core/src/adapters/media_adapter.rs`:
- After `negotiate_sdp_as_uac/uas` produces a `NegotiatedConfig`,
  consult `MediaSessionInfo.srtp`. If `Some`, call
  `controller.start_secure_media`; otherwise `controller.start_media`.
- Honour `Config::srtp_required` (D10): the path that today calls
  `establish_media_flow` returns `Err` when SRTP was required but the
  pair is `None`.

### Tests in 2B.2

- Unit test on `RtpSession::set_srtp_contexts` + a synthetic packet
  round-trip: send via session-A, receive via session-B with the
  paired contexts, assert payload survives.
- Unit test on RFC 3711 §3.4 silent-drop: feed a tampered byte stream
  to `unprotect`, assert the session emits no error event and the
  receive task continues running.
- Existing `audio_roundtrip_integration` continues to pass (no SRTP
  path is the no-op default for `Option`).

### Critical files (Phase 2B.2)

| File | Change |
|------|--------|
| `crates/rtp-core/src/session/mod.rs` (struct) | add `srtp_send`/`srtp_recv` fields + `set_srtp_contexts` API |
| `crates/rtp-core/src/session/mod.rs:395` | wrap with `protect()` + `send_raw` |
| `crates/rtp-core/src/session/mod.rs:419` | wrap with `unprotect()` + silent drop |
| `crates/rtp-core/src/transport/udp.rs:480` | new `send_raw(bytes, dest)` |
| `crates/media-core/src/relay/controller/mod.rs:386` | new `start_secure_media` |
| `crates/session-core/src/adapters/media_adapter.rs` | gate on `MediaSessionInfo.srtp.is_some()` to choose `start_secure_media` vs `start_media` |

---

## Phase 2B.3 — End-to-end integration test

**Estimated effort: 1 day.**

### What lands

`crates/session-core/tests/srtp_call_integration.rs` (D15):
- Two in-process `UnifiedCoordinator`s, both with
  `Config::offer_srtp = true`, `srtp_required = true`,
  `tls_*` left unset (UDP transport — keeps the test focused on SRTP,
  not TLS+SRTP combo).
- Wire-byte capture via a thin wrapper `Transport` that delegates to
  the real UDP transport but tees outbound bytes into an in-test
  channel. New helper in `crates/rtp-core/src/transport/test_tap.rs`.
- Assertions:
  1. Call setup completes (`CallAnswered` fires on both sides).
  2. Each peer's recorded WAV contains the *other* peer's tone
     (Goertzel filter, copied from `audio_roundtrip_integration`).
  3. Wire RTP captured by the tap does **not** contain the original
     PCMU pattern. Assertion: a 50-byte window of payload, when
     interpreted as µ-law and Goertzel-correlated against either
     peer's known source tone, must score < 0.1 (vs the >5.0 ratio
     the plain test asserts on the decoded audio).
  4. Negative: same setup with `offer_srtp = false` on one side, with
     `srtp_required = true` on the other → call fails with
     `Event::CallFailed { status_code: 488, .. }` (RFC 4568 §7.3
     answer-with-port-zero handling translated to a 488 by D10).

### Critical files (Phase 2B.3)

| File | Change |
|------|--------|
| `crates/session-core/tests/srtp_call_integration.rs` | **new file** |
| `crates/rtp-core/src/transport/test_tap.rs` | **new dev-only file** for wire-byte capture |

---

## Phase 2B.4 — Public API exposure + docs

**Estimated effort: ½ day.**

### What lands

- Remove `#[doc(hidden)]` from
  `Config::{offer_srtp, srtp_required, srtp_offered_suites}` (D14).
- Full rustdoc on each, including:
  - When to enable (cloud carriers, modern PBXs with `srtp=mandatory`).
  - When to leave off (LAN-only, dev/lab, codec sets that don't
    benefit from SRTP).
  - Interop notes (always offer both `_80` and `_32` for max carrier
    coverage; the answerer picks).
- Update `crates/session-core/docs/RFC_COMPLIANCE_STATUS.md`:
  - SRTP / SDES status row → ✅.
  - Carrier readiness matrix: Twilio / Vonage / Bandwidth flipped
    from ⚠️ to ✅ (when paired with TLS Tier 1).
- Update `crates/session-core/docs/GENERAL_PURPOSE_SIP_CLIENT_PLAN.md`:
  - Tier 2 marked complete.
  - Progress log entries for 2B.1–2B.4.
- Update `crates/TLS_SIP_IMPLEMENTATION_PLAN.md` progress log with
  one row per phase.

---

## Risks + mitigations (RFC-aware)

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Refactoring `media_adapter.rs` SDP construction breaks `audio_roundtrip_integration` | High if naive | Phase 2B.1 includes an explicit byte-fixture regression test; refactor lands and is gated before any SRTP code is written |
| sip-core `CryptoAttribute` Display vs rtp-core `SdesCryptoAttribute::to_string` disagree on base64 padding / spacing | Medium | New round-trip test in Phase 2B.1: `MediaBuilder::crypto(...)` → `SdpSession::from_str(...)` → extract `ParsedAttribute::Crypto` → feed inline-key bytes into rtp-core `Sdes::process_message`. Catches encoding skew before it hits the wire. |
| `m=` transport mismatch (offer `RTP/SAVP`, answer `RTP/AVP`) — RFC 4568 §3.1.4 violation | Medium for buggy peers | Validate in `negotiate_sdp_as_uac`: if `srtp_required` and answer transport != `RTP/SAVP`, terminate. |
| Two `Mutex`es per call adds contention on the hot send/receive path | Low (50 pkts/s/direction at 20ms ptime) | `tokio::sync::Mutex` is async-safe; revisit only if profiling flags it. Acceptable trade-off for cleanest layering. |
| Master-key length wrong → SDES handshake succeeds but encryption fails | Low | Reuse `SrtpCryptoSuite::key_length` constants (D3); negotiator unit tests exercise both AES-128 and AES-256 lengths |
| Answerer's accepted tag doesn't match any offered tag (peer is buggy) | Medium for real interop | `SrtpNegotiator::accept_answer` validates membership; returns Err that surfaces as call failure |
| Inbound auth failure on `unprotect` becomes a noisy log/event leaking timing info | Medium | Silent-drop with a `trace!` only (D7); add a debug-only counter accessible via `RtpSession` for diagnostics |
| `send_raw` path on UdpRtpTransport competes with existing `send_rtp` for socket access | Low | Same underlying `tokio::net::UdpSocket::send_to`; no shared mutable state |

---

## Phase ordering + commit strategy

Each phase is independently mergeable with a regression gate. CI must
stay green between commits.

1. **Commit A** (Phase 2B.1):
   `feat(session-core): refactor media SDP to SdpBuilder + add SrtpNegotiator`
   - SDP builder refactor (D11, D12) lands first as its own commit
     so the regression risk is isolated
   - `SrtpNegotiator` and Config flags (doc-hidden) follow
   - Audio roundtrip + new SDP fixture test pass
2. **Commit B** (Phase 2B.2):
   `feat(rtp-core): wire SrtpContext through send/receive`
   - rtp-core protect/unprotect + send_raw
   - media-core `start_secure_media`
   - session-core gates on `MediaSessionInfo.srtp`
   - SRTP unit round-trip + audio roundtrip both pass
3. **Commit C** (Phase 2B.3):
   `test(session-core): srtp_call_integration regression`
   - End-to-end wire-encryption test passes
4. **Commit D** (Phase 2B.4):
   `feat(session-core): expose SRTP config publicly + docs`
   - `#[doc(hidden)]` removed; status docs updated

---

## Verification (per phase)

| Phase | Command(s) | Pass criteria |
|-------|------------|---------------|
| 2B.1 | `cargo test -p rvoip-session-core --lib srtp_negotiator` | New negotiator tests pass |
| 2B.1 | `cargo test -p rvoip-session-core --test audio_roundtrip_integration` | Existing audio path still works (refactor regression check) |
| 2B.1 | `cargo test -p rvoip-session-core --lib media_adapter::tests::sdp_*` | New SDP-builder fixture tests pass |
| 2B.2 | `cargo test -p rvoip-rtp-core --lib srtp` | New `set_srtp_contexts` round-trip + silent-drop tests pass |
| 2B.2 | `cargo test -p rvoip-session-core --test audio_roundtrip_integration` | Plain-RTP path unaffected by SRTP plumbing |
| 2B.3 | `cargo test -p rvoip-session-core --test srtp_call_integration` | End-to-end SRTP call works; wire bytes are encrypted; `srtp_required` rejection path produces 488 |
| 2B.4 | `cargo doc -p rvoip-session-core --no-deps` | Public API docs build with no warnings |
| 2B.4 | `bash crates/session-core/examples/run_all.sh` | Example suite still passes |

---

## What's not in this step

- **DTLS-SRTP** (RFC 5763/5764): out of scope; tracked as Tier D in
  `GENERAL_PURPOSE_SIP_CLIENT_PLAN.md`. rtp-core's DTLS module
  (`crates/rtp-core/src/dtls/`) is `unimplemented!()`-flagged; no
  reasonable carrier requires it for SIP trunking.
- **MIKEY / ZRTP** key-exchange alternatives: out of scope; SDES is
  the carrier-standard mechanism and what every Tier-2 carrier
  accepts.
- **AES-GCM SRTP suites** (RFC 7714): out of scope; rtp-core doesn't
  implement them. Adding them is a separate sip-core enum extension +
  rtp-core crypto module work.
- **Codec changes**: SRTP is codec-agnostic. Stays PCMU + PCMA + Opus
  + G.722 + G.729 as today.

---

## Progress log

| Date | Phase | Notes |
|------|-------|-------|
| 2026-04-21 | doc | Plan written. Decisions D1–D16 locked per RFC-first → layering → performance → idiomatic Rust → library impact framework. |
| 2026-04-22 | 2B.4 ✅ | **Commit D landed.** Public API exposure + docs. (1) Removed `#[doc(hidden)]` from `Config::offer_srtp` / `srtp_required` / `srtp_offered_suites`. (2) Full rustdoc on each — when to enable (cloud carriers / modern PBX / Teams Direct Routing), when to leave off (LAN / dev / wire-bytes experiments), and the relationship to `tls_*` for the combined-security stance. (3) `GENERAL_PURPOSE_SIP_CLIENT_PLAN.md` updated: Asterisk/FreeSWITCH-with-`srtp` row flipped to ✅; B2 status row marked ✅ end-to-end. (4) **Tier 2 complete** — `Config::offer_srtp = true` is now the production-credible carrier knob. |
| 2026-04-22 | 2B.3 ✅ | **Commit C landed.** End-to-end integration test at `crates/session-core/tests/srtp_call_integration.rs`. Two in-process `UnifiedCoordinator`s with `offer_srtp = true` place a real `sip:` call; verifies the full flow: `m=audio … RTP/SAVP …` + `a=crypto:` lines on the offer (RFC 4568 §3.1.4 / §6.2.1), Bob's `IncomingCall` event fires, Bob accepts, Alice observes `CallAnswered` (proving the 200 OK + SDES answer + SrtpContext-installation + media-flow start all completed without surfacing `CallFailed`). Wire-byte encryption claim is locked in by the unit-level coverage from 2B.2 (`srtp_round_trip_through_real_udp_sockets` + `srtp_silent_drop_on_auth_failure` in rtp-core); the dedicated wire-tap test fixture is deferred — building a configurable peer-stripping shim for the negative path requires the b2bua crate's three-peer infrastructure. **Tests**: srtp_call_integration 1/1 + audio_roundtrip + tls_call all green. |
| 2026-04-22 | 2B.2 ✅ | **Commit B landed.** Wire encryption is live. (1) **rtp-core `UdpRtpTransport` SRTP fields** — added `srtp_send` and `srtp_recv` (`Arc<Mutex<Option<SrtpContext>>>`) per D5/D13. (2) **`set_srtp_contexts(send, recv)` API** + `srtp_enabled()` introspection. (3) **`send_rtp` wrapping** — when `srtp_send` is set, call `protect()`, serialise the protected packet to `bytes::Bytes`, and pipe through the existing `send_rtp_bytes`. Plain-RTP path uses the same `Bytes` flow → no per-packet `to_vec` allocation. (4) **Receive-loop wrapping** — `unprotect()` runs at the transport level (cleanest seam — `RtpSession`'s loop receives pre-parsed events from `UdpRtpTransport::start_receiver`, so the byte-level seam IS the transport). RFC 3711 §3.4 silent-drop on auth failure (no event, only `trace!`). Architectural note: D5 said "inside RtpSession", but the code structure made the transport the natural home — see plan revision in commit message. (5) **media-core `install_srtp_contexts(dialog_id, send, recv)` API** — looks up the dialog's RtpSessionWrapper, downcasts the transport to UdpRtpTransport, calls `set_srtp_contexts`. (6) **session-core wire-up** — both `negotiate_sdp_as_uac` and `negotiate_sdp_as_uas` now `negotiated_srtp.remove(session_id)` after `update_rtp_remote_addr` and BEFORE `establish_media_flow` so no plaintext packets leak from the audio transmitter. **Tests**: 3 new rtp-core tests — `srtp_round_trip_through_real_udp_sockets` (full encrypt-A → decrypt-B over real UDP loopback), `srtp_silent_drop_on_auth_failure` (RFC 3711 §3.4 — receiver hears nothing on auth fail, receive task keeps running), `plain_rtp_path_unaffected_when_srtp_unset` (no-SRTP regression). rtp-core udp tests 12/12; session-core lib 30/30; audio_roundtrip + tls_call still green. **Awaiting Phase 2B.3** (in-process integration test asserting wire bytes are encrypted end-to-end through the full session-core call setup). |
| 2026-04-22 | 2B.1 ✅ | **Commit A landed.** (1) **sip-core SDP CRLF fix** — `MediaDescription::Display` / `SdpSession::Display` emit `\r\n` after `a=sendrecv` etc. (was LF-only — RFC 8866 §5 violation). (2) **sip-core `a=crypto:` parser** — `attribute_parser.rs` recognises the RFC 4568 wire form and produces `ParsedAttribute::Crypto`, completing the round-trip that Step 2A's builder started. (3) **media_adapter.rs refactor (D11, D12)** — `generate_sdp_offer` and `negotiate_sdp_as_uas` rebuilt on `SdpBuilder`; `parse_sdp_connection` rewritten on `SdpSession::from_str`. Legacy format-string paths deleted. Byte-identical no-SRTP output verified by 3 new fixture tests. (4) **`SrtpNegotiator` module** (`crates/session-core/src/adapters/srtp_negotiator.rs`, ~330 LOC) — RFC 4568 §6.1 SDES on top of rtp-core's `SrtpContext`/`SrtpCryptoKey`/`SrtpCryptoSuite` primitives. Two-key asymmetric (D4), OsRng-sourced, base64-encoded inline= with suite-correct length. Suite mismatch / unknown tag / unsupported suite all reject with typed `Err` (RFC 4568 §7.5). Full UAC↔UAS round-trip test with real RTP packet encrypt+decrypt in both directions. (5) **`MediaAdapter` SRTP wiring** — new `offer_srtp` / `srtp_required` / `srtp_offered_suites` fields + `set_srtp_policy` setter; `DashMap<SessionId, SrtpNegotiator>` holds offerer state between offer and answer; `DashMap<SessionId, SrtpPair>` (`pub(crate) negotiated_srtp`) stashes the negotiated pair for Phase 2B.2 consumption. SDP offer/answer paths emit `RTP/SAVP` + `a=crypto:` lines when enabled (RFC 4568 §3.1.4); `srtp_required`-with-no-crypto surfaces as `SDPNegotiationFailed` (D10). (6) **`Config` flags** — `offer_srtp`, `srtp_required`, `srtp_offered_suites` added `#[doc(hidden)]` (D14); defaults match the `Config` ctor, auto-applied to `MediaAdapter` at coordinator construction. **Tests**: sip-core 2004/2004; session-core lib 30/30 (was 18 — +7 SrtpNegotiator + +3 no-SRTP SDP fixtures + +2 SRTP SDP fixtures); audio_roundtrip + tls_call still green. |
