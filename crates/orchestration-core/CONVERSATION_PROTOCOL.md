# Universal Conversation Transport Protocol (UCTP) — v0 specification

**Status:** v0 working draft. This is the **wire/SDK protocol** that implements the conceptual model defined in `voip-3-conversation-model.md`. Best-effort first pass; will iterate.

**Companion documents:**
- `voip-3-conversation-model.md` — the conceptual model (Conversation / Session / Message / Participant / Connection / Stream). Source of truth for terminology. **Not modified by this document.**
- `INTERFACE_DESIGN.md` — the rvoip Rust library that implements UCTP server-side and bridges to SIP / WebRTC.
- `PRD.md` — scope and product role of the rvoip library.

---

## 1. What UCTP is, what it isn't

The Universal Conversation Transport Protocol is a **substrate-agnostic application protocol** for real-time voice, video, and messaging engagements. It speaks the voip-3 nouns directly on the wire: a UCTP message refers to a `Conversation`, a `Session`, a `Connection`, a `Stream`, a `Message`, or a `Participant`, by ID and by intent.

UCTP runs over multiple substrates:

| Substrate | Role | Notes |
|---|---|---|
| **QUIC** | preferred for native apps, embedded devices, server-server | bidirectional streams carry UCTP envelopes; datagrams carry media |
| **WebTransport** | preferred for browsers and modern mobile WebViews | bidi streams carry UCTP envelopes; WT datagrams carry media |
| **WebSocket** | fallback for older browsers and constrained networks | UCTP envelopes as text frames; media via co-negotiated WebRTC PeerConnection |
| **(SIP / WebRTC)** | **not substrates** — interop boundaries | Gatewayed at the rvoip server boundary; never tunneled |

UCTP is what apps speak. SIP and WebRTC are what other systems speak, and a UCTP-speaking server (rvoip-based or otherwise) translates between UCTP and those protocols at the gateway boundary. **UCTP-over-SIP and UCTP-over-WebRTC are explicitly not supported** — that direction would be tunneling, which loses the per-protocol semantics that make SIP and WebRTC useful in their own right.

UCTP is similar in shape to:
- Matrix Client-Server API (substrate-agnostic, JSON-over-anything, durable Conversations)
- IETF MoQ (transport-agnostic media)
- WebRTC signaling protocols (capability negotiation, offer/answer)

UCTP is **not** similar to:
- SIP (which conflates signaling with session identity and uses dialogs as the unit of orchestration)
- IRC / XMPP (which model rooms and presence at the protocol level rather than as application concerns)

### 1.1 Layering

```
Application (mobile app, web app, desktop app, embedded device, AI agent)
   │
   │ uses an SDK that speaks UCTP
   ▼
Universal Conversation Transport Protocol — voip-3 nouns on the wire
   │
   │ travels over a substrate
   ▼
Substrate (QUIC | WebTransport | WebSocket)
   │
   ▼
Network
```

A UCTP-speaking server (Thelve, the canonical example) terminates UCTP from clients, terminates SIP from PSTN/SBCs, terminates WebRTC from third-party widgets, and bridges between them at the Session level. From a client's perspective there is only UCTP; the server does the mapping.

---

## 2. Design principles

1. **One protocol, many substrates.** The same UCTP envelope is valid over QUIC, WebTransport, or WebSocket. Substrates differ only in framing rules (§4).
2. **voip-3 nouns are first-class.** Every envelope identifies which Conversation / Session / Connection / Stream / Message / Participant it concerns.
3. **Signaling and media are separate.** Signaling is reliable and ordered (streams). Media is unreliable and unordered (datagrams). Never mix.
4. **JSON-first; binary later.** v0 uses JSON envelopes for human readability and tooling. A binary encoding (CBOR or a custom format) is reserved for v1 once the schema is stable.
5. **Capabilities, not assumptions.** Endpoints advertise what they can do; the Session negotiates the intersection. No protocol-level assumption that everyone supports Opus, H.264, etc.
6. **Forward compatibility.** Unknown envelope types and unknown payload fields are silently ignored by old endpoints. Required new behavior is gated on a capability flag.
7. **Idempotency where it matters.** Lifecycle transitions (`session.end`, `connection.end`, `message.send`) can be retried with the same envelope ID without side effects.
8. **Servers can speak both directions.** UCTP is symmetric — a server can send a session.start to a client just as a client can to a server. Telephony's caller/callee asymmetry is a property of the Participant role, not the protocol.

---

## 3. Wire format — envelopes

Every UCTP message is a JSON envelope with a fixed top-level shape:

```json
{
  "v": 1,
  "type": "session.start",
  "id": "01HXYZ...ULID",
  "ts": "2026-05-09T18:23:11.412Z",
  "cid": "conv_01HXYZ...",
  "sid": "sess_01HXYZ...",
  "connid": null,
  "in_reply_to": null,
  "payload": { ... }
}
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `v` | integer | yes | Protocol version. v0 = 1. |
| `type` | string | yes | Dotted message type, e.g. `session.start`, `message.send`. See §6 for the catalog. |
| `id` | ULID | yes | Globally unique envelope ID. Used for idempotency and correlation. |
| `ts` | ISO-8601 UTC | yes | Sender's clock. Receivers do not trust this for ordering — only for display/diagnostics. |
| `cid` | string \| null | conditional | Conversation ID. Required for any envelope tied to a Conversation. Null only for connection-level envelopes (auth, keepalive). |
| `sid` | string \| null | conditional | Session ID. Required for envelopes scoped to a Session. |
| `connid` | string \| null | conditional | Connection ID. Required for envelopes scoped to a specific Connection within a Session. |
| `in_reply_to` | ULID \| null | optional | Envelope ID being responded to. Used for request/response correlation. |
| `payload` | object | yes | Type-specific body. May be `{}` for envelopes that carry only routing fields. |

### 3.1 ID format

All IDs are ULIDs (RFC-flavor; 26 chars Crockford base32). Prefixes are conventions to aid debugging:

- `conv_` — Conversation
- `sess_` — Session
- `conn_` — Connection
- `strm_` — Stream
- `msg_`  — Message
- `part_` — Participant
- `id_`   — Identity (local to this server)
- `f_`    — Identity sourced from a federated peer server (see §13)
- `dev_`  — Device

Prefixes are advisory; receivers must not depend on them for routing logic. The `id_` / `f_` distinction is the one exception: a server MUST NOT silently treat a federated `f_` Identity as if it were locally homed.

### 3.2 Reserved fields

Unknown top-level fields are ignored. Unknown payload fields are ignored. This is the forward-compatibility seam.

### 3.3 Binary encoding (deferred)

A binary encoding is planned for v1. Likely candidates: CBOR (general-purpose, schema-flexible) or a hand-rolled wire format if profiling shows CBOR overhead is significant. The schema in this document defines the abstract envelope; encoding is an implementation choice.

---

## 4. Substrate framing

### 4.1 QUIC (native apps, embedded, server-server)

- **Signaling:** one bidirectional stream per logical UCTP channel. Each envelope is length-prefixed (4-byte big-endian uint32 followed by JSON bytes). Multiple envelopes per stream are allowed.
- **Media:** QUIC datagrams. One datagram per RTP packet. Datagrams carry standard RTP headers preceded by a 4-byte UCTP datagram header (`stream_id` reference, packet sequence). See §10.
- **Connection migration:** QUIC's native connection migration is supported; the UCTP-level Connection ID is invariant across QUIC migration events.
- **Multiple Streams over one QUIC connection:** allowed. The UCTP `Connection` is decoupled from the QUIC connection — one QUIC connection can carry several UCTP `Connection`s (e.g., for a Participant with multiple Devices on the same physical link, though that's unusual).

### 4.2 WebTransport (browsers, modern mobile)

- **Signaling:** one bidirectional WT stream per logical UCTP channel; same length-prefixed framing as QUIC.
- **Media:** WT datagrams; same shape as QUIC datagrams.
- **Connection lifetime:** tied to the WT session; reconnection requires a fresh UCTP `auth` exchange (§5).

### 4.3 WebSocket (fallback)

- **Signaling:** each WebSocket text frame contains exactly one JSON envelope.
- **Media:** WebSocket cannot carry low-latency unreliable media efficiently. UCTP over WebSocket negotiates a **co-located WebRTC PeerConnection** for media: signaling stays on WebSocket; media flows over a WebRTC `RTCPeerConnection` that is signaled via UCTP envelopes (§7.4) but uses WebRTC's own DTLS-SRTP transport. From the application's perspective this is still "UCTP" — the SDK hides the dual-transport detail.
- **Why this hybrid:** WebSocket-over-TCP head-of-line blocking is fatal for real-time audio. Browsers without WebTransport must still get usable media, so we use the substrate they already have.

### 4.4 SIP and WebRTC are NOT substrates

UCTP does not run over SIP and does not run over WebRTC. When a UCTP server interconnects with a SIP endpoint or a WebRTC peer, the server **gateways** at the Session level: a SIP `Dialog` becomes one `Connection` in a UCTP `Session`; a WebRTC `RTCPeerConnection` becomes one `Connection`. The gateway translates SIP/WebRTC events into UCTP envelopes for UCTP-speaking participants in the same Session and vice versa. This is interop, not tunneling.

---

## 5. Identity, authentication, and reachability

### 5.1 Lifecycle

```
client opens substrate transport
   │
   ▼
client sends {type:"auth.hello"}              ── advertises device + capabilities
   │
server replies {type:"auth.challenge"}        ── nonce, accepted methods
   │
client sends {type:"auth.response"}           ── credential proof
   │
server replies {type:"auth.session"}          ── session token + reachability state
   │
client periodically {type:"auth.keepalive"}   ── refreshes reachability
   │
client sends {type:"auth.bye"}                ── graceful logout
```

### 5.2 Envelopes

#### `auth.hello` (client → server)
```json
{
  "type": "auth.hello",
  "id": "01HXYZ...",
  "ts": "...",
  "cid": null,
  "payload": {
    "device": {
      "id": "dev_01HXYZ...",
      "kind": "mobile" | "web" | "desktop" | "embedded" | "server",
      "platform": "ios" | "android" | "browser-chrome-122" | "linux-x86_64" | ...,
      "sdk_version": "rvoip-client/0.1.0"
    },
    "auth_methods": ["bearer", "oauth2", "passkey", "sip-digest"],
    "capabilities": { /* CapabilityDescriptor — see §8 */ }
  }
}
```

#### `auth.challenge` (server → client)
```json
{
  "type": "auth.challenge",
  "in_reply_to": "<auth.hello.id>",
  "payload": {
    "nonce": "...",
    "accepted_methods": ["bearer", "oauth2"],
    "server_capabilities": { /* CapabilityDescriptor */ }
  }
}
```

#### `auth.response` (client → server)
```json
{
  "type": "auth.response",
  "in_reply_to": "<auth.challenge.id>",
  "payload": {
    "method": "bearer",
    "credential": "<bearer token>" | "<signed challenge>" | ...
  }
}
```

#### `auth.session` (server → client)
```json
{
  "type": "auth.session",
  "in_reply_to": "<auth.response.id>",
  "payload": {
    "identity_id": "id_01HXYZ...",
    "participant_id": "part_01HXYZ...",
    "session_token": "...",          // opaque server-issued token; used for reconnect
    "expires_at": "...",
    "assurance": "identified",       // IdentityAssurance level (§5.6)
    "reachability": [ /* ReachabilityHint[] — see §5.3 */ ]
  }
}
```

#### `auth.keepalive` (client → server, periodic)
```json
{
  "type": "auth.keepalive",
  "payload": { "session_token": "..." }
}
```

#### `auth.bye` (either direction)
```json
{
  "type": "auth.bye",
  "payload": { "reason": "user-logout" | "session-expired" | "server-shutdown" }
}
```

### 5.3 ReachabilityHint

Once authenticated, the client knows what addresses it can be reached on (for inbound Sessions). The server tells the client its current reachability state, which the client may pin or extend.

```json
{
  "transport": "quic" | "webtransport" | "websocket" | "sip" | "webrtc",
  "address": "cp.example.com:4433" | "sip:alice@example.com" | "...",
  "expires_at": "...",
  "priority": 100,                     // lower = preferred
  "device_id": "dev_01HXYZ..."
}
```

A single Identity may have multiple Devices, each with one or more Reachability hints across substrates. A server uses these to decide where to deliver an inbound `session.invite` for that Identity.

### 5.4 Auth methods supported in v0

| Method | Use case | Status |
|---|---|---|
| `bearer` | Plain bearer token for legacy / dev clients | Supported |
| `oauth2-dpop` | OAuth 2.1 access token + DPoP proof (RFC 9449) — production default | Supported |
| `oidc` | OIDC ID token, optionally bound to a key via `openid-key-binding` | Supported |
| `passkey` | WebAuthn / FIDO2 challenge-response with device key | Supported |
| `sip-digest` | Preserves SIP-world auth for hybrid deployments | Supported (legacy) |
| `aauth` | Hardt's [agent auth protocol](https://aauth.dev) (`draft-hardt-oauth-aauth-protocol`) — per-agent keypair, RFC 9421 HTTP Message Signatures, no bearer tokens, identity gradient native | **Experimental** in v0 |

The auth method list is extensible; new methods can be added in v1+ without protocol changes.

### 5.5 Per-request signing (RFC 9421)

Substrates that carry HTTP-shaped requests (UCTP-over-WebTransport, UCTP-over-WebSocket, UCTP-over-QUIC-h3) MAY add per-request [RFC 9421 HTTP Message Signatures](https://datatracker.ietf.org/doc/rfc9421/) to envelopes. The headers used are:

| Header | RFC | Purpose |
|---|---|---|
| `Signature` | 9421 | The signature value over the covered components |
| `Signature-Input` | 9421 | Names of the covered components and signature parameters |
| `Signature-Key` | Hardt sister draft | Public JWK identifying the agent that signed the request |
| `Signature-Agent` | Hardt sister draft | Public JWK of the delegating agent / user-side key, when delegation is in play |

A server that receives a signed envelope verifies the signature against the registered signing keys for the Identity claimed in `auth.session.payload.identity_id` and updates the Connection's `IdentityAssurance` accordingly. Servers MAY require signed requests for any envelope that crosses an assurance threshold (e.g., `session.invite` to a UserAuthorized-required Session).

### 5.6 IdentityAssurance gradient

Every authenticated Connection has an `IdentityAssurance` value, returned in `auth.session.payload.assurance`. The gradient is:

| Level | Meaning |
|---|---|
| `anonymous` | No identity claimed |
| `pseudonymous` | Ephemeral keypair the peer can re-prove ownership of, not bound to a durable Identity |
| `identified` | Durable Identity authenticated; no specific authorization granted yet |
| `task-scoped` | Identity + delegation: this token may take this specific action on this specific resource, expiring at a known time |
| `user-authorized` | Identity acts on behalf of a user with declared scopes; highest level |

Servers may require a minimum level for a Session via `identity_assurance_required` in the CapabilityDescriptor (see §8). Connections below the minimum are rejected with `403 forbidden-for-assurance-level` (§11).

### 5.7 Identity vs Participant vs Device

Per voip-3:
- **Identity** is the durable real-world entity (`id_*`).
- **Device** is a physical or software endpoint (`dev_*`).
- **Participant** is an Identity's appearance in a specific Conversation, with a `kind` (human/ai/system/external) and `role` (customer/agent/supervisor/observer/...).

Authentication binds (Identity, Device) to a session_token. When that authenticated client joins a Conversation, a Participant is created (or an existing one is rebound to the new Connection).

### 5.8 Identity envelopes

Three envelopes manage assurance changes after the initial `auth.session`. They cover the case where a client needs to step up from one assurance level to another mid-Session — for example, joining a Session whose `identity_assurance_required` exceeds its current `identity_assurance_offered`.

#### `identity.assurance-changed` (S→C)
Emitted when a Connection's `IdentityAssurance` is updated — typically because step-up auth completed, a delegation expired, or a signing key was revoked.

```json
{
  "type": "identity.assurance-changed",
  "connid": "conn_...",
  "payload": {
    "identity_id": "id_...",
    "previous_assurance": "pseudonymous",
    "new_assurance": "identified",
    "reason": "step-up-passkey-completed" | "delegation-expired" | "key-revoked" | "...",
    "changed_at": "..."
  }
}
```

#### `identity.step-up-request` (S→C)
The server requests the client to present higher-assurance credentials. Triggered when the client sends an envelope that requires assurance above its current level (e.g., `session.invite` to a Session with `identity_assurance_required: user-authorized` from a Connection at `identified`).

```json
{
  "type": "identity.step-up-request",
  "connid": "conn_...",
  "payload": {
    "required_assurance": "task-scoped" | "user-authorized",
    "accepted_methods": ["passkey", "oauth2-dpop", "aauth"],
    "nonce": "...",
    "reason": "session-requires-higher-assurance" | "tenant-policy" | "...",
    "context": {
      "blocked_envelope_id": "<id of the envelope that triggered the request>"
    }
  }
}
```

The blocked envelope is held server-side until step-up succeeds (or fails / times out, in which case it is rejected with `error` code `403-1`).

#### `identity.step-up-response` (C→S)
The client supplies the higher-assurance credential.

```json
{
  "type": "identity.step-up-response",
  "in_reply_to": "<identity.step-up-request.id>",
  "payload": {
    "method": "passkey" | "oauth2-dpop" | "aauth",
    "credential": "<method-specific>"
  }
}
```

On success the server emits `identity.assurance-changed` and processes the originally-blocked envelope. On failure the server emits `error` with code `401-2 step-up-failed` and discards the blocked envelope.

---

## 6. Envelope catalog (overview)

The full v0 catalog. Each is detailed in §7–§11.

| Type | Direction | Purpose |
|---|---|---|
| **Auth (§5)** | | |
| `auth.hello` | C→S | Open authenticated session |
| `auth.challenge` | S→C | Issue challenge |
| `auth.response` | C→S | Respond to challenge |
| `auth.session` | S→C | Confirm authenticated session |
| `auth.keepalive` | C→S | Refresh |
| `auth.bye` | bidi | Graceful close |
| **Conversation (§7)** | | |
| `conversation.create` | C→S, S→C | Open a Conversation explicitly |
| `conversation.opened` | S→C | Notify Participants of open |
| `conversation.closed` | S→C | Notify Participants of close |
| `conversation.list` | C→S | Query Conversations the Participant is in |
| **Session (§7)** | | |
| `session.invite` | bidi | Invite Participants to start a Session |
| `session.accept` | bidi | Accept an invite |
| `session.reject` | bidi | Reject an invite (with reason) |
| `session.cancel` | bidi | Cancel an invite before it's accepted |
| `session.started` | S→C | Session became Active (multicast to Participants) |
| `session.ended` | S→C | Session ended (multicast to Participants) |
| `session.update` | bidi | Mid-session change (codec re-negotiation, role change, etc.) |
| `session.participant.joined` | S→C | A Participant joined |
| `session.participant.left` | S→C | A Participant left |
| **Connection / Stream (§7)** | | |
| `connection.offer` | bidi | Negotiate a Connection's media plane |
| `connection.answer` | bidi | Respond to an offer |
| `connection.ready` | S→C | Connection's media plane is established |
| `connection.update` | bidi | Mid-call connection change (hold, mute, codec) |
| `connection.end` | bidi | End a single Connection |
| `stream.opened` | S→C | A media Stream started flowing |
| `stream.closed` | S→C | A media Stream ended |
| **Message (§9)** | | |
| `message.send` | bidi | Send a Message in a Conversation |
| `message.delivered` | S→C | Delivery receipt |
| `message.read` | bidi | Read receipt |
| `message.history` | C→S | Fetch historical Messages |
| **Capabilities (§8)** | | |
| `capability.advertise` | bidi | Re-advertise capabilities mid-session |
| **DTMF / control (§7.5)** | | |
| `dtmf.send` | bidi | Send DTMF digits on a Connection |
| `dtmf.received` | S→C | DTMF received from far end |
| **vCon (§7.6)** | | |
| `recording.vcon-ready` | S→C | Emitted at session.ended when the Session's vCon is finalized, signed, and persisted |
| `recording.vcon-fetch` | C→S | Request retrieval of a previously emitted vCon by handle |
| `recording.vcon-fetched` | S→C | Response carrying the signed vCon body or a download URL |
| **Quality (§10.3)** | | |
| `connection.quality` | bidi | Per-Stream quality snapshot (loss, jitter, RTT, MOS, bitrate) |
| **Identity (§5.6/§5.8)** | | |
| `identity.assurance-changed` | S→C | A Connection's IdentityAssurance was updated mid-Session (e.g., step-up auth completed) |
| `identity.step-up-request` | S→C | Server requests the client to present higher-assurance credentials |
| `identity.step-up-response` | C→S | Client supplies higher-assurance credentials |
| **Errors / control (§11)** | | |
| `error` | bidi | Out-of-band error report |
| `ack` | bidi | Generic acknowledgment |

---

## 7. Lifecycle on the wire

### 7.1 Conversation lifecycle

A Conversation may be **implicit** (created when the first envelope referencing it arrives, if the server's policy allows) or **explicit** (`conversation.create` issued first). For Thelve and most production servers, explicit creation is the default — it carries metadata (tenant, customer reference, channel binding) the server needs.

```
          C→S: conversation.create {tenant_id, metadata}
          S→C: conversation.opened {cid, participants:[]}
              │
              │ (Messages and/or Sessions added over time)
              │
          C→S: (last Participant leaves OR explicit conversation.close)
          S→C: conversation.closed {cid, reason}
```

A Conversation's **closure rule** is server-policy. UCTP defines two reference policies:
- **`ephemeral`** — close the Conversation when its last Session ends and no Messages have arrived in N seconds (default 60s).
- **`persistent`** — never auto-close; close only on explicit request.

The policy is set at `conversation.create` time (`payload.policy`) and may be changed via `conversation.update` (deferred to v1).

#### `conversation.create` (C→S)
```json
{
  "type": "conversation.create",
  "cid": null,
  "payload": {
    "tenant_id": "...",
    "policy": "ephemeral" | "persistent",
    "idle_close_secs": 60,
    "metadata": { /* application-specific key/value pairs */ },
    "initial_participants": [
      { "identity_id": "id_...", "role": "customer" | "agent" | "supervisor" | "observer" | "..." }
    ]
  }
}
```

`tenant_id` is required for production servers; the server assigns the resulting `cid`. `idle_close_secs` is honored only when `policy = ephemeral`. `initial_participants` is optional — Participants may also be added implicitly when they join Sessions or send Messages.

#### `conversation.opened` (S→C)
```json
{
  "type": "conversation.opened",
  "cid": "conv_...",
  "in_reply_to": "<conversation.create.id, when in response to a create>",
  "payload": {
    "tenant_id": "...",
    "policy": "ephemeral" | "persistent",
    "idle_close_secs": 60,
    "participants": [
      {
        "participant_id": "part_...",
        "identity_id": "id_...",
        "kind": "human" | "ai" | "system" | "external",
        "role": "customer" | "agent" | "supervisor" | "observer" | "...",
        "display_name": "..."
      }
    ],
    "opened_at": "...",
    "metadata": { ... }
  }
}
```

Emitted on initial open (with `in_reply_to`) and as the response payload for matching entries in `conversation.list` (without `in_reply_to`, or with `in_reply_to` pointing at the `conversation.list` envelope).

#### `conversation.closed` (S→C)
```json
{
  "type": "conversation.closed",
  "cid": "conv_...",
  "payload": {
    "reason_code": 200,
    "reason": "normal-closure" | "idle-timeout" | "explicit-close" | "policy-eviction" | "tenant-deleted",
    "closed_at": "..."
  }
}
```

Multicast to every Participant currently subscribed to the Conversation.

#### `conversation.list` (C→S)
```json
{
  "type": "conversation.list",
  "cid": null,
  "payload": {
    "filter": {
      "tenant_id": "...",
      "participant_id": "part_...",
      "identity_id": "id_...",
      "state": "open" | "closed" | "all",
      "since": "...",
      "until": "..."
    },
    "cursor": "...",
    "limit": 50
  }
}
```

> **Response.** The server replies with a stream of `conversation.opened` envelopes (one per matching Conversation), each carrying `in_reply_to` set to the `conversation.list` envelope's `id`, terminated by an `ack` whose payload includes `next_cursor` (string) when more results remain.

### 7.2 Session lifecycle

```
Initiator            Server                Invitee(s)
   │                    │                       │
   │ session.invite ───►│                       │
   │                    │── session.invite ────►│
   │                    │                       │
   │                    │◄─── session.accept ───│
   │◄─── session.accept │  (relayed)            │
   │                    │                       │
   │ connection.offer ─►│ (per Participant)     │
   │                    │── connection.offer ──►│
   │◄── connection.answer ◄── connection.answer │
   │                    │                       │
   │                    │── session.started ───►│ (multicast)
   │◄── session.started │                       │
   │                    │                       │
   │   ◄── media flows over substrates ──►      │
   │                    │                       │
   │                    │◄─── session.end ──────│ (any Participant)
   │                    │── session.ended ─────►│ (multicast)
   │◄── session.ended   │                       │
```

#### `session.invite`
```json
{
  "type": "session.invite",
  "cid": "conv_...",
  "sid": "sess_...",
  "payload": {
    "from": "part_...",
    "to": ["part_..."]  | ["id_..."]  | ["sip:bob@example.com"],
    "medium": "voice" | "video" | "voice+video" | "screen-share" | "text-chat" | "mixed",
    "intent": "synchronous-engagement",
    "capabilities_offer": { /* CapabilityDescriptor — initiator's offer */ }
  }
}
```

The `to` field accepts Participant IDs (already in the Conversation), Identity IDs (server resolves to Participant), or interop URIs (`sip:`, `tel:`, etc. — server gateways).

#### `session.accept`
```json
{
  "type": "session.accept",
  "in_reply_to": "<session.invite.id>",
  "sid": "sess_...",
  "payload": {
    "by": "part_...",
    "capabilities_answer": { /* CapabilityDescriptor — invitee's answer */ }
  }
}
```

The Session enters `Active` once at least one invitee has accepted **and** at least one Connection per accepted Participant is `connection.ready`.

#### `session.reject`
```json
{
  "type": "session.reject",
  "in_reply_to": "<session.invite.id>",
  "payload": {
    "by": "part_...",
    "reason_code": 486,
    "reason": "busy"
  }
}
```

#### `session.end`
```json
{
  "type": "session.end",
  "sid": "sess_...",
  "payload": {
    "by": "part_...",
    "reason_code": 200,
    "reason": "normal-clearing"
  }
}
```

#### `session.update`
Used for: codec re-negotiation, role change (escalate "observer" → "agent"), medium upgrade ("text-chat" → "voice+text-chat"). Idempotent; latest wins.

```json
{
  "type": "session.update",
  "sid": "sess_...",
  "payload": {
    "kind": "codec-renegotiate" | "role-change" | "medium-upgrade",
    "details": { ... }
  }
}
```

#### `session.cancel`
Cancels a `session.invite` before it has been accepted. After acceptance, use `session.end` instead.

```json
{
  "type": "session.cancel",
  "in_reply_to": "<session.invite.id>",
  "sid": "sess_...",
  "payload": {
    "by": "part_...",
    "reason_code": 487,
    "reason": "request-cancelled"
  }
}
```

#### `session.started` (S→C, multicast)
Emitted to every Participant once the Session enters `Active` (per the boundary rule in §7.3).

```json
{
  "type": "session.started",
  "sid": "sess_...",
  "cid": "conv_...",
  "payload": {
    "started_at": "...",
    "participants_present": ["part_...", "..."],
    "active_connections": [
      {
        "connid": "conn_...",
        "participant_id": "part_...",
        "transport": "quic" | "webtransport" | "websocket+webrtc" | "sip-interop" | "webrtc-interop"
      }
    ],
    "negotiated_capabilities": { /* CapabilityIntersection — see §8 */ }
  }
}
```

#### `session.ended` (S→C, multicast)
Emitted once the Session transitions to `Ended` (last Connection terminated and the §7.3 grace window elapsed without reconnect).

```json
{
  "type": "session.ended",
  "sid": "sess_...",
  "cid": "conv_...",
  "payload": {
    "ended_at": "...",
    "by": "part_..." | null,
    "reason_code": 200,
    "reason": "normal-clearing" | "all-connections-ended" | "session-failed" | "grace-window-expired" | "...",
    "vcon_handle": { /* see §7.6; present only if the vCon was finalized synchronously, otherwise delivered later via recording.vcon-ready */ }
  }
}
```

#### `session.participant.joined` (S→C, multicast)
```json
{
  "type": "session.participant.joined",
  "sid": "sess_...",
  "cid": "conv_...",
  "payload": {
    "participant": {
      "participant_id": "part_...",
      "identity_id": "id_..." | "f_...",
      "kind": "human" | "ai" | "system" | "external",
      "role": "customer" | "agent" | "supervisor" | "observer" | "...",
      "display_name": "...",
      "joined_at": "..."
    },
    "via_connection": "conn_..."
  }
}
```

#### `session.participant.left` (S→C, multicast)
```json
{
  "type": "session.participant.left",
  "sid": "sess_...",
  "cid": "conv_...",
  "payload": {
    "participant_id": "part_...",
    "left_at": "...",
    "reason": "explicit-leave" | "all-connections-ended" | "kicked" | "transferred-out"
  }
}
```

### 7.3 Session boundary rules

A Session is `Active` while **at least one Connection is in state `Connected` OR a `connection.offer`/`connection.answer` is in flight**. When the last Connection ends and no negotiation is in flight, the server sends `session.ended`.

Reconnection within a grace window (default 30s) is **not** a new Session — it is a new Connection within the existing Session. The grace window is server-policy and may be set at `session.invite` time. After the grace window, the Session has ended; reconnecting requires a new `session.invite`.

This addresses voip-3's open question on Session boundaries (voip-3 §11 does not specify; we settle it here).

### 7.4 Connection lifecycle

A Connection is one Participant's transport-bound attach to a Session. The same Participant may have multiple simultaneous Connections (rare; example: a worker on both a SIP desk phone and a UCTP mobile app).

```
connection.offer  ─► capabilities + substrate-specific media setup
connection.answer ◄─ negotiated codecs + answer to media setup
connection.ready  ◄─ media flowing in both directions
   │
   │ (mid-life: connection.update for hold/mute/codec change)
   │
connection.end    ─► stops media; Session may continue if other Connections live
```

#### `connection.offer`
```json
{
  "type": "connection.offer",
  "sid": "sess_...",
  "connid": "conn_...",
  "payload": {
    "by_participant": "part_...",
    "substrate": "quic" | "webtransport" | "websocket+webrtc" | "sip-interop" | "webrtc-interop",
    "capabilities": { /* CapabilityDescriptor */ },
    "streams_offered": [
      {
        "id": "strm_...",
        "kind": "audio" | "video" | "data",
        "direction": "sendrecv" | "sendonly" | "recvonly",
        "codec_preferences": ["opus", "g.711-mu", "g.711-a"]
      }
    ],
    "substrate_setup": { /* see §10; e.g. WebRTC SDP-frag for websocket+webrtc */ }
  }
}
```

#### `connection.answer`
Mirrors `offer`; selects codecs from the offerer's preferences within the answerer's capabilities.

#### `connection.update`
- Hold: `payload.action = "hold"` — sets all media Streams to `sendonly`/`recvonly` per direction.
- Resume: `payload.action = "resume"` — restores `sendrecv`.
- Mute: `payload.action = "mute"` with `payload.streams = ["strm_..."]`.
- Codec re-negotiation: `payload.action = "codec-renegotiate"` with new `codec_preferences`.

#### `connection.end`
```json
{
  "type": "connection.end",
  "connid": "conn_...",
  "payload": {
    "reason_code": 200,
    "reason": "normal-clearing"
  }
}
```

#### `stream.opened` (S→C)
Emitted once a media Stream begins flowing on a Connection (after `connection.ready` has fired and frames are observed in both directions, per stream direction).

```json
{
  "type": "stream.opened",
  "connid": "conn_...",
  "sid": "sess_...",
  "payload": {
    "stream": {
      "strm_id": "strm_...",
      "kind": "audio" | "video" | "screenshare" | "data",
      "codec": { "name": "opus", "params": { "sample_rate": 48000, "channels": 2 } },
      "direction": "sendrecv" | "sendonly" | "recvonly",
      "stream_local_id": 7,
      "opened_at": "..."
    }
  }
}
```

`stream_local_id` is the per-Connection 16-bit handle that appears in datagram headers (§10.1). It is assigned at `connection.ready` and announced here.

#### `stream.closed` (S→C)
```json
{
  "type": "stream.closed",
  "connid": "conn_...",
  "sid": "sess_...",
  "payload": {
    "strm_id": "strm_...",
    "closed_at": "...",
    "reason_code": 200,
    "reason": "normal-clearing" | "codec-renegotiated" | "endpoint-removed" | "transport-error"
  }
}
```

When `reason = codec-renegotiated`, a new `stream.opened` for the same `strm_id` (or a new `strm_id`, depending on the negotiation outcome) follows immediately.

### 7.5 DTMF

```json
{
  "type": "dtmf.send",
  "connid": "conn_...",
  "payload": {
    "digits": "1234#",
    "duration_ms": 100,
    "method": "rfc4733" | "info"  // the gateway translates to RFC 2833 / SIP INFO for SIP interop
  }
}
```

UCTP-native Connections deliver DTMF as `dtmf.received` envelopes. The gateway translates SIP DTMF (RFC 2833 / SIP INFO) into `dtmf.received` for UCTP-speaking peers in the same Session.

#### `dtmf.received` (S→C)
```json
{
  "type": "dtmf.received",
  "connid": "conn_...",
  "sid": "sess_...",
  "payload": {
    "digits": "1234#",
    "duration_ms": 100,
    "received_at": "...",
    "source": "uctp-native" | "sip-rfc4733" | "sip-info" | "webrtc"
  }
}
```

`source` lets the consumer attribute DTMF correctly when bridging across substrates (e.g., distinguish in-band SIP DTMF that survived translation from native UCTP DTMF).

### 7.6 vCon emission and retrieval

UCTP servers emit a [vCon](https://datatracker.ietf.org/doc/draft-ietf-vcon-vcon-core/) (the IETF Virtualized Conversations envelope) for **every Session at session.ended**, regardless of whether audio recording was enabled. The vCon is the durable signed JSON record of who joined, what happened, and what analyses were attached. See `INTERFACE_DESIGN.md` §3.9 / §11.4 for the server-side construction.

#### `recording.vcon-ready` (server → client, multicast to Session participants)
```json
{
  "type": "recording.vcon-ready",
  "cid": "conv_...",
  "sid": "sess_...",
  "payload": {
    "vcon_handle": {
      "uuid": "01HXYZ...",
      "url": "https://store.example.com/vcons/01HXYZ.vcon.jws",
      "content_hash": "sha256:abcdef...",
      "group": "01HXYZ...",
      "created_at": "..."
    },
    "encrypted": false,             // true if JWE-wrapped
    "signed_by": ["tenant", "ai-provider"]  // entities that JWS-signed
  }
}
```

The vCon is **not** delivered inline — clients fetch it explicitly via `recording.vcon-fetch`. This keeps `recording.vcon-ready` small (it broadcasts to all Session participants) while letting consumers pull only the vCons they want.

#### `recording.vcon-fetch` (client → server)
```json
{
  "type": "recording.vcon-fetch",
  "payload": {
    "uuid": "01HXYZ..."           // OR url+content_hash; either form acceptable
  }
}
```

#### `recording.vcon-fetched` (server → client, response)
```json
{
  "type": "recording.vcon-fetched",
  "in_reply_to": "<recording.vcon-fetch.id>",
  "payload": {
    "delivery": "inline" | "url",  // small vCons inline; large ones return a download URL
    "vcon": "<JWS body>",          // when delivery=inline
    "url": "https://...",          // when delivery=url; URL carries access token in query string or via co-issued Bearer
    "expires_at": "..."            // when delivery=url
  }
}
```

Access policy: a vCon is fetchable by participants of the Session (via their `auth.session` token) plus any tenant-level role granted vCon-read scope. The server enforces; UCTP carries the request and response.

---

## 8. Capability negotiation

A `CapabilityDescriptor` describes what an endpoint can encode/decode and what protocol features it supports.

```json
{
  "audio_codecs": [
    {"name": "opus", "params": {"sample_rate": 48000, "channels": 2, "fec": true}},
    {"name": "g.711-mu", "params": {"sample_rate": 8000}},
    {"name": "g.711-a", "params": {"sample_rate": 8000}}
  ],
  "video_codecs": [
    {"name": "h264", "params": {"profile": "baseline", "level": "3.1"}},
    {"name": "vp9", "params": {}}
  ],
  "data_protocols": ["text", "json", "binary"],
  "dtmf_modes": ["rfc4733", "info"],
  "max_streams_per_connection": 8,
  "transport_features": [
    "media-datagrams",
    "connection-migration",
    "session-resumption",
    "0rtt"
  ],
  "interop": ["sip", "webrtc"],     // present only if the endpoint can be gatewayed to/from these

  "identity_assurance_offered": "identified",      // §5.6 gradient
  "identity_assurance_required": "task-scoped"     // optional; minimum the peer requires from peers
}
```

Servers MAY require a minimum `identity_assurance` for a Session via `identity_assurance_required` in their offer. Connections whose `identity_assurance_offered` falls short are rejected with `403 forbidden-for-assurance-level` (§11). When two Connections of different assurance levels are in the same Session and their requirements are mutually compatible, the Session's effective assurance is the lower of the two; the vCon `parties[]` records each Participant's individual level.

### 8.1 Negotiation algorithm

At `connection.offer` / `connection.answer` exchange:

1. The offerer lists `streams_offered` with `codec_preferences` (ordered).
2. For each offered stream, the answerer:
   - Walks the offerer's `codec_preferences` in order
   - Picks the first codec it supports (i.e., advertises in its own `CapabilityDescriptor.audio_codecs` or `video_codecs`)
   - Returns it in the answer
3. If no codec matches for a stream, that stream is rejected; the rest may proceed.
4. If the result leaves the Connection with zero usable Streams, the answerer rejects the offer with reason `488 Not Acceptable Here`.

### 8.2 Mid-session re-negotiation

`connection.update` with `action: "codec-renegotiate"` re-runs the algorithm. Used for: bandwidth adaptation, switching to a more efficient codec mid-call, transcoding fallback after detecting loss.

### 8.3 Transcoding

If two Connections in the same Session have **disjoint** codec sets and the server (gateway) supports transcoding for that codec pair, the server inserts a transcoder in the media path. Common case: G.711 (SIP / PSTN) ↔ Opus (UCTP-native). The server advertises which transcoding pairs it supports via its own `CapabilityDescriptor.transport_features` (e.g., `transcode-g711-opus`).

If transcoding is unsupported and codec sets are disjoint, the Session fails with reason `488` and the gateway does not bridge.

### 8.4 Re-advertising capabilities mid-Session (`capability.advertise`)

A peer that gains, loses, or changes capabilities mid-Session uses `capability.advertise` to re-publish its descriptor. Common triggers: a Connection adds a video Stream after starting audio-only; an operator policy installs or revokes a codec; an endpoint detects degraded network and prefers a more efficient codec.

```json
{
  "type": "capability.advertise",
  "connid": "conn_...",
  "sid": "sess_...",
  "payload": {
    "by_participant": "part_...",
    "capabilities": { /* CapabilityDescriptor — see §8 */ },
    "trigger": "endpoint-change" | "operator-policy" | "renegotiation-requested" | "network-adapt"
  }
}
```

Receivers compare the new descriptor to the negotiated set; if the intersection changes, the receiver MAY initiate `connection.update {action: codec-renegotiate}` (per §8.2) to apply the new agreement. `capability.advertise` is idempotent — latest wins per `(connid, by_participant)`.

---

## 9. Messaging

Messages are atomic asynchronous events in a Conversation. They do not require a Session.

### 9.1 `message.send`

```json
{
  "type": "message.send",
  "cid": "conv_...",
  "payload": {
    "msg_id": "msg_...",
    "from": "part_...",
    "to": ["part_..."] | "all",
    "content_type": "text/plain" | "application/json" | "application/octet-stream" | "image/png" | ...,
    "body": "Hello, world",            // string for text/json; base64 for binary; reference URL for large attachments
    "attachments": [
      {
        "id": "...",
        "content_type": "image/png",
        "url": "https://...",
        "size_bytes": 12345
      }
    ],
    "in_reply_to_msg": "msg_..."       // optional; threads/replies
  }
}
```

### 9.2 Receipts

`message.delivered` (server→client when the message reaches the recipient's substrate) and `message.read` (client→server, relayed to other Participants) are independent. Either can be disabled by client preference.

#### `message.delivered` (S→C)
```json
{
  "type": "message.delivered",
  "cid": "conv_...",
  "payload": {
    "msg_id": "msg_...",
    "to_participant": "part_...",
    "delivered_at": "...",
    "via_connection": "conn_..."
  }
}
```

`via_connection` is optional and identifies which Connection received the delivery (useful when a Participant has multiple Devices).

#### `message.read` (bidi)
```json
{
  "type": "message.read",
  "cid": "conv_...",
  "payload": {
    "msg_id": "msg_...",
    "by_participant": "part_...",
    "read_at": "..."
  }
}
```

A client sends `message.read` to the server when the user views the Message; the server relays it to other Participants in the Conversation as `message.read` envelopes. Senders should not block on receipt arrival — receipts are advisory.

### 9.3 History

`message.history` queries past Messages in a Conversation. Pagination via `cursor`. Server may apply per-tenant retention rules and return only the visible window.

#### `message.history` (C→S)
```json
{
  "type": "message.history",
  "cid": "conv_...",
  "payload": {
    "since": "...",
    "until": "...",
    "since_msg_id": "msg_...",
    "cursor": "...",
    "limit": 100,
    "include_attachments": true
  }
}
```

All filter fields are optional. When `include_attachments = false`, replayed `message.send` envelopes carry an `attachments[]` summary (`id`, `content_type`, `size_bytes`) without the body or URL — clients can fetch full attachments lazily.

> **Response.** The server replays matching Messages as a stream of `message.send` envelopes (each with `in_reply_to` set to the `message.history` envelope's `id`) in chronological order, terminated by an `ack` whose payload includes `next_cursor` (string) when more results remain.

### 9.4 Large attachments

Attachments larger than ~64KB use **out-of-band upload**: client `PUT`s the binary to a server-issued upload URL (HTTPS), receives a content URL, and sends `message.send` with the URL in `attachments[].url`. UCTP envelopes themselves stay JSON-friendly.

---

## 10. Media transport (Streams)

### 10.1 Datagram framing (QUIC and WebTransport)

Each media datagram carries one media frame, framed as:

```
+----+----+----+----+----+----+----+----+
|        UCTP datagram header (8 bytes)   |
+---------------------------------------+
|          payload (RTP packet)         |
+---------------------------------------+
```

UCTP datagram header:
```
0       1       2       3       4       5       6       7
+-------+-------+-------+-------+-------+-------+-------+-------+
| ver=1 |  flags  |    stream_local_id (uint16, big-endian)   |
+---------------------------------------------------------------+
|              datagram_seq (uint32, big-endian)                |
+---------------------------------------------------------------+
```

- `stream_local_id` is a per-Connection 16-bit handle assigned at `connection.ready`. Maps to a `strm_*` Stream ID.
- `datagram_seq` lets the receiver detect loss and out-of-order arrival without parsing RTP.
- Payload is a standard RTP packet (RFC 3550) including its own RTP header.

This dual-header approach (UCTP datagram header + RTP header) is intentional: the UCTP header makes the datagram self-describing for routing across many Connections on one substrate; the RTP header preserves compatibility with codecs and tooling that expect RTP.

### 10.2 WebSocket fallback (no datagrams)

When UCTP runs over WebSocket, media does **not** flow on the WebSocket. Instead, the Connection negotiates a co-located WebRTC PeerConnection. The signaling for the PeerConnection (ICE candidates, DTLS fingerprints, SDP) is carried as `connection.offer`/`connection.answer` payload fields under `substrate_setup`. The PeerConnection's media uses standard WebRTC DTLS-SRTP.

This is the only case where UCTP envelopes carry SDP-shaped payloads. The SDP is for the WebRTC media plane only — not for the Session, not for the Connection's UCTP-level identity.

### 10.3 RTCP / quality feedback

Quality reports (loss, jitter, RTT, MOS) are carried as `connection.quality` envelopes (signaling channel), not as RTCP-on-datagrams. RTCP is preserved when interoperating with SIP/WebRTC, but UCTP-native peers exchange structured quality JSON.

```json
{
  "type": "connection.quality",
  "connid": "conn_...",
  "payload": {
    "interval_ms": 5000,
    "streams": [
      {
        "strm_id": "strm_...",
        "loss_pct": 0.4,
        "jitter_ms": 12,
        "rtt_ms": 80,
        "mos": 4.1,
        "bitrate_bps": 32000,
        "packets_sent": 250,
        "packets_received": 249
      }
    ]
  }
}
```

Cadence: default every 5s; tunable per Connection via `connection.update`.

---

## 11. Errors and acknowledgment

### 11.1 `error` envelope

```json
{
  "type": "error",
  "in_reply_to": "<offending envelope id, if applicable>",
  "payload": {
    "code": 488,
    "category": "protocol" | "auth" | "media" | "policy" | "transient",
    "reason": "incompatible-capabilities",
    "details": { /* type-specific */ }
  }
}
```

### 11.2 Error code ranges

| Range | Category | Examples |
|---|---|---|
| 200–299 | informational / success | 200 normal-clearing |
| 400–499 | client error | 401 unauthenticated, 401-1 invalid-signature (RFC 9421 verification failed), 401-2 step-up-failed (`identity.step-up-response` rejected by server), 403 forbidden, 403-1 forbidden-for-assurance-level (Connection's assurance below Session minimum), 404 not-found, 408 timeout, 409 conflict, 409-1 vcon-redaction-conflict (concurrent redaction attempts), 410 gone, 411 vcon-not-found, 486 busy, 487 request-cancelled (matches `session.cancel`), 488 incompatible-capabilities |
| 500–599 | server error | 500 internal, 502 upstream-failure, 503 unavailable, 504 gateway-timeout, 510 vcon-store-unavailable |
| 600–699 | global / terminal | 603 decline, 604 does-not-exist-anywhere |

Codes are intentionally aligned with SIP/HTTP for ease of mental mapping; UCTP servers may map SIP responses straight to UCTP codes when bridging.

### 11.3 `ack` envelope

For envelopes that do not have a structured response (e.g., `message.send`, `connection.update`), the receiver may send a generic `ack`:

```json
{
  "type": "ack",
  "in_reply_to": "<envelope id>",
  "payload": {}
}
```

Acks are advisory — clients should not block on them. Idempotency (via `id`) is the actual reliability mechanism.

---

## 12. SIP and WebRTC interop (gateway boundary)

This section describes how a UCTP server bridges UCTP-speaking participants to SIP / WebRTC participants in the same Session. Implementations MAY support gatewaying; UCTP itself is silent on the implementation.

### 12.1 SIP interop

| UCTP envelope | SIP behavior at gateway |
|---|---|
| `session.invite` toward a `sip:` URI | Gateway sends INVITE to the SIP target |
| `session.accept` from SIP side | Gateway received 200 OK; sends `session.accept` to UCTP peers |
| `connection.offer`/`answer` toward SIP | Gateway translates to SDP offer/answer in INVITE/200 OK |
| `connection.update` (hold) toward SIP | Gateway re-INVITEs with `a=sendonly` |
| `dtmf.send` toward SIP | Gateway sends RFC 2833 or SIP INFO per the SIP peer's preferences |
| `message.send` toward SIP | Gateway sends SIP MESSAGE |
| `session.end` | Gateway sends BYE |
| `session.participant.joined` (UCTP-side new participant) | Not visible on SIP; SIP is two-party at the dialog layer |

Capability translation: the gateway maps UCTP `CapabilityDescriptor` to SDP m-lines and vice versa. Codec set is the intersection of UCTP-side and SIP-side capabilities, optionally extended by transcoders.

### 12.2 WebRTC interop

| UCTP envelope | WebRTC behavior at gateway |
|---|---|
| `session.invite` toward WebRTC | Gateway initiates signaling with the WebRTC peer (via whatever signaling the peer expects — Janus, mediasoup, custom) |
| `connection.offer`/`answer` toward WebRTC | Gateway issues `createOffer`/`setRemoteDescription` and exchanges SDP |
| `connection.update` (track changes) | Gateway adds/removes tracks on the PeerConnection |
| `dtmf.send` toward WebRTC | Gateway sends DTMF on the matching audio sender |
| `message.send` toward WebRTC | Gateway sends DataChannel message |
| `session.end` | Gateway calls `close()` on the PeerConnection |

WebRTC does not have a native protocol-level "registration" — the gateway maintains its own mapping from WebRTC peers to UCTP `Identity` / `Device` records.

### 12.3 Gateway is not a tunnel

The gateway translates **intent** (what the UCTP envelope is trying to accomplish) to **protocol-native operations** (SIP method calls, WebRTC API calls), and translates **outcomes** back. UCTP envelopes are never serialized into SIP message bodies or WebRTC SDP. This is interop by design.

---

## 13. Federation (reserved for v1+)

UCTP is designed to support federation between UCTP-speaking servers (analogous to Matrix homeserver federation or SMTP/email federation), but v0 does not specify the federation envelopes.

Reserved namespace: `federation.*` envelope types and the `f_` prefix on Identity IDs that are sourced from a federated server. The `f_` prefix replaces the local `id_` prefix (it is not appended to it) — an Identity is either locally homed (`id_<ULID>`) or federated (`f_<ULID>`), never both. The trailing ULID is the federated server's local Identity ULID; the `f_` simply marks federation provenance to the receiving server.

The minimum federation surface in v1 will likely include:
- `federation.discover` — announce server capabilities
- `federation.session.invite` — invite a Participant whose Identity is on a remote server
- `federation.message.deliver` — deliver a Message across servers
- Identity verification / signing across the federation boundary

**Federation identity backbone (planned).** [AAuth's 4-party federated mode](https://aauth.dev) — the identity gradient with cross-issuer agent verification — is the planned identity backbone for v1+ federation. A UCTP server discovers a peer server's `/.well-known/aauth-agent`, exchanges signing keys, and then federates `session.invite` envelopes signed via RFC 9421 + Signature-Key headers. This is contingent on AAuth stabilizing; if it does not, the federation backbone will fall back to OAuth 2.1 client-credentials with DPoP. Per the project's PRD §14.2 item 10, AAuth is experimental in v1.

**vCon federation.** When a federated Session ends, each participating server's vCon is independently signed by that server's key and linked via the vCon `group` UUID. Cross-server vCon retrieval is gated by the federation-level access policy. Details to follow in v1.

Federation requires non-trivial decisions on trust, anti-spam, per-tenant policy, and identity portability. v0 does not address them.

---

## 14. Open questions

These need closure before v1.

1. **Binary encoding choice.** CBOR vs. custom. Decision after profiling.
2. **Federation model.** Mesh, hierarchical, hybrid. Out of scope for v0; major decision for v1.
3. **End-to-end encryption.** Currently UCTP relies on substrate TLS/QUIC TLS for hop-by-hop confidentiality, plus optional E2EE at the application layer (e.g., libsignal-style ratchet on `message.send` payloads). A protocol-level E2EE story is deferred.
4. **Multi-party media.** v0 specifies 1:1 media bridging at the gateway (per `INTERFACE_DESIGN.md` decision); >2-party voice/video would require an SFU/MCU integration. Whether UCTP itself reserves SFU control envelopes (`sfu.subscribe`, `sfu.unsubscribe`) or treats SFU as out-of-band is open.
5. **Recording / lawful intercept.** Should be UCTP-native `recording.start` / `recording.stop` envelopes, or pushed to a server-side admin API? Lean: UCTP-native, with strong policy controls.
6. **Conversation persistence semantics.** voip-3 says Conversations are durable. Does the server hold them indefinitely? Per-tenant retention? Deletion semantics? Needs settling for compliance.
7. **Presence beyond reachability.** `auth.session` returns reachability hints, but rich presence ("busy", "do-not-disturb", "in-meeting") is application-level today. Should UCTP define a small core (`available` / `busy` / `away` / `dnd`) and let applications extend?
8. **Group Conversations & Sessions at scale.** Default policy for joining a Conversation that already has 1000+ Participants — invite-only, open, ACL? Out of scope for v0.
9. **Push notifications.** Mobile clients sleep; UCTP server needs to wake them via APNs/FCM. The bridge (`push.register` / `push.deliver`) is needed but not specified here.
10. **Rate limiting and back-pressure.** Per-Connection envelope rate limits and how the substrate signals back-pressure (QUIC stream flow control vs. application-level pacing).

---

## 15. Gaps in voip-3 surfaced by writing this protocol

Per project direction, voip-3 (`/Users/jonathan/Developer/Rudeless/voip-3-conversation-model.md`) is **not modified**; gaps are surfaced here. Writing UCTP made these explicit:

1. **No identity/auth flow.** voip-3 §11 lists this as open. UCTP §5 commits to a flow.
2. **No formal envelope/command vocabulary.** voip-3 has lifecycle verbs only. UCTP §6 catalogs envelopes.
3. **No Session boundary rules.** When does a Session end vs. continue across Connection drops? UCTP §7.3 commits to a 30s reconnect grace window default.
4. **No Conversation closure rules.** voip-3 §6.1 says "closes when relationship concludes" — UCTP §7.1 names two reference policies (`ephemeral`, `persistent`) and a default closure rule.
5. **No capability-negotiation schema.** voip-3 §9.3 hand-waves "Session abstracts the differences." UCTP §8 commits to a `CapabilityDescriptor` and an intersection algorithm.
6. **No registration/reachability vocabulary.** UCTP §5.3 introduces `ReachabilityHint`.
7. **No mid-Session join semantics.** UCTP §7.2 commits to the `session.participant.joined`/`left` model and how invites work for already-active Sessions.
8. **No DTMF / signaling-side control.** UCTP §7.5.
9. **No quality reporting on the wire.** UCTP §10.3.
10. **No error model.** UCTP §11.
11. **No interop boundary specification.** voip-3 implies SIP/WebRTC are peer transports; UCTP §12 commits to gateway-not-tunnel.
12. **No multi-tenancy threading.** UCTP carries `tenant_id` in Conversation metadata and authenticated session_token; voip-3 defers multi-tenancy.
13. **No idempotency story.** UCTP §3.2 / §11.3 commits via envelope `id`.
14. **No federation reservation.** UCTP §13 reserves namespace.
15. **No conversation envelope.** voip-3 has no equivalent of [vCon](https://datatracker.ietf.org/doc/draft-ietf-vcon-vcon-core/) — the IETF Virtualized Conversations standard. UCTP §7.6 commits to vCon emission for every Session and adds `recording.vcon-ready` / `recording.vcon-fetch` envelopes. Mapping voip-3 nouns to vCon: Participant → Party, Session → Dialog, Message → Dialog with `type=text`, Conversation → vCon `group` UUID.
16. **No identity-assurance gradient.** voip-3 §11 lists identity as open. UCTP §5.6 commits to the gradient (Anonymous → Pseudonymous → Identified → TaskScoped → UserAuthorized) and routes assurance through `auth.session`, `CapabilityDescriptor`, and `403 forbidden-for-assurance-level` errors.
17. **No per-request signing model.** voip-3 has no protocol-level message authentication. UCTP §5.5 commits to [RFC 9421 HTTP Message Signatures](https://datatracker.ietf.org/doc/rfc9421/) plus Hardt's `Signature-Key` / `Signature-Agent` headers for substrates that carry HTTP-shaped requests.
18. **No agent-identity model.** UCTP §5 lists `aauth` as a supported (experimental) auth method. AAuth (`draft-hardt-oauth-aauth-protocol`) is the candidate agent-to-agent identity protocol — accommodated, not yet committed to.

These items are **decisions made by UCTP** for the purpose of being a real protocol. A future voip-3 revision may fold any of them into the conceptual model or explicitly defer them.

---

## 16. Versioning

UCTP uses a single integer version field (`v` on every envelope). The current revision is **v0 (working draft)** in this document, which on the wire is `v: 1`. The wire integer is monotonically incremented per breaking revision; the document label ("v0", "v1", ...) names the revision for human reference and may stay one version "behind" the wire field while a draft stabilizes. Once v0 is finalized, the document label and wire field move in lockstep (`v1` doc ↔ `v: 1` wire). Protocol changes:

- **Additive** (new envelope type, new optional field): no version bump. Old endpoints ignore unknown.
- **Breaking** (changed semantics of an existing field, removed envelope type): version bump. Servers MAY support multiple versions concurrently during transition windows.

The version negotiation happens implicitly: each envelope declares its version. A receiver that cannot speak the offered version replies with `error` code `505` (`version-not-supported`) and the sender retries at a lower version.

---

**Reviewers:** §3 (envelope format), §5 (auth), §7 (lifecycle), §8 (capabilities), and §10 (media transport) are the load-bearing sections — those settle the wire shape. §12 (interop) pins down the gateway boundary, which is the unique architectural commitment of UCTP relative to SIP and WebRTC.

This is a v0 working draft; expect breaking changes until v1.
