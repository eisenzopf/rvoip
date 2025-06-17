# rvoip - A Modern Rust VoIP Stack

rvoip is a 100% pure Rust implementation of a SIP/VoIP stack designed to handle, route, and manage phone calls. Built from the ground up with modern Rust practices, it aims to provide a robust, efficient, and secure foundation for VoIP applications.

## Core Design Principles

- **Pure Rust**: No FFI or C dependencies, leveraging Rust's safety and concurrency features
- **Async-first**: Built on tokio for maximum scalability
- **Modular Architecture**: Clean separation of concerns across crates
- **API-centric**: Designed to be controlled via REST/gRPC/WebSocket
- **Production-ready**: Aiming for a complete, battle-tested SIP/RTP stack

## Current Architecture

rvoip follows a **session-centric architecture** with `session-core` as the central coordination layer:

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
├─────────────────────────────────────────────────────────────┤
│  sip-client                    │  call-engine               │
│  (High-level SIP API)          │  (Call center logic)       │
├─────────────────────────────────────────────────────────────┤
│                 *** session-core ***                        │
│           (Session Manager - Central Coordinator)           │
│      • Session Management        • Bridge Management        │
│      • Dialog Lifecycle          • Conference Support       │  
│      • Media Coordination        • Unified Event System     │
├─────────────────────────────────────────────────────────────┤
│         Protocol & Processing Layer                         │
│  dialog-core                   │  media-core               │
│  (Dialog state machine)        │  (Media processing)       │
│                                │                            │
│  transaction-core              │                            │
│  (SIP transactions)            │                            │
├─────────────────────────────────────────────────────────────┤
│              Transport Layer                                │
│  sip-transport    │  rtp-core    │  ice-core               │
│  (SIP transport)  │  (RTP/RTCP)  │  (ICE/STUN)             │
├─────────────────────────────────────────────────────────────┤
│              Foundation Layer                               │
│                    sip-core                                 │
│                (Message parsing)                            │
└─────────────────────────────────────────────────────────────┘
```

## Session Manager Design

### Core Concept: **SessionCoordinator as Central Hub**

**session-core** provides a `SessionCoordinator` that serves as the primary interface for both SIP clients and servers. It coordinates between SIP signaling (dialogs, transactions) and RTP media streams, providing a unified API that maintains proper separation of concerns across layers.

### Key Design Principles

1. **Single Source of Truth**: SessionCoordinator maintains the authoritative state for all active sessions
2. **Layer Coordination**: Bridges SIP signaling layer with media processing layer
3. **Event-Driven**: Uses a unified event system for loose coupling between components
4. **Clean Public API**: Exposes high-level operations while hiding protocol complexity

### SessionCoordinator Public Interface

```rust
// High-level session management via SessionCoordinator
pub struct SessionCoordinator {
    // Session lifecycle management
    pub async fn create_outgoing_call(&self, from: &str, to: &str, sdp: Option<String>) -> Result<CallSession>;
    pub async fn terminate_session(&self, session_id: &SessionId) -> Result<()>;
    
    // Session discovery and management
    pub async fn find_session(&self, session_id: &SessionId) -> Result<Option<CallSession>>;
    pub async fn list_active_sessions(&self) -> Result<Vec<SessionId>>;
    pub async fn get_stats(&self) -> Result<SessionStats>;
    
    // Bridge management (2-party conferences)
    pub async fn bridge_sessions(&self, session1: &SessionId, session2: &SessionId) -> Result<BridgeId>;
    pub async fn destroy_bridge(&self, bridge_id: &BridgeId) -> Result<()>;
    pub async fn get_session_bridge(&self, session_id: &SessionId) -> Result<Option<BridgeId>>;
    pub async fn remove_session_from_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<()>;
    pub async fn list_bridges(&self) -> Vec<BridgeInfo>;
    
    // Advanced bridge operations
    pub async fn create_bridge(&self) -> Result<BridgeId>;
    pub async fn add_session_to_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<()>;
    pub async fn subscribe_to_bridge_events(&self) -> mpsc::UnboundedReceiver<BridgeEvent>;
    
    // Media control
    pub async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()>;
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String>;
}

// Session state and information
pub struct CallSession {
    pub id: SessionId,
    pub from: String,
    pub to: String,
    pub state: CallState,
    pub started_at: Option<Instant>,
}

// Bridge management types
pub struct BridgeInfo {
    pub id: BridgeId,
    pub sessions: Vec<SessionId>,
    pub created_at: Instant,
    pub participant_count: usize,
}

// Builder pattern for configuration
pub struct SessionManagerBuilder {
    pub fn new() -> Self;
    pub fn with_sip_port(self, port: u16) -> Self;
    pub fn with_local_address(self, address: impl Into<String>) -> Self;
    pub fn with_media_ports(self, start: u16, end: u16) -> Self;
    pub fn with_handler(self, handler: Arc<dyn CallHandler>) -> Self;
    pub async fn build(self) -> Result<Arc<SessionCoordinator>>;
    pub async fn build_with_transaction_manager(self, tm: Arc<TransactionManager>) -> Result<Arc<SessionCoordinator>>;
}
```

### Usage Examples

#### SIP Server Implementation
```rust
// Create session coordinator
let coordinator = SessionManagerBuilder::new()
    .with_sip_port(5060)
    .with_handler(Arc::new(MyCallHandler))
    .build()
    .await?;

// Start the coordinator
SessionControl::start(&coordinator).await?;

// The coordinator automatically:
// - Manages SIP dialogs via dialog-core
// - Handles transactions via transaction-core
// - Coordinates with media-core for RTP streams
// - Manages bridges (2-party conferences)
// - Provides unified event notifications
```

#### Call Center Implementation (via call-engine)
```rust
// Create call center with session-core integration
let call_center = CallCenterEngine::new(
    transaction_manager,
    config,
    database
).await?;

// Register agents
let agent_session = call_center.register_agent(&agent).await?;

// Call center uses session-core for:
// - Creating and managing agent sessions
// - Bridging customer and agent calls
// - Conference management for multi-party calls
// - Real-time bridge event monitoring

// Bridge a customer to an agent
let bridge_id = call_center.session_manager()
    .bridge_sessions(&customer_session, &agent_session)
    .await?;
```

## Current Library Structure

### Application Layer:
- **`sip-client`** → `client-core`, `session-core`, `media-core`, `rtp-core`, `ice-core`, `transaction-core`, `sip-transport`, `sip-core`
  - High-level SIP client API
  - Simplified interface for making/receiving calls
  
- **`call-engine`** → `session-core`, `transaction-core`, `sip-core`
  - Call center orchestration (agents, queues, routing)
  - Uses session-core for all SIP/media operations
  - No direct transport layer dependencies (proper separation)

### Central Integration Layer:
- **`session-core`** → `dialog-core`, `media-core`, `conference`, `bridge`, `manager`, `coordinator`
  - Central coordination hub for all session-related operations
  - Integrates dialog management from dialog-core
  - Coordinates media via media-core
  - Provides bridge management (2-party conferences)
  - Unified API for both client and server use cases

### Protocol & Processing Layer:
- **`dialog-core`** → `transaction-core`, `sip-transport`, `sip-core`
  - Dialog state machine implementation
  - Handles dialog lifecycle per RFC 3261
  
- **`transaction-core`** → `sip-transport`, `sip-core`
  - Client and server transactions
  - Timer management
  - Retransmission logic

- **`media-core`** → `rtp-core`, `ice-core`
  - Audio processing and codec support
  - Media session management

### Transport Layer:
- **`sip-transport`** → `sip-core`
  - UDP, TCP, TLS, WebSocket transports
  - Connection management
  
- **`rtp-core`** → (no internal dependencies)
  - RTP/RTCP packet processing
  - SRTP support
  
- **`ice-core`** → (no internal dependencies)
  - ICE/STUN/TURN support (partial)

### Foundation Layer:
- **`sip-core`** → (no internal dependencies)
  - SIP message parsing and serialization
  - SDP support
  - Core protocol types

### Infrastructure:
- **`infra-common`** → (standalone)
  - Event bus, configuration, lifecycle management
  - Currently underutilized but available

## Component Responsibilities

### Currently Implemented Components

#### Application Layer

- **sip-client**: High-level client library
  - Simple API for making/receiving calls
  - Abstracts protocol complexity
  - Includes example CLI applications

- **call-engine**: Call center orchestration
  - Agent management and routing
  - Queue management
  - Call distribution algorithms
  - Bridge orchestration via session-core

#### Central Coordination

- **session-core**: The heart of the system
  - **SessionCoordinator**: Main entry point for all operations
  - **Dialog Management**: Integrates dialog-core for SIP dialogs
  - **Media Coordination**: Integrates media-core for RTP
  - **Bridge Management**: 2-party conferences for call bridging
  - **Event System**: Unified event propagation

#### Protocol Implementation

- **dialog-core**: Dialog layer implementation
  - RFC 3261 compliant dialog state machine
  - Dialog matching and routing
  - Integrated with session-core

- **transaction-core**: Transaction layer
  - Client and server transactions
  - Timer management
  - Retransmission logic

- **sip-transport**: Transport management
  - Multiple transport protocols
  - Connection pooling
  - Message routing

- **sip-core**: Core protocol support
  - Message parsing/serialization
  - Header manipulation
  - SDP handling

#### Media Handling

- **media-core**: Media processing
  - Codec support (G.711, G.722, Opus)
  - Audio processing pipeline
  - Echo cancellation (basic)
  - Conference mixing support

- **rtp-core**: RTP/RTCP implementation
  - Packet processing
  - Jitter buffering
  - SRTP encryption
  - Statistics collection

#### Additional Components

- **ice-core**: NAT traversal (partial implementation)
  - STUN client
  - Candidate gathering
  - Basic ICE state machine

- **client-core**: Client-specific utilities
  - Used by sip-client
  - Call state management
  - Event handling

### Builder Crates (Higher-level abstractions)

- **rvoip-builder**: Fluent API for building VoIP applications
- **rvoip-presets**: Pre-configured setups for common use cases
- **rvoip-simple**: Simplified API for basic use cases

## Development Status

### Phase 1: Core Foundations ✅

- [x] SIP message parser/serializer
- [x] Basic SIP transaction state machine
- [x] UDP transport for SIP messages
- [x] Basic RTP packet handling
- [x] G.711 codec implementation
- [x] Session management via session-core
- [x] Dialog state machine in dialog-core
- [x] SIP client library

### Phase 2: Integration & Architecture ✅

- [x] SessionCoordinator as central hub
- [x] Dialog-core integration with session-core
- [x] Bridge management (2-party conferences)
- [x] Call-engine integration with session-core
- [x] Proper separation of concerns
- [x] Event propagation system
- [x] Media coordination

### Phase 3: Protocol Completeness 🔄

- [x] INVITE, BYE, CANCEL support
- [x] Basic SDP negotiation
- [ ] Full RFC 3261 compliance
- [ ] REGISTER support
- [ ] SUBSCRIBE/NOTIFY
- [ ] MESSAGE method
- [ ] UPDATE method
- [x] TCP transport (basic)
- [ ] TLS transport
- [ ] WebSocket transport

### Phase 4: Advanced Features 🔜

- [x] Basic bridge/conference support
- [ ] Full conference mixing
- [ ] Call transfer (REFER)
- [ ] Call forwarding
- [ ] Call recording
- [ ] Advanced codecs
- [ ] Video support
- [ ] Full ICE/STUN/TURN
- [ ] WebRTC gateway

### Phase 5: Production Features 🔜

- [ ] High availability
- [ ] Clustering support
- [ ] External API (REST/gRPC)
- [ ] Monitoring/metrics
- [ ] Admin interface
- [ ] Database persistence
- [ ] Message queue integration

## Getting Started

### Building the Project

```bash
git clone https://github.com/rudeless/rvoip.git
cd rvoip
cargo build
```

### Running Examples

```bash
# Simple peer-to-peer call
cd rvoip/crates/session-core/examples
cargo run --example simple_peer_to_peer

# SIP client demo
cd rvoip/examples/sip-client-demo
cargo run --bin caller -- -a 127.0.0.1:5070 -u alice -s 127.0.0.1:5071 -t sip:bob@example.com
```

### Creating a Basic SIP Server

```rust
use rvoip_session_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_handler(Arc::new(AutoAnswerHandler))
        .build()
        .await?;
    
    SessionControl::start(&coordinator).await?;
    
    // Server is now running and accepting calls
    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

## Architecture Decisions

### Why SessionCoordinator?

The SessionCoordinator pattern emerged as the best way to:
1. Provide a unified API for diverse use cases
2. Maintain proper separation between protocol layers
3. Coordinate between independent subsystems (dialog, media, transport)
4. Enable both client and server implementations from the same codebase

### Bridge as Conference

Bridges are implemented as 2-party conferences, which:
1. Reuses existing conference infrastructure
2. Provides a clean abstraction
3. Enables future extension to multi-party bridges
4. Maintains consistency in the API

### Event-Driven Architecture

The event system enables:
1. Loose coupling between components
2. Real-time monitoring capabilities
3. Easy extension points for new features
4. Debugging and troubleshooting

## License

MIT License

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. Areas particularly welcoming contributions:

1. Protocol completeness (REGISTER, SUBSCRIBE/NOTIFY, etc.)
2. Transport implementations (TLS, WebSocket)
3. Codec implementations
4. Test coverage
5. Documentation
6. Example applications 