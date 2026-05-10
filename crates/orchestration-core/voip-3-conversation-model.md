# The Conversation Model

**A Unified Communication Vocabulary for VoIP 3.0**

Editor: Jonathan Eisenzopf, Rudeless / Thelve
Status: Working draft, May 2026
Version: 0.1

---

## Status of This Document

This is a working draft of a vocabulary and conceptual model for unifying voice, video, text, and multi-modal real-time communication across the SIP, WebRTC, voice-AI, IVR, contact-center, and web-chat communities. It is intended to inform the architecture of the rvoip library and the Thelve platform, but its scope is broader than either: any system that needs to describe modern multi-channel real-time communication should be able to use this vocabulary directly or map to it without translation overhead.

This document is a starting point, not a finished specification. Comments, corrections, and extensions are invited.

---

## Abstract

Real-time voice and video communication has fragmented into multiple parallel ecosystems, each with its own protocols, vocabularies, and developer communities. SIP and the broader VoIP industry built one. WebRTC built another. Voice AI is building a third on top of both. IETF work on SIP-over-QUIC and RTP-over-QUIC is foreshadowing a fourth. Each ecosystem solves real problems and serves real customers, but the boundaries between them are crossed today only via gateways and translation layers. This document proposes a shared vocabulary — six primary nouns and a small set of verbs — that maps cleanly to the existing terminology of each community while providing a basis for a unified library and platform implementation. We refer to the convergence informally as *VoIP 3.0*.

---

## 1. Preamble

I worked on the W3C VoiceXML standard in the early 2000s. The intent of that work was, in retrospect, the same as the intent of this document: to bring the web and voice communities together so that voice applications could be built and deployed using the same patterns, vocabulary, and developer ergonomics that the web had popularized.

VoiceXML succeeded as an enterprise standard. Every major IVR vendor implemented it. Voice browsers from Tellme, Voxeo, BeVocal, Nuance, and IBM shipped at carrier scale. Hundreds of thousands of voice applications were written and deployed. By any measure of technical adoption inside the contact-center industry, it was a hit.

But it failed at the part that mattered most. The web and mobile development communities did not show up. Voice apps stayed inside the contact center. JavaScript developers did not write voice applications. When the iPhone shipped in 2007 and the App Store opened in 2008, mobile developers built voice into their apps using proprietary VoIP stacks or simply ignored voice entirely. The W3C voice browser working group's vision of a parallel "voice web," fetched and rendered like the HTML web, did not materialize. Voice fell back to the telephony stack.

WebRTC arrived in 2011 and explicitly distanced itself from SIP and VoiceXML. It got the web developers we had failed to attract. It also got the same fragmentation problem in reverse: WebRTC did not natively interoperate with the SIP world, and a generation of gateways grew up to translate between the two — Janus, mediasoup with SIP plugins, Asterisk's WebRTC support, Kamailio with RTPEngine, Jigasi for Jitsi. The bridge solved the immediate problem but reinforced the underlying split.

Today, fifteen years later, we have three real-time voice ecosystems — SIP, WebRTC, and an emerging QUIC-based wave — plus a fourth community of voice-AI builders sitting on top, struggling to compose all of the above into agents that work the way users actually expect. The contact-center industry, the web/mobile developer community, the AI agent builders, and the standards bodies are still working in their own silos and translating across boundaries when they have to.

I do not believe the answer is to force any of these communities to abandon their existing tools, conventions, or vocabularies. SIP works. WebRTC works. The IVR industry's investment in VoiceXML and contact-center routing was not wasted. The right move now is more modest and more durable: provide a shared vocabulary that maps cleanly into each community's native terms, build a unified library substrate underneath that lets a developer reach for whichever transport fits their problem, and let the communities continue doing their work while making it dramatically easier to collaborate across boundaries.

The work that is now possible because of LLMs, real-time speech models, mature WebRTC, and emerging QUIC media transport is the most exciting voice work I have ever seen. To realize it, we need a common language to describe what we are building, what each piece does, and how the pieces fit together. This document is a draft of that language.

I am calling the convergence VoIP 3.0 because it is the third generation of voice over IP — after the SIP era (1.0) and the web-meets-voice era of VoiceXML and WebRTC (2.0) — and because the version-number shorthand is honest about where we are in the arc. The number is not precious. The convergence is.

— Jonathan Eisenzopf

---

## 2. Goals and Non-Goals

### 2.1 Goals

This document aims to:

- Define a small set of unambiguous, cross-community nouns and verbs for describing real-time voice, video, text, and multi-modal communication.
- Provide an explicit mapping from these terms into the native vocabulary of the SIP, WebRTC, voice-AI, IVR, contact-center, and web-chat communities.
- Establish a foundation on which the rvoip library and the Thelve platform can express their architecture without inventing parochial dialect or forcing any community to relearn its own terminology.
- Support the long-term goal of a unified real-time communication library that treats SIP, WebRTC, and QUIC-based transports as peer transports rather than primary-and-bolt-on.

### 2.2 Non-Goals

This document does not:

- Replace any existing protocol, RFC, or standards-body specification. SIP, RTP, SRTP, ICE, DTLS-SRTP, WebRTC, MoQ, RoQ, and related specifications remain authoritative for their respective layers.
- Prescribe an implementation. The vocabulary is descriptive and architectural; concrete library APIs may name things differently in their public surface as long as the conceptual model holds.
- Dictate vocabulary inside any single community's existing tooling. SIP folks should keep saying "dialog" and "leg" inside SIP code. WebRTC folks should keep saying "peer connection" and "track" inside WebRTC code. The shared vocabulary is for the cross-community conversation and the unifying interface, not for replacing internal terms.
- Define billing, recording, lawful intercept, multi-tenancy, or autonomy semantics. These are addressed in companion documents.

---

## 3. The Six Primary Nouns

The model uses six primary nouns and intentionally no more. Each is defined here, with its scope and what it does *not* mean.

### 3.1 Conversation

A `Conversation` is the durable, cross-channel, cross-time record of a relationship or interaction between one or more `Participants`. It is the longest-lived business object in the model. A Conversation persists across channel changes (SMS to voice, voice to video, voice to text), participant changes (AI hands off to human), and gaps in time (today's call and tomorrow's follow-up SMS).

A Conversation contains:

- Zero or more `Messages` (asynchronous atomic communication events)
- Zero or more `Sessions` (synchronous bounded engagements)
- A `Participant` list, possibly evolving over time
- A timeline of events
- Memory and accumulated context

A Conversation is *not* a single call, a single chat session, a CRM record, or a ticket. A ticket or case may reference a Conversation, but the Conversation is the record of the actual communication itself.

### 3.2 Session

A `Session` is a synchronous, bounded engagement within a Conversation. A voice call is a Session. A video hangout is a Session. A live web chat with active typing indicators and presence is a Session. A screen-share is a Session.

Each Session has:

- A `kind` or `medium` (voice, video, text-chat, screen-share, mixed)
- A start time and an end time
- A list of `Participants` actively present in the Session
- A list of `Connections` (one per Participant per attached transport)
- A list of `Streams` (the media flowing within the Session)

A Session ends when the engagement ends. The Conversation persists. A new Session of the same or a different kind may begin in the same Conversation an hour later, a day later, or a year later.

A Session is *not* a Conversation, a Connection, or a Stream. It is the engagement object that connects Participants in real time and contains the media plumbing.

### 3.3 Message

A `Message` is an asynchronous, atomic communication event within a Conversation. An SMS, a chat message, an email, a voicemail drop, an asynchronous voice memo, an image, or a file transfer is a Message. A Message has a sender, an intended audience, a medium (text, image, audio, video, file), and a timestamp.

Messages are not Sessions. The distinction is synchronous vs asynchronous: a Session is a bounded continuous engagement during which Participants are actively present together; a Message is an atomic event that does not require simultaneous presence.

A live web chat is a Session of medium "text" because the participants are actively present together. SMS is a series of Messages because there is no expectation of continuous presence.

### 3.4 Participant

A `Participant` is an entity with identity present in a Conversation. A Participant has:

- A `kind` describing what type of entity it is, typically one of: `human`, `ai`, `system`, `external`.
- A `role` describing what it is doing in this Conversation, typically one of: `customer`, `agent`, `supervisor`, `observer`, or others as defined by the application.

The Participant supertype lets the model treat human and AI workers, customers, supervisors, recording bots, and external systems as peers in the architecture. Their kinds and roles distinguish what they are and what they do, but their structural place in the model is identical.

A Participant may join, leave, take over, or hand off across the lifetime of a Conversation. A Participant may be present in multiple Sessions concurrently, on multiple Devices, via multiple Connections.

### 3.5 Connection

A `Connection` is a single Participant's transport-level binding into a single Session. It is the plumbing — what SIP calls a *leg*, what WebRTC calls a *peer connection*, what QUIC-based transports call a *connection*.

A Connection has:

- A `transport` (SIP/RTP, WebRTC/DTLS-SRTP/ICE, QUIC, or other)
- Connectivity state (ICE candidate pair if applicable)
- Security state (DTLS-SRTP keys, SDES keys, QUIC TLS state, or none)
- A list of `Streams` flowing on it
- Per-endpoint state (mute, hold, codec, network quality)

One Participant may have multiple Connections to a single Session — for example, when joining from both a phone and a laptop simultaneously, or when one device carries audio and another carries video. One Connection carries one or more Streams.

A Connection is not a Session and not a Stream. It is the transport context for one Participant's attachment.

### 3.6 Stream

A `Stream` is a single media flow on a Connection. An audio stream, a video stream, a screen-share stream, or a data stream is a Stream. Each Stream has:

- A `kind` (audio, video, screenshare, data)
- A codec
- A direction (`send`, `receive`, `sendrecv`)
- An on-the-wire implementation, typically RTP over the Connection's transport, or SCTP-over-DTLS for non-media streams

The term Stream aligns with RFC 3550 (which uses "RTP stream") and with colloquial industry usage ("audio stream," "video stream," "live stream"). It corresponds approximately to the W3C WebRTC `MediaStreamTrack` concept; this model does not use a separate container concept analogous to W3C `MediaStream`, because BUNDLE and RTCP-mux render the container largely vestigial in modern usage.

A Stream is not a Connection and not a Session. It is the media flow itself.

---

## 4. Supporting Concepts

The six primary nouns above are sufficient for most architectural discussion. A small number of supporting concepts appear regularly in implementation and documentation.

### 4.1 Identity

An `Identity` is the durable real-world entity that a Participant points to. The same Identity may appear as a Participant in many Conversations over time. Identities are persistent; Participants are per-Conversation appearances of an Identity.

In simple implementations, Identity and Participant may be modeled as one object. More sophisticated systems separate them so that long-term behavior, memory, and authentication can attach to Identity while per-Conversation role, presence, and state attach to Participant.

### 4.2 Device

A `Device` is a physical or software endpoint a Participant uses to connect: a phone, a laptop browser, an embedded speaker, a mobile app, a SIP hard-phone, an in-vehicle endpoint. Devices have capabilities (microphone, camera, screen, codec support, polling/push behavior) and presence. A Participant may have multiple Devices and may use different Devices in different Sessions, or even in the same Session simultaneously — for instance, audio on a phone Connection and video on a laptop Connection within the same Session.

### 4.3 Workforce

A `Workforce` is the pool of Identities eligible to take the `agent` role in Conversations. Both human Identities and AI Identities can belong to a Workforce. A Workforce has skills, capacity, presence, permissions, and assignment policy. Routing assigns a Conversation (or a specific Session within a Conversation) to a Workforce member.

### 4.4 Track (note on usage)

This model does not use the term "Track" as a primary noun. The term is reserved for situations where mapping to W3C WebRTC `MediaStreamTrack` is being discussed. In all general use, a single media flow on a Connection is a `Stream`.

### 4.5 Channel (note on usage)

This model demotes "channel" from a primary noun to an informal property. Channel terminology is associated with the failed "omnichannel" architectural pattern, and in WebRTC the word has specific meanings (data channel, media channel) that we do not wish to overload. There is no `Channel` object in this model. A Session has a `kind` (or `medium`); we may informally describe a Session as "the voice channel" but the architectural object is the Session.

### 4.6 Role and Kind

A Participant has both a `kind` and a `role`. The `kind` is what type of entity the Participant is. The `role` is what the Participant is doing in this specific Conversation.

The same Identity may have different roles in different Conversations: a support engineer (kind: human) acts in role `agent` when helping a customer, and may act in role `customer` when calling a vendor. An LLM (kind: ai) may act in role `agent` in a customer support Conversation and in role `observer` in a coaching Conversation.

The word "agent" in conversational usage continues to mean "a Participant whose role is `agent`," which reads naturally to both contact-center and AI audiences.

---

## 5. The Verbs

A small set of verbs covers the lifecycle and dynamics of the model.

### 5.1 Conversation Verbs

A Conversation **opens**, **continues**, **resumes**, and **closes**. The first Message or first Session opens a Conversation. Activity on an existing Conversation continues it. Activity on a Conversation after a quiet period resumes it. A Conversation closes when the relationship is concluded.

### 5.2 Session Verbs

A Session **starts**, **ends**, and **upgrades**. An "upgrade" is a change of kind: a text Session that adds a voice stream, a voice Session that adds video. (Equivalently, this can be modeled as the existing Session ending and a new Session starting; both are valid implementation choices, and this document does not prescribe one.)

### 5.3 Participant Verbs

A Participant **joins**, **leaves**, **takes over**, and **hands off**. A handoff is a change of which Participant has the active `agent` role within a Conversation; it does not necessarily end any Session. An AI agent handing off to a human on a live voice Session is a handoff with no Session change.

### 5.4 Connection and Stream Verbs

A Connection **establishes**, **negotiates**, **renegotiates**, and **terminates**. A Stream **opens**, **flows**, **pauses**, and **closes**.

---

## 6. How They Work Together

The model has a clear containment and lifecycle structure.

```
Conversation
  ├── Messages          (async atomic events)
  ├── Sessions          (sync bounded engagements)
  │     ├── Participants present in this Session
  │     ├── Connections (one or more per Participant per transport)
  │     │     └── Streams  (audio, video, screenshare, data)
  │     └── lifecycle (start, end, upgrade)
  └── Participants in the Conversation
        └── Identity  (durable across Conversations)
              └── Devices
```

### 6.1 Lifecycle

1. A Conversation opens when the first communication event occurs.
2. Initial Participants are recorded; their Identities are linked.
3. As activity proceeds, Messages accumulate in the Conversation timeline.
4. When a synchronous engagement is needed, a Session starts.
5. Participants join the Session; each joining Participant establishes one or more Connections via available transports.
6. Each Connection negotiates and opens Streams of media flowing between the Participant's endpoints and the rest of the Session.
7. Streams flow until the Connection terminates or the Session ends.
8. Sessions end. The Conversation persists.
9. Further Messages and further Sessions may continue to be added to the Conversation.
10. The Conversation closes when the relationship concludes, or remains open indefinitely if the relationship is ongoing.

### 6.2 State and Durability

- Conversation state is durable and persists across system restarts and time gaps.
- Session state is durable for the duration of the Session and is preserved as a historical record afterwards.
- Connection state is real-time and ephemeral.
- Stream state is real-time and ephemeral.
- Memory and learned context attach at the Conversation level, and through Identity, at longer-lived levels for cross-Conversation patterns.

### 6.3 Asymmetry and Heterogeneity

The model accommodates asymmetric Participant configurations within a single Session:

- One Participant may have audio and video Streams while another has audio only.
- One Participant may attend via a SIP Connection while another attends via WebRTC and a third via QUIC. The Session abstracts the differences; codec compatibility is negotiated at Connection setup.
- A single Participant may have multiple Connections at once — for example, an audio Connection on a phone and a video Connection on a laptop, both attached to the same Session.
- A Participant may transfer across Devices mid-Session: the old Connection terminates, a new Connection establishes from the new Device, the Participant identity persists.

These cases are not edge cases. They are the normal mode of operation in modern real-time communication, and the model is shaped to handle them as first-class behaviors.

---

## 7. Mapping to Existing Communities

The vocabulary is designed to map without translation overhead into each existing community's native terminology. The following mappings are not lossy: the structural concepts are preserved across the rename.

### 7.1 SIP / VoIP / RTP

| This model | SIP / VoIP / RTP |
| --- | --- |
| Conversation | The customer engagement, often spanning many calls; tracked in CRM today |
| Session | A SIP dialog; the call itself |
| Message | SMS, MMS, voicemail, SIP MESSAGE |
| Participant | A SIP user agent (UA) participating in the dialog |
| Connection | A SIP leg (B2BUA leg or a Participant's UA-to-bridge connection) |
| Stream | An RTP stream (RFC 3550) |

### 7.2 WebRTC

| This model | WebRTC (W3C / IETF) |
| --- | --- |
| Conversation | The user-facing relationship; not a primary WebRTC concept |
| Session | The room or meeting; an SFU room in mediasoup, LiveKit, Janus, Daily |
| Message | Out-of-band signaling messages or DataChannel messages |
| Participant | A peer participant in the SFU/room (same word) |
| Connection | An RTCPeerConnection |
| Stream | A `MediaStreamTrack`; media flowing on the peer connection |

### 7.3 Voice AI / LLM Agents

| This model | Voice AI / LLM Agents |
| --- | --- |
| Conversation | The conversation with the assistant (same word) |
| Session | A voice or chat session with the agent |
| Message | A turn in chat-only contexts; a system event in voice contexts |
| Participant | The user, the AI agent, possibly tools or other agents |
| Connection | The audio/video transport — usually WebRTC, sometimes SIP for PSTN |
| Stream | The audio stream from user to model, model TTS to user |

### 7.4 IVR / Voice Browser

| This model | IVR / VoiceXML era |
| --- | --- |
| Conversation | The customer's history of IVR contacts |
| Session | The IVR application session for one call |
| Message | DTMF events, ASR results, prompts (interpreted as events) |
| Participant | The caller and the IVR application |
| Connection | The SIP leg from caller to the IVR platform |
| Stream | The audio stream carrying voice and DTMF |

### 7.5 Contact Center / CCaaS

| This model | Contact Center |
| --- | --- |
| Conversation | An *interaction* (Genesys), a *contact* (NICE), or a *conversation* (Salesforce, Zendesk, Intercom) |
| Session | A single contact or call within an interaction |
| Message | A chat message, SMS, or email in the customer record |
| Participant | A customer, an agent, a supervisor |
| Connection | A call leg through the ACD/queue |
| Stream | Audio carried between caller and agent |

### 7.6 Web Chat / Consumer Messaging

| This model | Web Chat / Consumer Messaging |
| --- | --- |
| Conversation | A conversation (same word) |
| Session | An active chat window with live presence |
| Message | A chat message (same word) |
| Participant | A participant in the conversation (same word) |
| Connection | A WebSocket or HTTP long-poll attachment |
| Stream | Typing/presence updates and the stream of messages |

In every case, every primary noun maps to a familiar concept. Where the existing community has the same word ("conversation," "participant," "stream," "message"), the meaning is preserved. Where the community has a different word ("dialog," "leg," "track"), the mapping is one-step and reversible.

---

## 8. Why Now: The Case for VoIP 3.0

Three technical developments make this convergence both possible and necessary now.

### 8.1 LLMs and Real-Time Speech Models

For the first time, voice applications can hold competent conversations across arbitrary domains. Real-time speech recognition and text-to-speech are inexpensive, fast, and high quality. AI agents can answer phones, respond on chat, escalate to humans, and accumulate memory across Conversations. The Voice 2.0 vision of voice as a computing primitive is finally feasible — but only if the substrate underneath supports it cleanly.

### 8.2 QUIC for Media

The IETF AVTCORE working group is actively standardizing RTP-over-QUIC (RoQ); the BBC and others have published drafts for SIP-over-QUIC and QUIC-RTP-Tunnelling (QRT); the MoQ Working Group is defining a new pub/sub media transport over QUIC. These efforts are not hypothetical — they are being deployed in production at Cloudflare, Meta, Twitch, and the BBC, and the standards are converging. The next decade of real-time media transport will not be RTP-over-UDP alone; it will be a mix of RTP-over-UDP, RTP-over-QUIC, MoQ, and WebRTC's existing DTLS-SRTP-over-UDP-with-ICE. A voice library or platform built today must accommodate all of them.

### 8.3 Multi-Modal Expectations

Users now expect a single customer service interaction to span SMS, web chat, voice, and video without losing context. They expect to start on their phone and continue on their laptop. They expect AI agents to hand off to humans without making them repeat their problem. None of this is met by channel-siloed architectures, and gateways between SIP and WebRTC are not enough — durable Conversations, a unified Participant model, and a shared lexicon must exist all the way down.

### 8.4 The 1.0 / 2.0 / 3.0 Periodization

- **VoIP 1.0 (1996–):** the SIP era. Voice over IP replaces TDM. Network-layer revolution. Telco-driven.
- **VoIP 2.0 (2000–2024, two waves):** the web-meets-voice era.
  - The first wave (W3C VoiceXML, the Voice 2.0 startup era of GrandCentral, JahJah, Truphone, Iotum) tried to bring web developers into voice and largely failed at that goal while succeeding at IVR adoption.
  - The second wave (WebRTC) succeeded with web developers but split the stack into a SIP world and a WebRTC world that interoperate only via gateways.
- **VoIP 3.0 (2025–):** the unification era. Multi-transport substrate, AI agents as first-class Participants, transport-agnostic Session model, shared vocabulary across communities.

The "VoIP" framing is used inclusively. Although the literal expansion is *voice over IP*, SIP and RTP have always carried video as well, and modern usage encompasses voice, video, and increasingly mixed media. We use VoIP as the historical category name with the understanding that the modern reality is voice, video, and data over IP.

---

## 9. Use Cases This Enables

The model is not theoretical. The following use cases are practical today if and only if the model holds and the substrate is built.

### 9.1 Multi-Modal AI Agent

A customer messages a company's web chat. An AI agent picks up the Conversation. The customer asks a complex question; the AI suggests a voice call. The customer clicks "talk." A voice Session starts in the same Conversation, with the same AI agent now speaking via TTS and listening via ASR. The customer's question is resolved. The next morning, the AI follows up by SMS in the same Conversation with the answer to a related question the customer mentioned but did not get to.

One Conversation. Two Sessions (one text, one voice) and several Messages. One AI Identity in the `agent` role throughout. Cross-modality continuity is a property of the Conversation, not a feature bolted on top.

### 9.2 Human-AI Handoff Without Disruption

A customer calls a company's main number. An AI agent answers (kind: ai, role: agent), gathers context, attempts resolution. The issue is too complex; the AI hands off to a human agent in the company's contact center. The customer does not need to repeat themselves; the human agent sees the entire Conversation history and joins the live voice Session as a new Participant. The AI remains in the Session as an `observer`, providing real-time suggestions and transcribing.

One Conversation. One Session. Multiple Participants with role transitions. The customer's Connection is unchanged throughout — only who is speaking on the other end has shifted.

### 9.3 SIP-to-WebRTC Bridging Without a Gateway Product

A SIP-based call center routes a call to an agent who is working from home in a browser-based softphone. Today this requires a WebRTC-to-SIP gateway. With a unified library, the customer's SIP Connection and the agent's WebRTC Connection are both attached to the same Session; transport differences are handled at the Connection layer; codec compatibility is negotiated by the Session; the developer writes no gateway code. The library *is* the gateway, transparently.

### 9.4 Cross-Device Continuity

A user starts a video Conversation with a vendor on their phone while walking. They arrive home and want to continue on their laptop with a larger screen. They tap "transfer to laptop." Their phone Connection is replaced by a laptop Connection in the same Session. The Conversation, Session, Streams, and other Participants are unchanged.

### 9.5 QUIC-Native AI Voice with PSTN Bridge

An AI voice agent communicates with browser-based users over RTP-over-QUIC for low latency, while simultaneously bridging to a PSTN customer via a SIP Connection in the same Session. The Session sees two Participants with two Connections of two different transports; neither Participant cares about the other's transport. As QUIC media transports mature, additional Participants may attend via MoQ-based publish/subscribe Streams without altering the Session model.

### 9.6 Practice and Simulation Conversations

A sales coaching application creates a practice Conversation between a salesperson and an AI agent acting as a simulated customer. The Session is a voice call. After the practice, the same Conversation accumulates analysis, scoring, and coaching feedback as additional Messages or Sessions; a coach AI may join later as a separate Participant to deliver personalized feedback. The same architectural primitives that handle real customer Conversations handle practice and simulation Conversations.

### 9.7 Conversation-Aware Coaching

A live customer call (a Session in a customer Conversation) is analyzed in real time. After the call, an AI coach is assigned to a coaching Conversation between itself and the salesperson; the coaching Conversation references the customer Conversation for context. This separation cleanly distinguishes the customer-facing Conversation from the internal coaching Conversation while letting them share evidence.

### 9.8 Heterogeneous Streams Per Participant

A Session contains:

- Participant A on a desktop (kind: human, role: agent) — Connection over WebRTC carrying audio + video Streams.
- Participant B on a phone (kind: human, role: customer) — Connection over SIP carrying an audio Stream only.
- Participant C, an AI assistant (kind: ai, role: observer) — Connection over QUIC carrying transcribed text as a data Stream and outgoing audio cues as an audio Stream.
- Participant D, a recording bot (kind: system, role: observer) — Connection over the platform's internal transport carrying receive-only Streams.

Each Participant brings what it can; the Session does not require uniformity. This is the normal case in production multi-modal AI deployments and is awkward to express in any current framework without per-channel special casing.

### 9.9 Cross-Conversation Memory and Learning

Patterns observed across many Conversations — preferred contact channel, time of day, language, tone — accumulate at the Identity level and inform routing and Session defaults for future Conversations. Because the model has Identity, Participant, Conversation, and Memory as first-class concepts, this learning is structurally possible without bolting a separate analytics system onto the side.

---

## 10. Implementation Notes

The vocabulary in this document is implementation-neutral. However, several implementation patterns follow naturally and are worth recording.

### 10.1 Transport Adapters

A Session does not own transport-specific logic. Instead, each Connection encapsulates transport-specific behavior behind a common interface. The unifying library exposes Sessions with Connections; transport adapters (SIP/RTP, WebRTC, RoQ, MoQ, others) implement the Connection interface for their transport.

### 10.2 Signaling Adapters

SIP signaling, WebRTC signaling (WHIP/WHEP, custom signaling), and QUIC-native signaling are signaling adapters that initiate Connections and report their state. The Session abstraction does not depend on which signaling adapter was used.

### 10.3 Codec and Capability Negotiation

Codec and capability negotiation occurs at Connection establishment. The Session may impose constraints (for example, requiring a common audio codec across all Connections), but the negotiation mechanics are transport-specific.

### 10.4 Recording, Transcription, and Compliance

Recording, transcription, and compliance attachments are themselves Participants of `kind: system` joining the Session. They establish their own Connections (typically receive-only) and consume Streams. This avoids out-of-band recording paths and keeps compliance subject to the same audit and consent flows as any other Participant.

---

## 11. Open Questions

The following questions remain open and will be addressed in companion documents:

- How a Conversation is identified externally — Thelve `ConversationId`, customer-owned external reference, or both.
- Where Identity and authentication live (the Interaction Plane, a separate identity service, or federated identity).
- How multi-tenancy is expressed across Conversations, Identities, Workforces, and Capability Access.
- How recording, lawful intercept, and compliance attach in detail — at the Session level, the Connection level, or both.
- How latency budgets are tracked and enforced for real-time path operations.
- How autonomy levels (observer, suggester, doer-with-approval, autonomous) attach to AI Participants and gate their actions.

---

## 12. Acknowledgments

This work draws on more than two decades of standards and community work in voice, web, and real-time communication. Specific debts are owed to the W3C Voice Browser Working Group, the IETF SIP, AVTCORE, MoQ, and QUIC working groups, the WebRTC project, the open-source projects FreeSWITCH, Asterisk, Kamailio, OpenSIPS, Janus, mediasoup, and LiveKit, and the engineers, researchers, and product teams across the SIP, WebRTC, voice-AI, IVR, contact-center, and web-chat communities whose work this model attempts to harmonize.

---

## Appendix A: Vocabulary Summary

**Primary nouns:**

- `Conversation` — durable cross-channel/cross-time relationship
- `Session` — sync bounded engagement within a Conversation
- `Message` — async atomic communication event within a Conversation
- `Participant` — entity with identity in a Conversation (has `kind` and `role`)
- `Connection` — one Participant's transport binding into one Session (= leg in SIP, peer connection in WebRTC)
- `Stream` — one media flow on a Connection (audio, video, screenshare, data)

**Supporting concepts:**

- `Identity` — durable real-world entity behind a Participant
- `Device` — a physical or software endpoint
- `Workforce` — pool of Identities eligible for the `agent` role
- `kind` — Participant entity type (human, ai, system, external)
- `role` — Participant function in a Conversation (customer, agent, supervisor, observer)

**Verbs:**

- Conversation: opens, continues, resumes, closes
- Session: starts, ends, upgrades
- Participant: joins, leaves, takes over, hands off
- Connection: establishes, negotiates, renegotiates, terminates
- Stream: opens, flows, pauses, closes

**The single-sentence model:**

> A Conversation contains Messages and Sessions; a Session has Participants who attach via Connections; each Connection carries Streams of media.
