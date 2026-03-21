<div align="center">
  <img src="rvoip-banner.svg" alt="rvoip - The modern VoIP stack" width="50%" />
</div>

<div align="center">

[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/openprx/rvoip#license)
[![Build Status](https://img.shields.io/github/workflow/status/openprx/rvoip/CI)](https://github.com/openprx/rvoip/actions)
[![Crates.io](https://img.shields.io/crates/v/rvoip.svg)](https://crates.io/crates/rvoip)
[![Documentation](https://docs.rs/rvoip/badge.svg)](https://docs.rs/rvoip)

**A comprehensive, 100% pure Rust implementation of a SIP/VoIP stack**

[📚 Documentation](https://docs.rs/rvoip) • [🚀 Quick Start](#-quick-start) • [💡 Examples](examples/) • [🏢 Enterprise](#-enterprise-deployment)

</div>

---

> **Beta Quality** - Core SIP, transport, security, and media features are complete and functional. APIs are stabilizing but may still see changes as we move toward 1.0. The intent is to make this library production-ready for enterprise VoIP deployments. We are in the process of doing real-world testing and would appreciate any feedback, feature requests, contributions, or bug reports.

## 📋 Table of Contents

- [🚀 Quick Start](#-quick-start)
- [🎯 Library Purpose](#-library-purpose)
- [📦 Library Structure](#-library-structure)
- [🔧 Core Crates](#-core-crates)
- [🚀 SIP Protocol Features](#-sip-protocol-features)
- [🧪 Testing & Quality](#-testing--quality)
- [🏢 Enterprise Deployment](#-enterprise-deployment)
- [📄 License](#-license)

---

rvoip is a comprehensive, 100% pure Rust implementation of a SIP/VoIP stack designed to handle, route, and manage phone calls at scale. Built from the ground up with modern Rust practices, it provides a robust, efficient, and secure foundation for VoIP applications ranging from simple softphones to enterprise call centers. This library is meant as a foundation to build SIP clients and servers that could in the future provide an alternative to open source systems like FreeSWITCH and Asterisk as well as commercial systems like Avaya and Cisco.

## 🚀 Quick Start

### 📦 Installation

Add rvoip to your `Cargo.toml`:

```toml
[dependencies]
rvoip = { version = "0.1", features = ["full"] }
tokio = { version = "1.0", features = ["full"] }
```

### ⚡ 30-Second SIP Server

```rust
use rvoip::session_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let session_manager = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .build().await?;

    println!("✅ SIP server running on port 5060");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

### 📞 Make Your First Call

```rust
use rvoip::client_core::{ClientConfig, ClientManager, MediaConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ClientConfig::new()
        .with_sip_addr("127.0.0.1:5060".parse()?)
        .with_media_addr("127.0.0.1:20000".parse()?)
        .with_user_agent("MyApp/1.0".to_string())
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            ..Default::default()
        });

    let client = ClientManager::new(config).await?;
    client.start().await?;

    let call_id = client.make_call(
        "sip:alice@127.0.0.1".to_string(),
        "sip:bob@example.com".to_string(),
        None
    ).await?;

    println!("📞 Call initiated to bob@example.com");
    Ok(())
}
```

### 🏢 Enterprise Call Center

```rust
use rvoip::call_engine::{prelude::*, CallCenterServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "0.0.0.0:5060".parse()?;
    config.general.domain = "127.0.0.1".to_string();

    let mut server = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database_path(":memory:".to_string())
        .build()
        .await?;

    server.start().await?;
    println!("🏢 Call Center Server starting...");
    server.run().await?;
    Ok(())
}
```

> 💡 **More Examples**: Check out the [examples/](examples/) directory for complete working applications including peer-to-peer calling, audio streaming, and call center implementations.

## 🎯 Library Purpose

<div align="center">

| 🦀 **Pure Rust** | 🏗️ **Modular** | 📋 **RFC Compliant** | 🔶 **Beta** |
|:---:|:---:|:---:|:---:|
| Zero FFI dependencies | Clean separation of concerns | Standards-compliant SIP | Core features complete, APIs stabilizing |
| Memory safety & performance | Specialized crates | Extensive RFC support | Real-world testing in progress |

</div>

rvoip is a pure Rust set of libraries built from the ground up and follows SIP best practices for separation of concerns:

- 🦀 **Pure Rust Implementation**: Zero FFI dependencies, leveraging Rust's safety and performance
- 🏗️ **Modular Architecture**: Clean separation of concerns across specialized crates
- 📋 **RFC Compliance**: Standards-compliant SIP implementation with extensive RFC support
- 🔶 **Beta Quality**: Core features complete, designed for enterprise deployment, real-world testing in progress
- 👨‍💻 **Developer Friendly**: Multiple API levels from low-level protocol to high-level applications

## 📦 Library Structure

rvoip is organized into 17 crates, each with specific responsibilities in the VoIP stack:

### 🏗️ Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │ call-engine  │  │ client-core  │  │  sip-client  │      │
│  │(Call Center) │  │ (SIP Client) │  │(Simple API)  │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
├─────────────────────────────────────────────────────────────┤
│               Session & Coordination Layer                  │
│                   ┌─────────────────┐                       │
│                   │  session-core   │                       │
│                   │ (Session Mgmt)  │                       │
│                   └─────────────────┘                       │
├─────────────────────────────────────────────────────────────┤
│               Protocol & Processing Layer                   │
│  ┌─────────────────┐  ┌─────────────────┐                   │
│  │   dialog-core   │  │   media-core    │                   │
│  │  (SIP Dialogs   │  │ (Audio Process) │                   │
│  │  + Transactions) │  └─────────────────┘                   │
│  └─────────────────┘                                        │
├─────────────────────────────────────────────────────────────┤
│               Transport & Media Layer                       │
│  ┌─────────────────┐  ┌─────────────────┐                   │
│  │ sip-transport   │  │   rtp-core      │                   │
│  │ (SIP Transport) │  │ (RTP/SRTP)      │                   │
│  └─────────────────┘  └─────────────────┘                   │
├─────────────────────────────────────────────────────────────┤
│                    Foundation Layer                         │
│  ┌─────────────────┐  ┌──────────────┐  ┌──────────────┐   │
│  │    sip-core     │  │  codec-core  │  │  audio-core  │   │
│  │ (SIP Protocol)  │  │  (Codecs)    │  │  (Audio DSP) │   │
│  └─────────────────┘  └──────────────┘  └──────────────┘   │
├─────────────────────────────────────────────────────────────┤
│                    Support Crates                           │
│  ┌───────────────┐ ┌────────────────┐ ┌──────────────────┐ │
│  │ infra-common  │ │ registrar-core │ │ intermediary-core│ │
│  └───────────────┘ └────────────────┘ └──────────────────┘ │
│  ┌───────────────┐ ┌────────────────┐ ┌──────────────────┐ │
│  │  users-core   │ │   auth-core*   │ │      rvoip       │ │
│  └───────────────┘ └────────────────┘ │    (facade)      │ │
│                                       └──────────────────┘ │
│  * auth-core exists on disk but is not yet a workspace     │
│    member                                                  │
└─────────────────────────────────────────────────────────────┘
```

## 🔧 Core Crates

<details>
<summary><strong>📞 call-engine</strong> - Call Center Orchestration</summary>

**Purpose**: Proof of concept call center orchestration with agent management, queuing, and routing
**Status**: ⚠️ **Alpha** — ~70% complete, not production tested

**🎯 Key Features**:
- 👥 Agent SIP registration and status management
- 🗄️ Database-backed call queuing with priority handling
- ⚖️ Round-robin load balancing and overflow management
- 🔗 B2BUA call bridging with bidirectional audio
- 📊 Real-time queue monitoring and statistics

**💼 Use Cases**: Call centers, customer support, sales teams, enterprise telephony

</details>

<details>
<summary><strong>📱 client-core</strong> - High-Level SIP Client Library</summary>

**Purpose**: Simplified SIP client library for building VoIP applications
**Status**: ⚠️ **Alpha** — Complete client functionality with comprehensive API but not yet tested in production. API will change significantly.

**🎯 Key Features**:
- 📞 High-level call management (make, answer, hold, transfer, terminate)
- 🎛️ Media controls with quality monitoring
- ⚡ Event-driven architecture for UI integration
- 🔧 Intuitive APIs with builder patterns
- 🛡️ Comprehensive error handling

**💼 Use Cases**: Softphones, VoIP apps, mobile clients, desktop applications

</details>

<details>
<summary><strong>📱 sip-client</strong> - Simplified SIP Client</summary>

**Purpose**: Higher-level SIP client with `simple-api` feature for rapid prototyping
**Status**: ⚠️ **Alpha** — Wraps client-core with a simpler interface

**🎯 Key Features**:
- 🔧 Simple API feature gate for easy usage
- 📞 Streamlined call operations
- ⚡ Quick prototyping for SIP applications

**💼 Use Cases**: Rapid prototyping, simple SIP integrations

</details>

<details>
<summary><strong>🎛️ session-core</strong> - Session Management Hub</summary>

**Purpose**: Central coordination for SIP sessions, media, and call control
**Status**: 🔶 **Beta** — Core session management with comprehensive API. Includes state-table feature for deterministic state machine behavior. Digest Auth (RFC 2617/7616) and DTLS-SRTP encryption are fully integrated.

**🎯 Key Features**:
- 🔄 Session lifecycle management from creation to termination
- 🤝 SIP-Media coordination with real media-core integration
- 🎮 Call control operations (hold, resume, transfer, bridge)
- ⚡ Event-driven architecture with session state management
- 📋 State-table feature for deterministic, table-driven state transitions

**💼 Use Cases**: VoIP platform foundation, session coordination, call control

</details>

<details>
<summary><strong>💬 dialog-core</strong> - SIP Dialog & Transaction Management</summary>

**Purpose**: RFC 3261 compliant SIP dialog state machine, message routing, and transaction layer (transaction-core was merged into this crate)
**Status**: ⚠️ **Alpha** — Full dialog lifecycle management and transaction support. Missing some SIP RFC extensions.

**🎯 Key Features**:
- 📋 Complete RFC 3261 dialog state machine implementation
- 🔁 SIP transaction layer with automatic retransmission and timeouts
- 🚀 Early and confirmed dialog management
- 📱 Client and server transaction support
- 🧭 In-dialog request routing and state tracking
- 🔧 Dialog recovery and cleanup mechanisms

**💼 Use Cases**: SIP protocol implementation, dialog state management, reliable message delivery

</details>

<details>
<summary><strong>🎧 media-core</strong> - Media Processing Engine</summary>

**Purpose**: Audio processing, codec management, and media session coordination
**Status**: ⚠️ **Alpha** — Advanced audio processing with quality monitoring but not yet tested in production.

**🎯 Key Features**:
- 🎙️ Advanced audio processing (AEC, AGC, VAD, noise suppression)
- 🎤 Multi-codec support via codec-core
- 📈 Real-time quality monitoring and MOS scoring
- 🎵 Conference mixing (mixer exists, session integration TODO)

**💼 Use Cases**: VoIP audio processing, codec transcoding, media quality

</details>

<details>
<summary><strong>📡 rtp-core</strong> - RTP/RTCP Implementation</summary>

**Purpose**: Real-time media transport with comprehensive RTP/RTCP support, security protocols, and NAT traversal
**Status**: 🔶 **Beta** — Core RTP stack functional, DTLS-SRTP complete, full ICE/STUN/TURN support.

**🎯 Key Features**:
- 📋 Complete RFC 3550 RTP/RTCP implementation
- 🔒 SRTP/SRTCP with AES-CM (AEAD GCM not yet implemented)
- 🔐 DTLS-SRTP complete (full handshake, cipher activation, media integration)
- 🔐 ZRTP (simplified implementation, not full RFC 6189)
- 🔑 MIKEY-PSK and MIKEY-PKE (PKE is framework-only, crypto placeholder)
- 🧊 Full ICE agent (gathering, connectivity checks, trickle ICE, consent freshness)
- 📡 STUN client (binding requests, NAT detection)
- 🔄 TURN client (relay allocation, channel binding)
- 📈 Adaptive jitter buffering and quality monitoring

**💼 Use Cases**: Secure media transport, RTP streaming, WebRTC compatibility, NAT traversal

</details>

<details>
<summary><strong>🌐 sip-transport</strong> - SIP Transport Layer</summary>

**Purpose**: Multi-protocol SIP transport
**Status**: 🔶 **Beta** — All transports complete: UDP, TCP, TLS, WebSocket (WS + WSS, client + server).

**🎯 Key Features**:
- 🔌 UDP and TCP transport (complete)
- 🔒 TLS transport (complete)
- 🌐 WebSocket transport (WS + WSS, client + server)
- 🏭 Transport factory for URI-based selection
- 🔧 Error handling and recovery mechanisms
- ⚡ Event-driven architecture

**💼 Use Cases**: SIP network transport, protocol abstraction, secure signaling

</details>

<details>
<summary><strong>🔧 sip-core</strong> - SIP Protocol Foundation</summary>

**Purpose**: Core SIP message parsing, serialization, and validation
**Status**: ⚠️ **Alpha** — Complete RFC 3261 implementation with strict and lenient parsing modes. Missing some SIP RFC extensions.

**🎯 Key Features**:
- 📋 RFC 3261 compliant message parsing and serialization
- 📝 60+ standard SIP headers with typed representations
- 🌐 Complete SDP support with WebRTC extensions
- 🔧 Multiple APIs (low-level, builders, macros)
- 🔗 Comprehensive URI processing (SIP, SIPS, TEL)

**💼 Use Cases**: SIP protocol foundation, message processing, parser

</details>

<details>
<summary><strong>🎵 codec-core</strong> - Audio Codec Library</summary>

**Purpose**: Audio codec implementations and codec negotiation
**Status**: ⚠️ **Alpha** — G.711 complete, others at varying stages

**🎯 Key Features**:
- 🎤 G.711 PCMU/PCMA codec (complete)
- 🔊 Opus codec (behind optional feature gate)
- 📋 Codec negotiation and selection framework

**💼 Use Cases**: Audio encoding/decoding, codec management

</details>

<details>
<summary><strong>🔊 audio-core</strong> - Audio DSP Library</summary>

**Purpose**: Low-level audio processing primitives
**Status**: ⚠️ **Alpha** — Core DSP functionality

**🎯 Key Features**:
- 🎙️ Audio sample processing and conversion
- 📈 Signal processing utilities
- 🔧 Audio buffer management

**💼 Use Cases**: Audio processing foundation, DSP operations

</details>

<details>
<summary><strong>🏗️ infra-common</strong> - Shared Infrastructure</summary>

**Purpose**: Common utilities and types shared across crates
**Status**: ⚠️ **Alpha**

**💼 Use Cases**: Internal shared code, common types

</details>

<details>
<summary><strong>📋 registrar-core</strong> - SIP Registrar</summary>

**Purpose**: SIP registration and contact management
**Status**: ⚠️ **Alpha**

**🎯 Key Features**:
- 📋 SIP REGISTER request handling
- 👤 Contact and binding management

**💼 Use Cases**: SIP registrar server, user location service

</details>

<details>
<summary><strong>👤 users-core</strong> - User Management</summary>

**Purpose**: User account and identity management for SIP systems
**Status**: ⚠️ **Alpha** — Not included in default workspace members

**💼 Use Cases**: User provisioning, identity management

</details>

<details>
<summary><strong>🔀 intermediary-core</strong> - SIP Proxy/Intermediary</summary>

**Purpose**: SIP proxy and intermediary functionality
**Status**: ⚠️ **Alpha**

**💼 Use Cases**: SIP proxy servers, routing logic

</details>

<details>
<summary><strong>📦 rvoip</strong> - Facade Crate</summary>

**Purpose**: Re-exports all crates under the `rvoip::*` namespace
**Status**: ⚠️ **Alpha**

**🎯 Key Features**:
- 🔗 Unified import path: `rvoip::session_core::prelude::*`
- 📦 Single dependency for full stack access

**💼 Use Cases**: Application development, unified API access

</details>

## 🚀 SIP Protocol Features

### 📋 Core SIP Methods Support

| Method | Status | RFC | Description | Implementation |
|--------|--------|-----|-------------|----------------|
| **INVITE** | ✅ Complete | RFC 3261 | Session initiation and modification | Full state machine, media coordination |
| **ACK** | ✅ Complete | RFC 3261 | Final response acknowledgment | Automatic generation, dialog correlation |
| **BYE** | ✅ Complete | RFC 3261 | Session termination | Proper cleanup, B2BUA forwarding |
| **CANCEL** | ✅ Complete | RFC 3261 | Request cancellation | Transaction correlation, state management |
| **REGISTER** | ✅ Complete | RFC 3261 | User registration | Contact management, expiration handling |
| **OPTIONS** | ✅ Complete | RFC 3261 | Capability discovery | Method advertisement, feature negotiation |
| **UPDATE** | ✅ Complete | RFC 3311 | Session modification | Mid-session updates, SDP negotiation |
| **SUBSCRIBE** | ✅ Complete | RFC 6665 | Event notification subscription | Full subscribe/notify lifecycle with callbacks |
| **NOTIFY** | ✅ Complete | RFC 6665 | Event notifications | Send + receive with presence integration |
| **MESSAGE** | ✅ Complete | RFC 3428 | Instant messaging | Inbound dispatch + outbound builder |
| **INFO** | ✅ Complete | RFC 6086 | Mid-session information | DTMF + trickle ICE support |
| **REFER** | ✅ Complete | RFC 3515 | Call transfer initiation | Blind + attended transfer |
| **PRACK** | ✅ Complete | RFC 3262 | Provisional response acknowledgment | Full handler with 200 OK response |
| **PUBLISH** | ✅ Complete | RFC 3903 | Event state publication | Full handler for event state publication |

### 🔐 Authentication & Security

| Feature | Status | Algorithms | RFC | Description |
|---------|--------|------------|-----|-------------|
| **Digest Authentication** | ✅ Complete | MD5, SHA-256, SHA-512-256 | RFC 2617/7616 | Challenge-response authentication |
| **Quality of Protection** | ✅ Complete | auth, auth-int | RFC 2617 | Integrity protection levels |
| **DTLS-SRTP** | ✅ Complete | ECDHE, RSA | RFC 5763 | Full handshake, cipher activation, media integration |
| **TLS Transport** | ✅ Complete | TLS 1.2/1.3 | RFC 3261 | Secure SIP signaling transport |
| **SDES-SRTP** | ✅ Complete | SDP-based | RFC 4568 | SIP signaling key exchange |
| **MIKEY-PSK** | ✅ Complete | Pre-shared keys | RFC 3830 | Enterprise key management |
| **SRTP/SRTCP** | ✅ Complete | AES-CM, HMAC-SHA1, AES-128-GCM, AES-256-GCM | RFC 3711/7714 | Full SRTP with AEAD-GCM support |
| **ZRTP** | ✅ Complete | DH, SAS | RFC 6189 | Key exchange, SAS verification, full handshake |
| **MIKEY** | ✅ Complete | PSK, PKE, DH (ECDH P-256) | RFC 3830 | All three key exchange modes implemented |

### 🎵 Media & Codec Support

| Category | Feature | Status | Standards | Description |
|----------|---------|--------|-----------|-------------|
| **Audio Codecs** | G.711 PCMU/PCMA | ✅ Complete | ITU-T G.711 | u-law/A-law, 8kHz |
| | Opus | ✅ Complete | RFC 6716 | Real encode/decode, feature-gated |
| | G.722 | ✅ Complete | ITU-T G.722 | Pure Rust ADPCM + QMF sub-band coding, 16kHz |
| | G.729 | 🔶 Partial | ITU-T G.729 | Framework + config complete, codec engine WIP |
| **Audio Processing** | Echo Cancellation | ✅ Complete | Advanced AEC | 16.4 dB ERLE improvement |
| | Gain Control | ✅ Complete | Advanced AGC | Multi-band processing |
| | Voice Activity | ✅ Complete | Advanced VAD | Spectral analysis |
| | Noise Suppression | ✅ Complete | Spectral NS | Real-time processing |
| **RTP Features** | RTP/RTCP | ✅ Complete | RFC 3550 | Packet transport, statistics |
| | RTCP Feedback | ✅ Complete | RFC 4585 | Quality feedback |
| | RTP Extensions | ✅ Complete | RFC 8285 | Header extensions |
| **Conference** | Audio Mixing | 🔶 Partial | N-way mixing | Mixer exists, session integration TODO |
| | Media Bridging | 🔶 Partial | B2BUA | Call bridging (B2BUA in progress) |

### 🌐 Transport Protocol Support

| Transport | Status | Security | RFC | Description |
|-----------|--------|----------|-----|-------------|
| **UDP** | ✅ Complete | Optional SRTP | RFC 3261 | Primary SIP transport |
| **TCP** | ✅ Complete | — | RFC 3261 | Reliable transport |
| **WebSocket** | ✅ Complete | WS + WSS | RFC 7118 | Client + server, secure WebSocket |
| **TLS** | ✅ Complete | TLS 1.2/1.3 | RFC 3261 | Secure SIP signaling |
| **SCTP** | ✅ Complete | DTLS-SCTP | RFC 4960 | DTLS-SCTP data channels (not SIP-over-SCTP) |

### 🔌 NAT Traversal Support

| Feature | Status | RFC | Description |
|---------|--------|-----|-------------|
| **Symmetric RTP** | ✅ Complete | RFC 4961 | Bidirectional media flow |
| **ICE** | ✅ Complete | RFC 8445 | Full agent, trickle ICE, consent freshness |
| **STUN Client** | ✅ Complete | RFC 5389 | Binding requests, NAT detection |
| **TURN Client** | ✅ Complete | RFC 5766 | Relay allocation, channel binding |

### 📞 Dialog & Session Management

| Feature | Status | RFC | Description |
|---------|--------|-----|-------------|
| **Early Dialogs** | ✅ Complete | RFC 3261 | 1xx response handling |
| **Confirmed Dialogs** | ✅ Complete | RFC 3261 | 2xx response handling |
| **Dialog Recovery** | ✅ Complete | RFC 3261 | State persistence |
| **Session Timers** | ✅ Complete | RFC 4028 | Keep-alive mechanism |
| **Dialog Forking** | ✅ Complete | RFC 3261 | Parallel/sequential forking |

### 📋 SDP (Session Description Protocol)

| Feature | Status | RFC | Description |
|---------|--------|-----|-------------|
| **Core SDP** | ✅ Complete | RFC 8866 | Session description |
| **WebRTC Extensions** | ✅ Complete | Various | Modern web compatibility |
| **ICE Attributes** | ✅ Complete | RFC 8839 | Connectivity attributes |
| **DTLS Fingerprints** | ✅ Complete | RFC 8122 | Security fingerprints |
| **Media Grouping** | ✅ Complete | RFC 5888 | BUNDLE support |
| **Simulcast** | ✅ Complete | RFC 8853 | Multiple stream support |

### 🎛️ Advanced Features

| Feature | Status | Description |
|---------|--------|-------------|
| **Call Hold/Resume** | ✅ Complete | Full hold/resume in client-core and sip-client |
| **Call Transfer** | ✅ Complete | Blind + attended transfer |
| **DTMF Support** | ✅ Complete | SIP INFO + RFC 4733 RTP events (dual mode) |
| **Conference Mixing** | ✅ Complete | AudioMixer + session integration + SDP |
| **Call Center Operations** | ✅ Complete | Agent management, queuing, routing, bridging |
| **Media Quality Monitoring** | ✅ Complete | Real-time MOS scoring |
| **B2BUA Operations** | ✅ Complete | Dual-leg management, SDP rewrite, header manipulation |



## 🧪 Testing & Quality

### 4-Level Test Architecture

| Level | Scope | Command |
|-------|-------|---------|
| **L1 Unit** | Per-crate isolated tests | `./scripts/test_all.sh unit` |
| **L2 Adapter** | Production library adapter roundtrips | `./scripts/test_all.sh adapter` |
| **L3 Integration** | Cross-crate module integration | `./scripts/test_all.sh integration` |
| **L4 End-to-End** | Complete call paths with audio | `./scripts/test_all.sh e2e` |

### Test Coverage
- **2,500+ unit tests** across 14 crates
- **Cross-crate integration tests** in dedicated `integration-tests` crate
- **Adapter roundtrip tests** for all 7 production library migrations (ICE, SCTP, RTP, RTCP, SRTP, DTLS, STUN)
- **E2E call tests** with G.711 audio verification, DTMF, hold/resume, encryption
- **RFC compliance** torture tests based on RFC 4475
- **Zero `unwrap()`** in production code, zero compiler warnings

### Quality Assurance
- **Production library adapters**: ICE, SCTP, SRTP, DTLS, STUN, RTP/RTCP backed by webrtc-rs (3M+ downloads)
- **Audio DSP**: Optional Google WebRTC AudioProcessing Module via `webrtc-apm` feature
- **Graceful shutdown**: Broadcast signal with timeout for all spawned tasks
- **Structured logging**: All `println` replaced with `tracing` macros

## 📋 Development Status

### 🔶 Beta Components
Core crates are **beta quality**. The architecture is stable and APIs are stabilizing.

- **sip-core**: RFC 3261 implementation, strict and lenient parsing
- **dialog-core**: Dialog state machine + merged transaction layer
- **session-core**: Session management with state-table feature, Digest Auth + DTLS-SRTP
- **media-core**: Audio processing (AEC, AGC, VAD, NS)
- **rtp-core**: RTP/RTCP with DTLS-SRTP, ICE, STUN, TURN
- **sip-transport**: UDP, TCP, TLS, WebSocket (WS + WSS) all complete
- **client-core**: High-level client framework
- **call-engine**: Call center orchestration
- **codec-core**: G.711, G.722, G.729A (pure Rust), Opus (feature-gated)
- **audio-core**: Core audio DSP

### 🚧 Known Gaps
- **Video codecs**: No H.264/VP8/VP9 encoding (audio-only currently)
- **SIP-over-SCTP**: Only DTLS-SCTP data channels, not SIP transport (RFC 4168)

### 🔮 Roadmap
- **Video codec support**: H.264, VP8, VP9 for video calling
- **SIP-over-SCTP**: RFC 4168 multi-streaming SIP transport
- **WebRTC Gateway**: Full browser-to-SIP interoperability
- **Mobile SDKs**: iOS and Android bindings via FFI
- **Clustering/HA**: High availability and horizontal scaling
- **REST/GraphQL API**: Management and monitoring interfaces

## 🏢 Enterprise Deployment

rvoip is designed for enterprise use cases with core features now at beta quality. The architecture supports:

### Deployment Options
- **Standalone**: Single binary deployment
- **Containerized**: Docker/Kubernetes ready
- **Cloud Native**: AWS/GCP/Azure optimized
- **On-Premises**: Traditional server deployment

### Design Goals
- **Event-Driven**: Real-time monitoring and control
- **Modular**: Use only the crates you need
- **Secure**: Enterprise-grade encryption and authentication (TLS, DTLS-SRTP, Digest Auth)
- **Reliable**: Comprehensive error handling and recovery

## 🤝 Contributing

We welcome contributions! Here's how you can help:

- 🐛 **Report bugs** - Open an issue with detailed reproduction steps
- 💡 **Suggest features** - Share your ideas for improvements
- 🔧 **Submit PRs** - Fix bugs or implement new features
- 📖 **Improve docs** - Help make our documentation better
- 🧪 **Add tests** - Increase our test coverage

<div align="center">

[![Contributors](https://img.shields.io/github/contributors/openprx/rvoip.svg)](https://github.com/openprx/rvoip/graphs/contributors)
[![Issues](https://img.shields.io/github/issues/openprx/rvoip.svg)](https://github.com/openprx/rvoip/issues)
[![Pull Requests](https://img.shields.io/github/issues-pr/openprx/rvoip.svg)](https://github.com/openprx/rvoip/pulls)

</div>

## 📄 License

Licensed under either of:
- Apache License, Version 2.0
- MIT License

at your option.

---

<div align="center">

### 🚀 Ready to Build the Future of VoIP?

**[📚 Read the Docs](https://docs.rs/rvoip)** • **[💡 Try Examples](examples/)** • **[🐛 Report Issues](https://github.com/openprx/rvoip/issues)** • **[💬 Join Discussions](https://github.com/openprx/rvoip/discussions)**

---

**💡 Ready to get started?** Check out the [examples](examples/) directory for working code samples, or dive into the individual crate documentation for detailed usage patterns.

**Core features are beta quality** with complete SIP signaling, secure transport (TLS, DTLS-SRTP), NAT traversal (ICE/STUN/TURN), and call control (hold, transfer, DTMF). APIs are stabilizing as we move toward 1.0.

<sub>Built with ❤️ in Rust</sub>

</div>
