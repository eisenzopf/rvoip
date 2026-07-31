<div align="center">
  <img src="rvoip-banner.svg" alt="rvoip — the Rust real-time communications platform" width="50%" />

# rvoip

**Rust-native real-time communications across SIP, WebRTC, QUIC, WebTransport, WebSocket, MoQ, voice AI, and enterprise integrations.**

[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](#license)
[![rvoip](https://img.shields.io/crates/v/rvoip.svg?label=rvoip)](https://crates.io/crates/rvoip)
[![rvoip-sip](https://img.shields.io/crates/v/rvoip-sip.svg?label=rvoip-sip)](https://crates.io/crates/rvoip-sip)
[![Facade API](https://docs.rs/rvoip/badge.svg)](https://docs.rs/rvoip)
[![SIP API](https://docs.rs/rvoip-sip/badge.svg)](https://docs.rs/rvoip-sip)

[**What ships**](#what-ships-today) · [**SIP interop**](#sip-interoperability) · [**Choose a crate**](#choose-your-entry-point) · [**Quick start**](#quick-start) · [**Capabilities**](#capability-matrix) · [**Extensions**](#extensions) · [**Architecture**](#architecture) · [**Evidence**](#release-evidence) · [**Roadmap**](#roadmap)

</div>

---

> [!IMPORTANT]
> **Unified `0.3.5` release train.** All 44 publishable workspace crates ship on
> the same version. Publication requires a fresh, strict full-beta run bound to
> the exact release source: no skipped gates, no carry-forward qualification,
> and passing workspace, security, four-peer interoperability, performance,
> resiliency, and long-soak evidence. The generated beta report is authoritative
> for the exact tested versions and results. The SIP product is the
> release-gated beta surface. WebRTC,
> UCTP, MoQ, the cross-transport APIs, Amazon Connect, and extension crates are
> available today as developer-preview surfaces unless their own documentation
> states a narrower qualification. Available does not mean API-stable or
> production-certified; breaking changes remain possible before `1.0`.
> The same unified release includes all 14 optional extension crates and the
> new native `rvoip-vapi` bidirectional raw-audio WebSocket transport.

## What ships today

rvoip is a modular real-time communications platform rather than a SIP crate
with future adapters. Applications can use one product by itself or register
multiple transports with the shared conversation model and bridge between
them.

| Product area | Available capabilities | Maturity |
| --- | --- | --- |
| **SIP telephony** | Endpoints, reactive servers, PBX/registrar/proxy building blocks, B2BUA bridging, call control, transfers, authentication, and full RTP media | **Beta-qualified** |
| **Media and devices** | RTP/RTCP, SDES-SRTP, G.711, optional codecs, DTMF, OS microphone/speaker integration, resampling, jitter buffering, and conference-mixing primitives | Beta-qualified core + developer-preview additions |
| **WebRTC** | WHIP/WHEP and WebSocket signaling, full-gather and trickle ICE, DTLS-SRTP, Opus/G.711 audio, VP8 video, SCTP data channels, and RFC 4733 DTMF | **Available — developer preview** |
| **UCTP substrates** | One conversation protocol over raw QUIC, WebTransport, or WebSocket, including capability negotiation and RTP datagram framing | **Available — developer preview** |
| **Media over QUIC** | MOQT draft-19 transport, native helper, embeddable relay, and an rvoip media-graph broadcast adapter | **Available — developer preview** |
| **Gateways and bridges** | SIP ↔ WebRTC ↔ UCTP routing, a high-level application builder, and SIP-to-Amazon-Connect audio/screen-pop integration | **Available — developer preview** |
| **Voice AI and conversation data** | Pluggable ASR, TTS, dialog, and recording providers; native bidirectional Vapi raw-audio WebSocket agents; signed vCon artifacts; and Postgres-backed vCon storage | **Available — developer preview** |
| **Identity and compliance** | Digest/Bearer foundations, OIDC, Keycloak, LDAP, Redis, SAML, SCIM, WebAuthn, IMS AKA, STIR/SHAKEN, and redacted audit/SIEM sinks | Beta-qualified SIP auth core + developer-preview extensions |

### Maturity labels

- **Beta-qualified** — covered by the SIP release gate and its bounded
  interoperability, security, standards, performance, and soak evidence.
- **Available — developer preview** — implemented and included in `0.3.2`,
  but API-unstable or outside the SIP beta attestation.
- **Planned** — not implemented; listed only in the [roadmap](#roadmap).

## SIP interoperability

The 0.3.2 full release run passed all 16 selected PBX and interoperability
gates. The table distinguishes peers that were actually exercised from proxy
targets that were only audited and deliberately excluded from the release
claim.

| Peer/tool | 0.3.2 status | Executed scope |
| --- | --- | --- |
| **Asterisk** | **Interop matrix passed** | `Endpoint`, `StreamPeer`, and `CallbackPeer`; registration, basic call, G.729A/G.729AB, hold/resume, ring-cancel, RFC 4733 DTMF, rejection, and blind transfer over UDP and TLS |
| **FreeSWITCH** | **Interop matrix passed** | The same API, scenario, codec, and UDP/TLS matrix as Asterisk |
| **SIPp** | **Standalone matrix passed** | 30, 100, 300, 1,000, and 2,000 CPS; every configured call completed |
| **baresip** | **Strict-UA check passed** | External user-agent call against the rvoip SIP listener |
| **Kamailio** | **Not release-tested** | Named proxy/RTPengine investigation track; the 0.3.2 gate records a de-scope audit, not a Kamailio interoperability claim |
| **OpenSIPS** | **Not release-tested** | Named proxy/RTPengine investigation track; the 0.3.2 gate records a de-scope audit, not an OpenSIPS interoperability claim |

See the [0.3.2 complete gate
record](crates/sip/rvoip-sip/docs/BETA_GATE_EXCEPTION.md) and
[compatibility matrix](crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md) for
the evidence boundaries. A passing lab matrix is not carrier certification or
a claim about every peer version and deployment topology.

### Native Vapi WebSocket agents

New in 0.3.2, [`rvoip-vapi`](crates/extensions/rvoip-vapi) implements Vapi's
bidirectional WebSocket call transport directly in Rust. It originates the
Vapi agent leg, streams full-duplex μ-law 8 kHz or PCM 16 kHz raw audio, exposes
typed agent events and control/context messages, bridges an existing SIP or
WebRTC caller connection through the shared orchestrator, and supervises
symmetric teardown. It does not require a third-party telephony intermediary
between rvoip and Vapi.

The adapter is a developer-preview extension. Start with the
[`14-vapi-agent`](examples/14-vapi-agent) server, which accepts either SIP or
WebRTC callers.

## Choose your entry point

| You want to build | Start with | Why |
| --- | --- | --- |
| SIP endpoint, softphone, PBX, IVR, registrar, proxy, B2BUA, or gateway | [`rvoip-sip`](crates/sip/rvoip-sip) | Highest-level release-gated SIP APIs: `Endpoint`, `StreamPeer`, `CallbackPeer`, and `UnifiedCoordinator` |
| One application spanning SIP, WebRTC, and UCTP | [`rvoip`](crates/rvoip) | Shared `Orchestrator`, conversation model, transport adapters, and optional `app` builder |
| Mobile, desktop, web, or embedded client SDK | [`rvoip-client`](crates/rvoip-client) | A single client/session/event surface with opt-in SIP, WebRTC, and UCTP transports |
| Browser or native WebRTC interop | [`rvoip-webrtc`](crates/webrtc/rvoip-webrtc) | WebRTC server, client, signaling, media, data-channel, and orchestrator adapter surfaces |
| QUIC, WebTransport, or WebSocket conversation transport | [`rvoip-uctp`](crates/uctp/rvoip-uctp) | UCTP protocol plus dedicated substrate adapters |
| Broadcast/fan-out over Media over QUIC | [`rvoip-moq`](crates/moq/rvoip-moq) | MOQT media-graph adapter with native transport and relay crates |
| SIP calls delivered to Amazon Connect agents | [`rvoip-amazon-connect`](crates/webrtc/rvoip-amazon-connect) | Turnkey SIP UAS, G.711 ↔ Opus bridge, contact attributes, and agent screen pops |
| SIP or WebRTC calls connected to Vapi voice agents | [`rvoip-vapi`](crates/extensions/rvoip-vapi) | Native bidirectional raw-audio Vapi WebSocket transport integrated with the shared orchestrator and media bridge |
| Microphone and speaker audio for a SIP app | [`rvoip-audio-device`](crates/media/rvoip-audio-device) | CPAL device I/O, pacing, resampling, jitter buffering, mute, and metering |
| Authentication, provisioning, AI, vCon, or audit integrations | [Extensions](#extensions) | Optional provider crates keep protocol cores independent of deployment backends |

## Quick start

Add the SIP product:

```toml
[dependencies]
rvoip-sip = "0.3.5"
tokio = { version = "1", features = ["full"] }
```

A complete local call: Bob answers, Alice dials, then Alice hangs up.

```rust
use std::time::Duration;
use rvoip_sip::{Config, Endpoint, EndpointProfile};

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    let bob = tokio::spawn(async {
        let mut bob = Endpoint::builder()
            .name("bob")
            .profile(EndpointProfile::Custom(Config::local("bob", 5071)))
            .build()
            .await?;
        let incoming = bob.wait_for_incoming().await?;
        let call = incoming.answer().await?;
        call.wait_for_end(None).await?;
        bob.shutdown().await
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    let alice = Endpoint::builder()
        .name("alice")
        .profile(EndpointProfile::Custom(Config::local("alice", 5070)))
        .build()
        .await?;

    let call = alice
        .call_and_wait("sip:bob@127.0.0.1:5071", Some(Duration::from_secs(10)))
        .await?;
    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    alice.shutdown().await?;
    bob.await.unwrap()
}
```

Run the equivalent checked-in example:

```sh
cargo run -p rvoip-sip --example endpoint_local_call
```

The standalone [`examples/`](examples/) progress from a first P2P call through
real audio, PBX registration, call control, transfers, SRTP/TLS, IVR, B2BUA,
voice AI, and cross-transport integrations:

- [`11-ai-harness-demo`](examples/11-ai-harness-demo) — ASR → dialog → TTS →
  recording → vCon with deterministic providers.
- [`12-customer-escalation-sip-webrtc`](examples/12-customer-escalation-sip-webrtc)
  — browser WebRTC chat escalated to a SIP agent voice call.
- [`13-sip-to-amazon-connect`](examples/13-sip-to-amazon-connect) — SIP custom
  headers translated into Amazon Connect contact attributes and a live audio
  bridge.
- [`14-vapi-agent`](examples/14-vapi-agent) — one high-level server accepts
  either SIP or WebRTC callers and bridges them to Vapi voice agents.

## Capability matrix

### SIP application and signaling

| Capability | Maturity | Supported behavior | Start/evidence |
| --- | --- | --- | --- |
| Endpoint and server APIs | **Beta-qualified** | Outbound/inbound calls through `Endpoint`, scripted `StreamPeer`, reactive `CallbackPeer`, and lower-level `UnifiedCoordinator` | [`rvoip-sip`](crates/sip/rvoip-sip) |
| Core dialog control | **Beta-qualified** | INVITE, ACK, BYE, CANCEL, REGISTER, OPTIONS, UPDATE, PRACK, REFER, SUBSCRIBE/NOTIFY, MESSAGE, and INFO within documented bounds | [RFC matrix](crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md) |
| PBX/gateway building | **Beta-qualified** | Registrar bindings, stateful proxy primitives, B2BUA call-leg coordination, media bridging, and custom SIP headers | [`10-call-center-b2bua`](examples/10-call-center-b2bua) |
| Blind transfer | **Beta-qualified** | REFER-driven transfer and typed NOTIFY progress/final outcomes | [`05-blind-transfer`](examples/05-blind-transfer) |
| Attended-transfer primitives | **Available — developer preview** | Consultation dialog identity, `Replaces` construction, REFER delivery, and a working orchestration example; not a complete RFC 3891 qualification | [`06-attended-transfer`](examples/06-attended-transfer) |
| SIP transport | **Beta-qualified** | UDP, TCP, and TLS; plain SIP-over-WebSocket has bounded evidence | [`rvoip-sip-transport`](crates/sip/sip-transport) |
| Secure WebSocket | **Available — developer preview** | WSS listener/lower-level support; outbound WSS dialing is not a SIP beta claim | [Transport README](crates/sip/sip-transport/README.md) |

### Media

| Capability | Maturity | Supported behavior | Start/evidence |
| --- | --- | --- | --- |
| RTP/RTCP and G.711 | **Beta-qualified** | PCMU/PCMA media delivery, RTCP receiver reports, telephone-event DTMF, hold/resume, and bridging | [`rvoip-media-core`](crates/media/media-core) |
| SDES-SRTP | **Beta-qualified** | Tested AES-CM/HMAC profiles with negotiated encrypted media | [`07-secure-call-srtp`](examples/07-secure-call-srtp) |
| G.729A/G.729AB | **Available — developer preview** | Fully integrated optional path: PT 18 SDP/Annex B negotiation, RTP encode/decode, G.711 transcoding, and Asterisk/FreeSWITCH matrix coverage; excluded only from the general SIP full-media performance claim | [0.3.2 gate record](crates/sip/rvoip-sip/docs/BETA_GATE_EXCEPTION.md) |
| Opus and G.722 paths | **Available — developer preview** | Feature-gated codec/media support; not part of the bounded SIP beta media claim | [`rvoip-media-core`](crates/media/media-core) |
| OS audio devices | **Available — developer preview** | Microphone/speaker bridge, drift-free pacing, resampling, jitter buffering, mute-as-silence, and VU metering | [`02-softphone-audio`](examples/02-softphone-audio) |
| Conference mixing | **Available — developer preview** | Lower-level N-way/N-1 mixing and conference monitoring primitives; not an integrated SIP beta conference product | [Media README](crates/media/media-core/README.md) |

### WebRTC, UCTP, MoQ, and integrations

| Capability | Maturity | Supported behavior | Start/evidence |
| --- | --- | --- | --- |
| WebRTC interop | **Available — developer preview** | WHIP/WHEP and WebSocket signaling, full-gather/trickle ICE, DTLS-SRTP, Opus/G.711, VP8, SCTP data channels, and DTMF | [`rvoip-webrtc`](crates/webrtc/rvoip-webrtc) |
| TURN integration | **Available — developer preview** | External TURN server configuration; rvoip does not ship or claim a hosted TURN service | [WebRTC scope](crates/webrtc/rvoip-webrtc/README.md) |
| UCTP | **Available — developer preview** | Envelopes, state machines, capability negotiation, authenticated resource binding, and RTP datagram framing | [`rvoip-uctp`](crates/uctp/rvoip-uctp) |
| UCTP substrates | **Available — developer preview** | Dedicated raw QUIC, WebTransport, and WebSocket adapters | [`crates/uctp`](crates/uctp) |
| Media over QUIC | **Available — developer preview** | MOQT draft-19 transport/native/relay packages plus rvoip media-graph broadcast integration | [`crates/moq`](crates/moq) |
| Cross-transport app builder | **Available — developer preview** | Role/capability policy, assignment, callbacks, SIP/WebRTC/UCTP listeners, and orchestration | [`rvoip::app`](crates/rvoip/src/app.rs) |
| Amazon Connect | **Available — developer preview** | `StartWebRTCContact`, Amazon Chime WebRTC media, SIP-header contact attributes, G.711 ↔ Opus bridging, and agent screen pops | [`13-sip-to-amazon-connect`](examples/13-sip-to-amazon-connect) |
| Vapi voice agents | **Available — developer preview** | Native bidirectional μ-law/PCM raw-audio WebSocket agent sessions bridged to rvoip-owned SIP or WebRTC legs, with typed events, control messages, and supervised teardown | [`rvoip-vapi`](crates/extensions/rvoip-vapi) |

The WebRTC implementation of ICE and DTLS-SRTP is separate from the SIP beta
claim. Likewise, UCTP and MoQ availability does not imply that SIP-over-QUIC or
RTP-over-QUIC has shipped.

## Extensions

All 14 extension crates ship at `0.3.5`. They are first-class workspace
capabilities, but remain optional so protocol crates depend on provider
contracts rather than deployment-specific services.

| Group | Extensions | Available capability |
| --- | --- | --- |
| **AI and conversation data** | [`rvoip-harness`](crates/extensions/rvoip-harness), [`rvoip-vapi`](crates/extensions/rvoip-vapi), [`rvoip-vcon`](crates/extensions/rvoip-vcon), [`rvoip-vcon-postgres`](crates/extensions/rvoip-vcon-postgres) | ASR/TTS/dialog/recording provider traits, native Vapi raw-audio WebSocket voice-agent bridging, signed vCon artifacts, in-memory interfaces, and Postgres storage |
| **Caller trust** | [`rvoip-stir-shaken`](crates/extensions/rvoip-stir-shaken) | STIR/SHAKEN PASSporT signing and verification for RFC 8224/RFC 8225/ATIS profiles |
| **Authentication providers** | [`rvoip-oidc`](crates/extensions/rvoip-oidc), [`rvoip-keycloak`](crates/extensions/rvoip-keycloak), [`rvoip-ldap`](crates/extensions/rvoip-ldap), [`rvoip-redis`](crates/extensions/rvoip-redis), [`rvoip-ims-aka`](crates/extensions/rvoip-ims-aka) | OIDC discovery and validation, Keycloak integration, LDAP password verification, clustered auth/revocation/replay state, and IMS AKA adapters |
| **User lifecycle** | [`rvoip-saml`](crates/extensions/rvoip-saml), [`rvoip-scim`](crates/extensions/rvoip-scim), [`rvoip-webauthn`](crates/extensions/rvoip-webauthn) | SAML 2.0 service-provider integration, SCIM 2.0 provisioning, and WebAuthn/passkeys |
| **Audit and observability** | [`rvoip-audit`](crates/extensions/rvoip-audit) | Redacted JSONL and tracing sinks plus OTLP and SIEM exports for generic webhooks, Splunk, Elastic/ECS, Microsoft Sentinel, and Datadog |

The supporting contracts live in
[`rvoip-auth-core`](crates/identity/auth-core),
[`rvoip-users-core`](crates/identity/users-core), and
[`rvoip-identity`](crates/identity/rvoip-identity).

### Enabling extensions

The facade exposes the conversation-model extensions together:

```toml
rvoip = { version = "0.3.5", features = ["voip-3"] }
```

`voip-3` enables SIP, WebRTC, UCTP, vCon, the identity provider surface, and
the AI harness. Vapi and STIR/SHAKEN have separate facade features:

```toml
rvoip = { version = "0.3.5", features = ["sip", "vapi", "sip-stir-shaken"] }
```

Deployment-specific extensions are direct dependencies:

```toml
rvoip-keycloak = "0.3.5"
rvoip-redis = "0.3.5"
rvoip-audit = "0.3.5"
```

The facade's `full` feature does **not** enable every workspace extension,
Amazon Connect, MoQ, or the audio-device crate.

## Architecture

```text
┌───────────────────────────────────────────────────────────────────┐
│ Applications                                                       │
│ softphone · PBX · contact center · browser · AI · broadcast       │
└───────────────────────────────┬───────────────────────────────────┘
                                │
                    rvoip facade / product APIs
                                │
┌───────────────────────────────▼───────────────────────────────────┐
│ Shared conversation model                                         │
│ Orchestrator · Conversation · Session · Connection · Stream       │
│ routing · admission · bridges · media graph · events              │
└──────────────┬────────────────┬────────────────┬──────────────────┘
               │                │                │
        ┌──────▼──────┐  ┌──────▼──────┐  ┌──────▼───────────────┐
        │ SIP + RTP   │  │ WebRTC      │  │ UCTP                │
        │ UDP/TCP/TLS │  │ ICE/DTLS    │  │ QUIC/WT/WebSocket   │
        └──────┬──────┘  └──────┬──────┘  └──────┬───────────────┘
               └────────────────┼────────────────┘
                                │
             ┌──────────────────▼───────────────────┐
             │ Optional products and extensions     │
             │ MoQ · Amazon Connect · AI · vCon     │
             │ identity · provisioning · audit      │
             └───────────────────────────────────────┘
```

Adapters depend on the shared `ConnectionAdapter` and core-trait surface; the
core does not import individual transport implementations. That dependency
direction lets one `Orchestrator` route and bridge different substrates
without coupling them to each other.

## Workspace crate map

The unified release contains 44 publishable crates:

| Family | Crates |
| --- | --- |
| Front doors | [`rvoip`](crates/rvoip), [`rvoip-client`](crates/rvoip-client) |
| Foundation | [`rvoip-core`](crates/foundation/rvoip-core), [`rvoip-core-traits`](crates/foundation/rvoip-core-traits), [`rvoip-infra-common`](crates/foundation/infra-common) |
| Media | [`rvoip-media-core`](crates/media/media-core), [`rvoip-codec-core`](crates/media/codec-core), [`rvoip-rtp-core`](crates/media/rtp-core), [`rvoip-audio-device`](crates/media/rvoip-audio-device) |
| SIP | [`rvoip-sip`](crates/sip/rvoip-sip), [`rvoip-sip-core`](crates/sip/sip-core), [`rvoip-sip-transport`](crates/sip/sip-transport), [`rvoip-sip-dialog`](crates/sip/sip-dialog), [`rvoip-sip-proxy`](crates/sip/sip-proxy), [`rvoip-sip-registrar`](crates/sip/sip-registrar) |
| WebRTC and Connect | [`rvoip-rtc`](crates/webrtc/rvoip-rtc), [`rvoip-webrtc-stack`](crates/webrtc/rvoip-webrtc-stack), [`rvoip-webrtc`](crates/webrtc/rvoip-webrtc), [`rvoip-amazon-connect`](crates/webrtc/rvoip-amazon-connect) |
| UCTP | [`rvoip-uctp`](crates/uctp/rvoip-uctp), [`rvoip-quic`](crates/uctp/rvoip-quic), [`rvoip-webtransport`](crates/uctp/rvoip-webtransport), [`rvoip-websocket`](crates/uctp/rvoip-websocket) |
| MoQ | [`rvoip-moq-transport`](crates/moq/rvoip-moq-transport), [`rvoip-moq-native`](crates/moq/rvoip-moq-native), [`rvoip-moq-relay`](crates/moq/rvoip-moq-relay), [`rvoip-moq`](crates/moq/rvoip-moq) |
| Identity | [`rvoip-auth-core`](crates/identity/auth-core), [`rvoip-users-core`](crates/identity/users-core), [`rvoip-identity`](crates/identity/rvoip-identity) |
| Extensions | The [14 extension crates](#extensions) listed above |

## Release evidence

SIP beta claims are intentionally bounded by checked-in evidence rather than
inferred from the presence of parser types, low-level primitives, or another
product's implementation:

- [0.3.2 release exception](crates/sip/rvoip-sip/docs/BETA_RELEASE_EXCEPTION.md)
  and [performance evidence](crates/sip/rvoip-sip/docs/BETA_PERFORMANCE_EXCEPTION.md)
  — the complete owner-approved disposition, strict 106/108 result, and the
  accepted high-density burst deviation.
- [Last strict beta candidate](crates/sip/rvoip-sip/docs/BETA_RELEASE_REPORT.md)
  — the most recent candidate that passed all 108 automated gates without an
  exception.
- [RFC evidence matrix](crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md) —
  exact supported, partial, and unsupported standards claims.
- [Security posture](crates/sip/rvoip-sip/docs/SECURITY_POSTURE.md) —
  qualified security behavior and explicit non-claims.
- [Next release notes](crates/sip/rvoip-sip/docs/RELEASE_NOTES_NEXT.md) —
  unified release identity, source compatibility notes, and attestation
  provenance.

The `0.3.5` release requires a fresh strict full-beta report bound to one clean,
unchanged release source fingerprint. The gate admits no skipped checks and
includes the workspace, SIP/media, public API, security, PBX, SIPp, strict-UA,
proxy interoperability, performance, resiliency, and long-soak scopes. The
historical `0.3.4` carry-forward receipt remains immutable release history; it
does not qualify `0.3.5`.

### SIP interoperability attestation

The strict full-beta gate requires an explicit PASS attestation for all four
independently managed peers below. The report generator binds every row to the
tested source tree, exact peer identity and configuration, selected matrix,
and hashed evidence; it refuses to produce a strict release-candidate report
if a required peer is missing, skipped, ambiguous, unpinned, or failing. This
four-peer matrix is mandatory for the `0.3.5` strict release gate.

| Peer | Attested boundary | Required release evidence |
| --- | --- | --- |
| **Asterisk** | PBX/B2BUA call control and RTP media | Provider-specific all-PASS rows from the recorded API, scenario, codec, and security matrix, plus the exact local revision and configuration fingerprint |
| **FreeSWITCH** | PBX/B2BUA call control and RTP media | Provider-specific all-PASS rows from the recorded API, scenario, codec, and security matrix, plus the exact local revision and configuration fingerprint |
| **Kamailio** | RFC 3261 transaction-stateful proxy interoperability | Digest-pinned peer, both hop orders, UDP/TCP/TLS, packet assertions, verified TLS evidence, and post-retention cleanup |
| **OpenSIPS** | RFC 3261 transaction-stateful proxy interoperability | Digest-pinned peer, both hop orders, UDP/TCP/TLS, packet assertions, verified TLS evidence, and post-retention cleanup |

The generated [beta release report](crates/sip/rvoip-sip/docs/BETA_RELEASE_REPORT.md)
is the authority for the exact versions, row counts, scenarios, hashes, and
PASS status of a particular candidate. This is bounded interoperability
evidence, not a claim of compatibility with every version, module,
configuration, transport, codec, or SIP extension.

Developer-preview products document their own supported scope and gaps in
their crate READMEs. A published crate or Cargo feature is evidence of
availability, not a blanket production-readiness statement.

## Roadmap

Implemented WebRTC, QUIC/WebTransport/WebSocket, MoQ, AI-provider, and
extension capabilities are described above rather than presented as future
work. Remaining major items include:

- **SIP-over-QUIC** and **RTP-over-QUIC (RoQ)** transport profiles.
- Integrated multi-party **SFU/MCU** products beyond the existing media
  primitives.
- Production graduation of **AAuth** as its standards work and deployment
  evidence mature.
- Deeper **AI participants** with multi-agent orchestration beyond today's
  pluggable provider harness.
- Additional qualification, compatibility guarantees, and release gates for
  the developer-preview products.

Detailed engineering gaps are tracked in [`docs/GAP_PLAN.md`](docs/GAP_PLAN.md).

## Why rvoip

- **One Rust stack:** signaling, media, orchestration, transport adapters, and
  deployment integrations share one type and event model.
- **Use only what you need:** ship the release-gated SIP product alone or opt
  into cross-transport and extension crates.
- **Bridge without protocol glue:** route conversations between SIP, WebRTC,
  UCTP, Amazon Connect, and media-graph consumers.
- **Evidence-aware documentation:** shipped capabilities are visible without
  collapsing availability, API stability, standards compliance, and
  production qualification into one label.

## Evaluating rvoip

```sh
git clone https://github.com/eisenzopf/rvoip.git
cd rvoip

# Build the default workspace members
cargo build

# Run a working SIP call
cargo run -p rvoip-sip --example endpoint_local_call

# Run the workspace test suite
scripts/test_all.sh
```

Review the relevant product README and evidence before deployment. Anything
outside a stated qualification boundary remains the application's
responsibility to validate in its own topology.

## Contributing

- **Bugs:** open an issue with reproduction steps.
- **Feature requests:** use discussions or issues and describe the target
  product and compatibility expectations.
- **Pull requests:** workspace-wide tests run through `scripts/test_all.sh`.

<a id="license"></a>
## License

Licensed under the [MIT License](LICENSE).

<div align="center">

---

**Built in Rust** · [Facade API](https://docs.rs/rvoip) · [SIP API](https://docs.rs/rvoip-sip) · [Examples](examples/) · [Issues](https://github.com/eisenzopf/rvoip/issues) · [Discussions](https://github.com/eisenzopf/rvoip/discussions)

</div>
