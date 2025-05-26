# rvoip - A Modern Rust VoIP Stack

rvoip is a 100% pure Rust implementation of a SIP/VoIP stack designed to handle, route, and manage phone calls. Built from the ground up with modern Rust practices, it aims to provide a robust, efficient, and secure foundation for VoIP applications.

## Core Design Principles

- **Pure Rust**: No FFI or C dependencies, leveraging Rust's safety and concurrency features
- **Async-first**: Built on tokio for maximum scalability
- **Modular Architecture**: Clean separation of concerns across crates
- **API-centric**: Designed to be controlled via REST/gRPC/WebSocket
- **Production-ready**: Aiming for a complete, battle-tested SIP/RTP stack

## Current Architecture

rvoip follows a layered architecture with **session-core as the central integration layer**:

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
├─────────────────────────────────────────────────────────────┤
│  sip-client                    │  call-engine               │
│  (High-level API)              │  (Call orchestration)     │
├────────────────────────────────┼────────────────────────────┤
│                session-core                                 │
│           (SIP Sessions + RTP Media Coordination)           │
├────────────────────────────────┼────────────────────────────┤
│  transaction-core              │  rtp-core    │  ice-core   │
│  (SIP transactions)            │  (RTP/RTCP)  │  (ICE/STUN) │
├────────────────────────────────┼──────────────┼─────────────┤
│  sip-transport                 │  media-core               │
│  (UDP/TCP/TLS/WebSocket)       │  (Media processing)       │
├────────────────────────────────┤                           │
│  sip-core                      │                           │
│  (Message parsing)             │                           │
└────────────────────────────────┴───────────────────────────┘
```

## Current Library Structure

**Corrected Dependencies (session-core as integration layer):**

### Application Integration Layer:
- `sip-client` → `call-engine`, `session-core`, `media-core`, `rtp-core`, `ice-core`, `transaction-core`, `sip-transport`, `sip-core`
- `call-engine` → `session-core`, `media-core`, `rtp-core`, `transaction-core`, `sip-transport`, `sip-core`

### **Central Integration Layer:**
- **`session-core`** → `transaction-core`, `rtp-core`, `sip-transport`, `sip-core`
  - *Coordinates SIP signaling (via transaction-core) with RTP media (via rtp-core)*
  - *Manages complete session lifecycle including both signaling and media*

### Core Protocol Stacks:
- `transaction-core` → `sip-transport`, `sip-core`
- `sip-transport` → `sip-core`
- `sip-core` → (no internal dependencies)

### Media Processing Stack:
- `media-core` → `rtp-core`, `ice-core`
- `rtp-core` → (no internal dependencies)
- `ice-core` → (no internal dependencies)

### Infrastructure:
- `infra-common` → (standalone, not currently used by other crates)

## Component Responsibilities

### Currently Implemented Components

#### High-Level Components

- **sip-client**: High-level client library providing unified access to all SIP and media functionality
- **call-engine**: Manages high-level call processing logic and coordinates between SIP signaling and media processing

#### Core Protocol Components

- **session-core**: Manages SIP dialogs and call state
  - **Dialog Management**: Tracks dialogs per RFC 3261, handles dialog matching, and manages dialog-related states

- **transaction-core**: Implements the SIP transaction layer
  - **Transaction Processing**: Handles client and server transactions according to the SIP specification

- **sip-transport**: Manages SIP message transport
  - **Protocol Transport**: UDP, TCP, TLS, and WebSocket transport implementations

- **sip-core**: Core SIP message processing
  - **Message Parsing**: SIP message parsing, serialization, URI handling, and SDP support

#### Media Components

- **media-core**: Manages media processing and codec operations
  - **Media Pipeline**: Processing chain for media (encoding, decoding, basic mixing)
  - **Codec Support**: G.711 (PCMU/PCMA), G.722, Opus codec implementations

- **rtp-core**: Handles RTP/RTCP packet processing
  - **Packet Handling**: RTP packet processing, RTCP reports, and media synchronization
  - **Security**: SRTP support for encrypted media

#### Infrastructure

- **infra-common**: Provides cross-cutting infrastructure
  - **Event Bus**: Inter-component communication
  - **Configuration**: Dynamic configuration management
  - **Lifecycle Management**: Component startup/shutdown coordination
  - **Logging**: Structured logging and tracing

### Planned Components (Not Yet Implemented)

#### Future High-Level Components

- **api-server**: External control API (REST/gRPC/WebSocket) - *directory exists but empty*

#### Future Media & Analysis Components

- **media-recorder**: Media recording, compliance, and analysis
  - **Capture Engine**: Records audio, video, text, and screen sharing in various formats
  - **Standards Compliance**: SIPrec (SIPREC) and vCon standards for interoperable recording
  - **Analysis Pipeline**: Real-time and post-call analysis of media content

- **ai-engine**: AI capabilities coordination
  - **Speech Processing**: Speech recognition, transcription, and generation interfaces
  - **Intelligent Routing**: Context-aware routing decisions based on call analysis
  - **Media Intelligence**: Media analysis for sentiment, intent detection, and conversation insights

#### Future Infrastructure Components

- **storage-service**: Distributed storage for recordings and metadata
  - **Object Storage**: Scalable storage for media recordings and large binary data
  - **Time Series DB**: Storage for metrics, events, and time-based analytics
  - **Metadata Store**: Structured storage for session records and call details

## State Management Architecture

rvoip implements explicit state machines at multiple layers:

1. **Transaction State Machine**
   - Client and server transaction states per RFC 3261
   - Handles retransmissions and timeout logic

2. **Dialog State Machine**
   - Early, Confirmed, and Terminated states
   - Manages dialog creation, updates, and termination

3. **Call State Machine**
   - Application-level call states (Initial, Ringing, Connected, etc.)
   - Maps user-facing operations to protocol operations

4. **Session State Machine**
   - Media negotiation and management states
   - Handles codec selection and media flow

## Development Status

### Phase 1: Core Foundations ✅

- [x] Project structure setup
- [x] SIP message parser/serializer
- [x] Basic SIP transaction state machine
- [x] UDP transport for SIP messages
- [x] Basic RTP packet handling
- [x] G.711 codec implementation
- [x] Simple call session management
- [x] SIP client library

### Phase 2: Library Integration 🔄

- [ ] Improved dialog layer integration
- [ ] Enhanced state management patterns
- [ ] Complete transaction handling
- [ ] Better separation of concerns across libraries
- [ ] Consistent event propagation
- [ ] Full SDP negotiation support
- [ ] Enhanced media session handling

### Phase 3: Softswitch Capabilities 🔜

- [ ] Complete SIP method support
- [ ] TCP/TLS transport
- [ ] Call transfer and forwarding
- [ ] Media relay functionality
- [ ] Call recording with SIPrec/vCon standard compliance
- [ ] Extended API for call control
- [ ] WebSocket events for call state changes
- [ ] Multi-format recording (audio, video, text, screen)
- [ ] Media storage with retention policies

### Phase 4: Advanced Features 🔜

- [ ] NAT traversal with ICE/STUN/TURN
- [ ] SRTP for media encryption
- [ ] Additional codec support (G.722, Opus, VP8, H.264)
- [ ] Transcoding engine for audio and video
- [ ] WebRTC gateway with full media support
- [ ] IVR capabilities
- [ ] Call queuing and distribution
- [ ] High availability and clustering
- [ ] AI agent framework integration
- [ ] Speech recognition and generation interfaces
- [ ] Real-time media analysis capabilities
- [ ] Multi-modal sessions (audio+video+text)
- [ ] Group communication support
- [ ] Broadcast streaming capabilities
- [ ] Real-Time Text (RTT) support
- [ ] End-to-end encryption for all media types
- [ ] Advanced media analytics (speaker identification, topic detection)
- [ ] Distributed storage for high-volume recording
- [ ] Compliance features (legal hold, PII redaction, GDPR controls)
- [ ] Conversation intelligence and insights

## Implementation Roadmap

### Current Focus: Improving Architecture and Component Integration

1. **Dialog Integration**
   - Fully integrate session-core Dialog implementation with sip-client
   - Refactor Call to use Dialog for SIP protocol state
   - Implement proper dialog matching and routing

2. **State Machine Refactoring**
   - Implement explicit state transition validation
   - Separate application and protocol states
   - Create modular state handlers

3. **Layer Integration**
   - Improve transaction-to-dialog routing
   - Enhance session-to-call coordination
   - Establish consistent event propagation model
   
4. **Common Infrastructure**
   - Implement shared event system across components
   - Standardize configuration and lifecycle management
   - Create common logging and metrics infrastructure

## Getting Started

### Building the Project

```bash
git clone https://github.com/rudeless/rvoip.git
cd rvoip
cargo build
```

### Running the Examples

```bash
# Run a SIP client demo (caller)
cd examples/sip-client-demo
cargo run --bin caller -- -a 127.0.0.1:5070 -u alice -s 127.0.0.1:5071 -t sip:bob@example.com

# Run a SIP client demo (receiver)
cargo run --bin receiver -- -a 127.0.0.1:5071 -u bob
```

## Comparison with Existing Solutions

Unlike PJSIP (C) and sofia-sip (C), rvoip is built as a pure Rust stack without C dependencies. While drawing inspiration from these battle-tested libraries, rvoip adopts Rust's memory safety guarantees and modern async programming model. The architecture is designed to be:

- More modular than PJSIP
- More concurrent than sofia-sip
- More type-safe than both
- Better suited for modern cloud-native deployments

The SIP client library is modelled off of PJSIP and sofia-sip and the server logic is inspired by Kamailio for carrier scalability and FreeSWITCH for media handling.

## License

MIT License

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. 