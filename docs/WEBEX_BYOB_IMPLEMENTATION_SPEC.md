# Webex BYOB connector implementation specification

Date: 2026-10-28. Status: implementation design plus WebSocket/gRPC prototype. Section 20 supersedes the original transport scope and checkpoint; section 21 records the WebSocket sandbox WAV exception; live Cisco qualification remains distinct from simulator/provider evidence.

Baseline: fresh checkout of rvoip main at f4d528eaabbac935bce9a6404d3ce4fdec35ed0f, workspace version 0.4.0. All proposed interfaces below are additions unless explicitly identified as existing. A successful simulator run does not establish Cisco or Vapi production qualification.

## Current library review scope — 2026-11-04

The submitted library includes the WebSocket protocol/session engine, bounded
startup/result metadata, optional gRPC protobuf validation and response framing,
opt-in WAV compatibility, fast-start answer buffering, and the separate ordered
`rvoip-vapi::raw_session` API. The protocol crate has no Vapi dependency. Default
builds do not compile the optional gRPC protobuf toolchain.

Full `ConnectionAdapter` registration, facade exports, server/authentication,
agent catalog and application policy in sections 4–6 remain design work, not
implemented library APIs. Deployment, renewal and live call orchestration belong
to the separate connector application. Its November 3 validation records include
successful gRPC/WebSocket appointment context round trips and automatic renewal
checks; those are application/sandbox observations, not general Cisco conformance
or a production qualification for this crate. Earlier dated checkpoints below
remain as historical evidence and are superseded by later observations where
explicitly stated.

The current application uses raw PCMU/8 kHz gRPC streaming and an opt-in PCM16
WAV/8 kHz WebSocket output profile; input remains raw PCMU. The one-second gRPC
WAV streaming experiment was silent in the tested deployment. See the crate
README for the available profiles and bounds. Early-access WebSocket playback,
interruption ordering and provider turn fences still require deployment-specific
qualification. Vapi speech-stop is not an authoritative audio-generation barrier.

The Cisco schema repository still has no declared top-level license in the
November 4 review. `publish = false` keeps this experimental crate out of automatic
crates.io releases until redistribution is resolved; this PR does not approve
publication of the vendored schemas. Path/Git library evaluation is supported.

## 1. Decision and outcome

Add a provider-neutral extension at crates/extensions/rvoip-webex-byob, beside the existing rvoip-vapi extension. A thin Rust application in the separate webex-vapi-connector repository composes the two. The Webex crate must not depend on Vapi, contain Vapi assistant IDs, or implement application-specific handoff policy.

Cisco names this integration BYOVA, Bring Your Own Virtual Agent. The requested BYOB name is retained for the package. Scope is Webex Contact Center voice flows; Meetings bots, Webex Calling SIP trunks and messaging bots are outside this protocol. Sections 1–19 describe WebSocket; section 20 adds the separately selectable gRPC variant.

Use rvoip 0.4 consistently. Bridgefu is an optional future host. Its reviewed main commit c4793d79e0c1810e42155d494f2ab1cc308bca11 pins the coordinated 0.3.8 graph and has no Webex or Vapi-WebSocket leg variant. Upgrading that gateway does not solve the turn-control problem and must not block this integration.

Success means an authenticated Webex conversation can use a selected Vapi assistant across multiple turns, interrupt playback, submit DTMF, request Contact Center handoff, and terminate without leaking an upstream call. Protocol correctness must be independently testable with a deterministic fake agent and a Webex simulator.

## 2. Sources and precedence

1. [Webex WebSocket behavioral contract](https://developer.webex.com/mcp/docs/bring-your-own-virtual-agent-over-websocket).
2. [Conversation AsyncAPI](https://github.com/webex/dataSourceSchemas/blob/fca090dfa806e28695acb145e927ad0d557ce7fe/Services/VoiceVirtualAgent_WebSocket/a38a10b7-43e4-4676-a076-a7d6dce9387d/AsyncApiSpec/VoiceVirtualAgent_WsSchema.json).
3. [List-agents AsyncAPI](https://github.com/webex/dataSourceSchemas/blob/fca090dfa806e28695acb145e927ad0d557ce7fe/Services/VoiceVirtualAgent_WebSocket/a38a10b7-43e4-4676-a076-a7d6dce9387d/AsyncApiSpec/VoiceVirtualAgent_WsListVASchema.json).
4. [BYODS registration/authentication](https://developer.webex.com/create/docs/bring-your-own-datasource).
5. [BYOVA onboarding](https://developer.webex.com/mcp/docs/bring-your-own-virtual-agent).
6. [Vapi WebSocket transport](https://docs.vapi.ai/calls/websocket-transport).
7. [rvoip 0.4 migration](MIGRATING_0.4.md), [Vapi adapter](../crates/extensions/rvoip-vapi/src/adapter.rs), [Vapi events](../crates/extensions/rvoip-vapi/src/events.rs), [core adapter contract](../crates/foundation/rvoip-core/src/adapter.rs).

Reviewed Vapi source: a local Vapi checkout at ec467b7d356841fb76f09fd6c2e53f8d718b4d14. Relevant files are libs/core/src/components/liveCallPipeline/streams/transports/websocketStream.ts; functions/liveCallFunctions/liveCallFunctions.streamBundleConnectPipes.ts; functions/liveCallFunctions/liveCallFunctions.onEndpointBufferData.ts; components/liveCallPipeline/liveCallPipeline.ts; types/message.types.ts; and types/assistant.types/assistant.serverClientMessage.types.ts, under libs/core/src.

Use AsyncAPI for field spelling and types and the behavioral guide for media/lifecycle semantics. Record discrepancies explicitly. Do not silently relax the entire parser or turn schema examples into authoritative media fixtures.

## 3. Confirmed compatibility and gaps

| Required behavior | Existing evidence | Implementation decision |
| --- | --- | --- |
| Raw PCMU audio | Vapi accepts raw mulaw/8000 and returns binary audio; rvoip-vapi defaults to MuLaw8Khz | Pass codec bytes through, changing framing only |
| Speech start | Vapi client speech-update started/user and user-interrupted | Qualify both paths, deduplicate; do not wait exclusively for user speech-start during bot playback |
| Committed caller turn | Classic Vapi user speech-stop is emitted by VAD utterance-stop; endpointing commits elsewhere | Do not equate VAD stop with an authoritative END_OF_INPUT |
| Reply completion | Assistant speech-stop comes from volume-control bot-end | Candidate signal only; require proof that the last converted audio is already serialized |
| Media/event ordering | rvoip-vapi publishes events separately while audio drains on 20 ms ticks | Add an ordered, unpaced stream mode; merely selecting on two receivers is insufficient |
| Interruption | Vapi clears its pipeline and emits user-interrupted; rvoip flushes on user speech-start | Handle both interruption events and fence stale audio across every queue |
| DTMF | No native Vapi raw-WebSocket DTMF transport | Collect digits in connector and inject application text once; do not synthesize tones |
| Handoff | Vapi WebsocketStream.transfer returns an error | Use a custom tool and Webex TRANSFER_TO_AGENT; do not call native Vapi transferCall |
| Silence/no-match | No proven one-to-one Vapi recognition outcome for each Webex event | Explicit application policy, bounded retries, no inference from a missing transcript alone |
| Audio variants | Vapi source documents a different PCM requirement for GPT-Live | Initially qualify a conventional STT/LLM/TTS assistant; do not promise all assistants work |

These gaps do not prevent implementing the generic Webex extension. They do prevent claiming strict production compatibility for an arbitrary existing Vapi assistant.

## 4. Components and ownership

~~~text
Webex Contact Center -- WSS JSON + Base64 --> rvoip-webex-byob
                                             |
                                      ordered session actor
                                             |
                               connector application policy
                                             |
                                     rvoip-vapi extension
                                             |
                                 WSS binary audio + JSON
                                             |
                                        Vapi assistant
~~~

The Webex extension owns protocol parsing, framing, application heartbeats, connection state, reply finality, and a typed event/command API. It accepts a host-provided token validator and agent catalog. Provide an optional Cisco JWS validator implementation with an injected tenant registry and key cache.

The service owns tenant configuration, mapping virtual_agent_id to assistantId, Vapi credentials, custom tools, transcript policy, deployment, BYODS registration renewal, and operator controls. The fake agent is interchangeable with the Vapi agent and cannot be enabled accidentally in a live tenant profile.

The rvoip orchestrator owns connection/session/participant lifecycle. Webex owns actual telephony playback and Contact Center routing. A Webex handoff request is not proof an agent answered.

## 5. Crate layout and integration

Proposed crate modules:

| Module | Responsibility |
| --- | --- |
| protocol | Serde envelopes, directional enums, bounded validation, error classification |
| session | Single-owner state machine, sequence allocation, local reply IDs and generations |
| media | PCMU validation, chunk assembly, byte/sample accounting |
| adapter | ConnectionAdapter integration and lifecycle fencing |
| auth | TokenValidator interface, verified principal, optional Cisco verifier |
| catalog | Tenant-scoped list-agents interface |
| server | Optional Axum router and WebSocket upgrade integration |
| error / metrics | Content-free errors and bounded-cardinality metrics |

Use workspace version/MSRV/lints and shared dependency versions. Add workspace membership and dependency; expose an opt-in webex-byob facade feature and rvoip::webex_byob module. Do not add SIP, RTP, WebRTC, AWS, or Vapi dependencies to the Webex protocol layer. Keep server and Cisco-JWS dependencies behind features where practical.

Introduce Transport::WebexByob with AdapterKind::Interop. Existing Transport::WebSocket means UCTP and must not be reused. Audit exhaustive matches, serde representations, metrics labels, persistence, capability negotiation, and facade tests. Adding a variant to the currently exhaustive public Transport enum is a source-compatibility change: coordinate the release, do not describe it as a patch-only change.

Implement inbound accept/reject/end, stream access, authenticated admission, terminal fallback, and resource counts. Outbound originate, hold/resume, arbitrary SIP-target transfer, and sending DTMF toward Webex are unsupported unless a later contract explicitly provides them. Return typed unsupported errors; never report success for a no-op.

Use the atomic authenticated inbound lifecycle path in ConnectionAdapter. Do not publish an unauthenticated connection before its principal. Mint local ConnectionId/ConversationId values; map external IDs to them rather than reconstructing lifecycle-fenced IDs from Webex text. Retain permits through cleanup. Respect notify_inbound_admission_outcome and prevent late admission callbacks from resurrecting ended sessions.

Generic MediaStream access is useful for other rvoip consumers, but a generic audio bridge alone cannot infer reply boundaries. Require a typed reply/control surface for BYOVA. The Vapi service must select an ordered application stream with exclusive ownership of media consumption; it must not simultaneously route duplicate audio through the media graph.

## 6. Proposed Rust API contract

Use constructor/builder configuration with validated bounds. All credentials and content-bearing types require redacted Debug implementations. The following names describe proposed API shape, not code already available in rvoip 0.4:

~~~text
WebexAdapter::new(config, token_validator, agent_catalog)
WebexAdapter::router()                         // server feature
WebexAdapter::take_session(connection_id)      // one primary consumer
WebexSession::next_input() -> WebexInput
WebexSession::begin_reply(policy) -> ReplyId
WebexSession::send_audio(reply_id, pcmu_bytes)
WebexSession::finish_reply(reply_id, final_fields)
WebexSession::report_input_started(input_id)
WebexSession::report_input_committed(input_id, optional_transcript)
WebexSession::cancel_reply(reply_id, cause)
WebexSession::request_handoff(metadata)
WebexSession::end(reason)
~~~

WebexInput distinguishes Started, Audio, DtmfDigits, DtmfStarted, NoInput, CustomEvent, Ended, and TransportFailed. Carry verified tenant/session context separately from untrusted additional_info. Byte arrays are Bytes, not UTF-8 strings after decoding.

ReplyId and InputId are opaque local generation tokens, never copied from envelope seq. Session methods enqueue commands into one bounded actor; callers cannot allocate seq, submit raw response envelopes, or bypass finality checks. Repeated finish/end calls must emit no duplicate FINAL. The current prototype returns a typed StaleReply error on repeated finish; hosts must handle this without retransmission. Audio after finality is rejected. Old-generation audio after cancellation is rejected and counted.

TokenValidator returns a verified service principal and registration binding, not just a boolean. AgentCatalog receives that verified tenant and returns only authorized agents. A caller-supplied organization or selector cannot choose a different Vapi credential.

## 7. Wire contract

The behavioral guide specifies one conversation per WSS connection, JSON/Base64 media, raw mono PCMU at 8 kHz, and exactly one FINAL per logical reply. Send output promptly; Webex paces playback. SESSION_END and transport loss end the session; there is no automatic resume/replay. Use /v1/va and the separate /v1/listVirtualAgents endpoint. TLS plus BYODS bearer authentication is required; gRPC-specific mTLS guidance does not apply. [Behavioral source](https://developer.webex.com/mcp/docs/bring-your-own-virtual-agent-over-websocket).

Conversation envelopes contain type, positive seq, RFC3339 ts, and nonempty conversation_id. Request payload also identifies conversation and customer organization. Require both conversation identities to match and remain stable. Each sender owns its sequence; gaps are valid. Do not use request seq as response correlation. PONG echoes the PING seq and is handled outside ordinary monotonic application-response validation.

Require SESSION_START as the first conversation application request. Allow transport/application liveness handling during initialization without allocating a Vapi call. Validate exactly one of audio_input, dtmf_input, or event_input. Freeze tenant, agent selection, and media profile once admitted; reject attempts to mutate them mid-session. Optional repeated context may be ignored according to documented policy, never used to replace credentials.

List requests are bare objects with customer_org_id and optional is_default_virtual_agent_enabled. Return a bare virtual_agents array of id/name/optional description. They are not VOICE_VA_REQUEST envelopes. Authenticate and tenant-bind the list operation as well. Empty authorized catalogs return an empty array. Unknown agent selection on /v1/va fails before upstream creation; omitted selection requires an explicitly configured default.

Response fields include prompts, output_events, response_type, input_mode, input_sensitive, optional input_handling_config, session_transcript, and disable_prompt_cancellation. The actor controls response_type. Transcript content is TextContent, not an arbitrary messages array. Input events use parameters; output events use metadata. Do not swap these fields.

DTMF enums are mapped explicitly to 0–9, A–D, star and pound, in received order. Unspecified digits are invalid for collection. Only no_input_timeout_msec is currently usable among the speech timer fields; the other three timers are reserved in the schema.

### Schema discrepancies

Two defects were reproduced with Python jsonschema 4.19.2 against unmodified upstream components:

1. WsEnvelopeBase has additionalProperties=false, while allOf-derived envelopes add payload/code/status/detail. A standard validator rejects the published SESSION_START example because payload is not permitted by the base.
2. VoiceInput.oneOf contains required=[caller_audio_b64] and required=[]. Audio-bearing objects match both branches and fail oneOf; omitting the audio passes.

Other discrepancies: examples begin with Base64 suggesting RIFF despite raw-audio requirements; Prompt documentation mentions PCM16/16 kHz although the qualified guide specifies PCMU; the list document declares AsyncAPI 3 but retains publish/subscribe syntax. Treat these as schema/tooling risks, not evidence of supported alternative media.

Implement explicit typed validation for the documented wire shape. Keep original schema snapshots and a narrowly scoped, reviewed normalization for test tooling (flatten envelope properties, express optional audio correctly, constrain discriminator constants). Test both upstream defects and the normalized validator. Do not broadly disable additional-property checks. Confirm actual Cisco messages during sandbox qualification and track upstream corrections before changing compatibility behavior.

## 8. Session state machine

~~~text
Authenticating -> AwaitingStart -> Admitting -> StartingAgent -> Active
        failure from any state ------------------------------> Closing
Active -> HandoffRequested -> Closing -> Closed
Active -> Closing -> Closed
~~~

- Before upgrade: reject invalid/missing credentials, quota exhaustion, or unavailable service with bounded HTTP errors.
- AwaitingStart: enforce a start deadline and input-size limits; no upstream call exists.
- Admitting: bind verified organization and configured agent, reserve capacity, mint IDs, await core policy admission.
- StartingAgent: create exactly one upstream call. Buffer at most the configured initial audio budget while still servicing heartbeats and cancellation. A disconnected Webex client cancels startup; late successful upstream creation is immediately cleaned up.
- Active: concurrent caller input and assistant reply are allowed, but output order is owned by the actor.
- HandoffRequested: admit one handoff command, fence future replies, deliver the agreed event once, then end the Vapi leg. Record requested, not completed.
- Closing: stop new work, attempt bounded terminal delivery if transport is writable, close upstream, join owned tasks, erase sensitive buffers and maps, release permits.
- Closed: terminal callbacks are idempotent. Never reopen or create another Vapi call for this lifecycle.

Duplicate live (verified tenant, conversation_id) admission is rejected. A bounded recent-ID tombstone prevents accidental replay within a configurable window. Tombstone eviction must be bounded and observable; it is not permanent call history.

Failure during Vapi POST /call is an uncertain create outcome. Do not blindly retry and create duplicates. Use a supported, verified provider idempotency mechanism if available; otherwise fail the session, reconcile by a protected correlation record, and report cleanup uncertainty. An arbitrary correlation string is not itself an idempotency guarantee.

## 9. Reply, interruption, and endpointing semantics

Each reply has Idle -> Streaming -> Finishing -> Finalized, or Streaming -> Cancelled -> Finalized. A local generation changes on interruption. Only one reply may write audio at a time. Greeting, no-input reprompt, ordinary response, and tool-generated speech all use this same machinery.

Normal completion: accept ordered audio for the active generation, flush its legal residual chunk, serialize one FINAL after that audio, then release the reply slot. An empty reply still gets one FINAL. A transcript-final, LLM token completion, silence interval, or WebSocket message boundary is not sufficient evidence of audio completion.

Interruption: invalidate the old generation immediately; discard its pending output; emit START_OF_INPUT for accepted caller speech; prevent any old-generation audio from reaching Webex; finalize the cancelled logical reply once before opening another reply. The exact placement of the cancelled FINAL relative to START_OF_INPUT must be verified with Cisco (Q1). A terminal session failure does not fabricate successful reply completion.

Vapi speech-update started/user and user-interrupted can describe the same interruption. Deduplicate by active reply/input generation, not timestamp equality. Respect configured prompt interruptibility and Vapi first-message/stop-speaking policy; do not advertise an interruptible greeting while Vapi ignores early speech.

END_OF_INPUT is emitted when the chosen agent runtime commits an input turn. VAD stop is provisional and may be followed by resumed speech. Do not emit repeated END_OF_INPUT for every pause or final transcription segment. For the generic extension the host supplies the authoritative boundary. Strict Vapi mode requires a confirmed provider contract; prototype heuristic mode must identify itself as unqualified.

PARTIAL is used only when allow_partial_responses permits it. Streaming audio uses CHUNK. The proposed event-only boundary representation uses CHUNK with output_events and no prompts; validate this when partials are disabled (Q2). Do not send a FINAL merely to carry a boundary event if it would prematurely finish the assistant reply.

No independent local VAD should compete with Vapi for turn ownership in the default Vapi mode. A future local-endpointing mode must explicitly take control of both providers and define cancellation acknowledgments.

## 10. Audio path and bounded ordering

The PCMU path avoids decoding/re-encoding. Input Base64 decoding has an allocation limit before decoding. Reject unsupported encoding/sample-rate combinations, binary frames on the Webex connection, and recognizable WAV/RIFF containers.

Qualified guide limits are a minimum 320 raw bytes and maximum 32 KB per output chunk, with 8,000–16,000 bytes recommended for larger batches. Choose smaller compliant chunks for conversational latency and flush by accumulated bytes, not playback-duration sleeps. Interpret the ambiguous maximum conservatively as 32,000 bytes until Cisco clarifies.

Prototype chunk assembly: retain 320 bytes until more data or reply completion arrives. Once at least 640 bytes are available, emit a prefix of 320–32,000 bytes while retaining at least 320. At normal completion send the remaining 320–32,319 bytes in legal chunks. This bounds look-behind to 40 ms of audio, preserves all bytes, and avoids a short tail after a preceding chunk. A complete utterance shorter than 320 bytes requires an agreed short-FINAL exception or bounded trailing-silence padding (Q3); it must not be silently dropped. Do not pad with leading silence.

Do not put a second RTP-style jitter buffer in the Webex output path. Preserve upstream burst delivery. Caller-audio pacing toward Vapi is a separate question: test actual Webex batch cadence and Vapi timestamp/endpointing behavior; do not automatically replay a multi-second batch on a wall-clock timer.

Add to rvoip-vapi an opt-in, exclusive ordered stream API carrying Audio, ProviderEvent, InputCommitted, ReplyFinished, ReplyCancelled, and Terminal. The latter three require verified provider semantics; parsing a new local enum cannot create missing information. Sequence each item in the socket-reader task before dispatch. Preserve partial raw frames at reply end. Existing RTP-oriented behavior remains the default for existing consumers.

Audio and control cannot travel through independent queues and later be merged by select without ordering metadata. One writer serializes application responses. Heartbeats may receive priority, but FINAL cannot overtake its audio, and stale audio cannot overtake cancellation. Bound queued bytes as well as item count. On non-cancellation overflow, fail observably rather than silently drop speech and claim a complete reply.

## 11. Vapi application mapping

Create calls through POST /call with saved assistantId and transport provider vapi.websocket, audioFormat format=mulaw, container=raw, sampleRate=8000. Connect to the returned transport.websocketCallUrl; never construct it from a caller-controlled URL. Use only configured provider hosts, HTTPS/WSS, bounded responses, and tenant-specific credentials.

Configure required clientMessages explicitly: speech-update, user-interrupted, transcript, status-update, tool-calls, tool-calls-result, conversation-update, and any verified new completion events. Do not rely on an assistant's defaults. Subscribe before activation to preserve greeting events.

Use a dedicated sandbox assistant with known STT/TTS settings and explicit interruption policy. Its handoff tool is a custom application function (for example request_webex_handoff) with allowlisted route/reason fields. Authenticate tool callbacks, correlate call ID to tenant/conversation, deduplicate tool-call IDs, return the expected tool result, and map to one Webex transfer event. Raw model-supplied destination URLs or phone numbers are not authority.

DTMF input is collected using configured length/terminator/inter-digit policy and submitted once as a structured, escaped user message such as a clearly labeled digit entry. This is an application convention, not native Vapi DTMF. START_OF_DTMF cancels the current reply according to policy; a digit-less start is not a completed input. Avoid mute/unmute as an assumed cancellation acknowledgment: Vapi unmute can restart prior conversation processing.

NO_INPUT invokes a bounded reprompt/escalation policy; ensure Webex and Vapi silence timers do not both independently reprompt. NO_MATCH requires an explicit failed-recognition/application result. CUSTOM_EVENT is an allowlist of names and typed payloads, never arbitrary prompt/system-message injection.

Publish caller transcript only when enabled by tenant policy and customer configuration. Attach assistant text to the corresponding prompt only when aligned with spoken output. Do not use session_summary as a handoff-transcript substitute or assume generated-but-cancelled text was heard.

### Proposed Vapi contract improvements

Prefer an authoritative input-committed notification with a stable input ID, plus reply-started/completed/cancelled notifications with a stable reply ID. Completed must be serialized on the same socket after all audio conversion/framing has flushed. Cancelled must identify the invalidated reply and prohibit later bytes for it. Because existing binary frames have no reply IDs, serialize generations without interleaving and define a reliable fence; a future framed-audio mode may explicitly tag them.

These are proposed provider additions, not existing public Vapi event names. If changes are unavailable, restrict compatibility to a measured assistant/profile and label heuristic behavior. Do not advertise strict BYOVA support based solely on assistant speech-stop plus a drain timeout.

## 12. Authentication and tenant isolation

BYODS uses customer-specific registration and rotating signed tokens. Verify signature/kid, an allowlisted issuer, configured audience/subject, expiration/issued-at with bounded skew, schema UUID, registered URL, and organization binding. Fetch keys only from preconfigured trusted issuers; never construct a network destination from an unverified token. Cache and refresh keys with rate limits and fail closed for unknown unverifiable keys. [BYODS source](https://developer.webex.com/create/docs/bring-your-own-datasource).

The service owns registration refresh before token expiry and Service App OAuth refresh-token storage. Token nonce is registration-level, not a single-use token per conversation; rejecting its second use would reject concurrent legitimate calls. Pin accepted registration versions and define bounded rotation overlap.

Bind the JWT organization to request customer_org_id on both endpoints and to the selected Vapi credential. The authenticated service principal represents the Webex organization; it does not establish the caller's human identity. additional_info and vendor_specific_config remain untrusted selectors/context.

Use standard TLS validation, no credentials in URLs, no static debug-token fallback in production. Mock bearer authentication is allowed only in explicitly selected simulator mode. Never include passwords, API keys, JWTs, socket URLs, audio, transcripts, or raw tool arguments in logs, container images, Terraform variables/state, or simulator reports.

An accepted connection may outlive the upgrade token's expiry; the protocol does not define midstream reauthentication. Confirm Cisco expectations (Q4). Independently support tenant deauthorization and bounded session duration. Do not terminate every live call simply because a registration token rotated without an agreed policy.

## 13. Operational defaults

These are proposed tunable safety budgets, not Cisco requirements:

| Setting | Initial value / rule |
| --- | --- |
| First application request | 10 seconds |
| Agent startup | 15 seconds |
| Initial caller-audio buffer | 2 seconds, 16,000 bytes |
| Incoming text frame | 128 KiB, plus bounded nesting/string/map sizes |
| Decoded single audio input | 32,000 bytes initially; qualify actual upstream batching |
| Per-session queued media | 256 KiB hard maximum; expose queued audio milliseconds |
| Control queue | 128 items, with reserved terminal capacity |
| Application heartbeat | configurable; prototype 15 seconds, deadline 10 seconds |
| Maximum session | 2 hours, tenant-overridable |
| Close/task-join deadline | 5 seconds |
| Concurrent sessions | configured admission limit; initial EC2 prototype 10 |

Respond promptly to Webex application PING and also handle native WebSocket ping/pong. Native heartbeat success must not mask a failed application heartbeat. Webex heartbeat initiation direction and interval need sandbox verification.

Expose /health/live and /health/ready without credentials or tenant data. Readiness reflects validated configuration and ability to admit new work; draining stops new sessions and allows bounded completion of existing ones.

Metrics: active/startup/closing sessions; setup outcome; bytes and frames; queue depth in bytes/ms; input-boundary lag; first output latency; duplicate final attempts; stale audio rejected; handoff requests; heartbeat failures; orphan cleanup outcomes. Do not use conversation IDs or tenant IDs as Prometheus labels.

Production is one process owner per live connection. Horizontal scaling uses normal long-lived WebSocket affinity; process restart is a failed conversation, not resumable media. Keep durable admission/correlation records only as needed for upstream cleanup; never persist bearer URLs.

## 14. Errors

Use schema codes unauthorized, bad_request, unsupported_media, upstream_error, rate_limit, timeout with numeric status and bounded content-free detail. Once a conversation exists, echo its verified identity. Before authentication/start, prefer HTTP rejection or WebSocket close without inventing a conversation ID.

Malformed known messages, sequence regression, identity mismatch, unsupported media, control-queue loss, upstream failure, and heartbeat timeout end the current conversation. Unknown optional metadata is not an error; unknown top-level message/event discriminators are not silently interpreted as valid state transitions.

Normal close maps to local cleanup. Never classify every Vapi disconnect as a normal end: preserve timeout/error versus intentional end/handoff. Do not retry an active Webex session or replay prior audio after reconnect.

## 15. Simulator and acceptance tests

Provide a Rust CLI simulator with a nonzero exit code on failure and concise PASS/FAIL output, plus machine-readable JSON. It connects through real WebSockets, sends deterministic PCMU fixtures, independently validates received envelopes and byte order, and asserts state transitions. Avoid merely sharing the server state machine as its oracle.

Required deterministic scenarios:

1. Authorized list/default/selected agent; unauthorized token and cross-tenant list rejection.
2. Start, greeting, at least three caller/reply turns on one socket; exactly one FINAL per reply.
3. Independent sender sequences, valid gaps, echoed PONG, regression and identity mismatch.
4. Arbitrarily split audio buffers; exact output bytes; legal chunk bounds; final residual handling.
5. Burst of several seconds delivered without replay pacing; FINAL cannot overtake the burst.
6. Voice interruption during queued output; no stale generation after the boundary; simultaneous completion race.
7. VAD pause followed by resumed speech; no false committed boundary in strict mode.
8. DTMF order, A–D/star/pound, terminator, timeout, and START_OF_DTMF with no digits.
9. NO_INPUT, explicit NO_MATCH, custom-event allowlist, duplicate handoff requests.
10. Empty/event-only reply, allow_partial_responses=false, short audio tail.
11. Invalid Base64, WAV input, wrong rate, oversized/deep JSON, duplicate/unknown fields, invalid enums.
12. Missing application PONG while native WebSocket pong still works.
13. Webex disconnect during Vapi creation, provider close, writer stall, queue overflow, cancellation.
14. Bounded repeated-session cleanup; no tasks, permits, maps, or active provider calls left behind.
15. Logs/debug/report redaction using canary secrets and transcript strings.

Use paused Tokio time for timeout tests and real network sockets for integration tests. Property-test chunk partitioning and final/cancel interleavings. Preserve failed traces with synthetic payloads only. Production gate requires real Cisco evidence in addition to these tests.

Vapi qualification must capture event/audio ordering for greeting, long TTS bursts, silence, multiple transcript finals, speech-to-speech if supported, interruption, tools, and hangup. Record provider version/assistant configuration/region. No byte-tagged test can prove the real provider emitted a missing boundary.

## 16. Container and AWS deployment

Ship a multi-stage Docker build, non-root runtime, read-only root filesystem where possible, bounded writable tmpfs, health checks, and pinned image digest in deployment outputs. A single documented compose command starts the demo; a second command runs the full simulator. Operators should not need a Rust toolchain.

Terraform provisions a dedicated, tagged prototype environment in the chosen AWS account: EC2 ARM instance, minimal instance role, encrypted root volume, networking, and container startup. Initial access is through Systems Manager, with no SSH ingress. Bind private simulator endpoints to loopback; never publish a mock-auth listener to the internet.

For actual Webex ingress, add a partner-controlled hostname, trusted TLS certificate, WSS on 443, WebSocket-compatible timeout settings, and the registered domain/schema. Do not treat the Cisco sandbox's domain as a hostname we control. Infrastructure outputs must distinguish simulator-ready from Webex-ready.

Load runtime credentials from Secrets Manager or equivalent, referenced by ARN; Terraform must never receive secret values. Grant the instance access to exact required secrets and image/log resources. Use IMDSv2. Tag resources with purpose/owner and document terraform destroy, expected billable resources, log retention, and token rotation.

Deployment verification: container health, simulator suite on EC2, restart recovery behavior, redacted logs, denied external access in private mode, and reproducible redeployment. A reachable health endpoint alone is not acceptance.

## 17. Delivery sequence and gates

1. Generic protocol/session implementation and deterministic mock agent. Exit: conformance scenarios pass without Vapi.
2. rvoip extension lifecycle integration, bounded cleanup, optional facade feature. Exit: admission/teardown and source-compatibility checks pass.
3. Vapi transport ordering mode and service mapping. Exit: explicit capability report distinguishes supported/provisional/missing semantics.
4. Container + Terraform + EC2 simulator deployment. Exit: non-developer runbook reproduced on the instance.
5. Cisco sandbox onboarding: Service App, data source, CCAI, flow and entrypoint; create dedicated Vapi assistant. Exit: live multi-turn, interruption, DTMF, handoff and close evidence.
6. Production qualification only after open protocol questions and provider fences are resolved. Exit: reviewed compatibility matrix, load results, rollback and cleanup evidence.

Each phase records actual tests and limitations. A prototype may implement heuristic Vapi turn mapping for experiments only if it cannot be mistaken for strict mode. Missing production semantics are not silently hidden behind a timeout.

## 18. Risk register and unresolved decisions

| ID | Risk / incompatibility | Severity | Mitigation / release condition |
| --- | --- | --- | --- |
| R1 | VAD stop differs from committed caller turn | Critical | Authoritative provider boundary or explicitly limited, qualified mode |
| R2 | Assistant stop can race conversion buffers; rvoip queues reorder control/audio | Critical | Same-socket completion barrier and ordered unpaced adapter stream |
| R3 | Cancelled reply finality and stale buffered playback | Critical | Generation fencing plus Cisco interruption fixtures |
| R4 | Published schema rejects valid examples | High | Narrow normalization, typed parser, upstream issue tracking and live captures |
| R5 | rvoip Vapi path paces and drops old frames for RTP | High | Separate opt-in unpaced path with explicit overflow failure |
| R6 | Native Vapi transfer/DTMF unavailable | High | Custom handoff tool and documented DTMF text convention |
| R7 | Endpoint/token provisioning fails or expires | High | Separate registration renewal, exact tenant binding, operational alerts |
| R8 | Short final audio and partial-disabled events underspecified | High | Q1–Q3 sandbox tests; no false conformance claim |
| R9 | No reconnect/resume; deployment or network drop ends a call | High | Drain deployments, fail clearly, cleanup orphan upstreams |
| R10 | Codec/turn behavior varies by Vapi assistant pipeline | High | Pin qualified assistant profile; explicitly exclude untested realtime variants |
| R11 | Tenant data or credentials leak through content/logging | High | Redacted types, content-free metrics, secret-store injection, negative tests |
| R12 | Transport enum addition breaks downstream matches | Medium | Coordinated rvoip release and downstream compile checks |
| R13 | Webex transcript publication/transfer completion unavailable | Medium | Confirm organization features; report request versus completion honestly |
| R14 | Bridgefu remains on 0.3.8 | Medium | Keep it out of the initial dependency graph |

Questions to resolve with Cisco sandbox evidence/support:

- Q1: Exactly how should a cancelled reply's one FINAL be ordered with START_OF_INPUT, and does it affect the next reply?
- Q2: Are event-only CHUNK responses supported when allow_partial_responses=false?
- Q3: Is a sub-320-byte final tail accepted; if not, is trailing PCMU silence padding acceptable?
- Q4: What happens when an accepted connection outlives its upgrade token or registration is deauthorized?
- Q5: Confirm path-prefix registration, catalog socket close behavior, heartbeat direction/timeouts, input batch sizes, and the 32 KB definition.
- Q6: Which handoff metadata is consumed by this flow, and is there any completion acknowledgment?
- Q7: How are prompt text, caller session_transcript updates, and interrupted speech represented in the agent handoff transcript?

Questions to resolve with Vapi evidence/maintainers:

- V1: Can committed endpointing be exposed as a stable client event?
- V2: Can reply completion/cancellation be ordered after audio conversion and prevent late audio for an ended generation?
- V3: Are ordinary assistant speech-stop events sufficient for any specifically qualified pipeline, including interrupted and empty replies?
- V4: What caller-audio burst cadence and call-create idempotency are guaranteed?

No passwords, API keys, or live bearer URLs belong in this specification or its evidence attachments.


## 19. Historical WebSocket-only checkpoint — 2026-10-28

Implemented in this checkout: provider-neutral protocol/session crate with bounded PCMU chunking, typed generation tokens, strict identity and sequence checks, cancellation fencing and terminal state; nine protocol tests. Added `rvoip-vapi::raw_session` as an opt-in ordered, unpaced socket API, with a loopback test demonstrating audio/control order and the correct `end-call` command. Existing RTP adapter behavior is unchanged.

The separate sibling `webex-vapi-connector` application contains an Axum private listener, 17-scenario independent mock simulator, experimental Vapi mapping, real provider probes, Docker image and Terraform/SSM runbook. See its `docs/VALIDATION.md` for measured deployment and test results.

This is a deliberately limited implementation checkpoint, not completion of all interfaces above. Still required: full ConnectionAdapter/core admission integration; Cisco JWS validation and BYODS refresh; qualified endpointing/audio-completion contracts; digit collection; custom handoff tools/transcript policy; load/chaos qualification; and live Cisco interoperability. The app refuses startup without PRIVATE_DEMO=true and reports productionQualified=false.

Live Vapi evidence: the dedicated assistant accepts raw PCMU and returns greeting audio. One 25-second probe received 199,835 audio bytes; 146,091 were outside assistant speech windows, all digital PCMU silence. The experimental mapping suppresses that idle silence. Non-silent audio outside an active assistant speech window fails the session; following interruption it discards media until another assistant-start event. These event fences are heuristic because the provider gives no authoritative per-audio generation ID. A successful greeting/NO_INPUT probe does not prove barge-in or committed user-turn semantics.

Cisco provisioning evidence, updated October 29: Cisco enabled an additional
early-access toggle for the test organization and the
WebSocket schema `a38a10b7-43e4-4676-a076-a7d6dce9387d` is now visible on the
existing **rvoip Webex Vapi Sandbox** Service App. It remains unselected and
all schema checkboxes are disabled. With explicit user approval we deauthorized
the app, confirmed deauthorization, and reloaded the developer portal; schema
controls remained disabled even though the organization token selector was gone.
We restored the existing gRPC authorization, retrieved replacement OAuth tokens,
and renewed the existing gRPC datasource. No WebSocket datasource or flow was
created during that investigation. Cisco subsequently corrected its guidance:
the current early-access implementation requires a **separate Service App** with
WebSocket selected at creation; an existing app cannot add a schema type.
Verified the new-app form accepts `VoiceVirtualAgent_WebSocket` and enables Add
Service App. With explicit approval, **rvoip Webex Vapi WebSocket Sandbox** was created
and authorized for the same org. Control Hub shows Authorized and the developer
portal shows Approved; its credentials and org tokens are saved separately.
The new datasource is `4aacb92a-c008-40fb-bda3-2df4f2225ccd`, with CCAI feature
`af9935c0-2684-4ade-95cc-5c93060407e5` and dedicated Test flow
`rvoip_Vapi_WebSocket_Sandbox` (`6ae3ce13e021f9d4c6143110`). Do not deauthorize
gRPC or overwrite its registration to test WebSocket.

The application now shares the Cisco JWS verifier across transports but binds
each independently to its schema, URL, nonce, issuer, subject, audience and org.
WebSocket uses `WS_AUTH_MODE=webex` and `WEBEX_WS_` variables; gRPC retains its
existing variables. `PRIVATE_DEMO=false` is supported with both authenticated
transports. Exact-path public ALB routes expose `/v1/va` and
`/v1/listVirtualAgents`; backend ports remain restricted to the ALB.
`VAPI_WS_ASSISTANT_ID` keeps the basic voice trial separate from the existing
gRPC context assistant. Direct public WebSocket-to-Vapi greeting/reprompt and
cross-transport authentication tests pass. Actual Cisco PSTN diagnostics revealed `metadata: null` and
`additional_info: null` on SESSION_START. The extension normalizes only these
two optional maps to empty maps, preserving all identity, sequence and media
validation; malformed populated maps remain errors. This is a measured
early-access compatibility adjustment beyond the schema's object-only types.
Actual PSTN inbound speech now reaches Vapi and is answered by the provider,
but the caller hears no return audio. Explicit response defaults and retaining
the final audio tail on FINAL did not restore playback. The sandbox number was
restored to the working context demo; the WebSocket resources remain provisioned.
A redacted Cisco-side playback-trace request is in the app repository at
`docs/CISCO_WEBSOCKET_PLAYBACK.md`. No full WebSocket interoperability or
production turn-control qualification is claimed. The connector repository's
`docs/WEBSOCKET.md`, `docs/VALIDATION.md` and `docs/CISCO_ENABLEMENT.md` record
the current deployment, diagnostic findings, routing and corrected provisioning.
The earlier private-only checkpoint above is historical.


## 20. gRPC implementation specification and checkpoint

The sandbox exposes gRPC schema `5397013b-7920-4ffc-807c-e8a3e0a18f43`.
Implement this as an optional `grpc` feature in the same provider-neutral crate,
with an independently selected transport in the executable. Do not translate gRPC
messages into WebSocket JSON: their interaction lifetimes and chunk constraints
differ. `WEBEX_TRANSPORT=websocket|grpc|both` selects listeners at startup;
production Cisco authentication currently requires `grpc` alone.

### 20.1 Wire contract and dependency boundary

Vendor Cisco's `voicevirtualagent.proto` and `byova_common.proto` from the same
pinned schema repository revision listed in section 2. Generate with prost 0.13,
tonic 0.12 and a vendored protoc at build time, behind the crate feature. The
package is `com.cisco.wcc.ccai.media.v1`; implement `VoiceVirtualAgent` methods
`ListVirtualAgents` and `ProcessCallerInput`, plus standard gRPC health Check.
No Vapi dependency belongs in the wire crate. Preserve upstream files and record
provenance. Upstream had no top-level license at retrieval; redistribution needs
review before publishing these files as a released crate.

Use the [gRPC behavior guide](https://developer.webex.com/mcp/docs/bring-your-own-virtual-agent-over-grpc)
for lifecycle behavior. `ProcessCallerInput` is bidirectional streaming, but a
new RPC is used for each interaction within a stable conversation ID. Input is
raw mono PCMU/8 kHz. Output CHUNK audio must be 100–64,000 bytes; hold back at
least 100 bytes to preserve arbitrary partitions, then send one empty-audio
FINAL and close the response stream normally. Do not pace output for playback
or attach a WAV header. Reject an entire reply shorter than 100 bytes; do not
silently fabricate padding. This differs from the WebSocket 320-byte minimum.

An alternate, conversation-wide `GRPC_RESPONSE_MODE=wav` uses Cisco's whole-WAV
mode: buffer the complete PCMU prompt, decode using rvoip codec-core G.711, and generate
a format-tag-1 WAV with 8 kHz/mono/16-bit little-endian samples and exact RIFF
and data sizes, then send one FINAL
and complete the RPC. Bound the audio buffer to 120,000 bytes (15 seconds);
fail longer replies explicitly. WAV mode adds whole-prompt generation latency.
The 15-second limit means at most 240,044 encoded bytes including the header.
Set the outgoing protobuf size limit to 256 KiB; input remains capped at 128 KiB.
No resampling or synthetic audio padding is performed. Never mix WAV and headerless CHUNK modes within a
conversation. Both framing modes require simulator validation.

### 20.2 Ownership, state and isolation

A bounded conversation registry maps a validated conversation ID to one actor.
That actor exclusively owns one ordered `RawVapiSession`, surviving RPC EOF
between interactions. A registry generation protects removal against reuse.
Each RPC carries its own input stream and bounded output channel. Replacing an
active RPC with meaningful input cancels its response and drops its input stream; no old stream can
write new caller audio. Continue draining Vapi between interactions so silence
cannot accumulate into the next reply.

Validate organization, agent, conversation, input oneof, audio codec/rate,
language and payload bounds on every message. The first request must establish
valid metadata. Live Cisco audio frames following the initial three-byte
placeholder omit organization, agent, encoding, sample rate and language. Bind
these omissions to the validated first request of that same RPC; never use
global codec defaults or metadata from another RPC. Preserve every audio byte
and reject conflicting explicit identities or formats. The protobuf makes agent ID
optional: the single-agent prototype accepts omission but rejects a nonempty ID
that differs from its configured agent. Capture timestamps are optional:
real sandbox traffic omits them despite the mandatory annotation in the proto.
Cisco opens a listening RPC during greeting playback with a three-byte sentinel.
Ignore audio buffers of 15 bytes or fewer, matching Cisco's Java/Python reference
connectors, and do not supersede the preceding reply until meaningful input
arrives. Complete placeholder-only half-closed RPCs normally without ending
the conversation or generating speech events. Bound this startup wait to 60 seconds and 128 placeholders, validating
identity and codec on each message. A regression scenario must cover overlap
between this placeholder RPC and a still-active output RPC. Bind the configured
single organization at authentication and again in request validation. New
conversations require SESSION_START; reject duplicate starts on existing calls.
SESSION_END, transfer terminal, shutdown, provider failure and call timeout
remove the registry entry and close the Vapi call. Stream half-close alone must
not synthesize END_OF_INPUT. Client cancellation during an active response ends
the conversation; normal FINAL/EOF retains it for the next interaction.

Initial limits: 10 concurrent conversations, 4 pending interactions per actor,
32 buffered response messages, 128 KiB decoded-message cap, 5 seconds to first
input, 60 seconds per interaction, 120 seconds idle between interactions and
600 seconds total per call. Provider setup and close have independent deadlines.
Backpressure failure aborts the session rather than dropping speech. Account
for pending actors in health and clear readiness before shutdown. HTTP readiness
must wait for authentication initialization and the gRPC socket to bind.

### 20.3 Vapi event mapping

Use the ordered, unpaced raw socket API. Preserve PCMU bytes; only framing changes.
User speech started/stopped maps to START_OF_INPUT/END_OF_INPUT. Assistant speech
started opens an output window; stopped flushes audio and sends FINAL. These are
experimental heuristics, gated by `ALLOW_HEURISTIC_TURNS=true`; they do not repair
the missing committed-endpoint and audio-generation guarantees in section 3.

Observed Vapi silence-only assistant windows can precede real answers. Ignore
leading digital-silence frames and a stop without any non-silent content; do not
close the Webex RPC prematurely. Preserve trailing silence after speech begins.
Unexpected non-silent audio outside an allowed reply window fails the call.
Fence interrupted output until the next assistant start. Real Cisco barge-in,
Vapi user-interrupted-only events and repeated cancellation must still be
qualified before promising production interruption behavior.

DTMF is validated and converted to an ordered text message; no native Vapi raw
socket transfer is implied. NO_INPUT requests one reprompt. Mock-only custom
events exercise input boundaries and TRANSFER_TO_AGENT. Native Vapi transfer
and a real agent desktop handoff remain unimplemented. SIP bridging is a
separate follow-up and is not part of this variant.

### 20.4 Authentication and control plane

A public business RPC must validate Cisco datasource RS256 JWS before reading
its body or creating a paid provider call. Require an allowlisted issuer,
matching audience, subject, organization, schema UUID, exact registered URL and
current registration nonce in jti; validate signature, exp, nbf when present,
iat and a maximum 24-hour lifetime. The nonce is reusable across calls, not a
per-request replay ID. Never fetch a URL taken from incoming token claims.
Cache trusted JWKS for at most one hour, rate-limit refresh attempts and fail
closed on expired/unavailable keys. Health Check is public and contains no
provider or tenant details. PRIVATE_DEMO test bearer must never be exposed by
the public route. mTLS is optional upstream and not implemented here.

Create and authorize a Webex Service App with datasource read/write scopes,
register the gRPC schema and TLS hostname, then create a Contact Center AI
feature using the authorized app and datasource ID as Resource identifier.
Flow Designer Virtual Agent Voice selects that feature and its returned agent.
The real API requires `tokenLifetimeMinutes` (not `tokenLifeMinutes` in some
samples). The sandbox currently uses issuer `https://idbroker-b-us.webex.com/idb`;
this is explicitly allowlisted, not automatically trusted from any JWT.

Store OAuth client credentials, refresh/access tokens, registration nonce and
JWS outside Git with mode 0600; runtime Vapi credentials and JWT bindings belong
in AWS Secrets Manager, never Terraform values/state. `webex-datasource.py`
provides list/register/inspect/refresh-oauth/renew without logging tokens. Save
pending nonce before a mutation, and reconcile unknown outcomes instead of
blindly repeating creation. Rotation changes the nonce; deploy matching runtime
bindings in a maintenance window until overlapping nonce support exists.
Automatic renewal/rotation is not yet qualified. A 24-hour registration expiry
must be renewed explicitly; do not market this checkpoint as unattended service.

### 20.5 Deployment and test acceptance

A non-root read-only ARM64 container hosts HTTP health and gRPC on separate
ports. Terraform creates EC2/SSM, ECR, Secrets Manager, ACM/DNS and a two-subnet
ALB. Public TLS 443 forwards only the protobuf service and health paths. EC2
8086 accepts only the ALB security group; no SSH ingress. Start with a fixed
503 listener; enable forwarding only after Cisco-mode runtime is verified.
Image digest changes replace the instance and interrupt calls. No HA or rolling
conversation drain is promised.

Require: crate byte-preservation/DTMF tests; actor silence-window, interruption
and backpressure tests; signature/claim rejection tests; independent 15-scenario
gRPC mock simulator in both CHUNK and WAV modes; unchanged 17-scenario WebSocket regression; container test;
real Cisco JWS verification; AWS simulator; real Vapi greeting, NO_INPUT and
paced synthetic caller audio with START/END and non-silent reply. Finally run a
real Webex call and verify two-way speech, interruption, digits, timeout and
cleanup independently. Simulator/provider passes alone do not satisfy that last
gate. Record results and current deployment in the application validation log.

Implemented at this checkpoint: optional wire module/chunker/validator, tonic
actor host and simulator, Cisco JWS validation, Service App/datasource scripts,
container and Terraform TLS edge. The authorized sandbox app and CCAI feature
are provisioned. Full rvoip ConnectionAdapter integration, authoritative Vapi
turn contract, handoff, automatic renewal, real Cisco call qualification and
load/soak tests remain release gates. The runtime reports productionQualified=false.


### 20.6 Observed Vapi fallback when user VAD events are absent

The sandbox can recognize caller speech and generate an answer without emitting
user speech-update events. In explicitly enabled heuristic mode, nonempty user
transcription after admitted caller media marks input started. An STT final
segment alone must not commit the utterance. If Vapi starts its assistant reply
while that recognized input remains uncommitted, send END_OF_INPUT once before
forwarding assistant audio. Late VAD/STT events cannot reopen that completed
input. Scope this to the active RPC and require actual caller audio, preventing
an unrelated transcript from starting a turn. Test absent VAD, repeated/final
transcripts, delayed VAD events, correct START/END ordering and complete audio
forwarding. This is an additional heuristic, not an authoritative provider
contract; recognition latency may delay barge-in. Strict qualification still
requires the explicit provider lifecycle described in section 11.

### 20.7 Optional native Webex bridge to a Vapi phone agent

A separate inbound flow path can use Cisco's Bridged Transfer activity to dial
a dedicated Vapi PSTN number while Webex retains the original caller. When the
Vapi leg ends, Webex resumes the flow and may start the gRPC virtual agent.
This is a native flow configuration, independent of the Rust media connector;
it is not an implementation of the provider-triggered handoff contract in
section 11 or an arbitrary SIP-URI transfer.

The sandbox's `rvoip_Vapi_Options_Sandbox` Test version 1 selects gRPC with menu
digit 1/default and bridged Vapi with digit 2. An automated real call on
2026-10-28 verified the Vapi bridge greeting and arithmetic answer, Vapi leg
hangup, Webex's return announcement, and a geography answer over gRPC. Cisco's
activity trace independently showed successful bridge, return, and gRPC nodes.
The application repository contains the published JSON export and
`docs/BRIDGED_TRANSFER.md` with exact resources, replay commands and rollback.

Keep the destination configured by the operator, not chosen freely by model
output. Place the bridge before queueing/assignment and only on supported
inbound next-generation voice flows. Bound outbound setup and the Vapi leg's
lifetime. Wire failure to an explicit fallback. Additional carrier charges,
caller identity, context propagation, direct SIP routing, and busy/no-answer
failure scenarios require separate evaluation. Returning starts a new Vapi
conversation; it does not carry the bridged assistant's history into gRPC.

## Context exchange prototype update — 2026-10-28

The companion `webex-vapi-connector` service now implements bidirectional context
using existing rvoip extension interfaces. No rvoip-vapi source change was needed:
`saved_with_overrides` supplies `assistantOverrides.variableValues` when the
connector creates the call. gRPC initial `EventInput.parameters.context` selects
a configured context assistant. An authenticated Vapi tool commits an immutable,
call-bound result; the actor returns final `CUSTOM_EVENT` named `context_result`
with `OutputEvent.metadata`. Cisco routes this to Virtual Agent V2 Handled and
exposes the result as JSON `MetaData`.

This gRPC round trip passed an actual sandbox call: caller
`01a18278-4d32-7000-8a47-f8c3a1935bed`, Cisco interaction
`eda542c8-5c6a-4e21-9f8f-fffe05efaf67`. The assistant received Ada / GRPC-101 /
cobalt; Webex subsequently announced the caller's returned value `violet`.
All Cisco activities succeeded.

For native bridged PSTN, the companion service implements authenticated HTTPS
create/read endpoints and a 24-digit, short-lived DTMF reference. Vapi's keypad
input invokes a context-load tool, binding the reference to that phone call.
Caller ID is not used to correlate records. Direct Vapi PSTN testing passed
keypad input, context loading, result commit and leg hangup. Full native Webex
bridge context readback subsequently passed after the approved secure-token
setup: call `01a18322-389d-766b-8e3f-2ee5d842b05a`, Cisco interaction
`ac2c1410-adb7-4d4b-9955-d5abfbcdd4c1`. Webex sent Grace / BRIDGE-202 / saffron,
Vapi committed `amber`, and Webex retrieved and spoke the result after the Vapi
leg ended. All ten Cisco activities succeeded. The active context flow is
`rvoip_Vapi_Context_Sandbox` Test version 2; both context paths are tested,
while production qualification remains subject to the limits below.

The service is single-instance and memory-backed (1,000 records, 15-minute TTL;
unclaimed references expire after 120 seconds). Restart loses context. Separate
Webex and Vapi bearer tokens protect HTTP routes. Terraform exposes only those
routes through the existing TLS ALB and scopes backend port 8080 to the ALB
security group. The full API contract, tests, rollout and rollback instructions
are in the companion repository's `docs/CONTEXT-EXCHANGE.md` and `docs/VALIDATION.md`.


## 21. WebSocket sandbox WAV output compatibility — 2026-10-29

This section qualifies an explicit early-access exception to the raw-output
requirements above. The documented/default WebSocket profile remains raw
PCMU/8 kHz. In sandbox the test organization, raw PCMU and
raw PCM16 were silent. A header-only comparison made the same PCM16/8 kHz sample
audible in WAV. Separate complete WAVs in CHUNK and FINAL also played. A real
Vapi PSTN call subsequently passed greeting, two spoken questions (four, Paris)
and normal caller goodbye. This is basic interoperability, not production
turn-control qualification. Evidence and Cisco questions are in the application
repository's `docs/WEBSOCKET_PLAYBACK_TESTS.md` and `docs/CISCO_WEBSOCKET_PLAYBACK.md`.

Implementation contract:

- The optional `wav-compat` feature exports `wav::pcmu_to_pcm16_wav` and
  `AudioOutput::Pcm16Wav`. `grpc` enables the shared converter; its existing
  whole-WAV output remains byte-equivalent.
- `Session::with_audio_output` selects the output representation before admission.
  `RawPcmu` is the default. Input remains strict raw PCMU/8 kHz; reject WAV input.
- Decode G.711 mu-law into signed PCM16 little-endian, 8 kHz, mono. Do not resample,
  relabel mu-law as linear PCM, use A-law, or fabricate padding.
- Each returned prompt is a complete 44-byte-header RIFF/WAV file. The entire
  decoded container must be at most 32,000 bytes, leaving 15,978 input samples per
  chunk. Preserve the 320-sample tail and include it in exactly one FINAL.
- Preserve generation fencing, monotonic sequence numbers, bounded buffers and
  per-reply finality. Disable partner prompt cancellation between WAV chunks;
  caller barge-in remains independently enabled and still needs qualification.
- The application requires both `WS_RESPONSE_MODE=wav` and
  `ALLOW_WS_WAV_COMPAT=true`; invalid modes fail startup. gRPC flags and all
  authentication, tenant-binding and lifecycle checks remain independent.
- Diagnostic fixtures require `ALLOW_WS_DIAGNOSTICS=true` plus a named profile.
  They bypass Vapi only for WebSocket and must be disabled for normal calls.
- Verify chunk size including headers, exact decoded PCM across arbitrary input
  partitions, one FINAL, stale-generation rejection, and raw default behavior.
  A live probe must examine decoded samples; RIFF header bytes cannot count as
  non-silent speech. Actual caller-side transcripts are the interoperability gate.

Cisco must clarify why its guide says raw PCMU/no container while the response
schema describes PCM16/16 kHz and includes a RIFF-prefixed example. Do not promote
this opt-in profile to the default or claim general availability support without
confirmation. WebSocket context handoff, DTMF, interruption, load, renewal and
recovery remain separate acceptance gates.
