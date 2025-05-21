# rvoip - A Modern Rust VoIP Stack

rvoip is a 100% pure Rust implementation of a SIP/VoIP stack designed to handle, route, and manage phone calls. Built from the ground up with modern Rust practices, it aims to provide a robust, efficient, and secure foundation for VoIP applications.

## Core Design Principles

- **Pure Rust**: No FFI or C dependencies, leveraging Rust's safety and concurrency features
- **Async-first**: Built on tokio for maximum scalability
- **Modular Architecture**: Clean separation of concerns across crates
- **API-centric**: Designed to be controlled via REST/gRPC/WebSocket
- **Production-ready**: Aiming for a complete, battle-tested SIP/RTP stack

## Architecture

rvoip follows a layered architecture inspired by established SIP stacks, with clean separation between components:

```
┌─────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                      Application Layer                                              │
│                             (API Server, Client Applications, UI)                                   │
└───────────────────────────────────────────────────┬─────────────────────────────────────────────────┘
                                                    │
                                                    ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                       call-engine                                                   │
│                                                                                                     │
│  ┌─────────────────────────────────┐  ┌─────────────────────────────────┐  ┌────────────────────┐  │
│  │         Call State Machine      │  │           Service Manager        │  │     Regulatory     │  │
│  │                                 │  │                                  │  │     Compliance     │  │
│  └─────────────────────────────────┘  └─────────────────────────────────┘  └────────────────────┘  │
└───────────────────────────────────────────────────┬─────────────────────────────────────────────────┘
                                                    │
                        ┌───────────────────────────┼───────────────────────────┐
                        │                           │                           │
                        ▼                           ▼                           ▼
┌────────────────────────────────────┐ ┌────────────────────────────┐ ┌────────────────────────────────────┐
│           session-core             │ │        media-core           │ │         media-recorder             │
│      (SIP Dialogs, Call Flow)      │◄┤   (Media Mixing, Codecs)    │◄┤      (Recording & Analytics)       │
│                                    │ │                             │ │                                    │
│ ┌────────────────────────────────┐ │ │┌───────────────────────────┐│ │┌──────────────────────────────────┐│
│ │        Dialog Management       │ │ ││      Media Pipeline       ││ ││         Capture Engine           ││
│ └────────────────────────────────┘ │ │└───────────────────────────┘│ │└──────────────────────────────────┘│
│                                    │ │┌───────────────────────────┐│ │┌──────────────────────────────────┐│
│                                    │ ││    Multi-Modal Engine     ││ ││      Standards Compliance        ││
│                                    │ │└───────────────────────────┘│ ││      (SIPrec, vCon)              ││
│                                    │ │┌───────────────────────────┐│ │└──────────────────────────────────┘│
│                                    │ ││    Stream Management      ││ │┌──────────────────────────────────┐│
│                                    │ │└───────────────────────────┘│ ││        Analysis Pipeline         ││
│                                    │ │                             │ │└──────────────────────────────────┘│
└──────────────────┬─────────────────┘ └─────────────┬───────────────┘ └───────────────────┬────────────────┘
                   │                                 │                                     │
                   ▼                                 ▼                                     │
┌──────────────────────────────────┐     ┌────────────────────────────────┐               │
│        transaction-core          │     │            rtp-core            │               │
│      (SIP State Machine)         │     │      (RTP/RTCP Processing)     │               │
│                                  │     │                                │               │
│┌────────────────────────────────┐│     │┌──────────────────────────────┐│               │
││    Transaction Processing      ││     ││      Packet Handling         ││               │
│└────────────────────────────────┘│     │└──────────────────────────────┘│               │
└──────────────────┬───────────────┘     └─────────────┬──────────────────┘               │
                   │                                   │                                  │
                   │                                   │                                  │
┌──────────────────▼───────────────────────────────────▼──────────────────────────────────▼───────────────┐
│                                                                                                         │
│  ┌────────────────────────────────────┐      ┌────────────────────────────────┐     ┌─────────────────┐ │
│  │          sip-transport             │      │        Media Transport          │     │     Storage     │ │
│  │     (UDP, TCP, TLS, WebSockets)    │      │      (Socket Management)        │     │     Service     │ │
│  └────────────────────────────────────┘      └────────────────────────────────┘     └─────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                        infra-common                                                 │
├─────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ ┌─────────────────────┐ ┌─────────────────────┐ ┌───────────────────────────┐ ┌───────────────────┐ │
│ │      Event Bus      │ │    Configuration    │ │   Lifecycle Management    │ │      Logging      │ │
│ └─────────────────────┘ └─────────────────────┘ └───────────────────────────┘ └───────────────────┘ │
├─────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ ┌───────────────────────────────────────────────────────────────────────────────────────────────┐   │
│ │                                   Connection Pool Management                                   │   │
│ │ ┌─────────────────────────────────────┐                    ┌─────────────────────────────────┐│   │
│ │ │          QUIC Connection            │                    │         UDP Socket Pool         ││   │
│ │ │               Pool                  │                    │      (for RTP/RTCP media)       ││   │
│ │ └─────────────────────────────────────┘                    └─────────────────────────────────┘│   │
│ └───────────────────────────────────────────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ ┌─────────────────────┐ ┌─────────────────────┐ ┌───────────────────────────┐                       │
│ │    Rate Limiting    │ │   Circuit Breakers  │ │    Resource Monitoring    │                       │
│ └─────────────────────┘ └─────────────────────┘ └───────────────────────────┘                       │
└─────────────────────────────────────────────────────────────────────────────────────────────────────┘

        ▲                                                       ▲
        │                                                       │
        │ QUIC/TLS (Signaling, Control)                         │ UDP (Real-time Media)
        │ - Multiplexed streams                                 │ - Low latency
        │ - Reliability                                         │ - Minimal overhead
        │ - Connection migration                                │
        ▼                                                       ▼

┌─────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                      External Networks                                              │
│                         (Public Internet, Carrier Networks, Local)                                  │
└─────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Library Structure

The project is organized into these primary crates:

- **sip-core**: SIP message parsing, serialization, URI handling
- **sip-transport**: UDP, TCP, TLS, WebSocket transport layers for SIP
- **transaction-core**: SIP transaction layer (RFC 3261 client/server transactions)
- **session-core**: Dialog management, SDP handling, state machines
- **rtp-core**: RTP/RTCP packet processing
- **media-core**: Codec management, media handling, formats
- **media-recorder**: Media recording, analysis, and standards compliance (SIPrec, vCon)
- **storage-service**: Distributed storage for media recordings, metadata, and analytics
- **call-engine**: Call routing, policy enforcement, application logic
- **sip-client**: High-level client library for SIP applications
- **api-server**: External control API (REST/gRPC/WebSocket)
- **infra-common**: Shared infrastructure for cross-cutting concerns (events, configuration, logging, lifecycle management)
- **examples**: Reference implementations and demos

## Component Responsibilities

### Top-Level Components

- **Application Layer**: External interfaces for applications to interact with the system, including API servers, client SDKs, and user interfaces.

- **call-engine**: Manages the high-level call processing logic:
  - **Call State Machine**: Maintains application-level call states and transitions, abstracts protocol details from applications.
  - **Service Manager**: Orchestrates call-related services like recording, transcoding, and conferencing. Handles registration, discovery, and chaining of services. Enables dynamic service insertion during active calls. Manages resource allocation for services.
  - **Regulatory Compliance**: Implements emergency services support (E911), lawful intercept capabilities, and number portability handling.

### Mid-Level Components

- **session-core**: Manages SIP dialogs and call state:
  - **Dialog Management**: Tracks dialogs per RFC 3261, handles dialog matching, and manages dialog-related states.

- **media-core**: Manages media processing and codec operations:
  - **Media Pipeline**: Processing chain for media (encoding, decoding, mixing, effects).
  - **Multi-Modal Engine**: Handles different media types (audio, video, text) with format-specific processing.
  - **Stream Management**: Controls stream publishing, subscription, and synchronization for different communication patterns.

- **media-recorder**: Handles recording, compliance, and analysis of media:
  - **Capture Engine**: Records audio, video, text, and screen sharing in various formats.
  - **Standards Compliance**: Implements SIPrec (SIPREC) and vCon standards for interoperable recording.
  - **Analysis Pipeline**: Real-time and post-call analysis of media content for transcription, sentiment analysis, etc.

- **transaction-core**: Implements the SIP transaction layer:
  - **Transaction Processing**: Handles client and server transactions according to the SIP specification.

- **rtp-core**: Handles RTP/RTCP packet processing:
  - **Packet Handling**: Processes RTP packets, RTCP reports, and provides media synchronization.
  - **Media Formats**: Support for multiple payload types including audio codecs (G.711, Opus), video codecs (VP8, H.264), and text (T.140, RTT).
  - **Bandwidth Management**: Dynamic bitrate adaptation for different network conditions and media types.

### Transport Components

- **Signaling Transport**: Manages SIP message transport:
  - **QUIC Transport**: For modern applications, providing multiplexed connections.
  - **UDP/TCP/TLS**: Legacy transports for compatibility with existing SIP endpoints.

- **Media Transport**: Manages media packet transport:
  - **UDP**: Low-latency transport for real-time media.
  - **QUIC**: For media control and non-real-time operations.

- **Storage Service**: Persistent storage for media recordings and metadata.

### Infrastructure Components

- **infra-common**: Provides cross-cutting infrastructure for all components:
  - **Event Bus**: High-performance (250K+ events/sec) message bus for inter-component communication.
  - **Configuration**: Dynamic configuration management.
  - **Lifecycle Management**: Component startup/shutdown coordination.
  - **Logging**: Structured logging and tracing.
  - **Connection Pools**: Managed network connection resources with protocol-specific optimizations.
  - **Rate Limiting**: Traffic control mechanisms.
  - **Resource Monitoring**: System resource tracking and alerting.
  - **Storage Services**: Manages persistent data across the platform:
    - **Object Storage**: Scalable storage for media recordings, files, and large binary data.
    - **Time Series DB**: Storage for metrics, events, and time-based analytics.
    - **Metadata Store**: Structured storage for session records, call details, and analytics results.

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