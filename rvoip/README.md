# rvoip - A Modern Rust VoIP Stack

rvoip is a 100% pure Rust implementation of a SIP/VoIP stack designed to handle, route, and manage phone calls. Built from the ground up with modern Rust practices, it aims to provide a robust, efficient, and secure foundation for VoIP applications.

## Core Design Principles

- **Pure Rust**: No FFI or C dependencies, leveraging Rust's safety and concurrency features
- **Async-first**: Built on tokio for maximum scalability
- **Modular Architecture**: Clean separation of concerns across crates
- **API-centric**: Designed to be controlled via REST/gRPC/WebSocket
- **Production-ready**: Aiming for a complete, battle-tested SIP/RTP stack

## Architecture

```
┌─────────────────────────┐
│    REST/gRPC/WS API     │
└─────────────┬───────────┘
              │
┌─────────────▼───────────┐      ┌─────────────────────────┐
│    Call Engine & Logic   │◄────►│   Session Management    │
└─────────────┬───────────┘      └───────────┬─────────────┘
              │                               │
┌─────────────▼───────────┐      ┌───────────▼─────────────┐
│      SIP Signaling      │◄────►│      Media (RTP)        │
└─────────────────────────┘      └─────────────────────────┘
```

## Crate Structure

The project is organized into the following crates:

- **sip-core**: SIP message parsing, serialization, URI handling
- **sip-transport**: UDP, TCP, TLS, WebSocket transport layers
- **rtp-core**: RTP packet encoding/decoding, RTCP support
- **media-core**: Codec management, media relay, DTMF, jitter buffer
- **session-core**: Call sessions, dialogs, state management
- **call-engine**: Routing logic, call flows, policies
- **api-server**: External control API (REST/gRPC/WebSocket)
- **utils**: Shared utilities (logging, config, UUIDs)
- **examples**: Reference implementations

## Development Phases

### Phase 1: Core Foundations

- [x] Project structure setup
- [x] SIP message parser/serializer
  - Full RFC 3261 message types and parsing
  - Support for all standard methods (INVITE, ACK, BYE, CANCEL, REGISTER, OPTIONS)
  - Extension methods (SUBSCRIBE, NOTIFY, UPDATE, REFER, INFO, MESSAGE, PRACK, PUBLISH)
  - Complete status code definitions (1xx-6xx)
  - Header parsing and serialization
- [x] Basic SIP transaction state machine
  - Client/server transaction management
  - INVITE and non-INVITE transaction types
  - Timer-based retransmission handling
- [x] UDP transport for SIP messages
  - Async transport layer
  - Event-driven message handling
- [x] Basic RTP packet handling
- [x] G.711 codec implementation
- [ ] Simple call session management
- [ ] Minimal REST API

### Phase 2: Softswitch Capabilities

- [ ] Complete SIP method support
- [ ] TCP/TLS transport
- [ ] Call transfer and forwarding
- [ ] SDP negotiation
- [ ] Media relay functionality
- [ ] Call recording
- [ ] Extended API for call control
- [ ] WebSocket events for call state changes

### Phase 3: Advanced Features

- [ ] NAT traversal with ICE/STUN/TURN
- [ ] SRTP for media encryption
- [ ] Additional codec support (G.722, Opus)
- [ ] Transcoding engine
- [ ] WebRTC gateway
- [ ] IVR capabilities
- [ ] Call queuing and distribution
- [ ] High availability and clustering

## Getting Started

*TBD as development progresses*

## Comparison with Existing Solutions

Unlike PJSIP (C) and sofia-sip (C), rvoip is built as a pure Rust stack without C dependencies. While drawing inspiration from these battle-tested libraries, rvoip adopts Rust's memory safety guarantees and modern async programming model.

## License

*TBD*

## Contributing

*TBD* 