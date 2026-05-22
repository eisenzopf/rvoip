# PRD — `rvoip` (the unified real-time gateway library)

**Status:** v1 draft. Reframes the v0 voice-harness scope around the three-layer architecture (SDK / Universal Conversation Transport Protocol / rvoip / gateway). Companion documents in this directory:
- `CONVERSATION_PROTOCOL.md` — the wire/SDK protocol (UCTP) that rvoip implements server-side. Speaks the voip-3 vocabulary (Conversation / Session / Connection / Stream / Message / Participant).
- `INTERFACE_DESIGN.md` — the rvoip Rust library design (crate layout, traits, types).
- `CONCURRENCY_PLAN.md` — prior, broader-scope concurrency plan; kept as historical reference.

Source-of-truth for terminology: `/Users/jonathan/Developer/Rudeless/voip-3-conversation-model.md`.

---

## 0. The three-layer architecture (orientation)

```
APPLICATION (mobile, web, desktop, embedded, AI agent)
   │ uses an SDK that speaks UCTP
   ▼
Universal Conversation Transport Protocol (UCTP) — substrate-agnostic; runs over QUIC, WebTransport, WebSocket
   │ (interoperates with SIP and WebRTC at the gateway boundary; never tunneled)
   ▼
rvoip — Rust library implementing UCTP server-side; provides voip-3 abstractions, adapter
        traits, identity/persistence trait surfaces, capability negotiation, bridging
   │
   ▼
A UCTP-speaking gateway/server you build with rvoip — Thelve is the canonical such gateway
```

**rvoip is the Rust library you build a UCTP-speaking gateway with.** Pure-SIP carriers can use rvoip's SIP surface directly without ever touching UCTP. UCTP-only application servers can use rvoip without ever touching SIP. Thelve uses both because it interconnects worker/customer apps (UCTP) with PSTN (SIP) and embedded WebRTC widgets (WebRTC).

---

## 1. Context and product role

`rvoip` is a **unified real-time communication library for software developers** — one library that brings together SIP, WebRTC, QUIC, and WebTransport so a single application or service can host voice, video, and messaging across all of them. It is the implementation underneath the Universal Conversation Transport Protocol (UCTP) on the server side, and it is also the SIP/WebRTC adapter on the interop side. It packages the rest of the rvoip workspace (session-core → rvoip-sip, dialog-core, sip-transport, media-core, registrar-core, infra-common) plus new substrate adapters (rvoip-quic, rvoip-webtransport, rvoip-websocket) and UCTP-aware orchestration (rvoip-core, rvoip-uctp) into a developer-facing surface.

We expect adoption across several developer audiences:

- **Carriers and ITSPs** — SIP trunking, PSTN interconnect, gateway functions.
- **Voice AI developers** — building single-purpose AI voice services (sales bots, IVRs, voice assistants).
- **CPaaS providers** — building programmable-voice platforms.
- **Call center developers** — building queue-and-route systems and IVRs on top of voice primitives.
- **AI agent platform developers** — Thelve (under the Rudeless brand) is the canonical consumer in our deployment; anyone building human+AI workforce platforms is in this category.

What rvoip provides is the **voice plane**: SIP B2BUA, media bridging, in-process AI runtime hosting, transcripts, recording, transfer mechanics, capacity protection. What rvoip does *not* provide is **orchestration above the voice plane** — workforce assignment, customer continuity, queues, skills, training flywheels, cross-channel interaction modeling. Those layers stay with the consumer. Thelve provides one such orchestration layer; another developer building a call center, CPaaS, or AI agent platform either builds their own or, if they want what Thelve does, uses Thelve.

This PRD draws the line. rvoip is *narrow on purpose* so the layer above can be anything.

Companion documents in this directory: `CONCURRENCY_PLAN.md` (prior, broader-scope concurrency plan, kept as historical reference).

## 1.1 Developer profiles and user stories

These illustrate why each audience reaches for rvoip and what they're actually building.

- **Voice AI developer**: *"As a developer building a voice AI telephone service, I use rvoip to receive PSTN calls, attach an AI runtime to each call, stream transcripts and recordings out, and let my own dialog logic run."*
- **Call center developer**: *"As a call center developer, I use rvoip as my voice channel conduit — calls in, calls out, transfers, recording, transcription — and I build my own queues, skills, and routing on top."*
- **CPaaS provider**: *"As a CPaaS provider, I use rvoip as the voice library underneath my programmable-voice product. My customers see my API; I drive rvoip with their commands."*
- **Carrier / ITSP developer**: *"As a carrier engineer, I use rvoip as a SIP gateway — trunking, codec negotiation, STIR/SHAKEN passthrough, billing-grade usage records — without paying for orchestration features I don't need."*
- **AI agent platform developer (Thelve, canonical)**: *"As the developer of an AI+human workforce platform, I use rvoip as the voice channel for my workers. My platform owns identity, presence, queues, skills, and the AI training flywheel; rvoip owns the call. I also use rvoip's UCTP server surface so my mobile, web, and embedded clients log in once and reach my workforce across substrates."*
- **UCTP application developer**: *"As a developer building a real-time app on Thelve (or another UCTP-speaking server), I use the rvoip-client SDK so my mobile/web/desktop app speaks UCTP without me writing protocol code. I get messaging, voice, video, and presence; I never see SIP or WebRTC even when my call ends up bridged to PSTN."*
- **WebRTC integrator**: *"As a developer of a WebRTC client, I want my browser-based media to bridge cleanly with PSTN/SIP — natively inside the rvoip server, not via a separate SBC."*
- **Provider integrator**: *"As an ASR / TTS / dialog provider, I want my model exposed to rvoip via a clean trait so any consumer can choose me."*
- **Operator / SRE**: *"As the operator of a Thelve / call center / CPaaS deployment built on rvoip, I want OpenTelemetry traces, per-tenant capacity gauges, and structured failure events so I can run this at scale."*

Two framing rules across all of these:

1. **Orchestration is the consumer's responsibility.** rvoip provides voice primitives; the consumer provides the meaning. Thelve is one such consumer; carriers, CPaaS, and call center developers are others. Anyone wanting Thelve-style workforce orchestration must build it themselves (or use Thelve).
2. **The AI harness is optional.** Carriers and CPaaS providers may never use it. Voice AI developers and Thelve use it heavily. The harness is a feature consumed by the `AttachAi` command; ignoring it leaves the rest of rvoip fully usable as a pure SIP gateway.

In short: **rvoip is the unified real-time library you use to build a Universal Conversation Transport Protocol-speaking gateway/server.** It carries SIP, WebRTC, QUIC, and WebTransport in one process, exposes a single set of voip-3 abstractions on top, and lets a consumer like Thelve focus on workforce orchestration instead of protocol bridging glue.

## 1.2 Vision: where rvoip could matter

The §3 use cases and §4 capabilities describe what v1 must do. This section names the broader bets motivating the design — the wedges where rvoip, fully realized, could change the shape of voice infrastructure for an industry that has not had a coherent unified library since the SIP era began. These are aspirational. Not all of them will pan out, and v1 does not need to deliver all of them. They are listed so the design choices in the rest of this document can be judged against the bets they enable, and so reviewers can tell vision from commitment.

### 1.2.1 Rust-native voice AI infrastructure (the near-term wedge)

The Vapi / Retell / Bland / OpenAI-Realtime cohort has demonstrated that real-time voice AI is a venture-scale market in 2025-2026. The tooling those products are built on — PJSIP plus custom dialog loops plus ad-hoc ASR/TTS plumbing plus bespoke recording pipelines — is C-shaped, fragile, and not designed for AI-as-Participant patterns. rvoip is the first end-to-end Rust substrate aimed at this category: a SIP B2BUA, an AI harness with clean ASR/TTS/Dialog provider traits, WebRTC interop for browser users, and a single command/event surface that handles all of the above. A team building a Vapi-class product on top of rvoip should be able to ship the voice infrastructure in weeks, not quarters, and inherit the rest (SRTP, codec negotiation, transfers, vCon emission) without writing it themselves. **This is the wedge that makes rvoip useful independent of Thelve.** It is also the wedge where the competitive set is most actively investing — LiveKit Agents, Pipecat, and the model providers' own realtime APIs are all aiming here — so execution speed matters more than design completeness.

### 1.2.2 Replacing the FreeSWITCH + Janus + glue stack

Contact centers, CPaaS providers, and voice-AI platforms that need both PSTN inputs and browser-based agents currently stitch together FreeSWITCH (or Asterisk) for SIP, Janus (or mediasoup) for WebRTC, RTPEngine or similar for media bridging, and custom Lua / Python / Erlang for the orchestration glue. This works; it is also operationally heavy, language-fragmented, hard to observe, and hard to extend. rvoip targets the same workload as a single Rust process: SIP, WebRTC, and (eventually) UCTP substrates handled by one library, with bridging and transcoding as first-class primitives instead of glue. The economic argument is operational simplification — fewer moving pieces, one language, one supervision tree, one observability stack — and the maturity argument is memory safety in a domain where uptime matters.

### 1.2.3 First-mover Rust adoption of vCon

vCon (the IETF Virtualized Conversations envelope) is on a credible standardization path. Compliance-driven voice deployments — healthcare, financial services, lawful intercept, regulated contact centers — will need a signed, addressable, redaction-aware conversation envelope, and vCon is the IETF answer. rvoip ships the first Rust implementation as `rvoip-vcon`: JWS-signed by default, JWE-encrypted on demand, with redaction lineage. For compliance-bound voice AI deployments (healthcare-routed AI agents, KYC-bound financial dialogs, recorded sales calls under MiFID II / Reg BI), this is structural — without a signed conversation envelope per Session, those deployments don't pass audit. Being the Rust answer when those buyers go shopping is durable positioning that doesn't depend on rvoip winning on raw performance or features.

### 1.2.4 Architectural runway for QUIC media

The next decade of real-time media transport will not be RTP-over-UDP alone. RTP-over-QUIC (RoQ), QUIC-RTP-Tunnelling (QRT), and Media-over-QUIC (MoQ) are being standardized at the IETF and deployed at Cloudflare, Meta, the BBC, and Twitch. A voice library architected around UDP+RTP today will need a major refactor when QUIC media becomes table stakes — and that refactor is likely to be deeply invasive (it touches transport, congestion control, NAT traversal, and the codec packetization assumptions). rvoip is built with QUIC and WebTransport as peer substrates from v1, so when RoQ and MoQ ship in production, the library is ready, not retrofitted. This is a long-horizon bet whose payoff window is 2027-2029, but the architectural cost of getting it wrong now compounds the longer it waits.

### 1.2.5 AI agents as first-class peer Participants

Most current voice-AI architectures treat the AI as a bolt-on: an audio Stream gets piped to ASR, ASR text gets piped to an LLM, LLM text gets piped to TTS, TTS audio gets piped back. The AI is not a Participant in the architectural model — it is an audio coupling. voip-3's `Participant.kind = ai` makes the AI a peer of human Participants, with identity, role, capabilities, and lifecycle. rvoip implements that model directly: an AI can join, leave, take over, hand off, observe, supervise, and be observed under the same primitives as a human worker. This is not cosmetic — it is what makes "AI handed off to human without disruption", "human consults AI then returns to caller", "AI listener provides real-time coaching to a human agent", and "two AI agents collaborate on a Session as peers" expressible without per-feature special-casing in the consumer. The vision payoff is multi-agent voice coordination becoming a normal pattern instead of a research demo.

### 1.2.6 Honest scope: vision vs. v1 commitment

This document, taken at face value, describes a v1 that is 3-5 years of focused engineering for a small team. The visionary uses above motivate the design choices in §3-§14, but they do not all need to ship in v1 to make rvoip useful. A trimmed v1 — production-grade SIP B2BUA, the AI harness, basic WebRTC interop, single-tenant `Client` SDK, and vCon emission — covers §1.2.1 (the voice AI infrastructure wedge) and partially §1.2.2 (FreeSWITCH+Janus replacement) on its own. The rest of the design — UCTP server over QUIC/WebTransport, full identity-assurance gradient, AAuth, federation, DTLS-SRTP fingerprint binding — can be deferred to v1.x or v2 without sacrificing the v1 wedge. Reviewers should weigh the v1 commitment in §3-§14 against the §1.2 vision and push back where the v1 over-reaches what one team can credibly ship before the competitive landscape moves past it.

## 2. Shape

rvoip is a **multi-substrate B2BUA / unified gateway with AI runtime hosting**, consumed as a Rust library by any real-time application or service. Four responsibilities:

1. **UCTP server**: terminate UCTP-native Connections over QUIC, WebTransport, and WebSocket substrates. Authenticate Identities; manage Conversations, Sessions, Connections, Streams, and Messages per the voip-3 model and UCTP wire spec.
2. **B2BUA + WebRTC interop**: terminate inbound SIP/PSTN legs and WebRTC offers, and originate outbound on either, per the consumer's command. Bridge them into the same Sessions as UCTP-native Connections at the gateway boundary.
3. **Media bridge with optional AI termination**: bridge two Connections of any transport combination (UCTP↔UCTP, UCTP↔SIP, SIP↔SIP, UCTP↔WebRTC, SIP↔WebRTC), with optional transcoding when codec sets differ. Or bridge a Connection to a locally-hosted AI runtime via the rvoip-harness crate. Or bridge two Connections with an AI listener tapping audio. Same media plumbing across all of these. **AI workers come in two modes** — they may be presented to rvoip as a regular endpoint over any supported transport (just like a human worker), or hosted in-process via the AI voice harness (§11). Both are first-class.
4. **North-facing command/event surface**: accepts structured commands from the consumer and emits normalized events back. Defined as Rust traits + serializable message types in rvoip-core. The integration is in-process (the consumer is in the same process as rvoip); message types are transport-neutral so a separate-process deployment over gRPC/NATS remains viable without redesigning the contract.

**In-process integration in v1.** The consumer (Thelve, a CPaaS, a call center app, etc.) holds rvoip as a Rust dependency and exchanges commands and events directly. No network hop on the contract surface. A future "rvoip as a service" deployment is supported by design but is not v1 scope.

## 3. Use cases (v1)

All of these must work end-to-end:

| Use case | Direction | Worker shape | Notes |
|---|---|---|---|
| Inbound CS, AI handles (in-process) | Inbound | AI worker, in-process | Call answered, ASR/TTS/dialog loop runs locally via the harness (§11) |
| Inbound CS, AI handles (SIP endpoint) | Inbound | AI worker as SIP endpoint | AI runtime exposes a SIP URI; rvoip bridges like to any human worker. Harness not used. |
| Inbound CS, human handles | Inbound | Human via SIP | Standard B2BUA bridge to a registered SIP endpoint |
| Inbound CS, human + AI listener | Inbound | Human + AI tap | 3-party audio (separated streams default; mixed on request); AI gets ASR + can whisper text/audio to human |
| Outbound sales, AI initiates (in-process) | Outbound | AI worker, in-process | Place call with CPD/AMD; if voicemail detected, do not start the pitch |
| Outbound sales, AI initiates (SIP endpoint) | Outbound | AI worker as SIP endpoint | rvoip places the call and bridges the AI's SIP leg in |
| Outbound sales, human initiates | Outbound | Human via SIP | Place call from worker's SIP endpoint to PSTN |
| Worker handoff (any direction) | Either | Worker → worker | Blind (REFER), attended (consult-then-merge), or external SIP URI. Works human→AI, AI→human, human→human, AI→AI. |
| Handoff with bridge-back | Either | Worker A → Worker B → optional return | Worker A may terminate after handoff, hand off again, or be bridged back in (e.g., human consults AI then returns to caller) |

Two-party at the SIP level in v1 except for the AI-listener pattern (which is 3-party media, two SIP legs). Conference of multiple humans, supervisor barge-in, and call parking are deferred.

**Symmetry note:** rvoip treats AI-as-SIP-endpoint and human SIP endpoints identically at the bridge / transfer / routing level. The only asymmetry is the in-process AI harness mode. This means handoffs and bridges compose freely in any direction — there is no privileged direction in the platform.

### 3.1 Cross-modality and cross-substrate use cases

The §3 table is voice-centric. The use cases below illustrate voip-3's load-bearing claim — *one Conversation across channels, transports, time, and Participants*. They are first-class v1 requirements, not edge cases. rvoip MUST support each via the voip-3 abstractions (`Conversation`, `Session`, `Connection`, `Stream`, `Message`, `Participant`) without per-channel special-casing in the consumer.

| Use case | Pattern | voip-3 ref |
|---|---|---|
| Chat → voice escalation in same Conversation | UCTP text Session → UCTP voice Session, same `cid`, AI Identity persistent across both | §9.1 |
| Human-AI handoff mid-Session, no disruption | One Session; Participants change roles (AI: `agent` → `observer`; human joins as `agent`); customer's Connection unchanged | §9.2 |
| SIP customer ↔ WebRTC agent in same Session | One Session, two Connections of different transports, codec transcoding (G.711 ↔ Opus) inserted by `rvoip-media` | §9.3 |
| Cross-device transfer mid-Session | One Connection ends; new Connection from new Device replaces it; Participant + Session unchanged | §9.4 |
| QUIC-native AI agent bridged to PSTN customer | Two Connections (QUIC, SIP) in one Session; neither Participant cares about the other's transport | §9.5 |
| Heterogeneous Streams per Participant | Audio-only on SIP, audio+video on WebRTC, data Stream from AI Connection — same Session | §9.8 |
| Conversation-aware coaching | Two related Conversations, one references the other for context | §9.7 |
| Asynchronous multi-channel relationship | One Conversation accumulates Messages and Sessions over days/weeks across SMS, web chat, voice; Identity-level memory informs future Sessions | §9.1, §9.9 |

These drive the requirements in §4 subsections "UCTP server", "WebRTC interop", "QUIC and WebTransport substrates", "Messaging", and "Conversation persistence" below.

## 4. Capabilities — in scope

### Call lifecycle
- Receive inbound SIP/PSTN INVITEs via session-core / sip-transport.
- Originate outbound calls with **call-progress detection (CPD)** and **basic answering-machine detection (AMD)**: report ringing / busy / no-answer / voicemail / human-answer.
- Reject inbound with a configurable SIP code/reason.
- End any active Connection with a structured reason.
- Hold/resume (`re-INVITE` with `a=sendonly`/`a=sendrecv`).
- Mute/unmute (per Connection, per direction).
- DTMF: capture (RFC 2833 and SIP INFO) and play (out-of-band per RFC 2833).

### Bridges
- Two-way SIP↔SIP bridge — works the same whether the far end is a human, an AI worker presented as a SIP endpoint, or an external SIP URI.
- SIP↔in-process-AI bridge (AI runtime terminated inside rvoip via the harness; no second SIP leg).
- 3-party listener bridge: caller + worker (human or AI-as-SIP-endpoint) + AI tap. Tap is **separated streams by default** (caller stream and worker stream as distinct ASR inputs with speaker labels) and can be switched to mixed mono on request.
- Blind transfer (REFER), attended transfer (consult call, then merge), external SIP transfer. Direction-agnostic — works for human→AI, AI→human, human→human, AI→AI.

### AI voice harness (in-process AI mode only)
The harness is the runtime rvoip provides for **in-process** AI workers — i.e., when Thelve says "attach AI to this Connection" rather than "bridge this Connection to that SIP URI." For AI workers presented as SIP endpoints, no harness involvement: standard SIP bridging applies and ASR/TTS live in the remote AI service.

- The AI dialog loop **runs inside rvoip** in this mode: it consumes ASR transcript turns, calls a `DialogManager` trait Thelve implements, and plays the returned text via TTS.
- Provider traits (`AsrProvider`, `TtsProvider`, `DialogManager`, `RecordingSink`) are **remote-capable**: implementations may run in-process or proxy to a separate gRPC/streaming service. The trait surface is the same. Audio streams sub-200ms reflexes (barge-in, fillers) stay in-process; the dialog brain may be remote.
- Barge-in: when caller speech is detected during AI TTS playback, cut the TTS and start listening.
- Filler/backchannel: short pre-recorded "uh-huh"/breathing fillers played locally during long LLM thinks. Optional, default on.
- Replaceable per-call: Thelve can detach an AI runtime mid-call (escalation, handoff, bridge-back) and the harness winds the runtime down cleanly.

### Recording, transcription, and vCons
- **Audio recording: per-command, not always-on.** Default off. `StartRecording` / `StopRecording` toggle audio capture; output streams to a Thelve-configured `RecordingSink` (per-tenant — see §6). Pause/resume supported (PCI-style suppress-during-card-collection).
- **Transcription: per-command.** `StartTranscription` / `StopTranscription` toggle ASR on caller and/or worker streams; transcript turns stream upward as events. Independent of audio recording.
- **vCon emission: ALWAYS, per Session.** Every Session produces a [vCon](https://datatracker.ietf.org/doc/draft-ietf-vcon-vcon-core/) — the IETF Virtualized Conversations envelope — at end-of-Session, regardless of whether audio recording was enabled. The vCon is the durable signed JSON record of the Session: who joined (`parties[]` from Participants), what happened (`dialog[]` per Connection / Stream / Message / transfer event), what analyses were attached (`analysis[]` for transcripts, sentiment, summaries), and what artifacts were captured (`attachments[]` for SIP signaling, STIR certs, consent records). Audio is one optional `dialog[].body` entry, not a precondition for emission.
- **vCon signing and encryption.** vCons are signed via JWS (general JSON serialization, multi-signer — tenant + AI provider + recorder all may sign) and optionally encrypted via JWE when a tenant key is configured. STIR/SHAKEN PASSporTs ride in `parties[].stir`. See §7 for the security model.
- **`RecordingComplete` event includes a `vcon_ref`** (URL + content hash, or — for small vCons — inline signed-JWS body). Consumers retrieve the full vCon via the configured `VconStore` (see `INTERFACE_DESIGN.md` §11).
- **vCon-without-audio is a feature, not an error.** Transcript-only and metadata-only Sessions still produce a complete vCon. This is the durable audit primitive across all rvoip Sessions.
- **Dual ASR when in-process AI is on the call.** When the in-process AI harness is active *and* transcription is enabled, two ASR sessions run on the same audio: the **dialog ASR** (drives the `DialogManager`, tuned for low-latency partials and turn-taking) and the **transcription ASR** (feeds `TranscriptTurn` events and the vCon's `analysis[]` entries, tuned for fidelity, diarization, and final accuracy). They may use different providers and configurations. They never share session state. The same applies to the AI-listener bridge — the listener's ASR is a separate session from any worker-leg transcription ASR.

### Registration event emission
- registrar-core remains the SIP registrar; rvoip does not store registrations.
- rvoip subscribes to registrar-core's events and emits a normalized `RegistrationChanged` event upward (AOR, contact, status: registered/expired/unregistered).
- Thelve consumes these and assembles the higher-level workforce presence model. rvoip never reasons about "who is available" — that is a Thelve concern.

### Capacity protection
- Per-process admission semaphore: rvoip rejects new calls if it is at saturation.
- Per-tenant quota layer (see §6).
- Capacity reports are emitted on a periodic schedule and on demand.

### UCTP server

rvoip MUST act as a UCTP server (per `CONVERSATION_PROTOCOL.md`) over each enabled substrate:

- **Substrate adapters**: QUIC, WebTransport, WebSocket. Each accepts incoming connections, runs the UCTP `auth.hello` / `auth.challenge` / `auth.response` / `auth.session` flow (UCTP §5), and surfaces authenticated Connections to the Orchestrator.
- **Envelope routing**: receive and dispatch every envelope in the UCTP §6 catalog — auth, conversation, session, connection, stream, message, capability, dtmf, recording, identity, error, ack.
- **Capability negotiation**: run the UCTP §8 algorithm at each `connection.offer` / `connection.answer` exchange; insert codec transcoders from `rvoip-media` automatically when the negotiated set requires it.
- **Identity assurance**: per-Connection `IdentityAssurance` enforced at Session join (see §7); step-up via `identity.step-up-request` / `identity.step-up-response`.
- **vCon emission**: every Session ends with a vCon emission (per the Recording subsection) regardless of substrate.
- **Reachability**: per-Identity reachability hints maintained from registrar-core (SIP) plus UCTP `auth.session` / `auth.bye` flows; consumed by `IdentityProvider::reachable_via` (per `INTERFACE_DESIGN.md` §8) to route inbound Sessions to the right Connection.

### WebRTC interop

rvoip MUST interoperate with WebRTC peers — both inbound (a WebRTC client signals into the rvoip server) and outbound (rvoip originates a WebRTC PeerConnection toward an addressable WebRTC peer):

- **Signaling**: WHIP/WHEP and a custom WebSocket-based signaler in v1; bridges to common third-party signalers (Janus, mediasoup, LiveKit) are out-of-tree adapter sub-types but use the same `Connection` abstraction.
- **Media**: DTLS-SRTP over UDP with ICE; bidirectional bundle of audio + video Streams per PeerConnection.
- **DataChannel**: surfaces as a `data` Stream on the Connection; carries Messages when the WebRTC Connection is the only transport in use.
- **Bridge to SIP**: SDP offer/answer translation, codec transcoding (G.711 ↔ Opus is the default pair), DTMF translation (RFC 4733 ↔ SIP INFO).
- **Bridge to UCTP**: a WebRTC PeerConnection becomes one `Connection` in a UCTP Session; the Session's other Participants observe a normal Connection regardless of substrate.

### QUIC and WebTransport substrates

rvoip MUST host UCTP over QUIC (preferred for native apps and server-server) and WebTransport (preferred for browsers and modern mobile WebViews):

- **Transport stack**: TLS 1.3 / QUIC TLS by default; substrates that cannot provide TLS are rejected in production mode.
- **Connection migration**: QUIC's native connection migration is supported; the UCTP-level `Connection` ID is invariant across QUIC migration events. WebTransport does not migrate; reconnection requires a fresh `auth.hello`.
- **Media datagrams**: per UCTP §10.1 framing (8-byte UCTP datagram header + RTP packet). QUIC and WT datagrams use identical framing — same code path in `rvoip-quic` / `rvoip-webtransport`.
- **0-RTT and session resumption**: where the substrate supports it, rvoip exposes a `transport_features` capability that lets clients resume without a full handshake.
- **WebSocket fallback**: when UCTP runs over WebSocket (older browsers, constrained networks), media uses a co-negotiated WebRTC PeerConnection per UCTP §4.3; rvoip orchestrates both transports transparently to the application.

### Messaging

rvoip MUST handle asynchronous Messages as a first-class noun, independent of Sessions (per voip-3 §3.3):

- **Send / receive**: consumer issues `SendMessage` (any Conversation, any Participant); rvoip emits `MessageReceived` for inbound and `MessageSent` to confirm substrate-side handoff.
- **Receipts**: `MessageDelivered` (server-emitted when the Message reaches the recipient's substrate) and `MessageRead` (relayed when a recipient marks read). Both can be opted out per Conversation.
- **History**: consumer queries past Messages via `ListMessages`; rvoip replays from `ConversationStore`. Pagination via opaque cursor.
- **Attachments**: small bodies inline (default ≤ 64 KB); large attachments via out-of-band upload to a content-addressable URL.
- **Cross-substrate**: a Message sent from a UCTP client to a SIP-bridged Participant translates to SIP MESSAGE; from UCTP to WebRTC translates to DataChannel send. The consumer issues one `SendMessage` regardless of recipient substrate.

### Conversation persistence

rvoip MUST persist Conversations and Messages across system restarts and time gaps (per voip-3 §6.2):

- **`ConversationStore` trait** (per `INTERFACE_DESIGN.md` §11.1): consumer provides the implementation. rvoip ships an in-memory default for tests.
- **Closure policy**: per-Conversation, set at creation. `Ephemeral` (default 60s idle close after the last Session ends with no Messages) prevents the "every PSTN call from an unknown number opens a Conversation forever" failure mode. `Persistent` (close only on explicit request) is the Thelve default for worker↔customer engagements.
- **Multi-Session per Conversation**: a single Conversation MAY hold zero or more Sessions over its lifetime — a chat Session today, a voice Session tomorrow, a video Session next week, all `cid`-correlated. Each Session is bounded; the Conversation is not.
- **Vcon storage**: every Session emits a vCon to a separate `VconStore` (per `INTERFACE_DESIGN.md` §11.4); independent of `ConversationStore` so deployments may use different backends (Postgres for Conversations, S3/object-store for vCons).
- **Identity-level memory**: cross-Conversation patterns (preferred contact channel, language, tone) accumulate at the Identity level via consumer-provided storage. rvoip surfaces the per-Conversation evidence (vCons, Messages, Session metadata); the consumer interprets and writes back.

## 5. Capabilities — out of scope (the consumer owns these)

These belong above the voice plane, in whatever consumer is using rvoip. Thelve owns them in our reference deployment; a CPaaS owns them in a CPaaS deployment; a call center app owns them in a call center deployment.

- Worker registry, presence model, cross-modality availability.
- Queues, skills, SLAs, routing decisions, "who takes this call".
- Customer / Interaction identity. Cross-channel continuity.
- AI training, flywheel, hypothesis testing, playbook rollout.
- Tool access, MCP integrations, CRM reads/writes.
- Predictive dialer / campaign engine. Do-not-call enforcement. Outbound retry policy.
- Compliance announcements ("this call may be recorded") — the consumer plays them via the `PlayAudio` command.
- Voicemail business logic — rvoip provides the recording primitive; the consumer owns greetings, retention, transcription policy.
- Rate cards, invoicing, contract terms — rvoip emits raw usage; the consumer (Thelve, the CPaaS biller, etc.) handles billing (see §9).
- Fraud detection rules — rvoip emits anomaly signals; the consumer decides what to do.

If a feature is about *who* takes the call, *why*, *with what context*, or *what to learn from it*, it lives above rvoip — in Thelve, in the CPaaS, in the call center app. rvoip stays under that line.

## 6. Multi-tenancy (cross-cutting)

rvoip is multi-tenant first-class. Adding `tenant_id` later is brutally invasive; adding it now costs almost nothing.

- Every **command** carries `tenant_id`. Every **event** rvoip emits echoes the `tenant_id` of the call it relates to.
- Per-tenant **quotas**: max concurrent calls, max concurrent recordings, max concurrent AI runtime sessions. Layered on top of the global admission semaphore. A runaway tenant cannot starve another.
- Per-tenant **outbound caller-ID**: `OriginateConnection` must include the `from_uri` to use; rvoip enforces that the `from_uri` belongs to the calling tenant (via a tenant→numbers map provisioned by Thelve at startup).
- Per-tenant **provider configuration**: ASR/TTS/dialog providers are sourced from a **Thelve-owned provider registry** (see §11 for shape). The registry is the authoritative directory of which models a tenant has access to and how to reach them. rvoip never decides which provider is available; it just consumes the registry and pools connections.
- Per-tenant **recording sink**: the destination for recording bytes is configured per tenant.
- Logs, metrics, traces are tenant-tagged. Operational dashboards can filter by tenant out of the box.

What rvoip does **not** know about tenancy: business contracts, rate cards, ownership policies for numbers (a number's relationship to a tenant is just a configured map), tenant onboarding workflows. All Thelve.

## 7. Security model

Most of the security surface is delegated; rvoip's own commitments are narrow but specific.

### 7.1 Transport and signaling

- **SIP transport security**: TLS for SIP signaling and SRTP/DTLS-SRTP for media. Required in v1. Configured at sip-transport / dialog-core level; rvoip refuses to route unencrypted calls in production mode.
- **UCTP / WebTransport / WebSocket / QUIC substrates**: TLS 1.3 / QUIC TLS by default; substrates that cannot provide TLS are rejected.
- **STIR/SHAKEN**: caller-ID attestation is signed and verified at the sip-transport / dialog-core layer; rvoip passes verstat through to Thelve in events and embeds the PASSporT into the Session's vCon (`parties[].stir`). Thelve makes the trust decisions.

### 7.2 Identity & agent authentication

rvoip treats identity as a layered concern. Concrete shape lives in `INTERFACE_DESIGN.md` §8 (the `IdentityProvider` trait and `IdentityAssurance` enum); the PRD-level commitments are:

- **Identity assurance gradient.** Every authenticated participant in a Session carries an assurance level: `Anonymous`, `Pseudonymous`, `Identified`, `TaskScoped`, or `UserAuthorized`. Sessions and tenants may require a minimum assurance for join. Bridging two Connections of different assurance produces a Session whose effective assurance is the lower of the two; the vCon records both.
- **Multiple identity backends, pluggable.** v1 ships:
  - **OAuth 2.1 + DPoP** (production default for client/server flows).
  - **OIDC** (composes with OAuth via `openid-key-binding`).
  - **SIP Digest** (legacy SIP path; preserved for hybrid deployments).
  - **FIDO/passkeys** (anchors the user side of agent delegation).
  - **AAuth (`draft-hardt-oauth-aauth-protocol`) — experimental.** Hardt's emerging agent-to-agent protocol; per-agent keypair, [RFC 9421 HTTP Message Signatures](https://datatracker.ietf.org/doc/rfc9421/), no bearer tokens, identity gradient native. **Marked experimental in v1**: design accommodates it, public API does not commit to its current draft.
- **Per-request signing where applicable.** Substrates that carry HTTP-shaped messages (UCTP-over-WebTransport, UCTP-over-WebSocket, QUIC-over-h3) verify RFC 9421 signatures via `Signature`, `Signature-Input`, and Hardt's sister `Signature-Key` / `Signature-Agent` headers. Plain SIP and WebRTC peers default to `IdentityAssurance::Anonymous` unless they present an HTTP-mediated AAuth or OAuth surface.
- **AI worker identity.** AI workers (in-process or remote) authenticate as agent identities, not opaque service tokens. An AI agent's signing key is registered against its Identity; calls it makes carry signed requests verifiable by Thelve. The agent's identity, capabilities, and delegation chain are recorded in the vCon's `parties[]` and `attachments[]`.
- **Thelve→rvoip auth**: in-process — the boundary is a Rust function call. No auth needed at this seam in v1. If split out later, mTLS + signed commands.

### 7.3 vCon signing and encryption

- **Default signing**: every emitted vCon is JWS-signed by rvoip on behalf of the tenant. Tenant supplies signing key via the Provider Registry (§11). Multi-signer mode allows the AI provider, the recording pipeline, and the consumer to add signatures.
- **Optional encryption**: when a tenant publishes a JWE encryption key, rvoip wraps the signed vCon in JWE before persisting/delivering. Consumers without the key see only signed metadata (`uuid`, `created_at`, `parties[].name`) at the storage layer.
- **Redaction lineage**: redacted vCons carry a `redacted` reference back to their predecessor (per the vCon spec). The predecessor is encrypted-at-rest after redaction; access is policy-gated. rvoip preserves the chain — *what* gets redacted (PII, recording, attachments) is consumer policy.
- **Lawful intercept**: rvoip exposes the vCon primitives (signing keys, redaction operations, the `consent` extension attachment); the consumer's compliance layer applies the jurisdiction-specific policy.

### 7.4 DTLS-SRTP fingerprint binding (rvoip value-add, feature-flagged)

When an Identity has a registered signing key, the rvoip-sip and rvoip-webrtc adapters can derive a DTLS certificate fingerprint from that key and pin it into the DTLS-SRTP handshake — so a Connection's signaling-time identity is cryptographically bound to its media-time identity. v1 ships the design behind a feature flag (`identity-fingerprint-binding`); default off. Implementation detail in `INTERFACE_DESIGN.md` §8.

### 7.5 Other

- **AI provider credentials**: stored in the per-tenant `ProviderRegistry`, configured at startup by Thelve. Credentials never appear in commands — only `provider_id` references. rvoip does not log credentials.
- **Recording delivery**: bytes leave rvoip toward a Thelve-controlled `RecordingSink`. The sink must be addressed via a TLS endpoint or signed in-process callback. rvoip does not write recordings to local disk in production mode.
- **Fraud / abuse**: rvoip emits anomaly signals (high failure rates, abnormal post-dial delays, suspected spam patterns). Decisions and enforcement (rate-limiting a tenant, blocking a number) happen in Thelve.

## 8. Observability & quality

Quality data is training fuel for the flywheel; observability is operational table-stakes. rvoip emits both.

### Quality data (for the flywheel)
- A `SessionQualityReport` envelope is included in every `SessionEnded` event:
  - Media: MOS estimate, packet loss %, jitter, RTT, codec, bitrate, talkover %, silence %.
  - Telephony timing: post-dial delay (PDD), ring time, setup time, hangup reason.
  - AI quality (if AI was attached): per-turn ASR confidence histogram, ASR latency, TTS latency, dialog-turn latency, barge-in count, runtime/provider error counts.
- Transcript turns carry per-turn confidence and timestamps (already implied; codified here).

### Operational metrics
- OpenTelemetry tracing for every call (one span per call, child spans per Connection / per AI turn / per transfer).
- Prometheus-style metrics: active calls, calls per second, admission rejects, per-tenant gauges, provider error rates, runtime memory.
- Structured logs (tracing crate) with `tenant_id`, `session_id`, `connection_id` correlation.

The tracing/metrics framework choice is OpenTelemetry — consistent with the rest of the workspace. rvoip exports OTLP; deployers point it wherever.

## 9. Billing / usage hooks

Thelve owns billing. rvoip owns the **raw usage data**.

- A `UsageRecord` event family is emitted at end-of-call (and periodically for long-running calls — every 15 minutes, configurable):
  - Tenant, session_id, direction (inbound/outbound), call type (SIP↔SIP, SIP↔AI, 3-party).
  - Call minutes (signaled vs talk-time).
  - PSTN minutes (subset, since carrier costs differ).
  - AI usage: ASR audio-seconds processed, TTS characters synthesized, LLM token consumption (when reported by provider).
  - Recording bytes generated, recording duration.
  - Transfer count.
- `UsageRecord` events are on a separate event channel from operational events so billing pipelines can subscribe independently and the operational stream isn't filtered.
- rvoip is **the source of truth** for raw usage. Thelve combines this with rate cards to invoice. If they disagree, rvoip wins on raw usage.

## 10. The contract — north interface

Defined as Rust traits and serializable message types. Embedded today; the message types are designed to round-trip through serde so the boundary can later go over gRPC/NATS without redesign.

### Commands (Thelve → rvoip)
All commands carry `tenant_id` and `correlation_id`. Asynchronous; rvoip acks immediately and emits the result as events.

| Command | Purpose |
|---|---|
| **Conversation lifecycle** | |
| `OpenConversation` | Open an explicit Conversation with policy (Ephemeral/Persistent), tenant, metadata, and optional initial Participants |
| `CloseConversation` | Close a Conversation (rejects if active Sessions remain unless `force=true`) |
| `ListConversations` | Query Conversations a Participant or Identity is in |
| **Session lifecycle** | |
| `StartSession` | Explicitly start a Session in a Conversation; rvoip issues invites to the listed Participants |
| `EndSession` | End a Session (terminates all its Connections; the Conversation persists) |
| `JoinSession` | Add a Participant to an active Session |
| `LeaveSession` | Remove a Participant from an active Session (Session continues if other Participants remain) |
| **Connection lifecycle** | |
| `RouteInboundConnection` | Bind an inbound Connection (any transport — SIP INVITE, UCTP `connection.offer`, WebRTC offer) to a routing decision |
| `OriginateConnection` | Originate an outbound Connection on a chosen transport, optionally as part of an existing Session |
| `BridgeConnections` | Bridge two existing Connections (1:1 media; transport-agnostic; transcoding inserted automatically when codecs differ) |
| `UnbridgeConnections` | Tear down a bridge without ending the Connections |
| `TransferConnection` | Blind / attended / external transfer of a Connection |
| `EndConnection` | Terminate a Connection (Session may continue if other Connections remain) |
| `Hold` / `Resume` | Per-Connection hold control |
| `Mute` / `Unmute` | Per-Connection, per-direction mute control |
| `RenegotiateMedia` | Change codec or stream set on an existing Connection |
| **Media operations** | |
| `SendDtmf` | Play DTMF on a Connection |
| `PlayAudio` | Play audio onto a Connection from either an `audio_url` (HTTPS to wav/opus) or a `tts_request` (text + voice + format). Cancellable. |
| **AI / harness** | |
| `AttachAi` | Attach an in-process AI runtime to a Connection (worker mode) |
| `AttachListener` | Attach an AI listener to a Connection or bridge (3-party tap) |
| `Detach` | Remove an attached AI runtime or listener |
| **Recording / transcription** | |
| `StartRecording` / `StopRecording` | Toggle audio capture on a Connection or Session |
| `StartTranscription` / `StopTranscription` | Toggle ASR streaming |
| `PauseRecording` / `ResumeRecording` | PCI-style transient suppression |
| **Messaging** | |
| `SendMessage` | Send a Message in a Conversation (recipient(s), content_type, body, attachments) |
| `MarkMessageRead` | Send a read receipt for a Message |
| `ListMessages` | Query historical Messages in a Conversation (filtered, paginated) |

### Events (rvoip → Thelve)
All events carry `tenant_id`, `conversation_id` (where applicable), `session_id` (where applicable), `connection_id` (where applicable), `correlation_id`, and `timestamp` — matching `INTERFACE_DESIGN.md` §5.

| Event | Notes |
|---|---|
| **Conversation lifecycle** | |
| `ConversationOpened` / `ConversationClosed` | Conversation lifecycle |
| **Session lifecycle** | |
| `SessionStarted` | Session became Active (≥1 Connection Connected, all negotiation complete) |
| `SessionEnded` | Terminal event; carries `SessionQualityReport` envelope |
| `SessionFailed` | Terminal event with structured failure reason |
| `ParticipantJoined` / `ParticipantLeft` | Per-Session participant changes (with role, kind, joined-via-Connection) |
| **Connection lifecycle** | |
| `ConnectionInbound` | A new inbound Connection arrived on some adapter; awaits routing command |
| `ConnectionOutbound` | An outbound Connection started (in response to OriginateConnection) |
| `ConnectionConnected` | Connection reached `Connected` state |
| `ConnectionProgress` | Early-media states: ringing, busy, no-answer, machine, human-answered |
| `ConnectionEnded` | Per-Connection terminal event (Session may continue if other Connections live) |
| `ConnectionFailed` | Per-Connection failure |
| `ConnectionsBridged` / `ConnectionsUnbridged` | Bridge state changes |
| `ConnectionTransferred` | Transfer completed (with type and target) |
| **AI / harness** | |
| `AiAttached` / `AiDetached` | AI runtime attached/removed |
| `ListenerAttached` / `ListenerDetached` | 3-party listener attached/removed |
| **Media** | |
| `TranscriptTurn` | Per-turn ASR result with `stream_id`, `speaker`, `text`, `confidence`, `is_final` |
| `DtmfReceived` | DTMF capture event |
| `MediaQuality` | Periodic per-Connection quality snapshot (loss, jitter, RTT, MOS) |
| **Recording / vCon** | |
| `RecordingStarted` / `RecordingStopped` / `RecordingComplete` | Audio recording lifecycle, with sink reference |
| `VconReady` | A Session's vCon was finalized, signed, and persisted to `VconStore` (per `INTERFACE_DESIGN.md` §11.4); carries handle for retrieval |
| `VconRedacted` | A redacted vCon was produced from an existing one (per consumer-supplied redaction policy) |
| **Messaging** | |
| `MessageReceived` | Inbound Message in a Conversation |
| `MessageSent` | Outbound Message confirmed by substrate |
| `MessageDelivered` | Recipient's substrate received the Message |
| `MessageRead` | Recipient marked the Message as read (relayed to other Participants) |
| **Identity** | |
| `IdentityAssuranceChanged` | A Connection's `IdentityAssurance` level changed (e.g., step-up auth completed, delegation expired) |
| **Registration** | |
| `RegistrationChanged` | Normalized SIP registrar event (registered / expired / unregistered / contact-changed) — emitted only on material change per dedup rule §14.2 |
| `RegistrationHeartbeat` | Periodic liveness signal per AOR (default 5 min) so Thelve's presence model can detect dropouts without flooding on every SIP REGISTER refresh |
| **Operational** | |
| `CapacityReport` | Periodic snapshot of utilization, per-tenant and global |
| `UsageRecord` | Billing-grade raw usage (separate channel from operational events) |
| `Anomaly` | Quality / fraud signal for Thelve to evaluate |

### Backbone
The events bus is `infra-common::events::GlobalEventCoordinator` (already used by session-core). rvoip does not maintain its own bus.

## 11. AI voice harness — internals

The harness is the substantive code in this gateway, and applies **only to the in-process AI mode**. AI workers presented as SIP endpoints (or any other transport — UCTP-native, WebRTC, etc.) do not go through the harness — they are bridged like any other Connection, and their ASR/TTS/dialog logic lives in the remote AI service.

### 11.0 Harness ↔ vCon integration

The harness writes into the in-flight Session's vCon as the call progresses:

- Each AI dialog turn (caller utterance → AI response) becomes one or more `analysis[]` entries with `type=transcript` (per-turn ASR with confidence) and `type=summary` or vendor-specific schema for the AI-side reasoning trace.
- The dialog provider's identity (which model, which version, which provider) is recorded as a `parties[]` entry with `kind=ai` and the agent's signing key in `validation`.
- Per-turn ASR confidence and dialog-turn latency from §8.x feed both `Anomaly` events and the vCon's per-turn analysis entries.

This makes the vCon the canonical AI training/audit record for every AI-handled Session, without a separate metadata pipeline.

### 11.1 Provider trait shapes

- **`AsrProvider` / `AsrSession`** — start session, push audio frames, receive transcript events (partials and finals), finish/cancel.
- **`TtsProvider` / `TtsStream`** — synthesize a request to an audio frame stream; cancellable.
- **`DialogManager`** — `start_call`, `on_transcript(turn) -> DialogTurn`, `on_dtmf(digit) -> DialogTurn`, `end_call`. Thelve implements this; the implementation may itself be a streaming proxy to a remote LLM service.
- **`RecordingSink`** — `start_recording(session, connection)`, `write_audio(frame)`, `stop_recording()`. Sink is provided per-tenant.

### 11.2 Provider registry — Thelve owns, rvoip consumes

Thelve maintains the authoritative **provider registry**. Each entry describes a model rvoip can reach:

```
ProviderEntry {
    provider_id,
    kind: Asr | Tts | Dialog,
    locality: InProcess | Remote,
    transport: <see below>,
    endpoint,
    auth_ref,                  // reference into per-tenant credentials, never the secret
    capabilities,              // languages, formats, sample rates, voice catalog, etc.
    health: Healthy | Degraded | Down,  // updated by Thelve
}
```

**Transport for remote providers** is **deferred** — the candidate set is HTTP/REST, gRPC streaming, QUIC, and WebRTC; the right choice differs by kind (e.g., gRPC streaming is natural for ASR audio frames; HTTP works for a TTS request returning a stream; WebRTC for browser-resident endpoints). The trait surface stays transport-agnostic so we can evolve without churning the harness. We decide per-provider type during implementation.

### 11.3 Per-call provider selection — preferred / assigned / available

For each call's commands (`AttachAi`, `StartTranscription`, etc.), Thelve specifies:

- **Available**: the provider pool the tenant has access to (sourced from the registry; usually implicit).
- **Preferred**: an ordered list of `provider_id`s rvoip should try first.
- **Assigned** *(event field, not command)*: the provider rvoip actually used. Reported on each event (`TranscriptTurn` includes `assigned_asr_provider`, `RecordingComplete` includes the recording sink, etc.). Used for billing and observability.

Failover: if the preferred provider fails (start error, mid-stream stall past timeout), rvoip falls through to the next entry on the preferred list, then the rest of available. An `Anomaly` event is emitted with the failure reason and the failover action. If no provider succeeds, the call fails with a structured reason.

### 11.4 Pooling

rvoip pools connections to providers. Pool semantics differ by locality:

- **In-process (linked or FFI)** — pool is a per-provider object pool with a per-runtime concurrency budget (open question §14 #4).
- **Remote** — pool is a per-provider connection pool (e.g., gRPC channel pool). Pool sizing per provider is configurable; default to a small floor with elastic growth.

The exact pooling and connection-management mechanism is **deferred** to implementation; what the PRD nails down is that rvoip is responsible for pooling (Thelve does not manage transport state).

### 11.5 Reflex policy

- Sub-200ms reflexes (barge-in detection, filler insertion) are in-process. They cannot tolerate a network hop.
- Per-turn dialog decisions tolerate 500ms–2s and are allowed to be remote.

### 11.6 Failure modes

- ASR provider stalls → fall through to next preferred (per §11.3), then to dialog manager with a stale-transcript signal.
- TTS provider stalls → cut and apologize / replay last understood turn; emit `Anomaly` and try next preferred.
- Dialog manager stalls → after a configurable timeout, play a polite filler and continue waiting; if exceeded, escalate.

## 12. Non-functional requirements

- **Concurrent scale (v1):** 1k–5k concurrent calls per single rvoip instance.
- **Architectural runway:** 10k+ concurrent calls per instance with lock-free / DashMap / atomic-state work on the call-leg, bridge, and runtime-session storage. (The prior `CONCURRENCY_PLAN.md` covers this scope but extends to subsystems that this PRD removes; treat that document as historical context, not the implementation plan.)
- **Per-tenant scale:** any single tenant can use up to its quota; quotas summed across tenants can exceed instance capacity (oversubscription is allowed).
- **Latency budgets:**
  - Inbound INVITE → `ConnectionInbound` event: < 50ms p99.
  - `RouteInboundConnection` command → 200 OK on the wire: < 100ms p99 (excluding any Thelve thinking time).
  - Outbound `OriginateConnection` → `ConnectionProgress` (early media): bounded by carrier; rvoip overhead < 20ms p99.
  - Caller speech → `TranscriptTurn` final emitted: < 1s p95 (provider permitting).
  - Barge-in detection → TTS cut: < 200ms p95 (in-process).
- **Embedded model:** single binary, single Rust process. No required network components beyond the SIP/PSTN edge.
- **Backwards compatibility:** none promised pre-v1.

## 13. Migration from current code

The current `orchestration-core` crate has a lot of code that, under this PRD, doesn't belong in its place. The migration is subtractive (lift workforce concerns to the consumer) plus a structural reshape into the three-layer crate layout in `INTERFACE_DESIGN.md` §2.

**New crate layout** (`INTERFACE_DESIGN.md` §2 is authoritative; summary here):

- `rvoip-core` — neutral abstractions (Conversation, Session, Connection, Stream, Message, Participant, ConnectionAdapter trait, IdentityProvider trait, MediaStream trait, Orchestrator entry).
- `rvoip-uctp` — Universal Conversation Transport Protocol wire implementation (envelope encode/decode, substrate framing, capability negotiation algorithm).
- `rvoip-quic`, `rvoip-webtransport`, `rvoip-websocket` — UCTP substrate adapters.
- `rvoip-sip` — SIP interop adapter (absorbs current `session-core`, `dialog-core` integration, `sip-transport` use, registrar-core consumption).
- `rvoip-webrtc` — WebRTC interop adapter.
- `rvoip-media` — transport-agnostic media (codec, mixing, audio processing, transcoding pairs).
- `rvoip-rtp` — RTP/SRTP-specific transport (used by rvoip-sip and rvoip-webrtc).
- `rvoip-harness` — AI voice harness, extracted from `orchestration-core::voice_ai` so SIP-only carriers don't pull provider deps.
- `rvoip` — facade crate; feature flags `[uctp, sip, webrtc, client, harness, full]` select what compiles in.

**Stays (relocated; gets the concurrency rework):**
- `voice_ai.rs` traits and types → `rvoip-harness`.
- `UnifiedCoordinator`, dialog/media wiring → `rvoip-sip` (with renamed types).
- Bridge management, call lifecycle, transfer mechanics → split: SIP-specific bits to `rvoip-sip`; transport-agnostic `BridgeConnections` to `rvoip-core` (per `INTERFACE_DESIGN.md` §10).
- Store layer for Connections and runtime sessions → `rvoip-core` (DashMap + atomic state, narrow scope).
- registrar-core consumption for `RegistrationChanged` events → `rvoip-sip` emits; `rvoip-core` normalizes into the cross-substrate event vocabulary.

**Renames** (vocabulary alignment with voip-3):
- `Call` → `Conversation` (with the new `Session` layer between Conversation and Connection).
- `CallLeg` → `Connection`.
- `MediaSession` (in media-core) stays as an internal concept; public surface uses the `MediaStream` trait per `INTERFACE_DESIGN.md` §3.6.

**Lifts up to the consumer (deletions in rvoip):**

In our reference deployment the consumer is Thelve; in a CPaaS or call center deployment it is the consumer's own application. Either way these concerns leave rvoip.

- `Agent`, `AgentStore`, `AgentKind`, `AgentConnector`, `AgentOffer`, `AgentOfferStore`.
- `Queue`, `QueueStore`, `QueueWaitlist`, queue policies, queue admission, queue ordering, overflow policies.
- `AssignmentManager` and the entire matching loop.
- `Router` / `RouteRequest` / `RouteDecision` (the *call-routing* trait — superseded by command-driven routing from the consumer).
- `QueueSelector` and all selector implementations.
- `ContactResolver` if the consumer does its own resolution; otherwise stays as a small SIP-URI resolver inside `rvoip-sip`.

**Reshapes:**
- Examples and tests are rewritten against the new command/event surface. Examples cover the developer profiles in §1.1: a minimal voice AI service, a minimal call-center driver, a Thelve-shaped integration, a UCTP-only application server, and a SIP↔UCTP bridge. The queue-management examples that lived in the old orchestration-core go.

**Crate rename:**
- `orchestration-core` is a misnomer once the workforce/queue/agent concerns leave. The post-migration name is **`rvoip-core`** for the neutral substrate and **`rvoip`** for the facade. Defer this rename to land alongside the structural moves so the noise is contained to one cutover, not spread across multiple commits.

**Migration order** (avoids a long dual-architecture window; per `INTERFACE_DESIGN.md` §13.3):
1. Create `rvoip-core` skeleton with types and traits (no implementations).
2. Carve `rvoip-sip` out of `session-core` + the SIP-coupled parts of `orchestration-core`. Implement `ConnectionAdapter` for SIP. **At this point rvoip-core is real; the SIP path uses it.**
3. Create `rvoip-uctp` (UCTP envelope encode/decode).
4. Create `rvoip-quic` and `rvoip-webtransport` substrate adapters. **UCTP-native flows work.**
5. Create `rvoip-webrtc`. **The unified gateway is feature-complete for v1.**
6. Extract `rvoip-harness` from `voice_ai.rs`.
7. Delete agent/queue/router code from `orchestration-core`; relocate examples; rename `orchestration-core` → `rvoip` facade.

Each step ships behind cargo features so existing rvoip-orchestration-core deployments keep working until the migration completes.

## 14. Open questions and resolved decisions

### 14.1 Resolved (recorded here for traceability)

1. **Codec policy.** Thelve sets the available codec list per tenant; rvoip negotiates within that constraint. The active codec ends up in `SessionQualityReport`.
2. **Attended-transfer hold audio.** Thelve passes an audio source to play during the held leg via the new `PlayAudio` command (see §10). The source is either an `audio_url` (HTTPS to wav/opus) or a `tts_request`. There is no widely-adopted SIP/WebRTC standard for this — RFC 4240 ("Basic Network Media Services with SIP") defines a `play` SIP service URI but is not broadly deployed; modern stacks use vendor-specific media-server APIs. Our `PlayAudio` primitive is intentionally simpler than RFC 4240 and works the same for SIP and (future) WebRTC legs.
3. **Provider failover.** Replaces the prior "fail and report" default. Thelve sends preferred provider lists per call; rvoip falls through to the next entry on failure, emits an `Anomaly` event each time, and only fails the call when all preferred + available entries are exhausted. (See §11.3.)
4. **`UsageRecord` for incomplete calls.** No. If a call fails before media flows, no `UsageRecord` is emitted. (Failure is reported on `SessionFailed` for operational and quality purposes — that's enough.)
5. **AI-as-SIP-endpoint registration.** Goes through registrar-core like a human. Skill profile and worker type (AI vs human) live in Thelve; rvoip stays out of skills entirely. The `RegistrationChanged` event carries only SIP-layer facts (AOR, contact, status); Thelve correlates AOR → worker_id → skill profile → kind.

6. **UCTP, QUIC/WebTransport, and WebRTC bridging architecture** *(was §14.2 item 5).* Decision: option **(a) — native bridging inside rvoip**, restructured into the three-layer architecture (§0). rvoip implements UCTP server-side natively over QUIC, WebTransport, and WebSocket substrates (new `rvoip-quic`, `rvoip-webtransport`, `rvoip-websocket` adapter crates). It bridges to SIP and WebRTC at the gateway boundary via interop adapters (`rvoip-sip`, `rvoip-webrtc`). All Connections — substrate-native or interop — appear in the same Session abstraction (per `INTERFACE_DESIGN.md` §3) and can be bridged to each other in 1:1 pairings with optional transcoding (§9 of INTERFACE_DESIGN). Sidecar (option b) and consumer-arranged bridging (option c) are rejected: they push protocol-bridging glue into Thelve, defeating the unified-library goal. Multi-party (>2-party) media is explicitly out of v1; an SFU adapter is v2 work.

7. **vCons as the canonical conversation envelope.** rvoip emits a [vCon](https://datatracker.ietf.org/doc/draft-ietf-vcon-vcon-core/) at end-of-Session for every Session, regardless of audio recording. This replaces ad-hoc "audio file + sidecar JSON + DB metadata" patterns with a signed, addressable, redaction-aware container that is the IETF standard direction for conversational data. JWS-signed by default; JWE-encrypted when tenant key is configured. See §4 (recording), §7.3 (signing/encryption), §11 (harness integration). rvoip ships the first Rust implementation of the vCon spec as `rvoip-vcon` (per `INTERFACE_DESIGN.md` §2).

8. **Identity backends are pluggable behind a trait; identity gradient is canonical.** The `IdentityProvider` trait (per `INTERFACE_DESIGN.md` §8) supports OAuth 2.1+DPoP, OIDC, SIP Digest, FIDO/passkeys, and AAuth. The **identity gradient** (Anonymous → Pseudonymous → Identified → TaskScoped → UserAuthorized) is the public-facing concept; backends translate their primitives into it. AAuth is **experimental in v1**: the trait accommodates it, but rvoip does not commit to AAuth's draft as the canonical agent-identity protocol until the IETF status stabilizes.

### 14.2 Still open

1. **Provider transport.** Per §11.2 the transport for remote providers (HTTP, gRPC streaming, QUIC, WebRTC) is to be decided per provider type during implementation. Likely outcome: gRPC streaming for ASR, HTTP for TTS request/stream, transport-flexible for DialogManager. Decide once we have concrete provider implementations to test.
2. **Provider connection pooling mechanism.** Pool semantics and sizing per provider locality are deferred. rvoip owns the responsibility; the implementation choice (custom pool vs library, eager vs lazy connection, health-check cadence) is open.
3. **In-process AI runtime concurrency budget.** Within a single in-process AI runtime, how many concurrent calls can it serve? Needs measurement against real LLM/ASR providers before we pick a default.
4. **Anomaly taxonomy.** What specific anomalies does rvoip emit (vs. Thelve detecting from raw quality data)? Needs a small enumeration — probably: provider failover, abnormal post-dial delay, abnormal hangup pattern, abnormal silence/talkover ratio, repeated re-INVITE flapping.
5. *(Resolved — moved to §14.1 item 6.)*

7. **vCon assembly cadence.** Immediate-on-BYE (commit synchronously, may delay the BYE response) vs. async-batched (commit shortly after, with a placeholder `vcon_ref` returned in `RecordingComplete`). Lean: async-batched with a 5-second SLA, surfaced as a separate `VconReady` event when committed. Decide once we measure JWS signing cost at scale.

8. **vCon storage.** rvoip ships an in-memory default `VconStore` for tests; production needs S3/Postgres. Open: does rvoip ship a reference Postgres implementation, or is it strictly consumer-provided? Lean: ship a reference Postgres impl as an optional crate (`rvoip-vcon-postgres`) so small deployments don't need to build their own.

9. **vCon redaction policy boundary.** rvoip-core surfaces the redaction primitive (produce a redacted vCon with the `redacted` linkage); the consumer's compliance layer decides *what* fields to redact. Open: should rvoip ship a default redactor that masks PII via vendor-specified patterns, or remain redaction-agnostic? Lean: agnostic; consumers integrate their existing PII-detection.

10. **AAuth conformance commitment.** When (if ever) does rvoip's AAuth backend stop being labeled experimental? Lean: only when AAuth is WG-adopted and reaches Last Call. Until then it stays experimental in public docs and is gated behind a `aauth-experimental` feature flag in `rvoip-identity`.

11. **`RegistrationChanged` deduplication.** registrar-core sees a SIP REGISTER every refresh interval (typical 60–3600s) for every registered phone. The "Alice is online with contact X" fact does not change between refreshes. The debounce policy must distinguish material change from refresh noise. Proposed rule:

   Emit `RegistrationChanged` if and only if **any** of the following differs from the most recent event for the same AOR:
   - status (registered / expired / unregistered)
   - any contact URI (added, removed, changed)
   - transport (UDP/TCP/TLS/WSS)
   - device flag (`+sip.instance`, `reg-id` from RFC 5626)
   - path set
   - explicit unregister (Expires: 0)

   Refreshes that only change the expiry timestamp do not emit. A periodic `RegistrationHeartbeat` event (e.g., every 5 minutes per AOR) tells Thelve "I'm still seeing this AOR alive" without flooding on every REGISTER. Thelve's presence model uses heartbeat absence (after grace period) as a separate signal from explicit `RegistrationChanged status=expired`.

   Open: the heartbeat cadence and grace period are tunable; default proposed at 5 min heartbeat, 2× heartbeat grace.

## 15. Relationship to the prior CONCURRENCY_PLAN.md

The prior `CONCURRENCY_PLAN.md` (in this same directory) was written before this PRD's scope was settled. It addressed scaling the *whole* current orchestration-core, including the agent / queue / offer / routing subsystems that this PRD removes from rvoip's scope.

That document is **kept as historical reference**, not revised. When implementation work begins under this PRD, a fresh, narrower concurrency plan will be authored alongside (focused on call-leg / bridge / runtime-session storage, GlobalEventCoordinator adoption, atomic state, and admission semaphore — the parts of the prior plan that survive the scope cut). The prior plan is useful for understanding the in-tree surface as it exists today; the new PRD is the source of truth for what rvoip will become.

---

**Reviewers:** please mark up §3 (use cases), §4 (in scope), §5 (out of scope), §10 (contract), and §13 (migration) — these are the load-bearing sections. §6–§9 (cross-cutting concerns) are the most likely to need refinement based on how Thelve is actually wired internally.
