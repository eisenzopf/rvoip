# RVoIP Platform Layer Roadmap

Date: 2026-05-05

This document records the next architectural step after the current
`session-core` work: how to build contact-center, voice-ai, CPaaS API,
and QSRP support without prematurely splitting the stack or exposing an
unstable API.

## Decision

Build a platform layer above `session-core`.

Do not split `session-core` into separate `dial-core` and media orchestration
crates yet. `session-core` already owns the practical SIP dialog/session/media
coordination needed by the next layer, and `UnifiedCoordinator` is now the
right low-level API for multi-call applications.

The next production crate should be `crates/b2bua`. The B2BUA layer proves the
core topology model: inbound leg, outbound leg, bridge lifecycle, transfer,
hangup propagation, event correlation, and route failure behavior. Once that
model is stable, extract a small shared platform command/event contract that
CPaaS, QSRP, contact-center, and voice-ai adapters can all use.

The sequencing decisions are:

1. Build `rvoip-b2bua` first.
2. Define CPaaS as a contract-first control plane, but delay the full API server
   until B2BUA has proven stable domain objects.
3. Define QSRP as a protocol-first adapter, not as a REST API feature.
4. Add full QSRP QUIC streams/datagrams and gateway runtime after B2BUA and
   basic media behavior are stable.

## Why This Shape

`session-core` is a programmable SIP user-agent/session layer. It exposes
`UnifiedCoordinator`, `Endpoint`, `StreamPeer`, and `CallbackPeer`. For B2BUA,
CPaaS, voice-ai, and contact-center workloads, `UnifiedCoordinator` is the load-bearing
surface because one service process needs to coordinate many sessions and route
events by session.

`media-core` exists and remains the RTP/media primitive layer. The current
bridge behavior is a transparent relay. There is not yet enough pressure to
create a new `dial-core` abstraction or to move SIP signaling out of
`session-core`. Doing that now would mostly move code around before we know the
shape required by real B2BUA, QSRP gateway, ASR/TTS, and contact-center flows.

CPaaS and QSRP are different layers:

- CPaaS is an external control plane: REST/WebSocket/SSE APIs for application
  developers and administrators to create calls, route calls, manage queues,
  inspect calls, and subscribe to events.
- QSRP is an edge protocol: QUIC streams for signaling, QUIC datagrams for
  media, JSON envelopes, protocol capability negotiation, and first-class
  voice/AI message types.

That means CPaaS should wrap a neutral platform command/event model, while QSRP
should be a protocol adapter that maps QSRP messages into the same command/event
model.

## Target Layering

```text
External applications
  |
  | REST / WebSocket / SSE
  v
rvoip-api-server
  |
  | platform commands/events
  v
rvoip-platform-core / rvoip-call-control
  ^
  | platform commands/events
  |
rvoip-qsrp-core + future rvoip-qsrp-server
  ^
  | QSRP over QUIC
  |
QSRP endpoints


Application libraries
  |
  +--> rvoip-contact-center
  +--> rvoip-voice-ai
  +--> rvoip-b2bua
            |
            | UnifiedCoordinator
            v
        session-core
            |
            +--> dialog-core / sip-core / sip-transport
            +--> media-core / rtp-core / codec-core
```

The important rule is that external product APIs should not expose
`UnifiedCoordinator` directly. `UnifiedCoordinator` is the internal telephony
control API. Public-facing APIs should expose platform commands, stable IDs,
and platform events.

## Crate Roadmap

### Phase 1: `crates/b2bua`

Create a new B2BUA crate, published as `rvoip-b2bua`.

This crate should depend on `rvoip-session-core` and use
`UnifiedCoordinator` directly. It should not depend on the future API server.

Core responsibilities:

- Accept or originate an inbound leg.
- Create an outbound leg.
- Bridge the two legs through `UnifiedCoordinator::bridge`.
- Correlate events for both legs.
- Propagate hangup in either direction.
- Surface call progress, answer, failure, transfer, DTMF, and bridge lifecycle.
- Provide hooks for routing policy without hard-coding contact-center or CPaaS
  behavior.

Minimum public concepts:

```rust
pub struct B2buaService;
pub struct B2buaConfig;
pub struct RouteRequest;
pub struct RouteDecision;
pub struct B2buaCall;
pub struct CallLeg;
pub struct BridgeId;

pub enum LegRole {
    Inbound,
    Outbound,
}

pub enum B2buaEvent {
    InboundReceived,
    OutboundDialing,
    OutboundProgress,
    OutboundAnswered,
    BridgeEstablished,
    DtmfReceived,
    TransferRequested,
    LegEnded,
    CallEnded,
    CallFailed,
}
```

The exact names can change during implementation, but these responsibilities
should not.

Initial flows:

1. Inbound SIP call to outbound SIP target.
2. Outbound failure with inbound rejection or fallback route.
3. Hangup from inbound leg tears down outbound leg.
4. Hangup from outbound leg tears down inbound leg.
5. DTMF events are correlated to the B2BUA call.
6. Blind/attended transfer events are surfaced at the B2BUA boundary.

Non-goals for this phase:

- No CPaaS HTTP API.
- No QSRP transport.
- No contact-center queues.
- No AI/ASR/TTS orchestration.
- No media transcoding unless already available through existing lower layers.

### Phase 2: `crates/platform-core` or `crates/call-control`

After the B2BUA MVP proves the call topology model, add a small shared contract
crate. The exact crate name can be chosen during implementation, but this
document uses `rvoip-platform-core`.

This crate should contain stable product-facing command/event types and IDs. It
should have minimal runtime behavior. Its main job is to prevent CPaaS, QSRP,
contact-center, and voice-ai from all inventing incompatible command/event shapes.

Responsibilities:

- Define stable IDs for calls, legs, bridges, endpoints, queues, agents,
  recordings, transcripts, bots, and external correlation IDs.
- Define command structs/enums for call control.
- Define event structs/enums for event streaming and audit.
- Define transport-neutral serialization with `serde`.
- Define error categories suitable for public APIs and protocol adapters.

Example command surface:

```rust
pub enum PlatformCommand {
    CreateCall(CreateCall),
    AnswerCall(AnswerCall),
    RejectCall(RejectCall),
    HangupCall(HangupCall),
    BridgeCalls(BridgeCalls),
    TransferCall(TransferCall),
    SendDtmf(SendDtmf),
    PlayAudio(PlayAudio),
    StartRecording(StartRecording),
    StopRecording(StopRecording),
    SubscribeAudio(SubscribeAudio),
    SendMessage(SendMessage),
    UpdatePresence(UpdatePresence),
}
```

Example event surface:

```rust
pub enum PlatformEvent {
    CallCreated(CallCreated),
    CallRinging(CallRinging),
    CallAnswered(CallAnswered),
    CallProgress(CallProgress),
    BridgeEstablished(BridgeEstablished),
    DtmfReceived(DtmfReceived),
    TransferRequested(TransferRequested),
    RecordingStarted(RecordingStarted),
    RecordingStopped(RecordingStopped),
    TranscriptPartial(TranscriptPartial),
    TranscriptFinal(TranscriptFinal),
    BotJoined(BotJoined),
    AgentStateChanged(AgentStateChanged),
    CallEnded(CallEnded),
    CallFailed(CallFailed),
}
```

Design rules:

- Commands must be transport-neutral.
- Events must include stable IDs and timestamps.
- Events should include enough correlation data for external subscribers to
  rebuild state without reading internal session-core objects.
- Do not expose SIP dialog IDs as the primary public identity, although keeping
  internal references for diagnostics is useful.
- Do not expose `UnifiedCoordinator` handles or session-core event enums in the
  public API server.

### Phase 3: `crates/qsrp-core`

Add a QSRP protocol/domain crate early, before implementing the full QUIC
runtime.

QSRP is not a CPaaS API. It is a protocol with its own envelope, message types,
capability negotiation, and media transport model. Keep it separate from
`api-server`.

Responsibilities:

- Define QSRP envelopes.
- Define QSRP message types and payload schemas.
- Implement JSON serialization/deserialization.
- Validate required fields and protocol-level invariants.
- Model capability negotiation from SETTINGS.
- Map QSRP messages to `rvoip-platform-core` commands/events.
- Preserve optional SDP fields for SIP gateway interop.

Core QSRP message support should include:

- `REGISTER`
- `PRESENCE`
- `INVITE`
- `ACCEPT`
- `REJECT`
- `END`
- `MESSAGE`
- `REACTION`
- `RECEIPT`
- `KEY_EXCHANGE`
- `MEDIA_UPDATE`
- `ERROR`
- `SYNC`
- `SESSION`
- `SCHEDULE`
- `TRANSCRIPT`
- `BOT`
- `COMMAND`
- `VOICEMAIL`

Important protocol behavior from the QSRP drafts:

- Envelope includes `QSRP_version`, `seq`, `message_id`, `timestamp`, `from`,
  `to`, `type`, optional `session_id`, and `payload`.
- Unknown top-level keys are ignored.
- Unknown message types produce `ERROR 4002 InvalidMessageType`.
- SETTINGS negotiates supported QSRP versions and extensions after the QUIC
  handshake.
- Native JSON session negotiation is the default.
- `sdp_offer` and `sdp_answer` remain optional gateway interop fields, not the
  mandatory negotiation model.

The first QSRP implementation should focus on schema correctness and mapping:

```text
QSRP INVITE     -> PlatformCommand::CreateCall or inbound call event
QSRP ACCEPT     -> PlatformCommand::AnswerCall
QSRP REJECT     -> PlatformCommand::RejectCall
QSRP END        -> PlatformCommand::HangupCall
QSRP SESSION    -> hold/resume/mute/record/transfer commands
QSRP TRANSCRIPT -> PlatformEvent::TranscriptPartial/TranscriptFinal
QSRP BOT        -> bot registration/session events
QSRP COMMAND    -> application command dispatch event
```

Non-goals for `qsrp-core`:

- No QUIC socket runtime.
- No RTP-over-QUIC datagram implementation.
- No production gateway process.
- No call routing policy.

### Phase 4: `crates/api-server`

Build the CPaaS-style API server after B2BUA and the platform command/event
contract exist.

The existing `crates/api-server` directory is currently only a placeholder. It
can become the API adapter crate, but it should not own telephony behavior.

Responsibilities:

- Provide REST endpoints for commands and configuration.
- Provide WebSocket or SSE streams for platform events.
- Use `users-core` authentication, API keys, JWTs, permissions, and rate
  limiting where practical.
- Translate HTTP requests into platform commands.
- Translate platform events into external event payloads.
- Provide developer/admin API surfaces without leaking session-core internals.

Initial REST shape:

```text
POST   /v1/calls
GET    /v1/calls/{call_id}
POST   /v1/calls/{call_id}/answer
POST   /v1/calls/{call_id}/reject
POST   /v1/calls/{call_id}/hangup
POST   /v1/calls/{call_id}/transfer
POST   /v1/calls/{call_id}/dtmf
POST   /v1/calls/{call_id}/recordings
DELETE /v1/calls/{call_id}/recordings/{recording_id}

GET    /v1/events
GET    /v1/events/ws

GET    /v1/agents
POST   /v1/agents/{agent_id}/state
GET    /v1/queues
POST   /v1/queues/{queue_id}/members

GET    /v1/endpoints
POST   /v1/endpoints
```

Initial API implementation can be deliberately thin:

- Axum handlers.
- Auth extraction from `users-core`.
- In-memory service wiring for development.
- JSON request/response types backed by platform-core.
- Event stream backed by broadcast channels.

Non-goals for the first API server:

- No billing.
- No tenant provisioning UI.
- No webhook retries.
- No long-term analytics store.
- No QSRP transport.
- No independent call state machine separate from B2BUA/session-core.

### Phase 5: `crates/voice-ai`

Build voice-ai and speech automation capabilities as a library over B2BUA/platform commands, not as
an API-server feature.

Responsibilities:

- Prompt playback.
- DTMF collection.
- Speech recognition adapter boundary.
- TTS/audio generation adapter boundary.
- Bot/session state.
- Transcript events.
- Turn-taking policy.
- Fallback from speech to DTMF.
- Handoff to human agent or SIP destination.

Core abstractions:

```rust
pub struct VoiceAiService;
pub struct VoiceAiSession;
pub struct Prompt;
pub struct SpeechInput;
pub struct BotTurn;

pub trait SpeechRecognizer;
pub trait SpeechSynthesizer;
pub trait BotRuntime;
```

Initial flows:

1. Answer inbound call.
2. Play greeting.
3. Collect DTMF or speech.
4. Emit transcript events.
5. Route to an outbound SIP target, queue, voicemail, or bot response.
6. End call cleanly with correlated platform events.

The voice-ai crate should not know whether the external controller is REST, QSRP, or
an embedded application. It should consume commands/events and produce
commands/events.

### Phase 6: `crates/contact-center`

Build contact-center behavior as another library over B2BUA/platform commands.

Responsibilities:

- Queues.
- Agents.
- Skills.
- Agent state.
- Queue membership.
- Routing strategies.
- Escalation.
- Supervisor monitoring.
- Transfer/conference policy.
- Integration hooks for voice-ai and CPaaS.

Core abstractions:

```rust
pub struct ContactCenterService;
pub struct Queue;
pub struct Agent;
pub struct Skill;
pub struct RoutingPolicy;
pub struct Assignment;

pub enum AgentState {
    Offline,
    Available,
    Ringing,
    OnCall,
    WrapUp,
    Busy,
}
```

Initial flows:

1. Inbound call enters queue.
2. Queue chooses available agent.
3. B2BUA dials agent endpoint.
4. Answer bridges caller to agent.
5. Agent hangup, caller hangup, no-answer, and reject all update queue state.
6. Transfer and supervisor escalation are emitted as platform events.

The contact-center crate should not own SIP details and should not be embedded
inside the API server.

### Phase 7: `crates/qsrp-server`

After B2BUA and media behavior are stable, implement the QSRP runtime and
gateway process.

Responsibilities:

- QUIC listener, default UDP 4433.
- TLS 1.3 configuration.
- QUIC SETTINGS exchange.
- Reliable ordered QUIC streams for signaling messages.
- QUIC datagrams for media.
- RTP-over-QUIC datagram mapping, with flow ID mapped to RTP SSRC.
- QSRP endpoint registration and presence.
- Session resumption and reconnect behavior.
- Optional embedded-device polling extension.
- Mapping QSRP sessions to B2BUA calls for SIP/PSTN interop.

Gateway model:

```text
QSRP endpoint
  |
  | QSRP over QUIC
  v
rvoip-qsrp-server
  |
  | platform commands/events
  v
rvoip-b2bua
  |
  | SIP/RTP
  v
SIP trunk / PBX / PSTN gateway
```

QSRP-to-SIP behavior:

- QSRP `INVITE` creates a platform call or B2BUA outbound leg.
- Optional QSRP SDP is passed through when present.
- Native JSON media fields are translated into SDP when needed.
- QSRP `ACCEPT` maps to SIP answer.
- QSRP `END` maps to hangup.
- QSRP `SESSION transfer` maps to B2BUA/session-core transfer behavior.
- QSRP `TRANSCRIPT`, `BOT`, `COMMAND`, and caregiving extensions remain
  platform/application events, not SIP features.

Non-goals for the first QSRP server:

- No browser-native QSRP support.
- No attempt to replace SIP/PSTN interop.
- No mandatory SDP-over-QSRP model.
- No media transcoding unless lower layers already provide it.

## CPaaS Contract

The CPaaS API should expose platform concepts, not session-core objects.

Public IDs:

- `call_id`
- `leg_id`
- `bridge_id`
- `endpoint_id`
- `agent_id`
- `queue_id`
- `recording_id`
- `transcript_id`
- `bot_id`
- `correlation_id`

API command characteristics:

- Idempotency keys for mutating operations.
- Correlation IDs for tracing.
- Tenant/account IDs once multi-tenancy is introduced.
- Auth context from `users-core`.
- Clear error categories: validation, auth, not found, conflict, unavailable,
  upstream failure, timeout.

Event characteristics:

- Monotonic event sequence per stream where practical.
- Timestamp on every event.
- Stable public IDs.
- Internal SIP/session references only in debug fields.
- JSON serialization stable enough for external subscribers.

This contract can also back webhooks later, but webhook delivery should not be
part of the first API implementation.

## QSRP Contract

QSRP should be modeled as a first-class protocol adapter.

Core object model:

```rust
pub struct QsrpEnvelope<T> {
    pub qsrp_version: String,
    pub seq: u64,
    pub message_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub from: QsrpAddress,
    pub to: QsrpAddress,
    pub message_type: QsrpMessageType,
    pub session_id: Option<QsrpSessionId>,
    pub payload: T,
}

pub enum QsrpMessage {
    Register(RegisterPayload),
    Presence(PresencePayload),
    Invite(InvitePayload),
    Accept(AcceptPayload),
    Reject(RejectPayload),
    End(EndPayload),
    Message(MessagePayload),
    Session(SessionPayload),
    Transcript(TranscriptPayload),
    Bot(BotPayload),
    Command(CommandPayload),
    Voicemail(VoicemailPayload),
    Error(ErrorPayload),
}
```

Mapping rules:

- QSRP schemas live in `qsrp-core`.
- Transport runtime lives later in `qsrp-server`.
- QSRP extension support should be capability-gated.
- Embedded-device support should begin as schema and mapping support, then add
  server-side queueing/polling after the base runtime exists.
- Caregiving extension support should be treated as application events and
  permissions context, not as SIP-specific behavior.

## When to Revisit `dial-core` / Media Split

Do not split now. Revisit only when at least one of these pressures is concrete:

1. A second signaling protocol, such as QSRP runtime, needs to share call
   topology and media control without depending on SIP concepts.
2. Media injection, transcoding, recording, ASR, or TTS requires a separate
   graph/orchestration model that no longer fits `session-core`.
3. CPaaS and QSRP adapters both need the same non-SIP call control layer and
   are duplicating translation logic.
4. B2BUA needs advanced media policies that are not simple RTP relay.

If those happen, the split should be based on proven boundaries:

- `session-core`: SIP session implementation.
- `platform-core` or future `dial-core`: protocol-neutral call control.
- `media-core`: packet/media primitives.
- future media orchestration crate: higher-level media graph, mixing,
  recording, ASR/TTS injection, and transcoding.

Until then, keep the architecture simple and keep using `session-core` as the
implementation engine.

## Concrete Next Implementation Pass

The next code pass should do this:

1. Add `crates/b2bua` to the workspace.
2. Create a minimal `B2buaService` around `UnifiedCoordinator`.
3. Implement inbound-to-outbound bridge.
4. Add route-decision hooks.
5. Add event correlation for both legs.
6. Add hangup propagation in both directions.
7. Add integration tests using loopback peers or existing session-core examples.
8. Keep the API server and QSRP runtime out of this pass.
9. Note the emerging command/event shapes for later extraction into
   `platform-core`.

Success criteria:

- A B2BUA example can accept an inbound SIP call, dial an outbound SIP target,
  bridge media, and tear down both legs cleanly.
- The B2BUA crate emits enough structured events to drive a future API server.
- No public API depends on old `call-engine` APIs.
- No new crate exposes `UnifiedCoordinator` as the external product API.

## Test Strategy

### B2BUA tests

- Inbound call is answered only after outbound leg succeeds.
- Outbound busy/reject maps to the expected inbound response.
- Outbound timeout maps to call failure.
- Inbound hangup tears down outbound leg.
- Outbound hangup tears down inbound leg.
- Bridge event is emitted exactly once.
- DTMF from either leg is correlated to the B2BUA call.
- Transfer events surface without losing leg correlation.

### Platform-core tests

- Commands serialize and deserialize with stable JSON.
- Events serialize and deserialize with stable JSON.
- Unknown optional fields do not break forward compatibility where appropriate.
- Public IDs round-trip cleanly.
- Error categories map to API-friendly status classes.

### CPaaS API tests

- Auth-required endpoints reject unauthenticated requests.
- API-key and JWT auth paths work through `users-core`.
- `POST /v1/calls` maps to the expected platform command.
- Event stream receives call lifecycle events.
- Invalid command payloads return validation errors.
- Not-found and conflict cases use stable error shapes.

### QSRP-core tests

- Envelope round-trip serialization.
- Unknown top-level keys are ignored.
- Unknown message type maps to `ERROR 4002 InvalidMessageType`.
- SETTINGS capability negotiation chooses the expected version/extensions.
- `INVITE`/`ACCEPT` native JSON media negotiation round-trips.
- Optional `sdp_offer`/`sdp_answer` fields are preserved.
- `SESSION` actions map to platform call-control commands.
- `TRANSCRIPT`, `BOT`, and `COMMAND` payloads map to platform events.

### QSRP-server tests

Add these only when the QUIC runtime exists:

- QUIC connection establishment.
- SETTINGS exchange.
- Signaling over ordered streams.
- Media over datagrams.
- RTP SSRC to datagram flow-ID mapping.
- Connection migration or reconnect behavior.
- Embedded polling/resumption behavior.
- QSRP-to-SIP gateway call setup and teardown.

## Explicit Non-Goals

- Do not revive `crates/call-engine` as-is. It targets an older version of the
  library and should be used only for ideas.
- Do not make the CPaaS API server the owner of telephony state machines.
- Do not make QSRP a REST API feature.
- Do not require SDP for native QSRP calls.
- Do not split `session-core` before a real second protocol or media graph
  forces the boundary.
- Do not build contact-center queues before B2BUA proves the leg/bridge model.
- Do not build voice AI as an API-server submodule.

## Reference Material

Local RVoIP docs:

- `crates/session-core/docs/PRE_B2BUA_ROADMAP.md`
- `crates/session-core/docs/TELCO_USE_CASE_ANALYSIS.md`
- `crates/session-core/docs/AUDIO_MODES.md`
- `crates/api-server/README.md`
- `CALL_CENTER_REFERENCE_ARCHITECTURE.md`
- `B2BUA_IMPLEMENTATION_PLAN.md`

QSRP docs:

- `/Users/jonathan/Documents/Work/Rudeless_Ventures/qsrp/QSRP-core-00.md`
- `/Users/jonathan/Documents/Work/Rudeless_Ventures/qsrp/QSRP-core-00-appendixA.md`
- `/Users/jonathan/Documents/Work/Rudeless_Ventures/qsrp/QSRP-extension-01-embedded.md`
- `/Users/jonathan/Documents/Work/Rudeless_Ventures/qsrp/QSRP-extension-02-caregiving.md`
- `/Users/jonathan/Documents/Work/Rudeless_Ventures/qsrp/qsrp-assessment.md`
