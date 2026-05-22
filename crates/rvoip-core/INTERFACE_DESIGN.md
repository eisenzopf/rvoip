# rvoip — Library Interface and Language Design

**Status:** v1 draft. Companion to `PRD.md` (which sets scope and product role) and `CONVERSATION_PROTOCOL.md` (which specifies the wire/SDK protocol). This doc specifies the **vocabulary, abstractions, and contract surface of the rvoip Rust library** — the library you build a UCTP-speaking gateway with.

**Goal:** Define the rvoip library so that:
- It implements the Universal Conversation Transport Protocol (UCTP) server-side natively over QUIC, WebTransport, and WebSocket substrates.
- It bridges SIP and WebRTC clients into the same Sessions via gateway adapters.
- Per-protocol developers (SIP, WebRTC) can still use rvoip's per-protocol surfaces directly without learning UCTP.
- Thelve, the canonical consumer, can build its server on top with minimal glue.

**Source of truth for terminology:** `/Users/jonathan/Developer/Rudeless/voip-3-conversation-model.md`. This document adopts voip-3 nouns end-to-end.

---

## 1. The three-layer architecture

```
APPLICATION (mobile app, web app, desktop app, embedded device, AI agent)
   │ uses an SDK that speaks UCTP
   ▼
UCTP — Universal Conversation Transport Protocol (specified in CONVERSATION_PROTOCOL.md)
   │ travels over a substrate
   ▼
─── rvoip library boundary ───
   │
   │  rvoip-core (this document) implements UCTP server-side and provides:
   │    • Conversation / Session / Connection / Stream / Message / Participant types
   │    • ConnectionAdapter trait — substrate adapters and interop adapters
   │    • Capability negotiation, bridging, identity, persistence trait surfaces
   │    • Orchestrator entry point
   │
   ▼
SUBSTRATES (UCTP-native) and INTEROP (UCTP-foreign)
   ┌───────────────────────────────────────────────────┐
   │  rvoip-quic        — QUIC substrate adapter        │
   │  rvoip-webtransport — WT substrate adapter         │
   │  rvoip-websocket   — WS substrate adapter          │
   │  rvoip-sip         — SIP interop adapter (gateway) │
   │  rvoip-webrtc      — WebRTC interop adapter (gw)   │
   └───────────────────────────────────────────────────┘
```

Two kinds of `ConnectionAdapter` (§6):
- **Substrate adapters** — speak UCTP natively (QUIC, WebTransport, WebSocket). The Connection's transport is UCTP.
- **Interop adapters** — gateway UCTP intent into protocol-native operations (SIP method calls, WebRTC API calls). The Connection's transport is the foreign protocol.

Both kinds are interchangeable from rvoip-core's perspective. The Session does not care whether a Participant joined via QUIC or via SIP; it just sees a Connection with a transport tag.

### 1.1 Vocabulary commitment

rvoip-core uses voip-3 nouns as its public type surface:

`Conversation`, `Session`, `Message`, `Participant`, `Connection`, `Stream`, `Identity`, `Device`.

Per-adapter crates expose their native vocabulary on top:
- `use rvoip::sip::*` — `Call`, `Dialog`, `INVITE`, `REFER`, `Registration`. SIP developers see SIP.
- `use rvoip::webrtc::*` — `PeerConnection`, `Offer`, `Answer`, `IceCandidate`. WebRTC developers see WebRTC.
- `use rvoip::uctp::*` — `Envelope`, `EnvelopeType`, `ReachabilityHint`. UCTP developers see UCTP.
- `use rvoip::*` (top-level facade) — voip-3 nouns. The unifying surface.

A SIP-only carrier never needs to learn UCTP. A UCTP-only client never needs to learn SIP. Only the gateway operator (Thelve) sees all of it. This is the §1 rvoip principle.

### 1.2 Naming-collision notes

- **`Session`** collides with `session-core` (existing crate). Mitigation: `session-core` is renamed and absorbed into `rvoip-sip` as part of the migration (see §13).
- **`Connection`** collides with QUIC's transport-layer "connection." A UCTP/voip-3 `Connection` is application-level (a Participant's attach to a Session) and may run over one or more QUIC connections. Documented prominently in `rvoip-quic`.
- **`Stream`** collides with `tokio::Stream` (a futures trait) and QUIC streams. The Rust type is **`MediaStream`** even though the conceptual noun is "Stream." `Stream` remains the user-facing word in docs and `cp::*` envelopes.

---

## 2. Crate layout

```
rvoip                         facade crate; re-exports rvoip-core + adapters as a single
                              `use rvoip::*` surface; feature flags select which adapters compile in
├── rvoip-core                neutral abstractions: Conversation, Session, Connection, Stream,
│                             Message, Participant, Identity, Device, IdentityAssurance,
│                             commands, events, ConnectionAdapter trait, MediaStream trait,
│                             ConversationStore trait, VconStore trait, Orchestrator entry point
├── rvoip-uctp                  UCTP wire implementation: envelope encode/decode, substrate framing,
│                             capability negotiation algorithm, error model
├── rvoip-quic                UCTP substrate adapter — QUIC streams + datagrams
├── rvoip-webtransport        UCTP substrate adapter — WT streams + datagrams
├── rvoip-websocket           UCTP substrate adapter — WS text frames + co-located WebRTC for media
├── rvoip-sip                 SIP interop adapter; absorbs current dialog-core, sip-transport,
│                             registrar-core, session-core (the SIP B2BUA surface lives here)
├── rvoip-webrtc              WebRTC interop adapter; ICE, DTLS-SRTP, SDP munging,
│                             peer-connection lifecycle
├── rvoip-media               transport-agnostic media: codec, mixing, audio processing,
│                             MediaStream trait, transcoding pairs (G.711 ↔ Opus, etc.)
├── rvoip-rtp                 RTP/SRTP-specific transport (used by rvoip-sip and rvoip-webrtc)
├── rvoip-vcon                FIRST Rust implementation of the IETF vCon spec
│                             (draft-ietf-vcon-vcon-core). Builder pattern, serde-based,
│                             JWS sign/verify, JWE encrypt/decrypt, voip-3 → vCon adapter,
│                             feature-gated SIP-signaling and consent extensions
├── rvoip-identity            IdentityProvider trait + verifier implementations:
│                             OAuth 2.1+DPoP (default), OIDC, SIP Digest, FIDO/passkeys,
│                             AAuth (experimental, RFC 9421 HTTP Message Signatures).
│                             IdentityAssurance gradient types. DTLS-SRTP fingerprint binding
│                             (feature-flagged)
├── rvoip-harness             AI voice harness (ASR, TTS, dialog providers, recording sinks).
│                             Separate crate so SIP-only carriers don't pull provider deps.
├── rvoip-client              Client SDK: thin `Client` type for single-Identity, single-tenant
│                             apps (mobile, web, desktop, embedded). Handles auth, reachability,
│                             and active Conversations / Sessions. Re-exports per-protocol
│                             native client surfaces from rvoip-sip / rvoip-webrtc / rvoip-uctp
│                             so developers can mix the unifying `Client` with native types.
│                             See §15.
└── rvoip-sms (later)         SMS/MMS interop adapter via SMPP or carrier APIs
```

### 2.1 Dependency direction

- `rvoip-core` depends on `rvoip-media` and `rvoip-uctp` only.
- Each adapter depends on `rvoip-core`, `rvoip-uctp`, `rvoip-media`, and a transport-specific crate (`rvoip-rtp` for SIP/WebRTC, `quinn` for QUIC, `tungstenite` for WebSocket, etc.).
- `rvoip-core` **never** imports an adapter crate. Enforced via `cargo deny` (§18).

### 2.2 Feature flags on the facade

The `rvoip` facade is feature-flagged so consumers compile only what they need:

| Feature | Pulls in | Use case |
|---|---|---|
| `uctp` | rvoip-core, rvoip-uctp, rvoip-quic, rvoip-webtransport, rvoip-websocket, rvoip-media | UCTP-native server (lightest UCTP gateway) |
| `sip` | rvoip-core, rvoip-sip, rvoip-rtp, rvoip-media | Pure SIP carrier (no UCTP) |
| `webrtc` | rvoip-core, rvoip-webrtc, rvoip-rtp, rvoip-media | Pure WebRTC SFU-bridge use cases |
| `vcon` | rvoip-vcon | vCon emission, signing, encryption (default-on; vCons are emitted for every Session) |
| `identity` | rvoip-identity | Identity backends (default-on; OAuth 2.1+DPoP minimum) |
| `aauth-experimental` | rvoip-identity[aauth] | Enables the AAuth backend (RFC 9421 + Signature-Key headers); off-by-default |
| `identity-fingerprint-binding` | rvoip-identity[fingerprint] | DTLS-SRTP fingerprint binding from Identity signing keys; off-by-default |
| `harness` | rvoip-harness | Optional AI runtime |
| `client` | rvoip-client | Client SDK for mobile / web / desktop / embedded apps (additive; server-side `Orchestrator` is unaffected) |
| `full` | all of the above (incl. aauth-experimental and identity-fingerprint-binding) | Thelve-shaped deployment |

Default features: `[uctp, sip, rtp, media, vcon, identity]` — the UCTP + SIP-bridge minimum plus vCon emission and the standards-track identity backends. WebRTC, AAuth, and DTLS-SRTP fingerprint binding are opt-in. Full Thelve uses `full`.

---

## 3. Core abstractions

The rvoip-core layer borrows voip-3's six-noun model directly. Concepts are named neutrally so SIP, WebRTC, and UCTP developers all recognize them.

### 3.1 `Conversation`

The durable cross-channel container. May span Sessions and Messages over time.

```rust
pub struct Conversation {
    pub id: ConversationId,
    pub tenant_id: TenantId,
    pub state: ConversationState,        // Open | Closed
    pub policy: ConversationPolicy,      // Ephemeral { idle_close_secs } | Persistent
    pub participants: Vec<Participant>,
    pub sessions: Vec<SessionId>,        // ordered by start time
    pub messages: Vec<MessageId>,        // ordered
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub metadata: HashMap<String, String>,
}
```

Persistence is plug-in via `ConversationStore` (§11). Default in-memory store ships with rvoip-core.

When rvoip is used standalone, the Conversation is the durable record of "this engagement happened." When rvoip is embedded in Thelve, the rvoip Conversation shares its `id` with a Thelve `Interaction` (or is bound to one of Thelve's `ChannelSessions`).

### 3.2 `Session` *(new layer relative to prior INTERFACE_DESIGN)*

A bounded synchronous engagement within a Conversation. Several Sessions may occur within one Conversation over time.

```rust
pub struct Session {
    pub id: SessionId,
    pub conversation_id: ConversationId,
    pub state: SessionState,             // Initiating | Active | Ending | Ended | Failed
    pub medium: SessionMedium,           // Voice | Video | VoiceVideo | ScreenShare | TextChat | Mixed
    pub participants: HashSet<ParticipantId>,
    pub connections: HashMap<ConnectionId, ConnectionRef>,
    pub negotiated_capabilities: CapabilityIntersection,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub end_reason: Option<EndReason>,
}

pub enum SessionState { Initiating, Active, Ending, Ended, Failed }
```

**Session boundary rule (commits the voip-3 open question):** A Session is `Active` while ≥1 Connection is `Connected` OR a `connection.offer`/`answer` is in flight. When the last Connection ends and no negotiation is in flight, a grace window starts (default 30s, configurable per Session). Reconnects within the window are new Connections in the same Session; reconnects after are a new Session.

The Session owns:
- Presence-of-participants set (who is currently in the Session).
- Codec/capability negotiation result (what was agreed).
- Recording/transcription attachment point (per-Session, applied to one or more Connections).

### 3.3 `Message`

An asynchronous atomic communication event within a Conversation. Independent of Sessions.

```rust
pub struct Message {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    pub origin: MessageOrigin,           // Connection(ConnectionId) | System | Ai(ParticipantId)
    pub from_participant: ParticipantId,
    pub to: MessageRecipients,           // All | Participants(Vec<ParticipantId>)
    pub direction: Direction,            // Inbound | Outbound
    pub content_type: ContentType,       // Text | Json | Binary | Image | Audio | Attachment(url)
    pub body: Bytes,
    pub attachments: Vec<Attachment>,
    pub in_reply_to: Option<MessageId>,
    pub timestamp: DateTime<Utc>,
}
```

Voice and messaging are peers in rvoip-core. A consumer may build a voice-only product (no messaging), a messaging-only product (no voice), or unified (both).

### 3.4 `Participant`

The actor on one or more Connections within a Conversation.

```rust
pub struct Participant {
    pub id: ParticipantId,
    pub conversation_id: ConversationId,
    pub identity_ref: Option<IdentityId>,    // points to durable Identity if known
    pub kind: ParticipantKind,               // Human | Ai | System | External
    pub role: ParticipantRole,               // Customer | Agent | Supervisor | Observer | Custom(String)
    pub display_name: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,
}
```

rvoip does **not** model skills, presence, capabilities, or workforce — those are the consumer's. rvoip just knows there's a Participant on a Connection.

### 3.5 `Connection` *(replaces "Leg")*

A single Participant's transport-bound attach to a Session. Each Connection lives on one transport (substrate or interop) and is the boundary where rvoip-core meets a `ConnectionAdapter`.

```rust
pub struct Connection {
    pub id: ConnectionId,
    pub session_id: SessionId,
    pub participant_id: ParticipantId,
    pub transport: Transport,            // Quic | WebTransport | WebSocket | Sip | WebRtc | InProcessAi
    pub direction: Direction,            // Inbound | Outbound
    pub state: ConnectionState,          // Connecting | Connected | Held | Ending | Ended | Failed
    pub capabilities: CapabilityDescriptor,
    pub negotiated_codecs: NegotiatedCodecs,
    pub streams: Vec<MediaStreamHandle>, // present if Connection carries voice/video
    pub messaging_enabled: bool,         // present if Connection carries messaging
    pub transport_handle: TransportHandle, // opaque; resolves via the adapter
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

pub enum ConnectionState { Connecting, Connected, Held, Ending, Ended, Failed }
```

A Connection can carry voice, messaging, both, or neither (a control-only Connection is unusual but legal).

### 3.6 `MediaStream` (the "Stream" noun, in code)

The transport-agnostic media flow. Defined as a trait so each adapter implements it for its transport.

```rust
pub trait MediaStream: Send + Sync {
    fn id(&self) -> StreamId;
    fn kind(&self) -> StreamKind;             // Audio | Video | Data
    fn codec(&self) -> CodecInfo;
    fn direction(&self) -> Direction;

    /// Receiver for incoming frames. Channel-based to avoid per-frame async-await overhead.
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame>;

    /// Sender for outgoing frames.
    fn frames_out(&self) -> mpsc::Sender<MediaFrame>;

    fn quality_snapshot(&self) -> QualitySnapshot;

    async fn close(self: Arc<Self>) -> Result<()>;
}
```

Implementations:
- `RtpMediaStream` (in rvoip-rtp) — RTP over UDP for SIP and WebRTC interop.
- `QuicDatagramMediaStream` (in rvoip-quic) — RTP-in-QUIC-datagram per UCTP §10.
- `WebTransportDatagramMediaStream` (in rvoip-webtransport) — RTP-in-WT-datagram per UCTP §10.
- `DtlsSrtpMediaStream` (in rvoip-webrtc) — DTLS-SRTP for WebRTC interop.

Bridging works against this trait — see §10.

**Why channel-based, not per-frame `async fn recv_frame()`:** RTP at 50fps × N Streams × M codecs in a 10k-call server is an enormous number of awaits per second. A channel-based shape lets each adapter feed a `mpsc::Sender<MediaFrame>` from its own task and the bridge task pumps `Receiver → Sender` pairs at minimal overhead.

### 3.7 `Identity` and `Device`

Durable real-world entity and physical/software endpoint. Modeled as plain types; the **provider** (auth, lookup, reachability, signature verification) is plug-in via `IdentityProvider` (§8).

```rust
pub struct Identity {
    pub id: IdentityId,
    pub display_name: Option<String>,
    pub kind: IdentityKind,            // Human | Ai | Service | System  (per voip-3 §3.4 Participant kinds; Identity kinds mirror them at the durable level)
    pub external_refs: HashMap<String, String>, // arbitrary refs into Thelve / CRM / etc.
    pub signing_keys: Vec<Jwk>,        // public keys associated with this Identity (for AAuth, DPoP, DTLS binding)
    pub assurance: IdentityAssurance,  // current attestation level (per §3.8)
}

pub struct Device {
    pub id: DeviceId,
    pub identity_id: IdentityId,
    pub kind: DeviceKind,              // Mobile | Web | Desktop | Embedded | Server
    pub platform: String,              // "ios" | "android" | "browser-chrome-122" | ...
    pub registered_at: DateTime<Utc>,
    pub device_signing_key: Option<Jwk>,  // optional per-device key (separate from Identity-level keys)
}
```

### 3.8 `IdentityAssurance`

The identity gradient — one type that all identity backends translate their primitives into. Adopted from Dick Hardt's AAuth thinking; reusable regardless of which backend implements it.

```rust
pub enum IdentityAssurance {
    /// No identity claimed. Treated as a stranger; subject to anonymous-rate limits and policy.
    Anonymous,

    /// An ephemeral keypair the peer can re-prove ownership of, but which is not bound
    /// to any durable Identity. Useful for short-lived session continuity without disclosing identity.
    Pseudonymous { ephemeral_key: Jwk },

    /// A durable Identity has been authenticated (the subject is who they say they are)
    /// but no specific authorization has been granted for this action.
    Identified { credential_kind: CredentialKind },

    /// A durable Identity, plus a task-scoped delegation: this token may take this specific
    /// action on this specific resource, expiring at `expires_at`.
    TaskScoped {
        identity: IdentityId,
        task_id: String,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
    },

    /// Full user-authorized identity: this Identity acts on behalf of `user_id` with `scopes`.
    /// Highest assurance; required for sensitive operations like recording with PII.
    UserAuthorized {
        identity: IdentityId,
        user_id: IdentityId,
        scopes: Vec<String>,
    },
}

pub enum CredentialKind {
    OAuth2Dpop,
    Oidc,
    SipDigest,
    Passkey,
    AAuth,        // experimental
}
```

Sessions and tenants may require a minimum assurance level (per §9). When a Connection's assurance is below the Session's required minimum, it is rejected with `403 Forbidden-For-Assurance`. When two Connections of different assurance are bridged, the effective Session assurance is the lower of the two; the vCon `parties[]` records each Participant's assurance level individually.

### 3.9 In-flight `Vcon` builder

Every Session has an associated in-flight vCon builder that is populated as the Session progresses. This is owned by the Session and accessible to harness/transcription via a handle:

```rust
impl Session {
    /// Returns a handle for writing into the in-flight vCon.
    /// The vCon is finalized (signed and emitted to VconStore) at session.ended.
    pub fn vcon_handle(&self) -> VconBuilderHandle;
}

/// The handle exposes a narrow append-only API:
pub trait VconBuilderHandle: Send + Sync {
    fn add_party(&self, party: VconParty);                       // on ParticipantJoined
    fn add_dialog(&self, dialog: VconDialog);                    // on Stream open/close, transfer
    fn add_analysis(&self, analysis: VconAnalysis);              // on transcript, sentiment, summary
    fn add_attachment(&self, attachment: VconAttachment);        // on SIP signaling, STIR cert, consent
    fn snapshot(&self) -> VconSnapshot;                          // read-only view (for debugging)
}
```

Lifecycle:
- **On `ConversationOpened`** the Conversation gets a `group` UUID; all its Sessions' vCons share that group (the vCon spec's mechanism for linking related vCons).
- **On `ParticipantJoined`** rvoip-core appends a `Party` populated from the Participant's Identity (name, did/stir if present, `validation` reflecting `IdentityAssurance`).
- **On Connection lifecycle** rvoip-core appends a `Dialog` per audio Stream + per text Stream + per transfer event. SIP signaling is captured into Attachments via `rvoip-sip` (when the SIP-signaling vCon extension is enabled).
- **On harness/transcription events** the harness or transcription pipeline appends `Analysis` entries (transcripts with confidence, dialog-turn summaries, sentiment).
- **On `SessionEnded`** rvoip-core finalizes the vCon: validates structure, signs via JWS using the tenant key, optionally encrypts via JWE if a tenant encryption key is configured, persists via `VconStore` (§11), and emits `VconReady` event with a `VconHandle`.

---

## 4. Commands (consumer → rvoip-core)

Commands are issued by the consumer (Thelve, a CPaaS, a call center app, or directly by the SDK on behalf of a client). rvoip-core dispatches each to the right adapter via the `transport` tag on the Connection.

| Command | Purpose |
|---|---|
| `OpenConversation` | Open an explicit Conversation with a policy and metadata |
| `CloseConversation` | End a Conversation (only after Sessions ended unless force=true) |
| `RouteInboundConnection` | Bind an inbound Connection (any transport) to a Session. The adapter has already received the protocol-level invite/INVITE/offer; this is the consumer telling rvoip what to do with it. |
| `OriginateConnection` | Originate an outbound Connection on a chosen transport, optionally as part of an existing Session or starting a new one |
| `StartSession` | Explicitly start a Session (issues `session.invite` to invitees over their preferred Connections) |
| `EndSession` | End all Connections in a Session and close the Session |
| `JoinSession` | Add a Participant to an active Session (issues invites; new Connection on accept) |
| `LeaveSession` | Remove a Participant from an active Session (ends their Connection(s) but Session continues if others remain) |
| `BridgeConnections` | Bridge two Connections' media (1:1 relay; transport-agnostic) |
| `UnbridgeConnections` | Tear down a bridge without ending the Connections |
| `TransferConnection` | Transfer a Connection out of one Session (to a URI / endpoint / another Session). Blind / attended / external. |
| `AttachAi` | Attach an in-process AI runtime to a Connection (in-process AI mode; see PRD §11) |
| `AttachListener` | Attach a tap (AI or recorder) to a Connection or Session |
| `Detach` | Remove an attached AI runtime or listener |
| `EndConnection` | End a single Connection with a reason |
| `Hold` / `Resume` | Per-Connection media-direction control |
| `Mute` / `Unmute` | Per-Connection, per-direction mute |
| `SendMessage` | Send a `Message` on a Connection (or directly on a Conversation if no Session) |
| `SendDtmf` | Play DTMF on a Connection that carries audio |
| `PlayAudio` | Play an audio source (URL or TTS request) on a Connection |
| `StartRecording` / `StopRecording` | Toggle audio capture on a Connection or Session |
| `StartTranscription` / `StopTranscription` | Toggle ASR on a Connection or Session |
| `PauseRecording` / `ResumeRecording` | Transient suppression |
| `RenegotiateMedia` | Change codec or stream set on an existing Connection |

All commands carry `tenant_id` and `correlation_id`. The transport-tag on the affected Connection tells rvoip-core which adapter to dispatch to; the adapter performs the protocol-native action.

---

## 5. Events (rvoip-core → consumer)

Events are emitted on `infra-common::events::GlobalEventCoordinator` (per PRD §10).

| Event | Notes |
|---|---|
| `ConversationOpened` / `ConversationClosed` | Lifecycle |
| `SessionStarted` / `SessionEnded` / `SessionFailed` | Session lifecycle (separate from Conversation) |
| `ConnectionInbound` | A new inbound Connection arrived on some adapter; awaits routing |
| `ConnectionOutbound` | An outbound Connection started (in response to OriginateConnection) |
| `ConnectionConnected` | Connection reached `Connected` state |
| `ConnectionProgress` | Early states (ringing, busy, no-answer, machine, human-answered) |
| `ConnectionEnded` / `ConnectionFailed` | Terminal events with structured reason |
| `ConnectionsBridged` / `ConnectionsUnbridged` | Bridge state changes |
| `ConnectionTransferred` | Transfer completed (with type and target) |
| `ParticipantJoined` / `ParticipantLeft` | Per-Session participant changes |
| `AiAttached` / `AiDetached` | In-process AI runtime lifecycle |
| `ListenerAttached` / `ListenerDetached` | Tap lifecycle |
| `MessageReceived` / `MessageSent` / `MessageDelivered` / `MessageRead` | Messaging events |
| `TranscriptTurn` | Per-turn ASR result with `stream_id`, `speaker`, `text`, `confidence`, `is_final`, `assigned_provider` |
| `RecordingStarted` / `RecordingStopped` / `RecordingComplete` | Recording lifecycle with sink reference |
| `VconReady` | Emitted when the in-flight vCon for a Session is finalized, signed, and persisted to `VconStore`. Carries the `VconHandle` (URL + content hash) for retrieval. Emitted for every Session at end-of-Session per §3.9. |
| `VconRedacted` | A new redacted vCon was produced from an existing one; carries both old and new `VconHandle`s. |
| `IdentityAssuranceChanged` | A Connection's IdentityAssurance level changed mid-Session (e.g., user stepped up from Pseudonymous to Identified via passkey challenge). |
| `DtmfReceived` | DTMF from far end |
| `RegistrationChanged` / `RegistrationHeartbeat` | Per PRD §10 (rvoip-sip and rvoip-uctp emit these from their registrars / auth services) |
| `CapacityReport` | Periodic per-tenant + global utilization |
| `UsageRecord` | Billing-grade raw usage (separate channel) |
| `Anomaly` | Quality / fraud signal for the consumer to evaluate |
| `MediaQuality` | Periodic per-Connection quality snapshot |

All events carry `tenant_id`, `conversation_id` (where applicable), `session_id` (where applicable), `connection_id` (where applicable), `correlation_id`, and `timestamp`.

---

## 6. The adapter contract

Each adapter crate (rvoip-quic, rvoip-webtransport, rvoip-websocket, rvoip-sip, rvoip-webrtc) implements `ConnectionAdapter` toward rvoip-core. This is what makes the layering work.

```rust
#[async_trait]
pub trait ConnectionAdapter: Send + Sync {
    fn transport(&self) -> Transport;
    fn kind(&self) -> AdapterKind;       // Substrate | Interop

    async fn originate(
        &self,
        request: OriginateRequest,
    ) -> Result<ConnectionHandle>;

    async fn accept(&self, conn: ConnectionId) -> Result<()>;
    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> Result<()>;
    async fn end(&self, conn: ConnectionId, reason: EndReason) -> Result<()>;
    async fn hold(&self, conn: ConnectionId) -> Result<()>;
    async fn resume(&self, conn: ConnectionId) -> Result<()>;
    async fn transfer(&self, conn: ConnectionId, target: TransferTarget) -> Result<()>;

    async fn streams(&self, conn: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>>;
    async fn send_message(&self, conn: ConnectionId, message: Message) -> Result<()>;
    async fn send_dtmf(&self, conn: ConnectionId, digits: &str, duration_ms: u32) -> Result<()>;
    async fn renegotiate_media(
        &self,
        conn: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs>;

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent>;

    /// Capability advertisement for the negotiation algorithm in §9.
    fn capabilities(&self) -> CapabilityDescriptor;

    /// Verify a per-request signature on an incoming envelope or HTTP-shaped request.
    /// Returns the IdentityAssurance level the signature establishes.
    ///
    /// Substrate adapters that carry HTTP-shaped requests (rvoip-webtransport, rvoip-websocket;
    /// rvoip-quic over h3) implement this against RFC 9421 + Hardt's Signature-Key /
    /// Signature-Agent headers. SIP and WebRTC interop adapters return Anonymous unless the
    /// peer has presented an HTTP-mediated AAuth/OAuth surface.
    async fn verify_request_signature(
        &self,
        conn: ConnectionId,
        signature: SignatureHeaders,
    ) -> Result<IdentityAssurance>;
}

pub struct SignatureHeaders {
    pub signature: String,            // RFC 9421 Signature header
    pub signature_input: String,      // RFC 9421 Signature-Input header
    pub signature_key: Option<Jwk>,   // Hardt's Signature-Key header (sister draft)
    pub signature_agent: Option<Jwk>, // Hardt's Signature-Agent header (sister draft)
}

pub enum AdapterKind {
    Substrate,    // UCTP-native (QUIC, WebTransport, WebSocket)
    Interop,      // Gateway to a foreign protocol (SIP, WebRTC)
}
```

Adapter events are protocol-native (e.g., `SipDialogTerminated`, `WebRtcIceFailed`, `QuicConnectionMigrated`, `CpEnvelopeReceived`). rvoip-core **normalizes** them into the rvoip-core event vocabulary above. Adapter-native events are also exposed via `rvoip::sip::events::*`, `rvoip::webrtc::events::*`, etc., for consumers who want native-vocabulary access.

The rvoip-core type and trait surface is shaped by what's *common* across substrates and interop adapters. If a feature is genuinely unique to one transport (SIP REFER attended-transfer flow, WebRTC ICE restart, QUIC connection migration, UCTP federation), it lives only in that adapter's crate and surfaces as a protocol-native API there. rvoip-core does not lower-common-denominator everything; it abstracts what should be common and leaves specialist work to the adapters.

---

## 7. Translation tables

Every row corresponds to actual types and operations in code.

### 7.1 Concept translation

| rvoip-core | rvoip-sip | rvoip-webrtc | rvoip-uctp (over QUIC/WT/WS) | Thelve |
|---|---|---|---|---|
| `Conversation` | `sip::Call` (a sequence of Dialogs sharing a Call-ID family) | `webrtc::Session` (one or more PeerConnections under one app-level engagement) | UCTP `Conversation` (cid) | `Interaction` (cross-channel; rvoip Conversation is one of its parts) |
| `Session` | sequence of Dialogs in one engagement | a `Session` (room/meeting) | UCTP `Session` (sid) | one synchronous segment of a `ChannelSession` |
| `Participant` | From / To URI + display name | local / remote app identity | UCTP `Participant` (part_) | `Participant` (durable cross-channel identity) |
| `Connection` | `sip::Dialog` | `webrtc::PeerConnection` | UCTP `Connection` (connid) over QUIC/WT/WS | one transport-leg of a `ChannelSession` |
| `MediaStream` | RTP/SRTP stream over UDP | DTLS-SRTP track | UCTP `Stream` (RTP-in-datagram) | (not exposed; below ChannelSession) |
| `Message` | SIP MESSAGE method | DataChannel message | UCTP `message.send` | `TimelineEvent` of message kind |
| `Identity` | (implicit; AOR + SIP credential) | (no native concept) | UCTP `Identity` (id_) | `Worker` / `Customer` |
| `Device` | (`+sip.instance` from RFC 5626) | (no native concept) | UCTP `Device` (dev_) | `Device` |

### 7.2 Operation translation

| rvoip-core operation | SIP (rvoip-sip) | WebRTC (rvoip-webrtc) | UCTP (rvoip-quic / -wt / -ws) |
|---|---|---|---|
| `OriginateConnection` | INVITE | createOffer + signaling | `connection.offer` |
| `RouteInboundConnection → accept` | 200 OK + ACK | createAnswer + setRemoteDescription | `connection.answer` |
| `RouteInboundConnection → reject` | 4xx/5xx/6xx | reject | `error` envelope (4xx code) |
| `EndConnection` | BYE | close() | `connection.end` |
| `Hold` | re-INVITE with `a=sendonly` | renegotiate with track disabled | `connection.update {action: hold}` |
| `Resume` | re-INVITE with `a=sendrecv` | renegotiate with track re-enabled | `connection.update {action: resume}` |
| `TransferConnection` (blind) | REFER | adapter-specific signaling | `session.update {kind: transfer}` |
| `BridgeConnections` (in rvoip-core; not protocol-level) | media-stream pair-up via rvoip-media | same | same |
| `SendMessage` | SIP MESSAGE | DataChannel send | `message.send` |
| `SendDtmf` | RFC 2833 / SIP INFO | RFC 4733 | `dtmf.send` |
| `RenegotiateMedia` | re-INVITE | renegotiate | `connection.update {action: codec-renegotiate}` |
| Identity registration | REGISTER | (signaling-server-specific; if applicable) | `auth.hello` / `auth.session` |

`BridgeConnections` is interesting: it has no native protocol equivalent because the bridge is rvoip-core's media plane manipulating two `MediaStream` instances directly. That's the value-add of the gateway layer — bridging is transport-agnostic.

### 7.3 Lifecycle state translation

| rvoip-core `ConnectionState` | SIP dialog state | WebRTC PeerConnection state | UCTP Connection state |
|---|---|---|---|
| `Connecting` | Early / Trying | new / connecting | `connection.offer` sent, awaiting answer |
| `Connected` | Confirmed | connected | `connection.ready` received |
| `Held` | Confirmed (with re-INVITE sendonly applied) | connected (track disabled) | `connection.update {hold}` applied |
| `Ending` | Terminating (BYE in flight) | disconnecting / closing | `connection.end` sent, awaiting confirm |
| `Ended` | Terminated | closed | `connection.end` confirmed |
| `Failed` | Various failure terminations | failed | substrate dropped or `error` received |

### 7.4 Session state translation

| rvoip-core `SessionState` | SIP analogue | WebRTC analogue | UCTP `Session` |
|---|---|---|---|
| `Initiating` | INVITE in flight, no Dialog confirmed yet | offer/answer in flight | `session.invite` sent, awaiting `session.accept` |
| `Active` | ≥1 Dialog confirmed | ≥1 PeerConnection connected | `session.started` emitted |
| `Ending` | BYE in flight on last Dialog | last PC closing | `session.end` sent |
| `Ended` | all Dialogs terminated | all PCs closed | `session.ended` emitted |
| `Failed` | all Dialogs failed | all PCs failed | `session.ended` with error code |

---

## 7.5 What's currently in orchestration-core, mapped to the new layout

Most of orchestration-core's existing code is SIP-coupled; once you remove the workforce/queue/agent surface (which the PRD already lifts to the consumer / Thelve), what's left is mostly SIP plumbing.

| Current location in orchestration-core | New home | Why |
|---|---|---|
| `BridgeManager`, `BridgeHandle` plumbing | rvoip-sip (and rvoip-core for cross-transport) | Wraps session-core's SIP-flavored bridge handle; rvoip-core gets the cross-transport `BridgeConnections` |
| REFER / blind / attended transfer mechanics | rvoip-sip | SIP-specific protocol flow |
| `ContactResolver`, registrar-core consumption | rvoip-sip | Registration is a SIP-layer fact |
| Inbound INVITE → call-state machine | rvoip-sip | Call state coupled to SIP dialog state |
| Outbound `make_call` + CPD/AMD | rvoip-sip | SIP-leg origination |
| session-core / dialog-core / sip-transport integration | rvoip-sip | Absorbed; session-core renamed |
| `Agent`, `AgentStore`, `Queue`, `QueueStore`, `AgentOffer`, `AgentOfferStore` | (deleted from rvoip; lifted to consumer / Thelve) | Workforce orchestration, not voice plane (per PRD §13) |
| `AssignmentManager`, `Router`, `QueueSelector` | (deleted from rvoip; lifted to consumer / Thelve) | Routing decisions, not voice plane |
| `voice_ai.rs` | rvoip-harness | Transport-agnostic conceptually; pulls heavy provider deps. Splitting keeps SIP-only carriers light. |
| `Conversation`, `Participant`, `Session`, `Connection`, `Stream`, `Message` types (new) | rvoip-core | The neutral substrate |
| `ConnectionAdapter` trait and dispatch | rvoip-core | The contract every adapter implements |
| `BridgeConnections` primitive | rvoip-core | Transport-agnostic; calls down through rvoip-media |
| Event normalization (adapter-native → rvoip-core events) | rvoip-core | The translation layer |
| Conversation / Session / Connection stores, atomic state, capacity | rvoip-core | The narrowed concurrency surface |
| Admission semaphore, per-tenant quotas | rvoip-core | Tenancy is cross-cutting at the orchestrator |
| `ProviderRegistry` consumption | rvoip-core (consumed) + rvoip-harness (used) | Registry owned by consumer; rvoip-core resolves provider IDs; rvoip-harness instantiates them |

What remains in rvoip-core after the split is intentionally small — a few hundred lines of types, traits, and dispatch logic. That's correct: rvoip-core is the **spine** that carries commands across, events back, and bridges adapter-produced Connections. Substantive work happens in adapters and in rvoip-media.

---

## 8. Identity, registration, and reachability

Cross-surface login is the load-bearing claim of the unified rvoip vision. voip-3 §11 defers the question; UCTP §5 commits to a wire flow; this section commits to the Rust trait surface.

```rust
#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// Resolve a reference (UCTP identity_ref, SIP AOR, etc.) to an Identity.
    async fn resolve(&self, identity_ref: &str) -> Result<Identity>;

    /// Enumerate Devices registered for this Identity.
    async fn devices(&self, identity_id: IdentityId) -> Result<Vec<Device>>;

    /// Where this Identity is currently reachable.
    async fn reachable_via(&self, identity_id: IdentityId) -> Result<Vec<ReachabilityHint>>;

    /// Authenticate a credential and return the (IdentityId, IdentityAssurance) pair.
    /// The returned assurance reflects what this credential establishes.
    async fn authenticate(&self, credential: Credential) -> Result<(IdentityId, IdentityAssurance)>;

    /// Look up the current IdentityAssurance for an Identity (e.g., revoked? still in scope?).
    async fn assurance_level(&self, id: IdentityId) -> Result<IdentityAssurance>;

    /// Register a public signing key against an Identity (e.g., for AAuth agent identities,
    /// DPoP, or DTLS-SRTP fingerprint binding).
    async fn register_agent_key(&self, id: IdentityId, key: Jwk) -> Result<()>;

    /// Verify an RFC 9421 signature against a known Identity's signing keys; return the
    /// IdentityAssurance the signature establishes.
    async fn verify_signature(
        &self,
        id: IdentityId,
        sig: SignatureHeaders,
        body: &[u8],
    ) -> Result<IdentityAssurance>;

    /// Derive a DTLS certificate fingerprint from this Identity's signing key,
    /// for binding signaling-time identity into the DTLS-SRTP handshake (§8.4).
    /// Returns None when no signing key is registered or when fingerprint binding is disabled.
    async fn derive_dtls_fingerprint(&self, id: IdentityId) -> Result<Option<DtlsFingerprint>>;

    /// Subscribe to changes in reachability (registrar events from any substrate).
    fn subscribe_reachability(&self) -> mpsc::Receiver<ReachabilityChange>;
}

pub struct ReachabilityHint {
    pub transport: Transport,
    pub address: String,           // SIP URI / WebRTC signaling endpoint / UCTP server / etc.
    pub device_id: DeviceId,
    pub priority: u16,             // lower = preferred
    pub expires_at: Option<DateTime<Utc>>,
    pub quality_hint: Option<QualityHint>,
}

pub struct ReachabilityChange {
    pub identity_id: IdentityId,
    pub kind: ReachabilityChangeKind,    // Added | Removed | Updated | Expired
    pub hint: ReachabilityHint,
}

pub enum Credential {
    /// Plain bearer token; produces Identified.
    Bearer(String),

    /// OAuth 2.1 + DPoP. The DPoP proof binds the access token to a per-client key,
    /// preventing replay. Production-default v1.
    OAuth2Dpop { access_token: String, dpop_proof: String },

    /// OIDC ID token, optionally bound to a key via Hardt's openid-key-binding draft.
    Oidc { id_token: String, key_binding: Option<Jwk> },

    /// FIDO/WebAuthn challenge response. Anchors the user side of agent delegation.
    Passkey { challenge_response: Bytes, attestation: Option<Bytes> },

    /// SIP Digest auth. Preserved for hybrid deployments where SIP UAs authenticate
    /// against the same Identity service.
    SipDigest { username: String, response: String, nonce: String },

    /// AAuth (Hardt). EXPERIMENTAL in v1 — gated behind the `aauth-experimental`
    /// feature flag in `rvoip-identity`. Per-agent keypair, RFC 9421 HTTP Message
    /// Signatures, no bearer tokens. Carries the signed request, the agent's signing key,
    /// and (optionally) a delegating user-agent's signing key for delegation chains.
    AAuth {
        signed_request: SignedRequest,
        signature_key: Jwk,
        signature_agent: Option<Jwk>,
    },
}
```

### 8.4 DTLS-SRTP fingerprint binding

When an Identity has a registered signing key and the `identity-fingerprint-binding` feature is enabled, the rvoip-sip and rvoip-webrtc adapters bind the Identity into the DTLS-SRTP handshake:

1. `IdentityProvider::derive_dtls_fingerprint(id)` derives a fingerprint from the Identity's signing key (e.g., SHA-256 of the JWK's public key parameters).
2. The adapter generates a DTLS certificate whose public key matches that fingerprint and uses it for the DTLS-SRTP handshake.
3. The remote peer can verify that the DTLS fingerprint corresponds to the same key that signed the signaling-time auth — closing the gap between "who initiated this Connection" (signaling) and "who is sending this media" (transport).

In v1 this is **off by default** — strong claim, needs implementation experience first. The trait surface is in place so consumers and operators can opt in.

### 8.5 Backend availability and experimental status

| Backend | Status | Default in `rvoip-identity` |
|---|---|---|
| OAuth 2.1 + DPoP | Production | Default-on |
| OIDC | Production | Default-on |
| SIP Digest | Production (legacy SIP path) | Default-on |
| FIDO/passkeys | Production | Default-on |
| AAuth (`draft-hardt-oauth-aauth-protocol`) | **Experimental** | Off; gate behind `aauth-experimental` feature |
| DTLS-SRTP fingerprint binding | **Design + feature flag** | Off; gate behind `identity-fingerprint-binding` feature |

The trait shape accommodates AAuth and the fingerprint binding so consumers and operators can opt in. rvoip's public API does **not** commit to AAuth's current draft as canonical until the protocol stabilizes (PRD §14.2 item 10 tracks this).

### 8.1 Implementation expectations

- rvoip-core ships an in-memory default `IdentityProvider` for tests and small deployments.
- Production deployments (Thelve) implement `IdentityProvider` against their own user / worker / customer database.
- `rvoip-sip` produces `ReachabilityChange` events from registrar-core's REGISTER stream.
- `rvoip-uctp` produces `ReachabilityChange` events from `auth.session` / `auth.bye` / `auth.keepalive` flows.
- `rvoip-webrtc` produces `ReachabilityChange` events from consumer-driven announcements (WebRTC has no native registrar).

### 8.2 Cross-substrate reachability lookup

When the Orchestrator needs to deliver an inbound Connection or originate one toward a known Identity:

1. Call `IdentityProvider::reachable_via(identity_id)`.
2. Sort hints by `priority` (lowest first), filtering by transports the calling adapter can speak.
3. Deliver to the highest-priority reachable hint; on failure, fall through.

### 8.3 Registration normalization

Per-substrate registrar events are normalized into `RegistrationChanged` (rvoip-core event vocabulary) and `ReachabilityChange` (`IdentityProvider` callback). The deduplication policy from PRD §14.2 item 6 applies: emit only on material change, plus periodic `RegistrationHeartbeat`.

---

## 9. Codec and capability negotiation

`CapabilityDescriptor` is the neutral capability shape every adapter advertises. UCTP §8 defines its wire form; this section defines its Rust shape and the negotiation algorithm.

```rust
pub struct CapabilityDescriptor {
    pub audio_codecs: Vec<AudioCodecCapability>,
    pub video_codecs: Vec<VideoCodecCapability>,
    pub data_protocols: Vec<DataProtocol>,
    pub dtmf_modes: Vec<DtmfMode>,
    pub max_streams_per_connection: u16,
    pub transport_features: Vec<TransportFeature>,  // e.g., MediaDatagrams, ConnectionMigration, SessionResumption, TranscodeG711Opus
    pub interop: Vec<Transport>,                    // present on adapters that can be gatewayed

    /// The IdentityAssurance level this peer is currently providing (filled by the adapter
    /// after signature verification at connection.offer time).
    pub identity_assurance_offered: IdentityAssurance,

    /// Minimum IdentityAssurance this peer requires from peers in the Session.
    /// If a Session's Connections cannot meet this minimum, the peer rejects the Session.
    pub identity_assurance_required: Option<IdentityAssuranceRequirement>,
}

pub enum IdentityAssuranceRequirement {
    /// Anonymous is acceptable.
    None,
    /// Pseudonymous or higher.
    Pseudonymous,
    /// Identified or higher.
    Identified,
    /// TaskScoped or UserAuthorized.
    TaskScoped,
    /// UserAuthorized only.
    UserAuthorized,
}
```

### 9.1 Negotiation algorithm

When a Session adds a Connection (or a Connection re-negotiates):

1. The new Connection's adapter advertises a `CapabilityDescriptor` (offer).
2. For each stream the offer requests, walk the offerer's preferences in order:
   - Pick the first codec the answerer supports (advertised in its own `CapabilityDescriptor`).
3. The Session stores a `CapabilityIntersection` per Stream pair (offer-side codec, answer-side codec).
4. If the selected codecs **match** across all participating Connections, bridging is **relay** (rvoip-media just forwards frames).
5. If codecs **differ**, the Session checks whether any Connection (typically the gateway server itself) advertises a transcoding pair (`TranscodeG711Opus`, `TranscodeAmr-NbOpus`, etc.) covering the mismatch.
   - If yes, rvoip-media inserts a transcoder in the media path.
   - If no, the late-arriving Connection's offer is rejected with `488 Incompatible-Capabilities`.

### 9.2 Per-substrate translation

- **rvoip-sip** translates `CapabilityDescriptor` ↔ SDP m-lines and a-attributes.
- **rvoip-webrtc** translates `CapabilityDescriptor` ↔ SDP (with WebRTC-specific extensions: ICE, fingerprint, simulcast).
- **rvoip-uctp** translates `CapabilityDescriptor` ↔ UCTP envelope JSON directly (no SDP).

### 9.3 Re-negotiation

Triggered by `RenegotiateMedia` command. Adapter handles re-INVITE / renegotiate / `connection.update` protocol-natively. Session's `CapabilityIntersection` is updated; if codecs change, rvoip-media swaps in a new transcoder or removes the existing one.

---

## 10. Bridging model

### 10.1 Decision: explicit 1:1 bridges; no SFU/MCU in v1

`BridgeConnections(a, b)` explicitly bridges two Connections. **>2-party Sessions are out of scope for v1.** Multi-party voice/video requires a Selective Forwarding Unit (SFU) or Multi-point Conference Unit (MCU); both are substantial subsystems with their own product motivation (selective forwarding, simulcast/SVC, layer adaptation, audio mixing matrix). v1 deliberately does not bundle them.

Why 1:1 is enough for v1:
- Contact-center: caller ↔ worker. 1:1.
- Voice AI: caller ↔ in-process AI. 1:1 bridge to the AI's audio sink/source.
- SIP↔WebRTC interop: caller ↔ web client. 1:1.
- SIP↔UCTP interop: PSTN caller ↔ UCTP-native worker. 1:1.

These are the entire set of v1 use cases. Multi-party meetings can be added in v2 by integrating an SFU adapter (likely an adapter that fronts an existing SFU like LiveKit, mediasoup, or Janus).

### 10.2 Bridge implementation

A bridge is a tokio task that pumps frames between two `MediaStream` pairs:

```
Connection A           bridge task           Connection B
    │                        │                     │
    │ frames_in() ─►────────►│                     │
    │                        │ ─►── frames_out() ─►│
    │                        │                     │
    │◄── frames_out() ◄──────│                     │
    │                        │◄──── frames_in() ◄──│
```

If codec sets differ and a transcoder is available, the bridge inserts:

```
A.frames_in() → transcoder(A_codec → B_codec) → B.frames_out()
B.frames_in() → transcoder(B_codec → A_codec) → A.frames_out()
```

### 10.3 Bridge lifecycle

```rust
pub struct BridgeHandle {
    pub id: BridgeId,
    pub a: ConnectionId,
    pub b: ConnectionId,
    pub created_at: DateTime<Utc>,
    state: Arc<RwLock<BridgeState>>,
}

impl Orchestrator {
    pub async fn bridge_connections(&self, a: ConnectionId, b: ConnectionId)
        -> Result<BridgeHandle> { /* ... */ }
    pub async fn unbridge(&self, bridge: BridgeId) -> Result<()> { /* ... */ }
}
```

A Connection may be in at most one bridge at a time. Attempting a second bridge errors with `409 Conflict`. The consumer can `unbridge` and re-bridge to change pairings (e.g., for warm transfers).

### 10.4 Bridging across IdentityAssurance levels

When two Connections of different `IdentityAssurance` are bridged:
- The **effective Session assurance** is the lower of the two Connections' levels.
- The Session's vCon `parties[]` records each Participant's individual assurance (so an audit can show "the customer was Identified, the agent was UserAuthorized").
- If either Connection has set `identity_assurance_required` and the other Connection falls short, the bridge is rejected with `403 Forbidden-For-Assurance` and the Session does not transition to Active.
- Tenants may set a global minimum assurance for bridging in the Orchestrator config; bridging below that minimum is refused regardless of per-Session settings.

### 10.5 Listener taps (3-party)

The AI listener pattern from PRD §3 is **not** a multi-party bridge — it's a 1:1 bridge plus a tap. The tap clones frames from one or both Connections into a third sink (an AI runtime or a recorder). This is implemented as a spy on the bridge task; it does not require SFU machinery.

```rust
impl Orchestrator {
    pub async fn attach_listener(
        &self,
        target: ListenerTarget,        // Connection(id) | Bridge(id) | Session(id)
        sink: Arc<dyn ListenerSink>,
        mode: ListenerMode,            // SeparatedStreams | MixedMono
    ) -> Result<ListenerHandle> { /* ... */ }
}
```

---

## 11. Conversation persistence

### 11.1 `ConversationStore` trait

```rust
#[async_trait]
pub trait ConversationStore: Send + Sync {
    async fn create(&self, conv: Conversation) -> Result<()>;
    async fn get(&self, id: ConversationId) -> Result<Option<Conversation>>;
    async fn update(&self, conv: Conversation) -> Result<()>;
    async fn close(&self, id: ConversationId, reason: CloseReason) -> Result<()>;

    async fn list_for_participant(&self, participant: ParticipantId) -> Result<Vec<ConversationId>>;
    async fn list_for_identity(&self, identity: IdentityId) -> Result<Vec<ConversationId>>;
    async fn list_for_tenant(
        &self,
        tenant: TenantId,
        filter: ConversationFilter,
        cursor: Option<Cursor>,
    ) -> Result<(Vec<Conversation>, Option<Cursor>)>;

    async fn append_message(&self, msg: Message) -> Result<()>;
    async fn list_messages(
        &self,
        conv: ConversationId,
        cursor: Option<Cursor>,
    ) -> Result<(Vec<Message>, Option<Cursor>)>;
}
```

### 11.2 Default and production implementations

- rvoip-core ships an in-memory `MemoryConversationStore` (DashMap-backed; suitable for tests and small deployments).
- Production deployments (Thelve) implement `ConversationStore` against Postgres / their own store.

### 11.3 Default cardinality policy

A Conversation is `Ephemeral` by default with `idle_close_secs = 60`. This means: when the last Session ends and 60 seconds pass with no new Message arriving, the Conversation closes automatically. This avoids the "every PSTN call from an unknown number opens a Conversation forever" failure mode.

Consumers that want long-lived Conversations (Thelve worker↔customer engagements) set `policy = Persistent` at `OpenConversation` time.

### 11.4 `VconStore` trait

vCons (per §3.9) are persisted via a separate trait so that conversation state and conversation envelopes can have different storage backends (e.g., conversations in Postgres, vCons in S3 or a content-addressed object store).

```rust
#[async_trait]
pub trait VconStore: Send + Sync {
    /// Persist a signed (and optionally JWE-encrypted) vCon. Returns a handle that
    /// includes the canonical URL and content hash for later retrieval.
    async fn store(&self, vcon: SignedVcon) -> Result<VconHandle>;

    /// Fetch a stored vCon by handle. Returns None if not found or if the caller's
    /// access policy denies retrieval.
    async fn fetch(&self, handle: &VconHandle) -> Result<Option<SignedVcon>>;

    /// List all vCons associated with a Conversation (matched by the vCon's `group` UUID).
    async fn list_for_conversation(&self, c: ConversationId) -> Result<Vec<VconHandle>>;

    /// Produce a redacted vCon from an existing one. The redacted vCon carries a
    /// `redacted` reference back to its predecessor; the predecessor remains in the
    /// store but may have stricter access policy applied.
    async fn redact(&self, handle: &VconHandle, redaction: RedactionSpec) -> Result<VconHandle>;

    /// Verify a stored vCon's JWS signatures and return the verification report.
    async fn verify(&self, handle: &VconHandle) -> Result<VerificationReport>;
}

pub struct VconHandle {
    pub uuid: VconUuid,
    pub url: String,            // canonical retrieval URL (may be a content hash + storage prefix)
    pub content_hash: String,   // SHA-256 of the signed JWS body
    pub group: Option<String>,  // vCon group UUID for related-vCon linkage
    pub created_at: DateTime<Utc>,
}
```

### 11.5 Default and production implementations

- `rvoip-core` ships an in-memory `MemoryVconStore` (DashMap-backed; suitable for tests and small deployments).
- Production deployments typically implement `VconStore` against S3 / GCS / Azure Blob (content-addressable) plus a Postgres index.
- An optional `rvoip-vcon-postgres` crate may ship as a reference implementation (per PRD §14.2 item 8). Decision deferred.

---

## 12. Lifecycle state machines

### 12.1 Connection states

```
        OriginateConnection                  inbound arrives
                 │                                  │
                 ▼                                  ▼
            Connecting ─────────────► Connecting
                 │                          │
        accept (with codec match)    reject / fail
                 │                          │
                 ▼                          ▼
             Connected                   Failed ◄─── any state on adapter error
                 ├─► Hold ─► Connected
                 │
                 │ EndConnection
                 ▼
              Ending
                 │
                 ▼
              Ended
```

### 12.2 Session states

```
        StartSession (or first inbound Connection routes here)
                 │
                 ▼
            Initiating
                 │
        first Connection reaches Connected
                 │
                 ▼
              Active ◄─── Connections come and go; ParticipantJoined/Left events
                 │
        last Connection ends + no negotiation in flight
                 │
                 ▼
              Ending (grace window: default 30s)
                 │
        no reconnect within window
                 │
                 ▼
              Ended       (or Failed if terminal state was failure)
```

### 12.3 Conversation states

```
            Open ◄─── OpenConversation (or first envelope referencing it, if implicit-allowed)
              │
        all Sessions ended + idle_close_secs elapsed (Ephemeral)
        OR  CloseConversation         (Persistent or forced)
              │
              ▼
            Closed
```

---

## 13. Migration from current code

The current `orchestration-core` crate has a lot of code that, under this design, doesn't belong in rvoip-core. PRD §13 already specified the scope cut (lift agent/queue/router to consumer); this section adds the additional renames and moves needed to land voip-3 vocabulary and the three-layer architecture.

### 13.1 Moves and renames

| Current symbol | New home | Action |
|---|---|---|
| `session-core` crate | absorbed into `rvoip-sip` | rename + module reshape |
| `orchestration-core::Call` | `rvoip-core::Conversation` (with `Session` as new layer) | rename + restructure |
| `orchestration-core::CallLeg` | `rvoip-core::Connection` | rename |
| `orchestration-core::Agent` | (deleted; lift to consumer) | per PRD §13 |
| `media-core::BridgeHandle` | `rvoip-sip` keeps SIP-only `RtpBridgeHandle`; rvoip-core owns transport-agnostic `BridgeHandle` | split |
| `voice_ai.rs` | `rvoip-harness` crate | extract |

### 13.2 New crates to create

- `rvoip-core` (new; some content moves from `orchestration-core`)
- `rvoip-uctp` (new; greenfield UCTP wire implementation)
- `rvoip-quic`, `rvoip-webtransport`, `rvoip-websocket` (new; substrate adapters)
- `rvoip-webrtc` (partially-greenfield; reuses rtp-core's DTLS/SRTP infrastructure)
- `rvoip-harness` (extracted from `orchestration-core::voice_ai`)

### 13.3 Order

Reasonable order to avoid a long dual-architecture window:
1. Create `rvoip-core` skeleton with types, traits, but no implementations.
2. Carve `rvoip-sip` out of `session-core` + SIP-coupled parts of `orchestration-core`. Implement `ConnectionAdapter` for SIP. **At this point rvoip-core is real; the SIP path uses it.**
3. Create `rvoip-uctp` with UCTP envelope encode/decode.
4. Create `rvoip-quic` and `rvoip-webtransport` substrate adapters. **At this point UCTP-native flows work.**
5. Create `rvoip-webrtc`. **At this point the unified gateway is feature-complete for v1.**
6. Create `rvoip-harness` and migrate `voice_ai` consumers.
7. Delete agent/queue/router code from `orchestration-core`; relocate examples; rename `orchestration-core` → `rvoip` facade (or delete and re-create).

Each step ships behind cargo features so the existing rvoip-orchestration-core deployments keep working until the migration completes.

---

## 14. The high-level entry point — `Orchestrator`

The "thing a consumer holds and drives" is `rvoip_core::Orchestrator`. The implementation lives in rvoip-core. The `rvoip` facade re-exports it and adds feature-flagged convenience constructors that auto-register adapters.

```rust
// In rvoip-core
pub struct Orchestrator { /* adapters, conversation store, identity provider, capacity, ... */ }

impl Orchestrator {
    pub fn new(config: Config) -> Self { /* no adapters registered yet */ }

    pub fn register<A: ConnectionAdapter + 'static>(&mut self, adapter: A) -> Result<()> {
        // errors on duplicate transport (ambiguous routing)
    }

    pub fn set_identity_provider<P: IdentityProvider + 'static>(&mut self, p: P);
    pub fn set_conversation_store<S: ConversationStore + 'static>(&mut self, s: S);

    // Command surface (§4)
    pub async fn open_conversation(&self, req: OpenConversationRequest) -> Result<ConversationId> { /* ... */ }
    pub async fn route_inbound_connection(&self, ...) -> Result<...> { /* ... */ }
    pub async fn originate_connection(&self, ...) -> Result<ConnectionId> { /* ... */ }
    pub async fn bridge_connections(&self, a: ConnectionId, b: ConnectionId) -> Result<BridgeHandle> { /* ... */ }
    // ... rest of §4
}
```

```rust
// In the rvoip facade
pub use rvoip_core::{Orchestrator, Conversation, Session, Connection, Stream, Message, Participant /* ... */};

impl Orchestrator {
    /// Construct with every feature-enabled adapter pre-registered.
    pub fn with_default_adapters(config: Config) -> Result<Self> {
        let mut o = Self::new(config);
        #[cfg(feature = "cp")]    {
            o.register(rvoip_quic::adapter())?;
            o.register(rvoip_webtransport::adapter())?;
            o.register(rvoip_websocket::adapter())?;
        }
        #[cfg(feature = "sip")]    o.register(rvoip_sip::adapter())?;
        #[cfg(feature = "webrtc")] o.register(rvoip_webrtc::adapter())?;
        Ok(o)
    }
}
```

Two ergonomic entry points, one canonical implementation:
- **Typical consumer** (Thelve, CPaaS, voice-AI dev): `rvoip::Orchestrator::with_default_adapters(config)`. One line, all wired. Build features select which adapters compile in.
- **Advanced consumer** (custom adapter set, embedded test, weird topology): `rvoip_core::Orchestrator::new(config)` + manual `register(...)` calls.

This matches the Rust idiom of "type in the core crate, ergonomic glue in the facade" (the same shape `tokio::runtime::Runtime` has with `#[tokio::main]`).

### 14.1 `register` collision policy

Registering two adapters for the same `Transport` is almost always a config bug (or accidentally enabling overlapping features). Decision: **`register` returns `Err(DuplicateAdapter)`** at construction time, surfacing the bug loudly.

---

## 15. The client-side entry point — `Client` (rvoip-client)

The Orchestrator is server-shaped: multi-tenant, multi-adapter, command/event over an internal bus. For client applications — mobile, web, desktop, embedded, or AI agents acting as a single Identity — the right entry point is a **`Client`** type: one Identity, one tenant (often implicit), one or a small set of preferred substrates, and an active set of Conversations the user has joined.

`Client` lives in the **`rvoip-client`** crate (per §2). It is a thin wrapper over the same primitives the Orchestrator uses — the same `Conversation`, `Session`, `Connection`, and `MediaStream` types from `rvoip-core`; the same adapters from the substrate / interop crates; the same `IdentityProvider` trait. The wrapper hides the multi-tenant routing surface and provides verb-shaped methods that match a client's mental model: "place a call," "send a message," "answer this incoming Session."

### 15.1 Why a separate Client type

Three reasons the Orchestrator surface is a poor fit for clients:

1. **Tenancy.** A client is one Identity in one tenant. `tenant_id` on every command is overhead. `Client` carries it implicitly.
2. **Adapter management.** A client picks one substrate at construction time (or at most a small fixed set with priorities). `Orchestrator::register` for multiple adapters with collision detection is server logic.
3. **Lifecycle ergonomics.** A client wants `client.call(target).await` returning a `SessionHandle` plus a stream of state changes — not "issue `OriginateConnection`, subscribe to `ConnectionInbound`, correlate by `correlation_id`." The command/event surface is right for servers driving many calls; verb-shaped methods are right for one user driving one call at a time.

### 15.2 The `Client` API

```rust
// In rvoip-client
pub struct Client { /* identity, transport stack, in-flight conversations, ... */ }

impl Client {
    /// Authenticate against a UCTP server, attach to a SIP registrar, or attach to a WebRTC
    /// signaler. Substrate is chosen by the URL scheme (uctp+quic://, sip://, wss://, ...).
    pub async fn connect(server_uri: &str, credential: Credential) -> Result<Self>;

    /// Outbound: place a Session against a target. Target may be a Participant ID, an Identity
    /// ID, or a substrate-native URI (`sip:bob@example.com`, `tel:+15551234`, etc.).
    pub async fn call(&self, target: CallTarget, medium: SessionMedium) -> Result<SessionHandle>;

    /// Outbound messaging: send a Message in a Conversation.
    pub async fn send_message(&self, cid: ConversationId, body: MessageBody) -> Result<MessageId>;

    /// Inbound: subscribe to incoming Sessions, Messages, and assurance changes.
    pub fn incoming(&self) -> mpsc::Receiver<InboundEvent>;

    /// Lookup / list Conversations the user is part of.
    pub async fn conversations(&self, filter: ConversationFilter) -> Result<Vec<Conversation>>;

    /// Graceful shutdown.
    pub async fn close(self) -> Result<()>;
}

pub enum InboundEvent {
    Session(SessionHandle),
    Message(Message),
    AssuranceChanged { connection_id: ConnectionId, new: IdentityAssurance },
    Disconnected { reason: DisconnectReason },
}

pub struct SessionHandle {
    pub session_id: SessionId,
    pub conversation_id: ConversationId,
    /* ... */
}

impl SessionHandle {
    pub async fn accept(self) -> Result<ActiveSession>;
    pub async fn reject(self, reason: RejectReason) -> Result<()>;
    pub async fn end(&self) -> Result<()>;
    pub async fn hold(&self) -> Result<()>;
    pub async fn resume(&self) -> Result<()>;
    pub async fn mute(&self, direction: Direction) -> Result<()>;
    pub async fn send_dtmf(&self, digits: &str) -> Result<()>;
    pub fn streams(&self) -> Vec<Arc<dyn MediaStream>>;
    pub fn events(&self) -> mpsc::Receiver<SessionEvent>;
}
```

The verb-shaped methods compose with the per-protocol native surfaces — a `SessionHandle` for a SIP-bridged Session still exposes its underlying `sip::Dialog` via `as_sip()` (and equivalents for WebRTC and UCTP) when a developer needs protocol-specific operations.

### 15.3 Per-protocol native client surfaces

Developers who don't want the unifying `Client` reach for per-protocol native types directly — these are what a SIP softphone vendor or a WebRTC widget developer would use:

- **`use rvoip::sip::client::*`** — `SipUserAgent`, `Registration`, `Call`, `Dialog`. Looks like a SIP softphone library.
- **`use rvoip::webrtc::client::*`** — `WebRtcClient`, `PeerConnection`, `Signaler`. Looks like a WebRTC library.
- **`use rvoip::uctp::client::*`** — `UctpClient`, `Envelope`, `ReachabilityHint`. Looks like a UCTP wire-protocol client.

These are not separate crates — they are modules inside `rvoip-sip`, `rvoip-webrtc`, and `rvoip-uctp` respectively. The `rvoip-client` crate re-exports them at `rvoip::sip::client`, `rvoip::webrtc::client`, `rvoip::uctp::client` so a developer can mix the unifying `Client` with one or two native types where needed.

### 15.4 Client + server in the same process

Some applications are both — a Thelve worker desktop app that uses `Client` to talk to the Thelve UCTP server, and also embeds a small `Orchestrator` for in-app conferencing. Both can coexist; they share `rvoip-core` types. The library does not enforce mutual exclusion.

---

## 16. Hello world — four sketches

These are the load-bearing examples for the developer-profile claims in PRD §1.1. Each fits in a single `main.rs` and demonstrates a feature-flag set. They are *intent sketches*, not promised to compile against the v1 API as drawn — they commit to shape and proportion.

### 16.1 Pure SIP softswitch (server)

Cargo features: `[sip, rtp, media]`. ~50 lines. The "rvoip as a drop-in replacement for a small SIP B2BUA" demo.

```rust
use rvoip::{Orchestrator, Config, OriginateRequest, Event};
use rvoip::sip::{SipAdapter, SipConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut orchestrator = Orchestrator::new(Config::default());
    orchestrator.register(SipAdapter::new(SipConfig {
        bind: "0.0.0.0:5060".parse()?,
        tls_bind: Some("0.0.0.0:5061".parse()?),
        ..Default::default()
    }))?;

    let mut events = orchestrator.subscribe_events();
    while let Some(event) = events.recv().await {
        match event {
            Event::ConnectionInbound { connection_id, from, .. } => {
                let outbound = orchestrator.originate_connection(
                    OriginateRequest::sip("sip:downstream@pbx.example.com")
                ).await?;
                orchestrator.bridge_connections(connection_id, outbound).await?;
            }
            Event::ConnectionEnded { .. } => { /* nothing to do */ }
            _ => {}
        }
    }
    Ok(())
}
```

### 16.2 SIP ↔ WebRTC bridge (server)

Cargo features: `[sip, webrtc, rtp, media]`. ~100 lines. The "why use rvoip vs. FreeSWITCH+Janus+glue" demo.

A customer calls in via SIP (PSTN); an agent in a browser answers via WebRTC. Codec transcoding (G.711 ↔ Opus) is inserted automatically by `rvoip-media`.

```rust
use rvoip::{Orchestrator, Config, OriginateRequest, Event, TranscodePair};
use rvoip::identity::ReachabilityHint;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut orchestrator = Orchestrator::with_default_adapters(Config {
        transcoding_pairs: vec![TranscodePair::G711Opus],
        ..Config::default()
    })?;
    // with_default_adapters auto-registers SipAdapter + WebRtcAdapter from the enabled features.

    // Pre-register an agent's WebRTC reachability for routing.
    orchestrator.identity_provider().register_reachability(
        "agent-001",
        ReachabilityHint::webrtc("wss://agents.example.com/signal/agent-001"),
    ).await?;

    let mut events = orchestrator.subscribe_events();
    while let Some(event) = events.recv().await {
        if let Event::ConnectionInbound { connection_id, .. } = event {
            // Inbound is SIP. Originate WebRTC toward the agent.
            let webrtc_conn = orchestrator.originate_connection(
                OriginateRequest::for_identity("agent-001")
            ).await?;
            // Library inserts G.711↔Opus transcoder automatically; you don't.
            orchestrator.bridge_connections(connection_id, webrtc_conn).await?;
        }
    }
    Ok(())
}
```

~30 lines of business logic; everything else (transcoding, ICE, SDP munging, DTLS-SRTP) is in the library.

### 16.3 Pure UCTP application server (no SIP, no WebRTC)

Cargo features: `[uctp, vcon, identity, media]`. ~150 lines.

A messaging-and-voice app where mobile / web / desktop clients connect over QUIC / WebTransport / WebSocket and exchange Messages and Sessions with each other. No telephony.

```rust
use rvoip::{Orchestrator, Config, Event};
use rvoip::uctp::{QuicAdapter, WebTransportAdapter, WebSocketAdapter};
use rvoip::identity::OauthDpopProvider;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut orchestrator = Orchestrator::new(Config {
        identity_provider: Box::new(OauthDpopProvider::from_env()?),
        conversation_store: Box::new(PostgresConversationStore::connect(/* ... */).await?),
        vcon_store: Box::new(S3VconStore::new(/* ... */)?),
        ..Default::default()
    });

    orchestrator.register(QuicAdapter::new("0.0.0.0:4433", tls_config()?))?;
    orchestrator.register(WebTransportAdapter::new("0.0.0.0:4434", tls_config()?))?;
    orchestrator.register(WebSocketAdapter::new("0.0.0.0:4435", tls_config()?))?;

    let mut events = orchestrator.subscribe_events();
    while let Some(event) = events.recv().await {
        match event {
            Event::ConnectionInbound { connection_id, identity_id, .. } => {
                tracing::info!(?identity_id, "client connected");
            }
            Event::MessageReceived { conversation_id, message, .. } => {
                tracing::info!(?conversation_id, "message received");
            }
            Event::SessionStarted { session_id, .. } => {
                tracing::info!(?session_id, "voice session active");
            }
            _ => {}
        }
    }
    Ok(())
}
```

Routing of inbound `session.invite` to the target Identity's currently-reachable Connection happens automatically using the `IdentityProvider`'s reachability hints (§8.2). No SIP, no WebRTC, no telephony — pure UCTP.

### 16.4 Full UCTP + SIP + WebRTC gateway (Thelve-shape)

Cargo features: `[full]`. ~300 lines.

Workers connect via UCTP (mobile, desktop, web). Customers call in via SIP/PSTN or via WebRTC widgets embedded in a partner site. AI agents are attached in-process via the harness. vCons emit per Session. Identity is OAuth+DPoP for human workers, SIP Digest for legacy SIP devices, AAuth (experimental) for AI agents.

```rust
use rvoip::{Orchestrator, Config, AttachAi, OriginateRequest, TransferTarget};
use rvoip::identity::{IdentityProviderChain, OauthDpopProvider, AAuthProvider, SipDigestProvider};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let identity = IdentityProviderChain::new()
        .with(OauthDpopProvider::from_env()?)            // human workers + customers
        .with(SipDigestProvider::from_registrar(/* ... */)?) // SIP devices
        .with(AAuthProvider::experimental()?);           // AI agent identities

    let mut orchestrator = Orchestrator::with_default_adapters(Config {
        identity_provider: Box::new(identity),
        conversation_store: Box::new(MyConversationStore::connect()?),
        vcon_store: Box::new(MyVconStore::s3_backed()?),
        provider_registry: Box::new(MyProviderRegistry::load()?),  // ASR/TTS/Dialog providers per tenant
        admission_semaphore: 5_000,
        ..Default::default()
    })?;
    // Registers QUIC, WebTransport, WebSocket, SIP, and WebRTC adapters per feature flags.

    // Wire to your application's command channel (e.g., from your workforce orchestration layer).
    let mut cmds = my_thelve::commands_from_workforce_layer();
    let evts = orchestrator.subscribe_events();
    tokio::spawn(my_thelve::handle_events(evts));

    while let Some(cmd) = cmds.recv().await {
        match cmd {
            ThelveCommand::AttachAi { connection_id, dialog_manager, asr, tts } => {
                orchestrator.attach_ai(AttachAi {
                    connection_id,
                    dialog: dialog_manager,
                    asr_provider: asr,
                    tts_provider: tts,
                }).await?;
            }
            ThelveCommand::BridgeWorker { customer_conn, worker_identity } => {
                let worker_conn = orchestrator.originate_connection(
                    OriginateRequest::for_identity(&worker_identity)
                ).await?;
                orchestrator.bridge_connections(customer_conn, worker_conn).await?;
            }
            ThelveCommand::TransferToHuman { current_conn, human_identity } => {
                let human_conn = orchestrator.originate_connection(
                    OriginateRequest::for_identity(&human_identity)
                ).await?;
                orchestrator.transfer_connection(current_conn, TransferTarget::Connection(human_conn)).await?;
            }
            // ... other workforce-layer commands
        }
    }
    Ok(())
}
```

Workforce orchestration (queues, skills, presence, AI training flywheel, customer continuity) lives in `my_thelve`, not in rvoip. The library provides voice / video / messaging plumbing across substrates; the consumer provides workforce semantics.

### 16.5 Notes on the sketches

- All four sketches are *server-shape* (`Orchestrator`). A client-side sketch using `rvoip-client::Client` lives in the `rvoip-client` crate's own examples once that surface is finalized.
- Sketches commit to shape: `Orchestrator` is the server entry, command/event surface is feature-flag-gated, bridging across transport pairs is a one-call operation, the developer never writes adapter glue.
- Line-count targets (~50 / ~100 / ~150 / ~300) are for the *complete* example including configuration, error handling, and graceful shutdown — not the snippet shown here.

---

## 17. Documentation strategy

What enforces vocabulary segregation in practice — not just at the API level but in everything the developer reads.

- **Top-level README** (`rvoip` facade): the three-layer framing (SDK / UCTP / library / gateway), who this is for, the four communities (UCTP, SIP, WebRTC, harness). Short. Points to per-adapter landing pages and to `CONVERSATION_PROTOCOL.md`.
- **rvoip-core landing**: the abstractions in this doc, the bridge model, the AI harness, the consumer command/event contract.
- **rvoip-uctp landing**: UCTP wire implementation. Pointer to `CONVERSATION_PROTOCOL.md` for protocol semantics.
- **rvoip-sip landing**: SIP-developer onboarding. Uses dialog, transaction, REGISTER, INVITE, REFER. Examples in SIP terms. Never says "PeerConnection."
- **rvoip-webrtc landing**: WebRTC-developer onboarding. Uses PeerConnection, ICE, offer/answer, DataChannel. Never says "dialog."
- **rvoip-quic / -webtransport / -websocket landing**: substrate-developer onboarding. Datagram framing, stream framing, reconnection.
- **Cross-community translation table**: this doc's §7, published as a standalone reference.
- **Migration / interop guides**: short docs like "from FreeSWITCH to rvoip-sip," "from mediasoup to rvoip-webrtc," "from a custom WebSocket signaling server to UCTP." Lower priority, real adoption value.

---

## 18. Enforcement

To stop vocabulary bleed in practice, two mechanical rules:

1. **No symbol from `crate::sip::*` may appear in the public surface of `crate::webrtc::*` or `crate::cp::*`** (and reciprocally). Enforced via a `clippy` lint or a doc-test that scans the API. Mechanical and durable.
2. **rvoip-core never imports an adapter crate.** rvoip-core defines traits; adapters implement them. Dependency points one way. Enforced by `cargo deny` or a workspace lint.

These two rules turn the architectural commitment into something a contributor cannot accidentally violate.

---

## 19. Gaps in voip-3 terminology surfaced by this design

Per project direction, voip-3 (`/Users/jonathan/Developer/Rudeless/voip-3-conversation-model.md`) is **not modified**; gaps voip-3 leaves underspecified that this design had to invent or stub are listed here so a future voip-3 revision can fold them in or explicitly defer them.

1. **No formal command/event vocabulary.** voip-3 has lifecycle verbs only. This doc adds 25+ commands and 25+ events.
2. **No Session boundary rules.** When does a Session end vs. continue across Connection drops? §3.2 commits to a 30s reconnect grace window default.
3. **No bridging primitive vocabulary.** voip-3 says "Session abstracts the differences"; this doc adds `BridgeConnections` as an explicit primitive (§10).
4. **No identity/auth trait shape.** voip-3 §11 defers identity. This doc adds `IdentityProvider` (§8).
5. **No registration/reachability vocabulary.** §8.3 maps SIP REGISTER and UCTP `auth.session` to `RegistrationChanged` / `ReachabilityChange`.
6. **No capability/codec negotiation schema.** §9 commits to `CapabilityDescriptor` and an intersection algorithm.
7. **No persistence model.** §11 adds `ConversationStore` and a default closure policy.
8. **No quality/observability event shape.** §5 adds `MediaQuality`, `CapacityReport`, `Anomaly`.
9. **No mid-Session join semantics.** §4 adds `JoinSession` / `LeaveSession`; events `ParticipantJoined` / `ParticipantLeft`.
10. **No multi-tenancy threading on commands/events.** This doc carries `tenant_id` and `correlation_id` on every command and event.
11. **No formal state machines.** §12 defines Connection / Session / Conversation state diagrams.
12. **No transcoding / cross-codec story.** §9.1 defines a transcoder-as-Session-feature model.
13. **No Conversation cardinality at scale.** §11.3 commits to `Ephemeral` default with idle close.
14. **No interop boundary specification.** §7 + UCTP §12 commit to gateway-not-tunnel for SIP and WebRTC.
15. **No listener-tap pattern.** §10.5 carves out the 3-party listener as distinct from multi-party SFU.
16. **No conversation envelope.** voip-3 has no equivalent of [vCon](https://datatracker.ietf.org/doc/draft-ietf-vcon-vcon-core/) — the IETF Virtualized Conversations standard. This doc adopts vCon as the canonical signed JSON envelope for conversation recording and analysis (§3.9, §11.4). Mapping: `Participant` → `parties[]`, `Session` → `dialog[]`, `Message` → `dialog[type=text]`, `Conversation` → vCon group.
17. **No agent-identity model.** voip-3 §11 lists identity as open. This doc commits to `IdentityAssurance` (Anonymous → Pseudonymous → Identified → TaskScoped → UserAuthorized) as the public-facing concept and accommodates AAuth (`draft-hardt-oauth-aauth-protocol`) as one of several backends behind `IdentityProvider`.
18. **No per-request signing model.** voip-3 has no protocol-level message authentication. This doc adopts [RFC 9421 HTTP Message Signatures](https://datatracker.ietf.org/doc/rfc9421/) for substrates that carry HTTP-shaped requests, with hooks on `ConnectionAdapter::verify_request_signature` to surface assurance from signature verification.
19. **No signaling↔media identity binding.** §8.4 introduces DTLS-SRTP fingerprint binding (feature-flagged in v1) that ties Identity signing keys to the DTLS handshake — closing the gap between signaling-time and media-time identity.

---

## 20. Gaps still open in this interface design

Items deferred or left for a later version:

1. **SFU/MCU integration for >2-party Sessions.** v1 is 1:1 only. v2 adds an SFU adapter (likely fronting LiveKit / mediasoup / Janus).
2. **Video.** `MediaStream` trait shape supports `StreamKind::Video`; v1 implements audio only. Video bridging adds simulcast/SVC layer adaptation that's out of v1 scope.
3. **Cross-transport quality unification.** SIP gives RTCP-XR; WebRTC gives `RTCStatsReport`; UCTP gives `connection.quality` envelope. All produce a `QualitySnapshot`, but mapping fidelity differs. Iterate as we measure.
4. **Lawful intercept / compliance hooks.** Consumer concern; rvoip exposes `StartRecording` only. Compliance jurisdiction-by-jurisdiction is the consumer's problem.
5. **Latency budget tracking.** Per-Session latency budget enforcement (alarm on degradation) deferred to v2.
6. **End-to-end encryption.** rvoip relies on substrate TLS/QUIC and SRTP for hop confidentiality. Application-layer E2EE (libsignal-style) is the consumer's add-on for v1.
7. **Federation across UCTP servers.** UCTP §13 reserves the namespace; rvoip-core does not implement federation in v1.
8. **`prelude` module on the facade.** Lean: yes — convention (tokio, futures, serde all do this); ship it. Decide what's in it close to v1 cut.
9. **Default cargo features.** §2.2 commits to `[uctp, sip, rtp, media, vcon, identity]`. Revisit if profiling shows a smaller default helps embedded use cases.
10. **Whether `rvoip-harness` lives in rvoip-core or stays separate.** Lean: separate (per PRD §11) so SIP-only carriers don't pull provider deps.
11. **Push notifications for mobile.** APNs/FCM bridge for waking sleeping UCTP clients on inbound. Needed for production mobile but not v1 protocol surface.
12. **Anomaly taxonomy.** PRD §14.2 item 4 is open; resolve by enumerating the specific anomalies rvoip emits.
13. **AAuth conformance.** AAuth backend is **experimental** in v1; trait shape supports it but the public API does not commit until the IETF status stabilizes (PRD §14.2 item 10).
14. **DTLS-SRTP fingerprint binding.** Designed in §8.4; implementation behind feature flag `identity-fingerprint-binding`, default off in v1. Needs implementation experience before promoting.
15. **vCon storage default.** rvoip-core ships an in-memory `MemoryVconStore` for tests; whether to ship a reference Postgres implementation as `rvoip-vcon-postgres` is open (PRD §14.2 item 8). Lean: yes, as an optional crate.
16. **vCon emission cadence.** Default plan: async-batched at session.ended, with `VconReady` event when committed (PRD §14.2 item 7). 5-second SLA target. Decide once we measure JWS signing cost at scale.
17. **vCon-without-audio policy.** §3.9 commits to "always emit a vCon for every Session, even without audio." Useful as the durable audit primitive; needs telemetry to confirm storage cost is acceptable for high-volume tenants.
18. **vCon redaction default.** rvoip-core surfaces redaction primitives only; the consumer's compliance layer applies jurisdiction-specific redaction. Whether rvoip ships a default PII-detection redactor is open — lean: no (consumers integrate their existing detection pipelines).

---

## 21. Open questions

These need finalization before implementation begins on each section.

1. **`Stream` vs `MediaStream` in code.** This doc lands on `MediaStream` (trait) with `Stream` as the conceptual term in docs and UCTP envelopes. Final?
2. **Conversation auto-close grace window default.** 60s for `Ephemeral` and 30s for Session reconnect. Empirically tune once we have telemetry.
3. **`OpenConversation` must-be-explicit vs. implicit-allowed.** Lean: implicit allowed for inbound calls from unknown numbers (server creates an Ephemeral Conversation on demand); explicit required for messaging-first flows.
4. **`session.invite` retargeting.** If an Identity is unreachable on its highest-priority hint, does the Session auto-fall-through to lower-priority hints, or does that decision belong to the consumer? Lean: consumer decides; rvoip exposes the failure as `ConnectionFailed` and the consumer chooses to retry on another hint.
5. **`MediaStream` channel size.** What's the right `mpsc` buffer depth? Must absorb scheduling jitter without unbounded memory. Lean: 64 frames audio, 16 frames video; tune empirically.
6. **Capability descriptor versioning.** When new codecs are added, how do older endpoints react? Lean: unknown codecs are ignored (the negotiation algorithm naturally falls through).
7. **AI harness as in-process AI Connection.** When `AttachAi` is used, is the AI runtime modeled as a Connection (so it appears in `Session.connections` like any other) or as a special attachment? Lean: as a Connection with `Transport::InProcessAi`, so it composes with the rest of the model uniformly.
8. **`Orchestrator` shutdown semantics.** Drain in-flight Connections gracefully vs. hard-stop. Needed for graceful redeploys at scale.
9. **UCTP version negotiation for clients connecting to a server that supports multiple UCTP versions.** UCTP §16 sketches this; the rvoip-uctp implementation must commit to the negotiation algorithm.
10. **Identity caching policy in Orchestrator.** How long does the Orchestrator cache `IdentityProvider::reachable_via` results before re-querying? Lean: TTL from the hint's `expires_at`, with a floor of 30s.

---

**Reviewers:** §1 (three-layer framing), §3 (core abstractions), §6 (adapter contract), and §10 (bridging) are the load-bearing sections. §8 (Identity), §9 (codec negotiation), and §11 (persistence) close the gaps that prior INTERFACE_DESIGN drafts left open. §19 lists what voip-3 still defers; §20 lists what this design defers; §21 is what we decide before code lands.

This is a v1 working draft; expect revisions as implementation discovers reality.
