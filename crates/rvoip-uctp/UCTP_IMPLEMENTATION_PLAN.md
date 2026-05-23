# UCTP v0 — Implementation Plan

**Status:** working draft, 2026-05-22. Pre-implementation design doc — code is not yet written.

**Companion documents (authoritative; this plan defers to them on conflicts):**
- `../rvoip-core/CONVERSATION_PROTOCOL.md` — UCTP v0 wire spec (envelope shape, lifecycle, error codes)
- `../rvoip-core/INTERFACE_DESIGN.md` — rvoip Rust library architecture, `ConnectionAdapter` contract, bridging, vCon
- `../rvoip-core/voip-3-conversation-model.md` — conceptual model (terminology source of truth)
- `../rvoip-core/PRD.md` — scope of rvoip-core

This plan also surfaces a §10 list of points where it diverges from INTERFACE_DESIGN.md; those are explicit discussion topics for the design owner, not unilateral commitments.

---

## 1. Context

We are creating UCTP support in the rvoip workspace. UCTP (Universal Conversation Transport Protocol) is rvoip's substrate-agnostic application protocol — it speaks the voip-3 nouns directly on the wire over QUIC / WebTransport / WebSocket. The full wire protocol is specified in `../rvoip-core/CONVERSATION_PROTOCOL.md`.

The first use case is a **call-center agent client speaking UCTP to a backend service**, modeled after how today's `rvoip-sip` provides a client/server SIP surface. The backend in this first cut is a process running `rvoip_core::Orchestrator` with **both** a UCTP substrate adapter and a `SipAdapter` registered — that way a UCTP-speaking agent (native or browser) can be bridged to a SIP-speaking customer, demonstrating the cross-transport payoff rvoip-core was designed for.

**Multi-party — protocol-supported, v0-implementation-deferred.** The UCTP spec (CONVERSATION_PROTOCOL.md §7.7) and INTERFACE_DESIGN.md §10.1 / §10.6 commit to N-Participant Sessions with explicit subscribe/unsubscribe routing. v0 envelopes parse the new types (`stream.subscribe`, `stream.unsubscribe`, `stream.active-speaker`) so the wire format is stable, but the v0 implementation **still ships 1:1 only** — the demo's value is validating substrate adapters and cross-transport bridging, not multi-party fan-out. Multi-party routing (`Orchestrator::add_subscription` / `remove_subscription`, the per-Session routing table) is v0.x / v1 rvoip-core work and lands after the substrate adapters are stable.

### 1.1 Confirmed product decisions

| Dimension | Decision |
|---|---|
| Crate layout | **3 crates**, aligned with INTERFACE_DESIGN.md §2: `rvoip-uctp` (shared protocol + substrate helpers) plus per-substrate adapters `rvoip-quic` and `rvoip-webtransport`. The fourth (`rvoip-websocket`) is deferred to v1 because WS media requires a co-located `webrtc-rs` PeerConnection. |
| Substrates v0 | **QUIC + WebTransport** together. WebTransport is HTTP/3-over-QUIC, so both adapters share one quinn `Endpoint` (multi-ALPN) and the same rustls config; the marginal cost of adding WT alongside QUIC is small and the browser-reach payoff is large. |
| Functional scope v0 | Signaling + messaging + media. Out of scope for v0 spike (all tracked in §1.4 v0.x roadmap): vCon emission, identity step-up + assurance enforcement (403), RFC 9421 signing, DTMF, quality reports, multi-party routing. |
| Demo | Cross-transport bridging: orchestrator with both UCTP adapters and `SipAdapter`; native UCTP agent (over raw QUIC) and browser-shaped UCTP agent (over WebTransport) bridge to the same SIP customer. |
| Encoding | JSON envelopes via `serde_json`. Binary encoding deferred per CONVERSATION_PROTOCOL.md §3.3. |
| IDs | UUID-prefixed strings (`conv_<simple-uuid>`, …), mirroring `rvoip_core::ids`. CONVERSATION_PROTOCOL.md §3.1 marks format advisory. |

### 1.2 Crate map

| Crate | Role | INTERFACE_DESIGN.md anchor |
|---|---|---|
| `rvoip-uctp` | Envelopes, type catalog, capability negotiation, **UCTP-specific state machine**, **shared `substrate::quinn` helpers** (TLS, length-prefixed codec, datagram pack/unpack, correlation map). No I/O loops of its own. | §2 (`rvoip-uctp`) |
| `rvoip-quic` | Thin substrate adapter. Owns the raw-QUIC accept/dial loops on a `quinn::Endpoint` registered with ALPN `uctp/1`. Implements `rvoip_core::ConnectionAdapter` returning `Transport::Quic`. | §2 (`rvoip-quic`), §3.5 |
| `rvoip-webtransport` | Thin substrate adapter. Owns the WT accept/dial loops via `web-transport-quinn` on a `quinn::Endpoint` registered with ALPN `h3`. Implements `rvoip_core::ConnectionAdapter` returning `Transport::WebTransport`. | §2 (`rvoip-webtransport`), §3.5 |

The two adapters can be deployed **on a single quinn::Endpoint with both ALPNs**, or as separate endpoints. The plan exercises the dual-ALPN single-endpoint shape because it's the production deployment pattern.

### 1.3 Workspace dependencies (existing + planned)

This plan adds three new crates (above) and depends on these existing/planned workspace crates rather than re-implementing their surfaces:

| Crate | Status | Used by this plan for |
|---|---|---|
| `auth-core` | Exists (`crates/auth-core/`) | Home of the v0 `BearerValidator` (stub, no validation), alongside the existing `DigestAuthenticator`. Future DPoP, JWT, AAuth, RFC 9421 signing implementations land here too — not in `rvoip-uctp`. |
| `rvoip-media-core` | Exists (`crates/media-core/`, package `rvoip-media-core`) | Home of `Transcoder` (`media-core/src/codec/transcoding.rs`), mixer, jitter buffer, AEC/AGC/VAD, processing pipeline. Phase 4 demo uses `rvoip_media_core::codec::transcoding::Transcoder` for SIP G.711 ↔ UCTP Opus bridging. **Naming note:** INTERFACE_DESIGN.md §2 calls this crate `rvoip-media`; the actual workspace name today is `rvoip-media-core`. Plan uses the actual name; reconciling the design-doc name is a separate rename project (see §10). |
| `rvoip-vcon` | Planned, v0.x (does not exist yet) | Home of `VconBuilder`, `VconStore` trait, JWS sign/verify, JWE encrypt/decrypt, redaction lineage, voip-3 → vCon adapter. Per INTERFACE_DESIGN.md §2 it ships as the FIRST Rust implementation of the IETF vCon spec. Created in v0.x; consumed by `rvoip-uctp`, `rvoip-sip`, `rvoip-webrtc` for `recording.vcon-ready` emission at `session.ended`. |

### 1.4 After v0: v0.x roadmap

This document is scoped to the **v0 spike** — the smallest end-to-end cut that proves the substrate-adapter architecture and cross-transport bridging. The following are deliberately deferred to **v0.x** (next milestone after v0 ships), not "indefinite":

| v0.x track | What ships | Reference docs | Why deferred from v0 |
|---|---|---|---|
| **vCon emission** | New `rvoip-vcon` crate; in-flight `VconBuilder` per Session; `MemoryVconStore` default; `recording.vcon-ready` envelope emitted at `session.ended`; `RecordingComplete.vcon_ref` populated. JWS signing default-on, JWE/redaction opt-in. | PRD.md §4 (in scope for v1), INTERFACE_DESIGN.md §3.9 + §11.4, CONVERSATION_PROTOCOL.md §7.6 | New crate to build; not on the critical path for proving substrate adapters. v0 emits `RecordingComplete` with `vcon_ref: None` and does not emit `recording.vcon-ready`. |
| **Multi-party routing** | `Orchestrator::add_subscription` / `remove_subscription` / `apply_subscriptions`; per-Session routing table; `stream.active-speaker` emission. | CONVERSATION_PROTOCOL.md §7.7, INTERFACE_DESIGN.md §10.6 | v0 envelopes parse the new types so the wire stays stable; routing is rvoip-core work and lands after the substrate adapters are stable. |
| **Identity assurance enforcement (403)** | `IdentityAssurance` gradient checked at `session.invite`; `identity.step-up-request` / `step-up-response` / `step-up-complete` envelope flow. | CONVERSATION_PROTOCOL.md §5.6 + §8, INTERFACE_DESIGN.md §3.8 | v0 bearer-stub validator returns `IdentityAssurance::Pseudonymous` unconditionally; enforcement is meaningful only once `auth-core` has real validators (DPoP/JWT/AAuth). |
| **`Orchestrator::bridge_connections` automation** | `BridgeManager` / `BridgeHandle` per INTERFACE_DESIGN.md §10.2; automatic frame-pump replacing the v0 demo's manual pump. | INTERFACE_DESIGN.md §10.2–3 | rvoip-core stub today (orchestrator.rs:329); separate PR. |
| **`rvoip-websocket` substrate adapter** | Fourth substrate crate; WS text frames for signaling + co-located `webrtc-rs` PeerConnection for media. | CONVERSATION_PROTOCOL.md §4.3, INTERFACE_DESIGN.md §2 | Requires `webrtc-rs` integration (DTLS-SRTP, ICE), which is its own significant work item; QUIC + WT already cover the v0 demo's reach. |
| **DTMF, quality reports** | `connection.dtmf`, `connection.quality` envelope handling end-to-end. | CONVERSATION_PROTOCOL.md §7.5 + §10.3 | Wire types parse in v0 but adapters return `NotImplemented`. |
| **RFC 9421 / DPoP / AAuth backends** | Real validators in `auth-core` consumed by `rvoip-uctp`'s capability negotiator. | INTERFACE_DESIGN.md §8 | Standards-track identity work; orthogonal to substrate adapters. |

The v0 spike is a single coherent cut; v0.x is then **one milestone per row above**, each shippable independently.

---

## 2. Phase 0 — Workspace scaffolding

Goal: register the 3 new crates, add `quinn` and `web-transport-quinn` workspace deps. **No new `Transport` variant** — `Transport::Quic` and `Transport::WebTransport` already exist in `rvoip-core/src/connection.rs` per INTERFACE_DESIGN.md §3.5 (verify at implementation time).

### 2.1 `/Volumes/D2-2019/Developer/rvoip/Cargo.toml`

- Append to `[workspace] members` (alongside the SIP-family cluster):
  ```
  "crates/rvoip-uctp",
  "crates/rvoip-quic",
  "crates/rvoip-webtransport",
  ```
- Append the same three to `default-members`.
- Add to `[workspace.dependencies]`:
  ```
  rvoip-uctp           = { path = "crates/rvoip-uctp",           version = "0.1.26" }
  rvoip-quic           = { path = "crates/rvoip-quic",           version = "0.1.26" }
  rvoip-webtransport   = { path = "crates/rvoip-webtransport",   version = "0.1.26" }
  rvoip-auth-core      = { path = "crates/auth-core",            version = "0.1.0"  } # if not already present
  rvoip-media-core     = { path = "crates/media-core",            version = "0.1.0"  } # if not already present; used by Phase 4 bridge demo for transcoding
  quinn                = { version = "0.11", default-features = false, features = ["runtime-tokio", "rustls-ring"] }
  web-transport-quinn  = "0.5"
  tokio-util           = { version = "0.7", features = ["codec", "compat"] }
  ```
  Existing workspace entries we reuse: `rustls`, `rustls-pemfile`, `rcgen`, `tokio`, `bytes`, `serde`, `serde_json`, `thiserror`, `async-trait`, `dashmap`, `tracing`, `uuid`, `chrono`, `futures`, `parking_lot`. **Confirmed at plan time:** `rvoip-media-core` is already a workspace.dependencies entry; `rvoip-auth-core` is **not** and must be added in this PR. Both crates exist on disk under `crates/auth-core/` and `crates/media-core/`.

### 2.2 `rvoip-core` — verify Transport variants, no source changes expected

Per INTERFACE_DESIGN.md §3.5, the `Transport` enum already includes `Quic` and `WebTransport`. Confirm at implementation time:

```rust
pub enum Transport {
    Quic,           // <- used by rvoip-quic adapter
    WebTransport,   // <- used by rvoip-webtransport adapter
    WebSocket,      // <- future rvoip-websocket adapter
    Sip,
    WebRtc,
    InProcessAi,
}
```

No new variant required. If the current enum is missing `WebTransport`, that's an additive change in rvoip-core (1 line) — INTERFACE_DESIGN.md §3.5 already mandates it.

The `ConnectionAdapter` trait surface is already wide enough. UCTP-out-of-scope methods (`send_dtmf`, `verify_request_signature`, `renegotiate_media`) return `NotImplemented` in v0 — matching the `SipAdapter` posture.

### 2.3 `rvoip-core` — extend `CapabilityDescriptor` to match CONVERSATION_PROTOCOL §8

Separate PR against `rvoip-core` (lands before Phase 1 starts). Edits `crates/rvoip-core/src/capability.rs`:

- Add the seven `#[serde(default)]` fields enumerated in §3.4 below.
- Add the four small enums (`DataProtocol`, `DtmfMode`, `TransportFeature`, `IdentityAssuranceRequirement`) per INTERFACE_DESIGN.md §9 so the new fields are typed rather than `Vec<String>`-shaped.
- Keep `supports_dtmf_rfc4733` as a back-compat alias derived from `dtmf_modes` on deserialize.
- Update the `Default` impl so existing callers compile unchanged.
- Add a unit test that round-trips a CONVERSATION_PROTOCOL §8 example payload (the full JSON block from the spec) through `serde_json`.

This is the only rvoip-core source change required to start Phase 1. `IdentityAssurance` already exists in rvoip-core (`crates/rvoip-core/src/identity.rs:51-70`) with variants `Anonymous` / `Pseudonymous { ephemeral_key: Jwk }` / `Identified { credential_kind }` / `TaskScoped { ... }` / `UserAuthorized { ... }` — no changes to that enum needed.

### 2.4 `rvoip-core` — placeholder `VconRef` so `RecordingComplete` compiles in v0

§7 commits v0 to emitting `RecordingComplete { vcon_ref: None, .. }` (the full `rvoip-vcon` crate ships in v0.x — see §1.4). For `vcon_ref: None` to be a legal value today, the `RecordingComplete` struct in rvoip-core must already carry the field with a real type. Phase 0 adds a placeholder:

```rust
// crates/rvoip-core/src/recording.rs (or wherever RecordingComplete lives)

/// Opaque reference to a vCon document. v0 always carries None; v0.x's rvoip-vcon
/// populates Some(VconRef::Local { uuid }) at session.ended. The variant set is
/// intentionally minimal — extending it is a v0.x decision once VconStore's
/// addressing model is firmer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum VconRef {
    /// Local store; the uuid resolves through whatever VconStore the orchestrator was built with.
    Local { uuid: uuid::Uuid },
    /// Future: HTTPS-resolvable vCon URI. Variant reserved; not constructed in v0.
    Url { url: String },
}

pub struct RecordingComplete {
    // ... existing fields
    pub vcon_ref: Option<VconRef>,
}
```

The `Url` variant is reserved (not constructed) so the serde wire shape doesn't churn between v0 and v0.x when `rvoip-vcon` introduces remote-resolvable references. This lands in the same Phase 0 rvoip-core PR as the `CapabilityDescriptor` extension (§2.3), or as a third small PR — implementer's choice.

### 2.5 `auth-core` — introduce `BearerValidator` trait + `bearer_stub()`

`crates/auth-core/` exists today with `DigestAuthenticator` (for SIP) and a `CredentialKind::Bearer` enum variant, but **no `BearerValidator` trait and no bearer stub**. Phase 1's `UctpCoordinator` cannot compile without one, so it is its own Phase 0 PR — landed before Phase 1 starts, independent of the rvoip-core changes in §2.3 / §2.4.

The PR adds `crates/auth-core/src/bearer.rs` with the trait and stub from §3.5 verbatim:

```rust
// crates/auth-core/src/bearer.rs (new)
use std::sync::Arc;

#[async_trait::async_trait]
pub trait BearerValidator: Send + Sync {
    async fn validate(&self, token: &str) -> Result<rvoip_core::IdentityAssurance, AuthError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("empty bearer token")]
    Empty,
    #[error("invalid bearer token: {0}")]
    Invalid(String),
    #[error("validator unavailable: {0}")]
    Unavailable(String),
}

/// v0 stub: returns IdentityAssurance::Pseudonymous { ephemeral_key } for any non-empty
/// token (ephemeral_key is a freshly-generated throwaway JWK); rejects empty tokens with
/// AuthError::Empty. Replaced by real DPoP / JWT / AAuth / RFC 9421 validators in v0.x.
pub fn bearer_stub() -> Arc<dyn BearerValidator>;
```

Also: `crates/auth-core/src/lib.rs` re-exports `BearerValidator`, `AuthError`, and `bearer_stub`. The `AuthError` type is what `rvoip_uctp::UctpError::Auth(#[from] rvoip_auth_core::AuthError)` consumes (§3.2.1).

**No changes** to the existing `DigestAuthenticator` — bearer validation is purely additive.

### 2.6 Per-crate `Cargo.toml` skeleton

**State today:** `crates/rvoip-uctp/` exists but contains only this plan file — no `src/`, no `Cargo.toml`. `crates/rvoip-quic/` and `crates/rvoip-webtransport/` do not exist at all. Phase 0 creates all three directories, their `src/lib.rs` stubs, and the `Cargo.toml` skeletons below.

Each new `crates/*/Cargo.toml` inherits workspace metadata:
```toml
[package]
name = "rvoip-..."
version.workspace = true
edition.workspace = true
license.workspace = true
homepage.workspace = true
repository.workspace = true
documentation.workspace = true
authors.workspace = true
rust-version.workspace = true
description = "..."

[lints]
workspace = true

[dependencies]
# per-crate (Phases 1–3 below)
```

---

## 3. Phase 1 — `rvoip-uctp` (shared protocol + substrate helpers)

This is the load-bearing crate. It owns the entire UCTP protocol — envelopes, state machine, capability negotiation — plus the substrate-agnostic helpers that both adapter crates consume. The adapter crates only own substrate-specific accept/dial loops.

### 3.1 Dependencies
```
rvoip-core.workspace = true
rvoip-auth-core.workspace = true  # BearerValidator (stub in v0; real DPoP/JWT/AAuth land in auth-core in v0.x)
tokio.workspace = true
tokio-util.workspace = true
quinn.workspace = true            # for substrate::quinn helpers (shared by quic + webtransport)
rustls.workspace = true
rustls-pemfile.workspace = true
rcgen.workspace = true            # self-signed certs for local dev
bytes.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
chrono.workspace = true
uuid.workspace = true
async-trait.workspace = true
dashmap.workspace = true
parking_lot.workspace = true
tracing.workspace = true
futures.workspace = true
```

`auth-core`'s workspace dependency key is `rvoip-auth-core` to match the existing convention (`crates/auth-core/Cargo.toml` ships as `package = "rvoip-auth-core"`). Verify at implementation time.

### 3.2 Module layout

```
crates/rvoip-uctp/src/
  lib.rs                        -- pub uses; doc references CONVERSATION_PROTOCOL.md §-anchors
  errors.rs                     -- UctpError (decode failures, unknown type, state-machine errors, transport errors)
  ids.rs                        -- new_envelope_id(), new_conversation_id(), ...
  envelope.rs                   -- UctpEnvelope<T>, encode/decode, two-layer typing
  types.rs                      -- MessageType enum
  payloads/
    mod.rs
    auth.rs, conversation.rs, session.rs, connection.rs,
    message.rs, capability.rs, control.rs        -- per CONVERSATION_PROTOCOL.md §5–§9, §11
    stream.rs                                    -- StreamSubscribe, StreamUnsubscribe,
                                                    StreamActiveSpeaker per §7.7. Parsing only
                                                    in v0; routing logic lives in rvoip-core
                                                    and lands in v0.x (see §7 known tensions)
  capability.rs                 -- UctpCapabilityDescriptor + negotiate_streams() (§8.1)
  state/
    mod.rs                      -- UctpStateMachine entry point
    session.rs                  -- UctpSessionState + transitions
    connection.rs               -- UctpConnectionState + transitions + stream_local_id allocator
    coordinator.rs              -- UctpCoordinator: per-peer driver; routes envelopes to machines
    events.rs                   -- UctpSessionEvent (InboundInvite, Connected, Ended, MediaFrame, ...)
  substrate/
    mod.rs                      -- shared helpers consumed by adapter crates
    quinn.rs                    -- make_endpoint(addr, tls, &[alpn]), TLS config, datagram pack/unpack,
                                   dispatch_by_alpn() for the dual-ALPN shared endpoint (§3.7, §5.4)
    framing.rs                  -- LengthPrefixedCodec wrapper (4-byte BE prefix per §4.1/§4.2)
    tls.rs                      -- self_signed_for_dev(domains), dev_client_config_trusting(cert)
    correlation.rs              -- Pending: DashMap<EnvelopeId, oneshot::Sender<UctpEnvelope>> with TTL cleanup
```

**`lib.rs` public surface.** The adapter crates need a stable set of re-exports; nothing else should be reachable. Concretely:

```rust
// rvoip-uctp/src/lib.rs
pub use crate::envelope::UctpEnvelope;
pub use crate::types::MessageType;
pub use crate::ids::{
    new_envelope_id, new_conversation_id, new_session_id, new_connection_id, new_stream_id,
    EnvelopeId, UctpSessionId, UctpConnId, StreamId,
};
pub use crate::errors::{UctpError, SubstrateError};
pub use crate::payloads;                                // re-exports the entire payloads tree
pub use crate::capability::{UctpCapabilityDescriptor, negotiate_streams, NegotiationOutcome};
pub use crate::state::{
    UctpCoordinator, UctpSessionEvent,
    UctpSessionState, UctpConnectionState,              // useful for adapters that inspect state
};
pub mod substrate;                                       // adapter crates use substrate::quinn / ::framing / ::datagram / ::correlation / ::tls
```

Anything not on this list is `pub(crate)`. Adapter authors that need something else should open a PR rather than reach into module internals.

#### 3.2.1 Error variants

Every fn signature in the crate depends on this partition, so it is committed up front rather than left to first-implementer discretion.

```rust
// errors.rs
#[derive(Debug, thiserror::Error)]
pub enum UctpError {
    #[error("envelope decode failed: {0}")]
    Decode(#[from] serde_json::Error),

    #[error("unknown envelope type: {0}")]
    UnknownEnvelopeType(String),                 // surfaced when MessageType::Unknown is encountered in a context that requires a known type

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("illegal state transition: state={state} event={event}")]
    IllegalTransition { state: &'static str, event: &'static str },

    #[error("capability negotiation failed: code={code}")]
    CapabilityNegotiationFailed { code: u16 },   // typically 488

    #[error("authentication failed: {0}")]
    Auth(#[from] rvoip_auth_core::AuthError),

    #[error("stream-handle exhausted (u16 wrap)")]
    StreamHandleExhausted,

    #[error("operation timed out")]
    Timeout,

    #[error("coordinator closed")]
    Closed,

    #[error(transparent)]
    Transport(#[from] SubstrateError),
}

#[derive(Debug, thiserror::Error)]
pub enum SubstrateError {
    #[error("quinn connection error: {0}")]
    Quinn(#[from] quinn::ConnectionError),

    #[error("quinn write error: {0}")]
    Write(#[from] quinn::WriteError),

    #[error("quinn read error: {0}")]
    Read(#[from] quinn::ReadError),

    #[error("rustls error: {0}")]
    Tls(#[from] rustls::Error),

    #[error("invalid datagram: {0}")]
    InvalidDatagram(&'static str),               // version mismatch, too short, bad flags

    #[error("frame too large: {0} bytes (max 1 MiB)")]
    FrameTooLarge(usize),

    #[error("alpn dispatch closed")]
    DispatchClosed,

    #[error("substrate closed")]
    Closed,
}
```

The adapter crates wrap these with one outer variant each:

```rust
// rvoip-quic/src/errors.rs
#[derive(Debug, thiserror::Error)]
pub enum UctpQuicError {
    #[error(transparent)] Uctp(#[from] rvoip_uctp::UctpError),
    #[error(transparent)] Substrate(#[from] rvoip_uctp::SubstrateError),
    #[error("adapter not started")] NotStarted,
    #[error("adapter shutdown")] Shutdown,
}
```

`rvoip-webtransport` mirrors this with `UctpWtError` plus one variant for `web_transport_quinn::SessionError`.

### 3.3 The `UctpEnvelope<T>` type

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UctpEnvelope<T = serde_json::Value> {
    pub v: u8,                                   // always 1 in v0
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub id: String,
    pub ts: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    pub payload: T,
}
```

**Two-layer typing.** On the wire we parse to `UctpEnvelope<serde_json::Value>` so unknown payload fields are tolerated (forward compat per §3.2). Application code calls `env.decode_payload::<SessionInvite>()` on demand against the typed `payloads::*` structs. `MessageType` includes `#[serde(other)] Unknown` so unknown wire types decode cleanly.

### 3.4 `CapabilityDescriptor` (extend rvoip-core, do not fork)

CONVERSATION_PROTOCOL.md §8 is the wire-format authority. INTERFACE_DESIGN.md §9 commits `rvoip_core::CapabilityDescriptor` as the unified neutral shape **with typed enums per field**, not raw strings. Today's struct (`crates/rvoip-core/src/capability.rs:11-18`) is narrower than both — it has `audio_codecs`, `video_codecs`, `supports_dtmf_rfc4733`, `supports_message_text`, `supports_srtp`. Bringing it up to §8 + INTERFACE_DESIGN §9 is an **additive change in rvoip-core**, scheduled as a Phase 0 sub-task (§2.3 above). No `UctpCapabilityDescriptor` fork.

**Fields to add to `rvoip_core::CapabilityDescriptor`** (all `#[serde(default)]` for back-compat). Per INTERFACE_DESIGN.md §9, each list-typed field uses a typed enum so the Rust API can't drift from the spec catalog; serde derives map them to/from the §8 JSON strings via `#[serde(rename = "...")]`:

```rust
pub data_protocols: Vec<DataProtocol>,                       // §8: "text" | "json" | "binary"
pub dtmf_modes: Vec<DtmfMode>,                               // §8: "rfc4733" | "info" — supersedes supports_dtmf_rfc4733
pub max_streams_per_connection: u16,                         // §8
pub transport_features: Vec<TransportFeature>,               // §8: MediaDatagrams | ConnectionMigration | SessionResumption | ZeroRtt | TranscodeG711Opus | ...
pub interop: Vec<Transport>,                                 // §8: gatewayable endpoints only; reuses rvoip_core::Transport
pub identity_assurance_offered: IdentityAssurance,           // §5.6 + §8 — gradient enum already in rvoip-core
pub identity_assurance_required: Option<IdentityAssuranceRequirement>, // §8 — typed requirement per INTERFACE_DESIGN §9

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataProtocol { Text, Json, Binary }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DtmfMode {
    #[serde(rename = "rfc4733")] Rfc4733,
    #[serde(rename = "info")]    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportFeature {
    MediaDatagrams,
    ConnectionMigration,
    SessionResumption,
    #[serde(rename = "0rtt")] ZeroRtt,
    #[serde(rename = "transcode-g711-opus")] TranscodeG711Opus,
    // ... extend per spec §8 as the catalog grows
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityAssuranceRequirement {
    None,            // Anonymous acceptable
    Pseudonymous,    // Pseudonymous or higher
    Identified,      // Identified or higher
    TaskScoped,      // TaskScoped or UserAuthorized
    UserAuthorized,  // UserAuthorized only
}
```

The four small enums (`DataProtocol`, `DtmfMode`, `TransportFeature`, `IdentityAssuranceRequirement`) land in `rvoip_core::capability` alongside the field additions, in the same Phase 0 PR. **`supports_dtmf_rfc4733` is converted from a field into a method** (`pub fn supports_dtmf_rfc4733(&self) -> bool { self.dtmf_modes.contains(&DtmfMode::Rfc4733) }`); existing call sites that read the field as `descriptor.supports_dtmf_rfc4733` must change to `descriptor.supports_dtmf_rfc4733()`. Making it a method (not a field with custom deserialize) avoids the serde round-trip pitfall where setting one would silently desync the other, and keeps `dtmf_modes` as the single source of truth on the wire. The Phase 0 PR audits and updates all call sites in one pass. `rvoip-uctp` consumes the unified struct directly via `UctpCapabilityDescriptor = rvoip_core::CapabilityDescriptor` (type alias). The negotiation algorithm in §8.1 is implemented on the unified struct.

Why the §8 codec params shape (`{"name":"opus","params":{"sample_rate":48000,...}}`) maps onto today's `CodecInfo {name, clock_rate_hz, channels, fmtp}`: `clock_rate_hz` ← `params.sample_rate`, `channels` ← `params.channels`, `fmtp` ← serialized remainder. Mapping helpers live in `rvoip-uctp::payloads::capability`.

### 3.5 State machine — Rust enum + match for v0

CONVERSATION_PROTOCOL.md §7 specifies the lifecycle. The state machine has 4 Session states and 4 Connection states — plain `enum` + `match` is the right scale. If it grows past ~15 states, switch to the YAML-driven pattern from `rvoip-sip/state_table/`.

```rust
pub enum UctpSessionState {
    Inviting,        // session.invite sent, awaiting accept
    Active,          // ≥1 connection.ready
    Ending,          // session.end sent/received, awaiting last connection.end
    Ended,
}

pub enum UctpConnectionState {
    Negotiating,     // connection.offer sent, no answer yet
    Connected,       // connection.ready fired
    OnHold,
    Ending,
    Ended,
}
```

Happy path (§7.2): `session.invite → session.accept → connection.offer → connection.answer → connection.ready → session.started` then media datagrams. Teardown (§7.3): `session.end → connection.end per Connection → session.ended` after the 30s grace window if no reconnect.

Rejection paths v0 handles: **488** incompatible-capabilities (`negotiate_streams` returns `NotAcceptable488`); **487** cancelled (`session.cancel` mid-invite). **403** forbidden-for-assurance-level deferred to v0.x (see §1.4).

Transitions are derived directly from CONVERSATION_PROTOCOL.md §7.2 (Session lifecycle ASCII diagram), §7.3 (boundary rules including the 30s grace window), and §7.4 (Connection lifecycle). Error codes per §11.2. The implementer writes the `match` arms verbatim from those sections; do not reinterpret. Where the spec is silent (e.g., `stream_local_id` exhaustion at u16::MAX — return `error` code `503` with reason `stream-handle-exhausted` and refuse further `stream.opened`), the plan commits to behavior in line with the rest of the §11.2 error model.

**Authentication flow.** CONVERSATION_PROTOCOL.md §5.1 specifies a four-envelope handshake: `auth.hello → auth.challenge → auth.response → auth.session`. In v0 the relevant handler runs on `auth.response`: the coordinator extracts `payload.credential`, calls `rvoip_auth_core::BearerValidator::validate(&credential).await`, and on success emits `auth.session` populated with `identity_id`, `participant_id`, the issued `session_token`, `expires_at`, and the resulting `IdentityAssurance` (`Pseudonymous` for the bearer stub). On validation failure the coordinator emits `error` code `401` with `category: "auth"` and the bearer error reason, then closes the substrate connection. The trait signature `auth-core` exposes for the Phase 0 PR:

```rust
// crates/auth-core/src/bearer.rs (new)
#[async_trait::async_trait]
pub trait BearerValidator: Send + Sync {
    async fn validate(&self, token: &str) -> Result<rvoip_core::IdentityAssurance, AuthError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("empty bearer token")]
    Empty,
    #[error("invalid bearer token: {0}")]
    Invalid(String),
    #[error("validator unavailable: {0}")]
    Unavailable(String),
}

/// v0 stub: returns IdentityAssurance::Pseudonymous { ephemeral_key } for any non-empty
/// token (the ephemeral_key is a freshly-generated throwaway JWK); rejects empty tokens
/// with AuthError::Empty. Replaced by real DPoP / JWT / AAuth / RFC 9421 validators in v0.x.
pub fn bearer_stub() -> Arc<dyn BearerValidator>;
```

`Pseudonymous` carries `ephemeral_key: Jwk` in rvoip-core's actual enum (see `crates/rvoip-core/src/identity.rs:51-70`); the stub generates one per-token rather than fabricating a permanent key.

**`UctpCoordinator` design.** Per-peer driver; one instance per substrate connection. Mirrors the `SipAdapter` concurrency model (see `crates/rvoip-sip/src/adapter.rs:33-43`): `&self` methods, internal `DashMap`s, per-machine `parking_lot::Mutex` around the actual state-machine instance.

```rust
pub struct UctpCoordinator {
    sessions:    Arc<DashMap<UctpSessionId, Mutex<SessionMachine>>>,
    connections: Arc<DashMap<UctpConnId,    Mutex<ConnectionMachine>>>,
    pending:     Arc<substrate::correlation::Pending>,
    events_tx:   mpsc::Sender<UctpSessionEvent>,
    out_tx:      mpsc::Sender<UctpEnvelope>,        // outbound envelopes → substrate writer
    cancel:      CancellationToken,                  // tokio_util::sync::CancellationToken
    bearer:      Arc<dyn rvoip_auth_core::BearerValidator>,
}

impl UctpCoordinator {
    /// Spawns the driver task. The returned coordinator owns the cancel token; dropping
    /// it (or calling shutdown) cancels the task and drains in-flight Pending entries
    /// with SubstrateError::Closed.
    pub fn start(
        in_rx: mpsc::Receiver<UctpEnvelope>,         // from substrate reader
        out_tx: mpsc::Sender<UctpEnvelope>,          // to substrate writer
        events_tx: mpsc::Sender<UctpSessionEvent>,
        bearer: Arc<dyn rvoip_auth_core::BearerValidator>,
    ) -> Arc<Self>;

    pub async fn shutdown(&self);                   // signals the cancel token; awaits the driver task
    pub fn events(&self) -> mpsc::Receiver<UctpSessionEvent>;  // taken once (Mutex<Option<...>>)
}
```

Driver-task behavior: read one envelope from `in_rx`; route by `sid` / `connid` to the matching machine (or create one on `session.invite` / `connection.offer`); lock the machine, apply the transition, unlock; for each `SideEffect` the machine emits, either send an outbound envelope through `out_tx` or surface a `UctpSessionEvent` through `events_tx`. Envelopes for an **unknown** sid/connid are answered with `error` code `404 not-found` (per §11.2); envelopes with an unknown `MessageType::Unknown(_)` are silently ignored per spec §3.2 forward-compat rule. Channel capacities match Phase 2 / 3 (envelopes = 256). The substrate adapter (Phase 2 / 3) feeds the coordinator envelopes and consumes its events; the adapter never touches `DashMap` state directly.

**Shutdown choreography.** Three layers, one explicit order so adapter authors don't reinvent it:

1. **Adapter receives shutdown signal** (drop, explicit `close()`, or `Orchestrator` teardown). Adapter calls `coordinator.shutdown().await`.
2. **Coordinator `shutdown()`**:
   a. Triggers its `CancellationToken`. The driver task observes it on its next `select!` and stops reading from `in_rx`.
   b. Drains `pending`: every outstanding `oneshot::Sender` in `substrate::correlation::Pending` is dropped, so any awaiting `wait_for` future resolves with `SubstrateError::Closed`.
   c. For each Session in `Active` / `Inviting`, synthesizes a local `session.end` transition with reason `"shutdown"`; emits `UctpSessionEvent::Ended` so the orchestrator sees terminal events for in-flight calls.
   d. Closes `out_tx` and `events_tx`. Joins the driver task.
3. **Substrate close**: only after `coordinator.shutdown()` returns does the adapter close the underlying `quinn::Connection` with `ApplicationClose { error_code: 0x00, reason: b"shutdown" }`. Closing the QUIC connection first would race with step 2c — the outbound `session.end` envelopes would never reach the peer.

The inverse path — peer-initiated close — runs steps 2c → 3 → 2a → 2b (substrate read loop sees `ConnectionError::ApplicationClosed`, surfaces `SubstrateError::Closed`, the coordinator's `in_rx` drains, the driver task exits, and `Pending` drains as the coordinator itself is dropped).

**Backpressure policy.** Two channels with different semantics:

- **Signaling envelope channel** (`in_rx` / `out_tx`, capacity 256, async `send`): signaling is correctness-critical; never drop. Substrate writer awaits `out_tx.send(env)`; if the channel is full (consumer wedged), `send` blocks naturally. Substrate reader uses `in_rx.send(env).await` likewise. The 256-deep buffer is sized so an attacker can't easily exhaust memory but a healthy peer never blocks. If a `send` future is observed to be pending for more than 5s (instrumented via `tokio::time::timeout`), the adapter logs at `warn`, emits an `error` envelope with code `503 transient` and `reason: "signaling-backpressure"`, then triggers the shutdown choreography above. There is no silent drop on this channel.
- **Datagram channel** (per-Stream `frames_out` / `frames_in`, capacity 1024, `try_send`): media is inherently lossy; drop is the correct failure mode. The writer task calls `frames_out.try_send(frame)`; on `TrySendError::Full` it increments a `uctp_datagram_drops_total{direction, connid}` counter, logs at `debug` (not warn — too noisy), and **does not** close the connection. On `TrySendError::Closed` the stream is treated as ended and a `stream.closed` envelope is emitted.

The asymmetry is deliberate: a backed-up signaling channel means the peer or coordinator has stopped reading and the session is unrecoverable; a backed-up datagram channel means a transient consumer hiccup that's not worth tearing down the call for.

### 3.6 Stream registration

On `connection.ready`, the Connection machine asks `streams::Allocator::allocate(connid) -> Vec<(StreamId, u16 stream_local_id)>`. Allocator is per-Connection, monotonically increments a u16 from 1. Each pair emits `stream.opened`. The `(connid, stream_local_id) → strm_id` map lives in Connection state so inbound `MediaDatagram`s route back to the right StreamId.

### 3.7 `substrate::quinn` shared helpers

The "WT is QUIC underneath" leverage point. Both `rvoip-quic` and `rvoip-webtransport` consume these:

```rust
// substrate::quinn
pub fn make_server_endpoint(
    addr: SocketAddr,
    tls: rustls::ServerConfig,                       // pre-configured with ALPNs
    transport_cfg: TransportConfig,
) -> Result<quinn::Endpoint, SubstrateError>;

pub fn make_client_endpoint(
    bind: SocketAddr,
    client_cfg: rustls::ClientConfig,
) -> Result<quinn::Endpoint, SubstrateError>;

/// Single-consumer ALPN dispatcher. Spawns one accept task on the given Endpoint,
/// reads `Connecting::handshake_data()` to learn the negotiated ALPN, and forwards
/// each `Connecting` to the matching adapter's mpsc channel. Unrecognized ALPNs are
/// closed with `error_code = 0x01, reason = "alpn-not-registered"`.
///
/// Required for the dual-ALPN shared-endpoint deployment described in §5.4; calling
/// `endpoint.accept()` from multiple adapters directly is a bug (single-consumer API).
pub fn dispatch_by_alpn(
    endpoint: Arc<quinn::Endpoint>,
    alpns: &[&[u8]],
) -> Result<AlpnRoutes, SubstrateError>;

pub struct AlpnRoutes { /* internal: HashMap<Vec<u8>, mpsc::Receiver<quinn::Connecting>> */ }
impl AlpnRoutes {
    /// Take ownership of the receiver for a specific ALPN. Returns None if the ALPN
    /// wasn't passed to dispatch_by_alpn or has already been taken.
    pub fn take(&mut self, alpn: &[u8]) -> Option<mpsc::Receiver<quinn::Connecting>>;
}

// substrate::tls
pub fn self_signed_for_dev(domains: &[String])
    -> Result<(rustls::Certificate, rustls::PrivateKey), SubstrateError>;

/// Build a rustls::ClientConfig that trusts the given (already-known) self-signed
/// certificate. Used by the demo agents in Phase 4 so they can connect to the
/// orchestrator's self_signed_for_dev() endpoint without skipping verification.
pub fn dev_client_config_trusting(cert: &rustls::Certificate)
    -> Result<rustls::ClientConfig, SubstrateError>;

/// rustls::ClientConfig with verification disabled. For tests and demos only;
/// gated behind the `dev-dangerous` feature so production builds can't depend on it.
#[cfg(feature = "dev-dangerous")]
pub fn dangerous_no_verify() -> rustls::ClientConfig;

// substrate::framing — wraps quinn::SendStream / RecvStream in tokio_util's codec
pub fn length_prefixed_codec() -> LengthDelimitedCodec;       // 4-byte BE prefix, max 1 MiB
pub fn envelope_reader(rx: quinn::RecvStream)
    -> impl Stream<Item = Result<UctpEnvelope, SubstrateError>>;
pub fn envelope_writer(tx: quinn::SendStream)
    -> impl Sink<UctpEnvelope, Error = SubstrateError>;

// substrate::datagram — 8-byte UCTP header per CONVERSATION_PROTOCOL.md §10.1, then RTP
/// In-memory shape of a UCTP media datagram. Wire layout per CONVERSATION_PROTOCOL.md §10.1:
/// `ver(u8=1) | flags(u8) | stream_local_id(u16 BE) | datagram_seq(u32 BE) | RTP packet`.
pub struct MediaDatagram {
    pub flags: u8,
    pub stream_local_id: u16,
    pub seq: u32,
    pub payload: Bytes,            // RTP packet (header + body); pack/unpack do not parse it
}
pub fn pack(d: &MediaDatagram) -> Bytes;
pub fn unpack(b: &[u8]) -> Result<MediaDatagram, SubstrateError>;

// substrate::correlation — envelope-id round-trips
pub struct Pending {
    inner: DashMap<EnvelopeId, oneshot::Sender<UctpEnvelope>>,
}
impl Pending {
    /// Default TTL: 30s (matches CONVERSATION_PROTOCOL.md §7.3 reconnect grace window).
    /// Callers override via the explicit ttl argument when a faster handshake budget applies.
    pub async fn wait_for(&self, id: EnvelopeId, ttl: Duration) -> Result<UctpEnvelope, SubstrateError>;

    /// Match on env.in_reply_to and forward to the waiting oneshot. Returns Err(env) if no
    /// pending entry matches, so the coordinator can route the unmatched envelope as a
    /// normal inbound (e.g., server-initiated connection.update arriving with no outstanding
    /// request to correlate against).
    pub fn deliver(&self, env: UctpEnvelope) -> Result<(), UctpEnvelope>;
}
```

The signaling-side framing is **identical** for QUIC and WebTransport (CONVERSATION_PROTOCOL.md §4.2 explicitly says WT framing is "the same as QUIC"). The datagram-side framing is also identical (WT datagrams go through QUIC datagrams underneath). The only thing the adapter crates do differently is how they obtain `quinn::SendStream`/`RecvStream` instances from their respective accept paths.

### 3.8 Tests
- **Envelope round-trip:** typed payload → JSON → re-parse; unknown extension fields preserved.
- **Unknown type:** `{"type":"future.feature",...}` → `MessageType::Unknown`.
- **Codec negotiation:** full overlap (pick top), partial (pick second), disjoint (NotAcceptable488).
- **State machine:**
  - `session_invite_accept_roundtrip`
  - `connection_negotiate_happy_path` (offer with overlapping codecs → answer carries chosen codec; ready follows)
  - `connection_negotiate_488`
  - `session_cancel_during_inviting` → session.ended code 487
  - `session_end_with_two_connections` → session.ended only after second connection.end (§7.3 boundary)
  - `stream_local_id_round_trip` — allocate id, route inbound datagram via the id back to a `MediaFrame`
- **Substrate framing:**
  - Length-prefix codec round-trip through `tokio::io::duplex`
  - Datagram pack/unpack including error branches (too short, bad version)
- **Correlation:** `Pending::wait_for` returns on `deliver`; times out after TTL.

### 3.9 Observability

`rvoip-sip` invests heavily in `tracing` spans plus a `StageMetrics` / `record_metrics` infrastructure (see `crates/rvoip-sip/src/adapter.rs`). UCTP must reach parity so a regression in the WT path vs. the QUIC path is visible — this is load-bearing for the demo's "cross-transport bridging works" claim. Three layers:

**Tracing spans.** All spans are `tracing::info_span!` or `debug_span!` and use the listed field set so log/trace aggregation can pivot consistently:

| Span | Level | Fields | Where opened |
|---|---|---|---|
| `uctp.coordinator.driver` | `info_span` | `peer.addr`, `transport` (`"quic"`/`"webtransport"`) | `UctpCoordinator::start` — parents every per-envelope span on that connection |
| `uctp.envelope.in` | `debug_span` | `type`, `id`, `sid`, `connid` | Each inbound envelope; closed when the matching machine transition completes |
| `uctp.envelope.out` | `debug_span` | `type`, `id`, `sid`, `connid`, `in_reply_to` | Each outbound envelope; closed at the `out_tx.send()` resolution |
| `uctp.session.invite` | `info_span` | `sid`, `from`, `to` | Created on outbound or inbound `session.invite`; closes at `session.started` or `session.ended` |
| `uctp.connection.negotiate` | `info_span` | `connid`, `sid`, `chosen_codec` (filled on answer) | Opens at `connection.offer`, closes at `connection.ready` or 488 |
| `uctp.connection.lifetime` | `info_span` | `connid`, `sid` | Opens at `connection.ready`, closes at `connection.end` — wraps the media-frame path |
| `uctp.stream.frame` | `trace_span` | `connid`, `stream_local_id`, `seq`, `bytes` | Per datagram; trace-level so production deployments can disable cheaply |
| `uctp.auth.bearer` | `info_span` | `participant_id` (post-validate), `assurance` | Opens on `auth.response`, closes on `auth.session` emission or 401 |

**Counters and gauges** (exposed via the `metrics` crate so `metrics-exporter-prometheus` or equivalent picks them up):

| Metric | Type | Labels | Description |
|---|---|---|---|
| `uctp_envelopes_total` | counter | `direction` (`in`/`out`), `type`, `transport` | Per-envelope-type traffic; matches the `MessageType` enum variants |
| `uctp_envelope_errors_total` | counter | `code`, `transport` | §11.2 error codes; one increment per `error` envelope emitted |
| `uctp_sessions_active` | gauge | `transport` | Sessions in `Active`; observed at every state transition |
| `uctp_connections_active` | gauge | `transport` | Connections in `Connected` |
| `uctp_connections_negotiating` | gauge | `transport` | Connections in `Negotiating` — alerting target (stuck-handshake detection) |
| `uctp_datagrams_total` | counter | `direction`, `transport` | Successful pack/send and unpack/deliver |
| `uctp_datagram_drops_total` | counter | `direction`, `transport`, `reason` (`channel-full`/`unpack-error`/`unknown-stream`) | The drop counter referenced in §3.5's backpressure policy |
| `uctp_capability_negotiations_total` | counter | `outcome` (`ok`/`488`), `transport` | 488 rate is the headline negotiation-quality signal |
| `uctp_handshake_duration_seconds` | histogram | `transport`, `outcome` | Session.invite → session.started latency; per-transport histogram for QUIC-vs-WT comparison |
| `uctp_substrate_pending_outstanding` | gauge | `transport` | `substrate::correlation::Pending` map size; correlated-request leak detector |

**Quinn-connection stats surfaced** (read via `quinn::Connection::stats()` on a `tokio::time::interval` of 5s per active connection, emitted as gauges with `connid` and `transport` labels):

- `path.rtt` → `uctp_quinn_rtt_seconds`
- `path.cwnd` → `uctp_quinn_cwnd_bytes`
- `udp_tx.datagrams` / `udp_rx.datagrams` → `uctp_quinn_udp_datagrams_total{direction}`
- `frame_tx.connection_close` / `frame_rx.connection_close` → `uctp_quinn_close_frames_total{direction}`
- `path.lost_packets` → `uctp_quinn_lost_packets_total`

The 5s sample interval is a v0 default; production deployments can tune via the adapter config struct (added to `UctpQuicConfig` and `UctpWtConfig` as `quinn_stats_interval: Duration`, default 5s; setting it to zero disables the sampling task).

**Implementation locations.** Spans are opened by the coordinator (rvoip-uctp); metrics emission lives in rvoip-uctp as well so both substrate adapters get parity for free. The `transport` label is set once at coordinator construction. The adapter crates do **not** instrument their accept/dial loops with their own metrics — that would split the surface and make per-transport comparison impossible.

**Tests.** `crates/rvoip-uctp/tests/observability.rs` installs a `tracing_subscriber` capture and a `metrics::set_recorder` test recorder, runs a full invite-accept-end flow, and asserts: (a) `uctp_envelopes_total{type="session.invite"}` increments once per side; (b) `uctp_handshake_duration_seconds` records exactly one observation per call; (c) the `uctp.session.invite` span closes within the test timeout (no leaked spans). The Phase 4 `bridge_smoke.rs` test additionally asserts that QUIC-path and WT-path runs produce metric series with identical type/label cardinality — the parity check that justifies all of the above.

**Dependencies added by this section.** `tracing` is already a workspace dependency. Add `metrics = "0.23"` to `[workspace.dependencies]` (§2.1) and as a non-optional dep of `rvoip-uctp` (§3.1). Recorder choice (`metrics-exporter-prometheus`, `metrics-exporter-tcp`, etc.) is deployment-config, not a `rvoip-uctp` dep.

---

## 4. Phase 2 — `rvoip-quic` (raw-QUIC substrate adapter)

Thin. Owns the accept/dial loops for raw QUIC; everything else is shared helpers from `rvoip-uctp`.

### 4.1 Dependencies
```
rvoip-core.workspace = true
rvoip-uctp.workspace = true
tokio.workspace = true
async-trait.workspace = true
quinn.workspace = true
thiserror.workspace = true
tracing.workspace = true
dashmap.workspace = true
chrono.workspace = true
bytes.workspace = true
parking_lot.workspace = true
```

### 4.2 Module layout

```
crates/rvoip-quic/src/
  lib.rs                        -- pub uses
  errors.rs                     -- UctpQuicError (wraps rvoip-uctp errors + quinn errors)
  server.rs                     -- UctpQuicServer: binds quinn::Endpoint, accepts UCTP connections
  client.rs                     -- UctpQuicClient: dials, exposes call/message methods (agent role)
  adapter.rs                    -- UctpQuicAdapter: impl rvoip_core::ConnectionAdapter
  media_stream.rs               -- QuicDatagramMediaStream: impl rvoip_core::MediaStream
                                   (per INTERFACE_DESIGN.md §3.6 naming)
```

### 4.3 ALPN

Server registers ALPN `b"uctp/1"`. Client offers the same. If a server is co-deployed with `rvoip-webtransport` (the v0 demo), both ALPNs are registered on one endpoint; quinn dispatches based on the negotiated ALPN. See §5.4.

### 4.4 `UctpQuicAdapter`

Line-for-line counterpart of `rvoip-sip::SipAdapter` (see `crates/rvoip-sip/src/adapter.rs:33-43`). Same `by_connection`/`by_uctp_connid` `DashMap` pattern, same `out_tx`/`out_rx` mpsc channels, same `tokio::spawn` translator task that maps `UctpSessionEvent` → `AdapterEvent`.

`AdapterEvent` variant names (per `crates/rvoip-core/src/adapter.rs:72-91`): `InboundConnection { connection }`, `Connected { connection_id }`, `Ended { connection_id, reason }`, `Failed { connection_id, detail }`, `Native { kind, detail }`. The orchestrator normalizes these into its outward-facing `Event::ConnectionInbound` / `Event::ConnectionConnected` / `Event::ConnectionEnded` / `Event::ConnectionFailed` per `INTERFACE_DESIGN.md` §5; **don't conflate the two enums** — the adapter emits `AdapterEvent::*`, the orchestrator publishes `Event::*`.

```rust
pub struct UctpQuicConfig {
    pub endpoint: Arc<quinn::Endpoint>,                 // shared via substrate::quinn::dispatch_by_alpn (§3.7)
    pub accept_rx: mpsc::Receiver<quinn::Connecting>,   // ALPN-filtered stream from the dispatcher
    pub server_tls: Arc<rustls::ServerConfig>,
    pub max_concurrent_connections: usize,              // default 1024
    pub idle_timeout: Duration,                         // default 30s
    pub bearer_validator: Arc<dyn rvoip_auth_core::BearerValidator>,
    pub pending_ttl: Duration,                          // default 30s; matches CONVERSATION_PROTOCOL.md §7.3 grace window
}

pub struct UctpQuicAdapter {
    server: Arc<UctpQuicServer>,
    by_connection:   Arc<DashMap<ConnectionId, String>>,
    by_uctp_connid:  Arc<DashMap<String, ConnectionId>>,
    out_tx: mpsc::Sender<AdapterEvent>,
    out_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

#[async_trait::async_trait]
impl ConnectionAdapter for UctpQuicAdapter {
    fn transport(&self) -> Transport { Transport::Quic }
    fn kind(&self) -> AdapterKind { AdapterKind::Substrate }
    // ... dispatch to UctpCoordinator (in rvoip-uctp) for everything else
}
```

**`ConnectionAdapter` method split for v0** (full trait surface is in `crates/rvoip-core/src/adapter.rs:97-126`):

| Method | v0 behavior |
|---|---|
| `transport()` | Returns `Transport::Quic` (or `Transport::WebTransport` in Phase 3) |
| `kind()` | Returns `AdapterKind::Substrate` |
| `originate(req)` | **Real** — sends `session.invite` + `connection.offer` via the coordinator |
| `accept(conn)` | **Real** — sends `session.accept` + `connection.answer` |
| `reject(conn, reason)` | **Real** — sends `error` envelope with the matching reason code (§11.2) |
| `end(conn, reason)` | **Real** — sends `session.end` and/or `connection.end` |
| `streams(conn)` | **Real** — returns `Arc<dyn MediaStream>` clones for the Connection's open Streams |
| `subscribe_events()` | **Real** — yields the `out_rx` once (StdMutex<Option<...>> pattern; mirrors `SipAdapter::subscribe_events`) |
| `capabilities()` | **Real** — returns the adapter's `CapabilityDescriptor` |
| `send_message(conn, msg)` | **Real** — sends a `message.send` envelope; in v0 the demo doesn't exercise this but the wire type is in the spec catalog (§6) and stubbing it would gratuitously narrow the surface |
| `hold(conn)` / `resume(conn)` | `NotImplemented` (v0.x) |
| `transfer(conn, target)` | `NotImplemented` (v0.x) |
| `send_dtmf(conn, digits, dur)` | `NotImplemented` (v0.x; wire type parses) |
| `renegotiate_media(conn, caps)` | `NotImplemented` (v0.x) |
| `verify_request_signature(conn, sig)` | `NotImplemented` (v0.x; lands with RFC 9421 in auth-core) |

Channel capacities: envelopes = 256, datagrams = 1024 (audio frames at 50 Hz × 2-stream peers fill quickly; bigger buffer absorbs jitter). Overflow policy per channel is specified in §3.5's "Backpressure policy" paragraph — adapter must not invent its own behavior.

### 4.5 `QuicDatagramMediaStream`

Per INTERFACE_DESIGN.md §3.6 naming. Wraps a per-Stream pair of `mpsc::channel`s; one driver task drains `frames_out` → `rvoip_uctp::substrate::datagram::pack` → `quinn::Connection::send_datagram`; another reads incoming datagrams and pushes to `frames_in`.

### 4.6 Tests
- `tests/loopback.rs` — bind on `127.0.0.1:0`; client connects; 5 envelopes c→s + s→c; 10 datagrams each way; assert order.
- `tests/adapter.rs` — register the adapter against an `Orchestrator`; subscribe to events; assert `AdapterEvent::InboundConnection` fires on `session.invite`.

---

## 5. Phase 3 — `rvoip-webtransport` (WT substrate adapter)

Mirrors `rvoip-quic` line-for-line; the only differences are (a) the accept path goes through `web-transport-quinn` to do the HTTP/3 + WT extended `CONNECT` upgrade, and (b) the ALPN is `h3`. Everything below the WT session — TLS, QUIC, length-prefix framing on streams, 8-byte UCTP header on datagrams — is the **same code path** in `rvoip-uctp::substrate`.

### 5.1 Dependencies
```
rvoip-core.workspace = true
rvoip-uctp.workspace = true
tokio.workspace = true
async-trait.workspace = true
quinn.workspace = true
web-transport-quinn.workspace = true
thiserror.workspace = true
tracing.workspace = true
dashmap.workspace = true
chrono.workspace = true
bytes.workspace = true
parking_lot.workspace = true
```

### 5.2 Module layout

```
crates/rvoip-webtransport/src/
  lib.rs
  errors.rs                     -- UctpWtError
  server.rs                     -- UctpWtServer: wraps web-transport-quinn server, accepts WT sessions
  client.rs                     -- UctpWtClient: dials a WT URL (https://host:port/uctp)
  adapter.rs                    -- UctpWtAdapter: impl rvoip_core::ConnectionAdapter
  media_stream.rs               -- WebTransportDatagramMediaStream (per INTERFACE_DESIGN.md §3.6)
```

Config struct (mirrors §4.4's `UctpQuicConfig`; the `mount_path` field is the only WT-specific addition):

```rust
pub struct UctpWtConfig {
    pub endpoint: Arc<quinn::Endpoint>,                 // shared via substrate::quinn::dispatch_by_alpn (§3.7)
    pub accept_rx: mpsc::Receiver<quinn::Connecting>,   // ALPN-filtered stream (ALPN = "h3")
    pub server_tls: Arc<rustls::ServerConfig>,
    pub mount_path: String,                             // WT URL path the server accepts CONNECT on; default "/uctp"
    pub max_concurrent_connections: usize,              // default 1024
    pub idle_timeout: Duration,                         // default 30s
    pub bearer_validator: Arc<dyn rvoip_auth_core::BearerValidator>,
    pub pending_ttl: Duration,                          // default 30s
}
```

The same `ConnectionAdapter` method-split table from §4.4 applies verbatim — only `transport()` differs (returns `Transport::WebTransport`).

### 5.3 Why `web-transport-quinn` instead of `wtransport`

Both are mature WT libraries on quinn. **web-transport-quinn** is preferred for v0 because:
1. It works directly on a `quinn::Endpoint` — so `rvoip-quic` and `rvoip-webtransport` can literally share an endpoint instance.
2. It doesn't wrap quinn in its own types — fewer translation boundaries.
3. It's what MoQ chose (good precedent for an app-protocol-on-QUIC project of similar shape).

If we hit unmet needs (e.g., a feature only `wtransport` exposes), the swap is local to this crate.

### 5.4 Dual-ALPN single-endpoint deployment

The v0 demo's orchestrator process registers **one** `quinn::Endpoint` with both ALPNs:

```rust
let mut crypto = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(cert_chain, priv_key)?;
crypto.alpn_protocols = vec![b"uctp/1".to_vec(), b"h3".to_vec()];

let endpoint = rvoip_uctp::substrate::quinn::make_server_endpoint(addr, crypto, ..)?;

// Single accept task in rvoip-uctp::substrate::quinn reads ALPN and routes
// Connecting futures to per-adapter channels (see §3.7 dispatch_by_alpn).
let mut routes = rvoip_uctp::substrate::quinn::dispatch_by_alpn(
    Arc::clone(&endpoint),
    &[b"uctp/1", b"h3"],
)?;
let quic_accept_rx = routes.take(b"uctp/1").unwrap();
let wt_accept_rx   = routes.take(b"h3").unwrap();

let quic_adapter = UctpQuicAdapter::new(UctpQuicConfig {
    endpoint: Arc::clone(&endpoint),
    accept_rx: quic_accept_rx,
    .. /* see §4.4 config struct */
}).await?;
let wt_adapter = UctpWtAdapter::new(UctpWtConfig {
    endpoint: Arc::clone(&endpoint),
    accept_rx: wt_accept_rx,
    .. /* see §5.2 config struct */
}).await?;
orch.register(Arc::new(quic_adapter))?;
orch.register(Arc::new(wt_adapter))?;
```

**Why a single accept task — and not "each adapter calls `endpoint.accept()` and filters by ALPN".** `quinn::Endpoint::accept()` is single-consumer: it yields each incoming `Connecting` to exactly one caller, so two parallel accept loops on the same `Endpoint` race for each connection and the loser never sees it. The earlier draft of this plan proposed the "each adapter filters" approach; that's broken and is replaced here by the dispatcher. `dispatch_by_alpn` spawns one accept task that calls `connecting.handshake_data()` to learn the negotiated ALPN, then forwards the `Connecting` to the matching adapter's channel; connections with unrecognized ALPNs are closed with `ApplicationClose { error_code: 0x01, reason: b"alpn-not-registered" }`. The dispatcher is the right shape from day one and is no more code than the broken alternative.

WebTransport-side: the dispatcher forwards `Connecting` futures to `UctpWtAdapter`, which finishes the QUIC handshake, hands the `quinn::Connection` to `web_transport_quinn::Session::accept(...)` to perform the HTTP/3 + extended-`CONNECT` upgrade for `/uctp`, and from there uses the same envelope/datagram framing as the QUIC adapter.

### 5.5 Tests
- `tests/loopback.rs` — same shape as `rvoip-quic`'s loopback, but using a WT URL.
- `tests/adapter.rs` — same shape; `transport()` returns `Transport::WebTransport`.

---

## 6. Phase 4 — Integration & verification

### 6.1 Phase 4 dependencies

The Phase 4 bridge demo and integration test add:
```
rvoip-media-core.workspace = true   # codec transcoding (G.711 ↔ Opus) — used by the bridge frame-pump
rvoip-sip.workspace = true          # SIP side of the bridge
```

`rvoip-media-core` is the existing crate at `crates/media-core/`. The bridge frame-pump (manual in v0, replaced by `Orchestrator::bridge_connections` in v0.x — see §7) calls into `rvoip_media_core::codec::transcoding::Transcoder` when the two Connections' negotiated codecs differ. The actual signature today is:

```rust
// crates/media-core/src/codec/transcoding.rs
pub async fn transcode(
    &mut self,
    encoded_data: &[u8],
    from_codec: PayloadType,   // u8 RTP payload type: 0 = PCMU, 8 = PCMA, 18 = G.729, 111 = Opus
    to_codec: PayloadType,
) -> Result<Vec<u8>>;
```

Two things follow for the bridge pump: (1) there is no `Codec` enum — codec selection uses RTP payload-type numbers, so the SIP G.711-mu → UCTP Opus call is `transcoder.transcode(&frame.payload, 0u8, 111u8).await?`; (2) `&mut self` means each direction holds its own transcoder behind a `Mutex<Transcoder>` (or per-direction owned instance pinned to a single task), not a shared `Arc`. The supported transcoding paths come from `Transcoder::get_supported_paths()`; the v0 demo's SIP G.711-mu ↔ UCTP Opus path is included in the default set.

### 6.2 The demo: `crates/rvoip-uctp/examples/uctp_to_sip_bridge/`

Mirrors `crates/rvoip-core/examples/sip_only_orchestrator.rs` and extends it.

```
examples/uctp_to_sip_bridge/
├── README.md
├── orchestrator_bridge.rs        # binary 1 — central process
├── uctp_agent_quic.rs            # binary 2 — native CLI agent over raw QUIC
├── uctp_agent_wt.rs              # binary 3 — agent emulating a browser, over WT
└── sip_caller.rs                 # binary 4 — pretend SIP customer (rvoip-sip StreamPeer)
```

**Cargo discovery.** A subdir of `examples/` with multiple bare `.rs` files is **not** a layout `cargo run --example` recognizes — Cargo looks for either `examples/<name>.rs` or `examples/<name>/main.rs`. The plan commits to **explicit `[[example]]` entries**, which keeps the subdir grouping while allowing `cargo run --example <name>` to work. Add to `crates/rvoip-uctp/Cargo.toml`:

```toml
[[example]]
name = "orchestrator_bridge"
path = "examples/uctp_to_sip_bridge/orchestrator_bridge.rs"

[[example]]
name = "uctp_agent_quic"
path = "examples/uctp_to_sip_bridge/uctp_agent_quic.rs"

[[example]]
name = "uctp_agent_wt"
path = "examples/uctp_to_sip_bridge/uctp_agent_wt.rs"

[[example]]
name = "sip_caller"
path = "examples/uctp_to_sip_bridge/sip_caller.rs"
```

The flatten-to-top-level alternative was considered and rejected — the subdir grouping makes the four-binary demo legible at a glance, which matters for a doc-driven example.

**`orchestrator_bridge.rs`**:
1. Build shared `quinn::Endpoint` on `127.0.0.1:4433` with self-signed cert + ALPNs `[uctp/1, h3]`. Spawn the single `substrate::quinn::dispatch_by_alpn` accept task (§3.7) so both adapters get their own `mpsc::Receiver<Connecting>`.
2. Wrap in `UctpQuicAdapter` and `UctpWtAdapter` (each consumes its dispatcher channel; adapters do NOT call `endpoint.accept()` directly).
3. Build a SIP `UnifiedCoordinator` on `127.0.0.1:5072`, wrap in `SipAdapter`.
4. `let orch = Orchestrator::new(Config::default());`
5. `orch.register(Arc::new(quic_adapter))?; orch.register(Arc::new(wt_adapter))?; orch.register(Arc::new(sip_adapter))?;` — `Orchestrator::register` takes `Arc<dyn ConnectionAdapter>` (per `crates/rvoip-core/src/orchestrator.rs:90`); each adapter struct is wrapped at the call site.
6. Subscribe to `orch.subscribe_events()`. On `Event::ConnectionInbound` (SIP), originate a matching UCTP connection to whichever agent is registered. Manually pump streams between the two: SIP side delivers G.711-mu frames from `RtpMediaStream::frames_in()`; the bridge holds two `Mutex<Transcoder>` (one per direction) and calls `transcoder.lock().await.transcode(&frame.payload, 0u8, 111u8).await?` (PCMU → Opus); transcoded Opus frames go to the UCTP-side `MediaStream::frames_out()`. Reverse direction calls `transcode(&opus_bytes, 111u8, 0u8)`. (Manual pump goes away in v0.x when `Orchestrator::bridge_connections` lands — see §7.)
7. Write self-signed cert PEM to `/tmp/uctp_demo_cert.pem` so agent binaries can trust it.

**`uctp_agent_quic.rs`**: reads the orchestrator's cert from `/tmp/uctp_demo_cert.pem`, builds its `rustls::ClientConfig` via `rvoip_uctp::substrate::tls::dev_client_config_trusting(&cert)` (§3.7), dials raw QUIC at `127.0.0.1:4433` with ALPN `uctp/1`, runs the agent flow (auth, accept inbound invite, send media frames for 5s, end). The `dev-dangerous` feature is **not** enabled — the demo uses an explicit cert-pinning client config, not verification-disabled mode, so the example mirrors how a production agent would be configured (just with a different trust anchor).

**`uctp_agent_wt.rs`**: same cert-loading + `dev_client_config_trusting` setup; dials `https://127.0.0.1:4433/uctp` (WebTransport URL), runs the same agent flow. This binary is the stand-in for a browser client; a real browser would call the JS `new WebTransport(...)` API against the same URL and would trust the cert through the OS/browser trust store (or, in dev, a `serverCertificateHashes` constructor option keyed off the cert's SHA-256 — the README documents both paths). See §6.3.1 for the manual browser smoke test that exercises this.

**`sip_caller.rs`**: dials `sip:agent@127.0.0.1:5072` and plays a tone for 5s.

### 6.3 Integration test (automated) and browser smoke (manual)

This section has two parts: an automated `cargo test` smoke that runs both substrate paths in-process (§6.3.1), and a manual browser test that exercises a real browser's `WebTransport` API against the same orchestrator (§6.3.2). Both must pass before Phase 4 is declared complete.

### 6.3.1 Browser interop smoke (manual)

The `uctp_agent_wt` binary stands in for a browser in the automated test, but the v0 demo's "browser reach" claim (§1.1) needs at least one real browser session before the spike can be called done. Add to the demo tree:

```
examples/uctp_to_sip_bridge/
└── browser/
    ├── index.html          # opens new WebTransport(...) against the same /uctp URL
    ├── agent.js            # the WT handshake + a minimal UCTP envelope decoder for sanity
    └── README.md           # serve-via-python-http-server + chrome --origin-to-force-quic-on=... flags
```

`agent.js` does just enough to prove the WT path works end-to-end from a browser: opens the `WebTransport` session against `https://127.0.0.1:4433/uctp`, sends an `auth.hello` envelope on a bidi stream, awaits `auth.challenge`, responds with a bearer token, awaits `auth.session`, then closes. No media in v0 browser smoke — that requires the Web Audio API plumbing that's out of scope for this spike. The README documents the Chrome flags needed for the self-signed cert (`--ignore-certificate-errors-spki-list=<sha256>` or `--webtransport-developer-mode`) so a developer can reproduce the smoke in ~2 minutes.

A passing manual browser smoke is a gate on declaring Phase 4 complete; it does not need to be in CI (the cert-pinning ergonomics are too brittle for headless automation), but the README must be specific enough that any team member can reproduce it.

### 6.3.2 Automated integration test

`crates/rvoip-uctp/tests/bridge_smoke.rs`:
1. Spawn the orchestrator in-process on `127.0.0.1:0` (kernel-assigned ports).
2. Spawn a UCTP-QUIC agent + a UCTP-WT agent.
3. Spawn a SIP-caller side via `rvoip-sip::StreamPeer::call(...)`.
4. Bridge to the QUIC agent. Assert event sequence on `orch.subscribe_events()` (the orchestrator's user-facing `Event` enum per `INTERFACE_DESIGN.md` §5): `Event::ConnectionInbound` (SIP), `Event::ConnectionOutbound` (UCTP-QUIC), `Event::ConnectionConnected` × 2. (Note: the **adapter-internal** event stream uses `AdapterEvent::InboundConnection` / `Connected` / `Ended` / `Failed` / `Native` — see `crates/rvoip-core/src/adapter.rs:72-91`. The orchestrator normalizes those into the `Event::*` variants used here. When implementing Phase 2/3, the adapter emits `AdapterEvent::*`; when consuming events via `orch.subscribe_events()`, callers see `Event::*`.)
5. Inject one synthesized `MediaFrame` into the UCTP side via `MediaStream::frames_out().send(...)`; assert arrival on the SIP-side `RtpMediaStream::frames_in()` within 500 ms.
6. Repeat steps 3–5 for the WT agent.
7. Tear down: assert `Event::ConnectionEnded` × 2 per pair.

### 6.4 Verification commands

```bash
# 1. Workspace sanity.
cargo check --workspace

# 2. Per-crate unit + integration tests.
cargo test -p rvoip-uctp
cargo test -p rvoip-quic
cargo test -p rvoip-webtransport

# 3. End-to-end smoke (in-process, both substrates).
cargo test -p rvoip-uctp --test bridge_smoke -- --nocapture

# 4. The interactive 4-binary demo (manual).
cargo run -p rvoip-uctp --example orchestrator_bridge &
cargo run -p rvoip-uctp --example uctp_agent_quic &           # or
cargo run -p rvoip-uctp --example uctp_agent_wt &
cargo run -p rvoip-uctp --example sip_caller
```

---

## 7. Known tensions / gaps to revisit after v0

- **`rvoip_core::Orchestrator::bridge_connections` is stubbed** (orchestrator.rs:329). v0 example pumps frames manually. Closing this is an rvoip-core follow-up; INTERFACE_DESIGN.md §10.2 already specifies the `BridgeManager`/`BridgeHandle` shape.
- **Multi-party routing not implemented in v0.** Protocol supports it (CONVERSATION_PROTOCOL.md §7.7 + INTERFACE_DESIGN.md §10.6); implementation ships 1:1 only. `Orchestrator::add_subscription` / `remove_subscription` / `apply_subscriptions` and the per-Session routing table are v0.x rvoip-core work. v0 envelopes **parse** the new types (`stream.subscribe`, `stream.unsubscribe`, `stream.active-speaker`) so the wire format is stable; received `stream.subscribe` / `stream.unsubscribe` envelopes are rejected with `error` code `503` (`category: "transient"`, `reason: "multi-party-routing-not-implemented"`) until the routing layer lands. (`503` is the closest in-spec code per CONVERSATION_PROTOCOL.md §11.2; an earlier draft of this plan invented `501 not-implemented`, which is not defined in the spec. See the explicit spec-reconciliation action item at the end of this section.)
- **`MediaStream::frames_in/out` return owned channel ends per call.** v0 wraps them in `StdMutex<Option<...>>` (the pattern `SipAdapter::subscribe_events` already uses). A trait revision to return clones is out of scope for v0.
- **Auth in v0 is `bearer` stub with no validation.** `rvoip_auth_core::BearerValidator::stub()` returns `IdentityAssurance::Pseudonymous` for non-empty tokens. Step-up, DPoP, JWT, AAuth, RFC 9421 — all v0.x in `auth-core` (see §1.4). When real validators land they slot into the same trait; `rvoip-uctp` picks them up unchanged.
- **No vCon in v0** — deferred to v0.x in the new `rvoip-vcon` crate (see §1.4 roadmap). Consequence: v0 emits `RecordingComplete` with `vcon_ref: None` and does **not** emit `recording.vcon-ready`; the envelope parser still recognizes the type so the wire stays forward-compatible. Per INTERFACE_DESIGN.md §2 / §3.9 / §11.4 and CONVERSATION_PROTOCOL.md §7.6, vCon is v1-mandatory; v0 is explicitly a spike that ships before this work.
- **No DTMF, quality reports in v0** — wire types parse; adapters return `NotImplemented`. v0.x.
- **`rvoip-websocket` deferred to v0.x** — needs `webrtc-rs` for the co-located WebRTC PeerConnection that CONVERSATION_PROTOCOL.md §4.3 mandates for WS media (browsers without WT). The `webrtc-rs` integration (DTLS-SRTP, ICE, SDP munging) is its own significant work item; QUIC + WebTransport already cover the v0 demo's reach.
- **`noq` migration path** is open if mobile UCTP agents later require multipath/NAT traversal. Quinn API-compatible; deferred.
- **RoQ wire compatibility** (`draft-ietf-avtcore-rtp-over-quic-14`) is intentionally not pursued. UCTP §10.1's 8-byte header diverges from RoQ for multi-Connection multiplexing reasons. Future v1 may revisit if IETF adoption builds.
- **The `rvoip` facade's `lib.rs`** still references removed `rvoip-call-engine` / `rvoip-client-core` crates; out of scope for this plan but flagged.

### 7.1 Action items for the spec owner

The plan as written depends on the spec being internally consistent in two places where it currently is not. These need to be reconciled in `CONVERSATION_PROTOCOL.md` before v0.x ships routing, and the action is owned by the spec maintainer (not the rvoip-uctp implementer):

1. **§11.2 error-code catalog gap.** §16 references `505 version-not-supported` and §11.2's table does not list it. Either add `505` to the §11.2 table or remove the reference in §16. The plan currently uses only in-spec codes (§11.2 lists `400`, `401`, `403`, `404`, `408`, `487`, `488`, `500`, `503`); if `505` is added it should land before any v0 client tries to negotiate a future protocol version.
2. **`501 not-implemented` is missing.** The v0.x multi-party-routing rejection currently overloads `503 transient` because there is no code for "feature recognized but not implemented on this server." Adding `501 not-implemented` to §11.2 (with semantics "the server understands the envelope type but has not implemented the behavior; do not retry") would let v0.x stop overloading `503` and would also clean up the `NotImplemented` returns from `ConnectionAdapter` methods listed in §4.4. This is a small additive spec change; tracking it explicitly so the implementer doesn't have to relitigate the workaround later.

Plan §10 also surfaces these in the "plan-only choices" table; this section restates them as **assigned work for the spec owner** so they don't get lost between v0 ship and v0.x start.

---

## 8. Critical files (reference)

| Purpose | Path |
|---|---|
| UCTP wire spec (authoritative) | `crates/rvoip-core/CONVERSATION_PROTOCOL.md` |
| rvoip architecture (authoritative) | `crates/rvoip-core/INTERFACE_DESIGN.md` |
| voip-3 terminology | `crates/rvoip-core/voip-3-conversation-model.md` |
| `ConnectionAdapter` trait | `crates/rvoip-core/src/adapter.rs` |
| `Transport` enum | `crates/rvoip-core/src/connection.rs` |
| Template adapter to mirror | `crates/rvoip-sip/src/adapter.rs` |
| Template example to mirror | `crates/rvoip-core/examples/sip_only_orchestrator.rs` |
| Closest QUIC-app-protocol precedent (study) | `kixelated/moq/rs/moq-native/src/` (server.rs, client.rs) |
| Workspace manifest (edit to add crates + deps) | `Cargo.toml` |

---

## 9. Phase ordering

1. **Phase 0** — workspace + Cargo.toml (§2.1), verify Transport enum (§2.2), extend `rvoip-core::CapabilityDescriptor` per §2.3, add `VconRef` placeholder per §2.4, introduce `auth-core::BearerValidator` per §2.5, per-crate skeletons (§2.6). **Four PRs** — see expanded breakdown below.
2. **Phase 1** — `rvoip-uctp` shared crate (envelopes, types, capability negotiation, state machine, `substrate::quinn` helpers, correlation primitive, `auth-core` integration for `bearer_stub()`, observability per §3.9). Tests pass standalone. (1 large PR or split by sub-module.)
3. **Phase 2 + 3 in parallel** — `rvoip-quic` and `rvoip-webtransport` adapter crates. Each consumes the same shared helpers; the diff between them is ~adapter glue + accept loop differences. (2 PRs, parallelizable.)
4. **Phase 4** — integration test + demo binaries. The four-binary demo demonstrates the cross-transport bridging end-to-end.

### Phase 0 PR breakdown (four PRs)

| # | PR | Touches | Blocks |
|---|---|---|---|
| 0a | Workspace + crate scaffolding | Root `Cargo.toml` (§2.1), `crates/rvoip-uctp/` `Cargo.toml` + `src/lib.rs` stub, new `crates/rvoip-quic/` and `crates/rvoip-webtransport/` directories with `Cargo.toml` + `src/lib.rs` stubs (§2.6). Adds `rvoip-auth-core` and `metrics` to `[workspace.dependencies]`. | All later PRs |
| 0b | `rvoip-core::CapabilityDescriptor` extension | `crates/rvoip-core/src/capability.rs` — seven new fields + four enums (§2.3, §3.4); converts `supports_dtmf_rfc4733` from field to method (§3.4); spec-§8 JSON round-trip test. | Phase 1 (needed for negotiation) |
| 0c | `rvoip-core::VconRef` placeholder | New `crates/rvoip-core/src/recording.rs` (or extends existing recording types) with `VconRef` enum + `Option<VconRef>` field on `RecordingComplete` (§2.4). | Phase 1 (needed for `recording.complete` envelope payload) |
| 0d | `auth-core::BearerValidator` | New `crates/auth-core/src/bearer.rs` (§2.5); `lib.rs` re-exports. | Phase 1 (needed for `UctpCoordinator` to compile) |

0b, 0c, and 0d are independent of one another and can ship in any order after 0a lands. Phase 1 starts only after all four are merged.

---

## 10. Differences from INTERFACE_DESIGN.md (resolved + remaining)

INTERFACE_DESIGN.md is the architectural source of truth. The places where the plan and the design doc were out of sync were surfaced; the load-bearing items were folded back into INTERFACE_DESIGN.md so the plan and design stay aligned.

### Resolved in INTERFACE_DESIGN.md

| Topic | Resolution |
|---|---|
| **UCTP envelope-level state machine placement** | INTERFACE_DESIGN.md §2 now lists the state machine and shared substrate helpers as `rvoip-uctp` responsibilities. Plan §3.5 implements it there. |
| **Single quinn::Endpoint, dual ALPN** | INTERFACE_DESIGN.md §2.3 now documents that `rvoip-quic` and `rvoip-webtransport` may share one `quinn::Endpoint`. Plan §5.4 deploys exactly this shape. |
| **v0 spike scope vs production scope** | INTERFACE_DESIGN.md §2.4 now contains a v0-vs-production feature matrix; the plan's deferrals (vCon, identity gradient, RFC 9421, DTMF, quality, `rvoip-websocket`, manual frame-pump) are listed there. |
| **RoQ explicit non-goal** | INTERFACE_DESIGN.md §3.6 now contains a paragraph stating UCTP datagram format is intentionally not RoQ-compatible and citing the rationale. |

### Items where the plan corrects itself, not the design doc

| Topic | Note |
|---|---|
| **`CapabilityDescriptor` field set + typed enums** | The Rust struct at `crates/rvoip-core/src/capability.rs:11-18` is narrower than CONVERSATION_PROTOCOL §8 / INTERFACE_DESIGN §9. **Action:** pinned in plan §2.3 as a Phase 0 sub-task — add the seven `#[serde(default)]` fields **plus** the four typed enums (`DataProtocol`, `DtmfMode`, `TransportFeature`, `IdentityAssuranceRequirement`) enumerated in §3.4 to `rvoip_core::CapabilityDescriptor`. The plan deliberately follows INTERFACE_DESIGN.md §9's typed-enum shape rather than the looser `Vec<String>` shape an earlier draft used, so the Rust API can't drift from the spec catalog. No `UctpCapabilityDescriptor` fork. |
| **`Transport::Uctp` enum variant** | An earlier plan draft proposed adding this. INTERFACE_DESIGN.md §3.5 is clear that the Transport tag is the **substrate** (`Transport::Quic`, `Transport::WebTransport`), not the application protocol. Plan now uses existing variants. |
| **Manual frame-pumping in v0 demo** | `rvoip_core::Orchestrator::bridge_connections` is stubbed today (orchestrator.rs:329); INTERFACE_DESIGN.md §10.2 already specifies the `BridgeManager` shape. v0 demo pumps manually as a workaround. Closing the gap is a separate rvoip-core PR, scheduled in v0.x (see §1.4). |
| **Shared-endpoint accept dispatcher in v0** | An earlier plan draft proposed "each adapter calls `endpoint.accept()` and filters by ALPN." That's broken: `quinn::Endpoint::accept()` is single-consumer, so two parallel loops race for each connection. Plan §3.7 + §5.4 now commit to a single `substrate::quinn::dispatch_by_alpn` accept task that fans `Connecting` futures out to per-adapter mpsc channels — the only correct shape for the dual-ALPN shared endpoint that INTERFACE_DESIGN.md §2.3 commits to. |
| **`MediaStream` impl naming** | Plan uses `QuicDatagramMediaStream` / `WebTransportDatagramMediaStream` per INTERFACE_DESIGN.md §3.6 — no divergence. |
| **Shared crate naming: `rvoip-media` vs `rvoip-media-core`, `rvoip-identity` vs `auth-core`** | INTERFACE_DESIGN.md §2 lists prospective names `rvoip-media`, `rvoip-rtp`, `rvoip-identity`. The crates that exist today are `rvoip-media-core` (`crates/media-core/`), `rvoip-rtp-core` (`crates/rtp-core/`), and `rvoip-auth-core` (`crates/auth-core/`). The plan uses the **actual** package names so Cargo.toml entries are correct. **Action:** the rename to the design-doc names is a separate cross-workspace project; out of scope for this plan but flagged so a future reader knows the inconsistency is known. |

### Plan-only choices (no design doc impact)

| Topic | Note |
|---|---|
| **`web-transport-quinn` over `wtransport`** | Picked because it shares a `quinn::Endpoint` cleanly with `rvoip-quic`. Implementation choice; INTERFACE_DESIGN.md needn't take a position. |
| **Spec §11.2 error-code catalog inconsistency** | CONVERSATION_PROTOCOL.md §11.2 lists the canonical error codes, but §16 references `505 version-not-supported` and v0.x routing work would benefit from a `501 not-implemented` code — neither in the §11.2 table. Plan §7 uses in-spec codes only (`503` for the multi-party rejection case) and flags the inconsistency for the spec owner to reconcile. |

---

The plan is consistent with INTERFACE_DESIGN.md, PRD.md, and CONVERSATION_PROTOCOL.md as a v0-spike-scoped document. v0.x scope is enumerated in §1.4. Ready to execute Phase 0.
