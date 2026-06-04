<div align="center">
  <img src="rvoip-banner.svg" alt="rvoip — the Rust real-time substrate" width="50%" />

# rvoip

**A unified Rust substrate for real-time voice — SIP today, WebRTC + QUIC + AI agents next.**

[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](#-license)
[![Crates.io](https://img.shields.io/crates/v/rvoip-sip.svg?label=rvoip-sip)](https://crates.io/crates/sip/rvoip-sip)
[![Documentation](https://docs.rs/rvoip-sip/badge.svg)](https://docs.rs/rvoip-sip)
[![Repository](https://img.shields.io/badge/github-eisenzopf%2Frvoip-24292f.svg)](https://github.com/eisenzopf/rvoip)

[**📚 Docs**](https://docs.rs/rvoip-sip) · [**🚀 Quick start**](#quick-start) · [**🎯 Build with rvoip**](#build-with-rvoip-today) · [**📊 Feature support**](#feature-support) · [**🏗️ Architecture**](#architecture) · [**🗺️ Roadmap**](#roadmap) · [**💡 Why rvoip**](#why-rvoip)

</div>

---

> [!NOTE]
> **Release status.** Maturity is encoded in the version number (no `-alpha`/`-beta`
> suffixes): **`0.1.x` = alpha, `0.2.x` = beta, `1.0` = stable**. The SIP product
> (`rvoip-sip` + its spine) is a **beta candidate at `0.2.1`** for bounded SIP client,
> server, PBX, gateway, and B2BUA scenarios. The rest of the workspace — WebRTC, QUIC,
> WebTransport, WebSocket, UCTP, vCon, identity, AI harness — is **alpha, published at
> `0.1.0`** (API-unstable; expect breaking changes before `1.0`).
> The [rvoip 3 vision](docs/voip-3-conversation-model.md) describes the destination.

## ⚡ rvoip in one breath

After the SIP era (VoIP 1.0) and the web-meets-voice era of WebRTC + VoiceXML
(VoIP 2.0), **rvoip is the third generation**: SIP, WebRTC, QUIC, and AI-agent
participants share a single transport-agnostic conversation model. One Rust
library hosts all of them. Cross-substrate bridging — SIP ↔ WebRTC ↔ QUIC —
is a first-class primitive, not glue code.

The full design lives under [`docs/`](docs/):

| Doc | What it covers |
| --- | --- |
| [voip-3-conversation-model.md](docs/voip-3-conversation-model.md) | The vocabulary — Conversation, Session, Connection, Stream, Message, Participant |
| [PRD.md](docs/PRD.md) | Product scope, audiences, positioning |
| [INTERFACE_DESIGN.md](docs/INTERFACE_DESIGN.md) | Crate architecture and dependency rules |
| [GAP_PLAN.md](docs/GAP_PLAN.md) | Implementation status (v1 shipped May 2026) |
| [CONVERSATION_PROTOCOL.md](docs/CONVERSATION_PROTOCOL.md) | UCTP wire specification |

<a id="build-with-rvoip-today"></a>
## 🎯 Build with rvoip today

What you can ship right now on the beta:

| 👤 Who you are | 🏗️ What you build | 🔧 Start with |
| --- | --- | --- |
| 📞 **Softphone / endpoint dev** | A SIP account that places, receives, and controls calls | [`Endpoint`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.Endpoint.html) |
| 🧪 **Test / script writer** | Linear test that drives a call from start to finish | [`StreamPeer`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.StreamPeer.html) |
| 🤖 **IVR / contact-center dev** | A reactive server that routes, queues, transfers | [`CallbackPeer`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.CallbackPeer.html) |
| 🔀 **B2BUA / gateway dev** | A back-to-back UA bridging two SIP legs (carrier, SBC, gateway) | [`UnifiedCoordinator`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.UnifiedCoordinator.html) |
| 📋 **Registrar / PBX dev** | A SIP REGISTER service with location bindings | [`rvoip-sip-registrar`](crates/sip/sip-registrar) |
| 🎙️ **Voice-AI agent dev** | A SIP-reachable AI agent (alpha — wire your ASR/TTS via the harness) | `CallbackPeer` + [`rvoip-harness`](crates/extensions/rvoip-harness) (alpha) |

Pick the lowest-ceremony API that gives you what you need. All four sit on the
same `UnifiedCoordinator` underneath; you can drop down a layer without
switching stacks.

<a id="quick-start"></a>
## 🚀 Quick start

```toml
[dependencies]
rvoip-sip = "0.2.1"
tokio = { version = "1", features = ["full"] }
```

A complete two-endpoint local call. **Bob** waits, **Alice** dials, they hold
the line for a second, then hang up.

```rust
use std::time::Duration;
use rvoip_sip::{Config, Endpoint, EndpointProfile};

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    // bob waits for an incoming call
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

    // alice dials bob
    let alice = Endpoint::builder()
        .name("alice")
        .profile(EndpointProfile::Custom(Config::local("alice", 5070)))
        .build()
        .await?;

    let call = alice
        .call_and_wait("sip:bob@127.0.0.1:5071", Some(Duration::from_secs(10)))
        .await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    alice.shutdown().await?;
    bob.await.unwrap()
}
```

Try it:

```sh
cargo run -p rvoip-sip --example endpoint_local_call
```

**New here? Start with the scenario examples in [`examples/`](examples/)** — a
guided, well-documented path from a first P2P call through audio, registration,
call control, transfers, SRTP/TLS, an IVR server, and a B2BUA call center, each a
standalone project with a `./run_demo.sh`.

For per-API-surface reference examples (one lane each for `endpoint`,
`stream_peer`, `callback_peer`, `unified`, plus protocol regression fixtures and
PBX interop), see [`crates/sip/rvoip-sip/examples/`](crates/sip/rvoip-sip/examples/).

<a id="feature-support"></a>
## 📊 Feature support

> ✅ **Beta** (`0.2.1`) = RFC-correct, tested · 🚧 **Alpha** (`0.1.0`) = published,
> API-unstable · 🔮 **Roadmap** = planned, not yet implemented

### 📞 SIP methods (RFC 3261 + extensions)

| Method | Status | RFC | Notes |
| --- | --- | --- | --- |
| INVITE / ACK / BYE | ✅ Beta | 3261 | Full state machines, media coordination |
| CANCEL | ✅ Beta | 3261 | Transaction correlation, glare handled |
| REGISTER | ✅ Beta | 3261 | Contact management, expiration |
| OPTIONS | ✅ Beta | 3261 | Capability negotiation |
| UPDATE | ✅ Beta | 3311 | Mid-session SDP renegotiation |
| PRACK | ✅ Beta | 3262 | Reliable provisionals |
| REFER | ✅ Beta | 3515 | Blind transfer |
| SUBSCRIBE / NOTIFY | ✅ Beta | 6665 | Event packages, subscription state |
| MESSAGE | ✅ Beta | 3428 | In-dialog and pager-mode |
| INFO | ✅ Beta | 6086 | DTMF relay, application data |
| PUBLISH | 🔮 Roadmap | 3903 | Parser support only; app flow post-beta |

### 🎵 Media plane

| Feature | Status | Notes |
| --- | --- | --- |
| G.711 PCMU / PCMA | ✅ Beta | RFC 3551, table-driven |
| RTP / RTCP | ✅ Beta | RFC 3550 |
| SRTP (SDES) | ✅ Beta | RFC 3711 + 4568, tested PBX profiles |
| DTMF (RFC 2833 / 4733) | ✅ Beta | In-band telephone-event payloads |
| Hold / resume | ✅ Beta | Standard `a=sendonly` / `a=inactive` |
| Blind transfer | ✅ Beta | REFER-based, B2BUA-bridged |
| Conference mixing | 🚧 Alpha | N-way mixing primitives in `rvoip-media-core` |
| Opus / G.722 / G.729 | 🔮 Post-beta | Codec hooks exist; full-media path is post-beta |
| DTLS-SRTP | 🔮 Post-beta | Design in place, feature-flagged |
| Echo cancel / AGC / VAD / NS | 🔮 Post-beta | Planned; not yet implemented |

### 🌐 Transport

| Transport | Status | Notes |
| --- | --- | --- |
| UDP | ✅ Beta | Primary transport |
| TCP | ✅ Beta | Connection management, reliability |
| TLS | ✅ Beta | rustls; tested at PBX edge |
| WebSocket (RFC 7118) | 🚧 Partial | Plain WS round-trip works; WSS / browser interop post-beta |
| QUIC (UCTP) | 🚧 Alpha | `rvoip-quic` workspace crate |
| WebTransport | 🚧 Alpha | `rvoip-webtransport` workspace crate |
| WebRTC | 🚧 Alpha | `rvoip-webrtc` pinned to upstream alpha |

### 🔐 Security & identity

| Feature | Status | Notes |
| --- | --- | --- |
| SIP Digest auth (MD5 / SHA-256 / SHA-512-256) | ✅ Beta | RFC 3261 + RFC 8760, qop=auth |
| TLS 1.2 / 1.3 transport | ✅ Beta | Cert validation, custom roots, SNI |
| OAuth 2 / Bearer | ✅ Beta | `rvoip-auth-core` |
| STIR/SHAKEN signing | 🚧 Alpha | `rvoip-stir-shaken` workspace crate |
| OIDC / Passkey / DPoP | 🚧 Alpha | `rvoip-identity` workspace crate |
| ICE / TURN / STUN | 🔮 Post-beta | STUN client landed; ICE/TURN are non-claims |
| ZRTP / MIKEY | 🔮 Post-beta | Not a beta claim |

### 🚀 Performance claim

| Workload | Status | Number |
| --- | --- | --- |
| General full-media SIP | ✅ Beta target | Up to **2,000 CPS** sustained |
| Higher CPS profiles | 🚧 Tuned | Available but caveated; see [BETA_PERFORMANCE_REPORT.md](crates/sip/rvoip-sip/docs/BETA_PERFORMANCE_REPORT.md) |
| 10,000 CPS general-user | 🔮 Roadmap | Tracked in [RELEASE_NOTES_NEXT.md](crates/sip/rvoip-sip/docs/RELEASE_NOTES_NEXT.md) |

The [`crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md`](crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md)
and [`crates/sip/rvoip-sip/docs/SECURITY_POSTURE.md`](crates/sip/rvoip-sip/docs/SECURITY_POSTURE.md)
documents are the authoritative source — this table is a summary.

<a id="architecture"></a>
## 🏗️ Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  📱 Application                                              │
│  (softphone, PBX, contact center, voice AI agent, ...)       │
└──────────────────────────────────────────────────────────────┘
                              ▲
                              │ rvoip-sip API surface
                              ▼
┌──────────────────────────────────────────────────────────────┐
│  📞 rvoip-sip       (✅ beta)  SIP-shaped session layer       │
│  ┌─────────────┬───────────────┬─────────────┬─────────────┐ │
│  │ sip-core    │ sip-transport │ sip-dialog  │ sip-proxy   │ │
│  │             │               │             │ sip-registrar│ │
│  └─────────────┴───────────────┴─────────────┴─────────────┘ │
└──────────────────────────────────────────────────────────────┘
                              ▲
                              │ ConnectionAdapter trait
                              ▼
┌──────────────────────────────────────────────────────────────┐
│  🧬 rvoip-core      (✅ beta)  transport-agnostic spine       │
│  rvoip-core-traits  (✅ beta)  cycle-breaker trait surface    │
│  rvoip-media-core   (✅ beta)  codec / mixing / MediaStream   │
│  rvoip-rtp-core     (✅ beta)  RTP / SRTP                     │
│  rvoip-codec-core   (✅ beta)  G.711 base codec               │
│  rvoip-auth-core    (✅ beta)  OAuth2 / Bearer / SIP Digest   │
└──────────────────────────────────────────────────────────────┘
                              ▲
                              │ UCTP (🚧 alpha)
                              ▼
┌──────────────────────────────────────────────────────────────┐
│  🚧 Substrate adapters  (alpha — not in beta closure)         │
│  ┌─────────┬──────────┬───────────────┬───────────────┐      │
│  │ rvoip-  │ rvoip-   │ rvoip-        │ rvoip-        │      │
│  │  webrtc │  quic    │  webtransport │  websocket    │      │
│  └─────────┴──────────┴───────────────┴───────────────┘      │
│  rvoip-uctp · rvoip-vcon · rvoip-harness · rvoip-identity    │
└──────────────────────────────────────────────────────────────┘
```

**The dependency direction is enforced.** `rvoip-core` never imports an adapter
crate. Adapters depend on `rvoip-core` and register themselves via
`ConnectionAdapter`. This is what lets a single `Orchestrator` bridge a SIP
call to a WebRTC client (and, later, to a QUIC peer or an AI participant)
without the substrates knowing about each other.

## 📦 Crate matrix

### ✅ Beta — published to crates.io as `0.2.1`

| Crate | Purpose |
| --- | --- |
| **[rvoip](crates/rvoip)** | Facade — opt into transports/extensions via features (default `sip`) |
| **[rvoip-sip](crates/sip/rvoip-sip)** | SIP umbrella — `Endpoint` / `StreamPeer` / `CallbackPeer` / `UnifiedCoordinator` |
| [rvoip-sip-core](crates/sip/sip-core) | RFC 3261 message parsing, SDP, URIs |
| [rvoip-sip-transport](crates/sip/sip-transport) | UDP / TCP / TLS / WebSocket transport |
| [rvoip-sip-dialog](crates/sip/sip-dialog) | Dialog state machine + transaction layer |
| [rvoip-sip-proxy](crates/sip/sip-proxy) | Stateful SIP proxy primitives (RFC 3261 §16) |
| [rvoip-sip-registrar](crates/sip/sip-registrar) | REGISTER processing + location service |
| [rvoip-core](crates/foundation/rvoip-core) | Transport-agnostic spine: Conversation / Session / ConnectionAdapter |
| [rvoip-core-traits](crates/foundation/rvoip-core-traits) | Cycle-breaker trait + type surface |
| [rvoip-infra-common](crates/foundation/infra-common) | Event bus, executors, shared infra |
| [rvoip-media-core](crates/media/media-core) | Codec negotiation, mixing, MediaStream trait |
| [rvoip-rtp-core](crates/media/rtp-core) | RTP / SRTP framing and transport |
| [rvoip-codec-core](crates/media/codec-core) | G.711 codec implementation |
| [rvoip-auth-core](crates/identity/auth-core) | OAuth2 + Bearer + token primitives |

### 🚧 Alpha — published to crates.io at `0.1.0`

These publish at `0.1.0` (API-unstable) so the [`rvoip`](crates/rvoip) facade can expose
them behind feature flags (`webrtc`, `uctp`, `voip-3`, `sip-stir-shaken`, `client`). Expect
breaking changes before each graduates to beta.

| Crate | Why it's alpha |
| --- | --- |
| [rvoip-client](crates/rvoip-client) | Client SDK — API still in motion |
| [rvoip-uctp](crates/uctp/rvoip-uctp) | UCTP protocol design ongoing ([GAP_PLAN](docs/GAP_PLAN.md)) |
| [rvoip-quic](crates/uctp/rvoip-quic) | New QUIC substrate adapter |
| [rvoip-webtransport](crates/uctp/rvoip-webtransport) | New WebTransport substrate adapter |
| [rvoip-websocket](crates/uctp/rvoip-websocket) | Deferred per rvoip 3 v1.x |
| [rvoip-webrtc](crates/webrtc/rvoip-webrtc) | Pinned to upstream `webrtc 0.20.0-alpha.1` |
| [rvoip-vcon](crates/extensions/rvoip-vcon) | First Rust impl of the IETF vCon draft — publishes |
| [rvoip-harness](crates/extensions/rvoip-harness) | ASR / TTS / DialogManager provider traits — publishes |
| [rvoip-identity](crates/identity/rvoip-identity) | OAuth 2.1 + OIDC + SIP Digest + Passkey backends |
| [rvoip-stir-shaken](crates/extensions/rvoip-stir-shaken) | STIR/SHAKEN signing + verification |
| [rvoip-users-core](crates/identity/users-core) | Reference user-management service |

<a id="roadmap"></a>
## 🗺️ Roadmap

Tracked in detail under [`docs/GAP_PLAN.md`](docs/GAP_PLAN.md).
Highlights below.

### 🚧 v1.x — incremental on rvoip 3 v1

- `rvoip-websocket` substrate adapter (graduate WS to ✅ beta)
- Full **AAuth** production status (waiting on IETF WG adoption)
- **DTLS-SRTP** fingerprint binding (feature-flagged, design in place)
- **vCon Postgres** reference store (`rvoip-vcon-postgres`)
- Inline envelope signature enforcement at adapter ingress

### 🔮 v2 — next major

- **SIP-over-QUIC** adapter
- **RTP-over-QUIC** (RoQ)
- **Media-over-QUIC** (MoQ) for broadcast fan-out
- Multi-party **SFU / MCU** integration (LiveKit / mediasoup)
- **AI agents as first-class peer Participants** in multi-agent flows

<a id="why-rvoip"></a>
## 💡 Why rvoip

Contact centers, CPaaS providers, and voice-AI platforms in 2025–2026 stitch
together a polyglot stack: **FreeSWITCH or Asterisk** for SIP, **Janus or
mediasoup** for WebRTC, **RTPEngine** for media bridging, and **custom Lua /
Python / Erlang** for orchestration glue. rvoip targets the same workload as a
single Rust process — SIP, WebRTC, and (eventually) UCTP substrates handled by
one library, with bridging and transcoding as first-class primitives.

### 🎙️ Rust-native voice AI infrastructure

The Vapi / Retell / Bland / OpenAI-Realtime cohort proved real-time voice AI
is a venture-scale market. **rvoip is the first end-to-end Rust substrate
aimed at that category** — SIP B2BUA, AI harness with clean ASR/TTS/Dialog
provider traits, WebRTC interop for browser users, and a single
command/event surface for all of it.

### 🔄 FreeSWITCH + Janus replacement

For new builds that don't need the legacy footprint. Single Rust binary,
async-first, memory-safe.

### ☎️ Carrier-grade pure SIP

For carriers and ITSPs needing SIP trunking, PSTN interconnect, codec
negotiation, STIR/SHAKEN passthrough, and billing-grade usage records
**without paying for orchestration features they don't need**.

### 📜 First-mover Rust adoption of vCon

For the conversation-compliance market — `rvoip-vcon` is the first Rust
implementation of the IETF vCon draft.

### 🛰️ Architectural runway for QUIC media

The UCTP substrate model gives rvoip a place to land SIP-over-QUIC, RoQ,
and MoQ when those mature (2027–2029) **without breaking the SIP path**.

See [`docs/PRD.md`](docs/PRD.md) §1.2 for the
full positioning analysis.

## 🧪 Evaluating rvoip

```sh
# Get the source
git clone https://github.com/eisenzopf/rvoip.git
cd rvoip

# Build the workspace
cargo build --workspace

# Run a working example
cargo run -p rvoip-sip --example endpoint_local_call

# Run the SIP test suite
cargo test -p rvoip-sip -p rvoip-sip-core -p rvoip-sip-dialog \
            -p rvoip-sip-transport -p rvoip-sip-proxy -p rvoip-sip-registrar
```

Run the workspace test suite:

```sh
scripts/test_all.sh                  # workspace-wide test runner
```

Treat this release as a beta candidate. The beta scope is documented above —
anything beyond it is the caller's responsibility to validate with their own
interop, security, and performance gates.

## 🤝 Contributing

- 🐛 **Bugs**: open an issue with reproduction steps
- 💡 **Feature requests**: discussions or issues — please reference the
  [rvoip 3 docs](docs/voip-3-conversation-model.md) for context
- 🔧 **Pull requests welcome** — workspace-wide tests run via
  `scripts/test_all.sh`

<a id="license"></a>
## 📄 License

Licensed under the **MIT** license. See [LICENSE](LICENSE).

<div align="center">

---

**Built with ❤️ in Rust** · [📚 Docs](https://docs.rs/rvoip-sip) · [💡 Examples](examples/) · [🐛 Issues](https://github.com/eisenzopf/rvoip/issues) · [💬 Discussions](https://github.com/eisenzopf/rvoip/discussions)

</div>
