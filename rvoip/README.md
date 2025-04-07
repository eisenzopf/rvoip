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
┌─────────────────────────────────────────────────────────┐
│                    Application Layer                     │
│           (API Server, Client Applications)              │
└─────────────────────────────┬───────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────┐
│                      Call Engine                         │
│            (Call Routing, Policies, Logic)               │
└───────────┬─────────────────────────────────┬───────────┘
            │                                 │
┌───────────▼────────────┐   ┌───────────────▼────────────┐
│    Session Management  │   │        Media Engine         │
│   (Dialogs, Call Flow) │◄──┤  (RTP, Codecs, Streaming)   │
└───────────┬────────────┘   └───────────────┬────────────┘
            │                                │
┌───────────▼────────────┐   ┌───────────────▼────────────┐
│     Transaction Layer  │   │        Media Transport      │
│  (SIP State Machine)   │   │    (RTP/RTCP Processing)    │
└───────────┬────────────┘   └───────────────┬────────────┘
            │                                │
┌───────────▼────────────────┬──────────────▼────────────┐
│          SIP Transport     │     Media Transport        │
│      (UDP, TCP, TLS)       │   (Socket Management)      │
└────────────────────────────┴───────────────────────────┘
```

## Library Structure

The project is organized into these primary crates:

- **sip-core**: SIP message parsing, serialization, URI handling
- **sip-transport**: UDP, TCP, TLS, WebSocket transport layers for SIP
- **transaction-core**: SIP transaction layer (RFC 3261 client/server transactions)
- **session-core**: Dialog management, SDP handling, state machines
- **rtp-core**: RTP/RTCP packet processing
- **media-core**: Codec management, media handling, formats
- **call-engine**: Call routing, policy enforcement, application logic
- **sip-client**: High-level client library for SIP applications
- **api-server**: External control API (REST/gRPC/WebSocket)
- **examples**: Reference implementations and demos

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
- [ ] Call recording
- [ ] Extended API for call control
- [ ] WebSocket events for call state changes

### Phase 4: Advanced Features 🔜

- [ ] NAT traversal with ICE/STUN/TURN
- [ ] SRTP for media encryption
- [ ] Additional codec support (G.722, Opus)
- [ ] Transcoding engine
- [ ] WebRTC gateway
- [ ] IVR capabilities
- [ ] Call queuing and distribution
- [ ] High availability and clustering

## Implementation Roadmap

### Current Focus: Improving State Management

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

The SIP client library is modelled off of PJSIP and sifia-sip and the server logic is inspired by Kamailio for carrier scalability and FreeSWITCH for media handling.

## License

MIT License

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. 