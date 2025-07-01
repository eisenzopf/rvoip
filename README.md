# rvoip - A Modern Rust VoIP Stack

> **⚠️ Alpha Release** - This is an alpha release with rapidly evolving APIs. Libraries will change significantly as we move toward production readiness, but the core architecture and design principles are stable. The intent is to make this library production-ready for enterprise VoIP deployments.

rvoip is a comprehensive, 100% pure Rust implementation of a SIP/VoIP stack designed to handle, route, and manage phone calls at scale. Built from the ground up with modern Rust practices, it aims to provide a robust, efficient, and secure foundation for VoIP applications ranging from simple softphones to enterprise call centers.

## 🎯 Core Design Principles

- **Pure Rust**: Zero FFI or C dependencies, leveraging Rust's safety and concurrency features
- **Event-Driven Architecture**: Comprehensive event system for loose coupling and real-time monitoring
- **Async-First**: Built on tokio for maximum scalability and performance
- **Modular Architecture**: Clean separation of concerns across specialized crates
- **Layer Separation**: Proper RFC-compliant protocol layer separation
- **Production-Ready**: Designed for enterprise deployment with high availability and monitoring

## 🏗️ Architecture Overview

rvoip follows a **layered architecture** with clear separation of concerns and event-driven communication:

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────┐ │
│  │   call-engine   │  │   sip-client    │  │ api-server  │ │
│  │ (Call center)   │  │ (High-level     │  │ (REST/WS    │ │
│  │                 │  │  SIP client)    │  │  API)       │ │
│  └─────────────────┘  └─────────────────┘  └─────────────┘ │
├─────────────────────────────────────────────────────────────┤
│               Integration & Coordination Layer               │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────┐ │
│  │  session-core   │  │  client-core    │  │             │ │
│  │ (Session mgmt & │  │ (Client library │  │             │ │
│  │  coordination)  │  │  abstraction)   │  │             │ │
│  └─────────────────┘  └─────────────────┘  └─────────────┘ │
├─────────────────────────────────────────────────────────────┤
│              Protocol & Processing Layer                    │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────┐ │
│  │   dialog-core   │  │   media-core    │  │ transaction │ │
│  │ (SIP dialogs &  │  │ (Audio codecs & │  │   -core     │ │
│  │  state machine) │  │  processing)    │  │(SIP trans-  │ │
│  │                 │  │                 │  │ actions)    │ │
│  └─────────────────┘  └─────────────────┘  └─────────────┘ │
├─────────────────────────────────────────────────────────────┤
│              Transport & Media Layer                        │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────┐ │
│  │ sip-transport   │  │   rtp-core      │  │  ice-core   │ │
│  │ (UDP/TCP/TLS/   │  │ (RTP/RTCP/SRTP) │  │ (NAT        │ │
│  │  WebSocket)     │  │                 │  │  traversal) │ │
│  └─────────────────┘  └─────────────────┘  └─────────────┘ │
├─────────────────────────────────────────────────────────────┤
│              Foundation Layer                               │
│  ┌─────────────────┐  ┌─────────────────┐                  │
│  │    sip-core     │  │ infra-common    │                  │
│  │ (Message        │  │ (Event system & │                  │
│  │  parsing/SDP)   │  │  infrastructure)│                  │
│  └─────────────────┘  └─────────────────┘                  │
└─────────────────────────────────────────────────────────────┘
```

## 📦 Library Structure

### 🎯 Application Layer

#### **call-engine** - Complete Call Center Solution
- **Purpose**: Full-featured call center orchestration with agent management, queuing, and routing
- **Status**: ✅ **Working** - Fully functional basic call center with SIPp-tested call flows
- **Features**: Agent registration, call queuing, load balancing, B2BUA bridging, database persistence
- **Use Cases**: Call centers, customer support, sales teams

#### **sip-client** - High-Level SIP Client
- **Purpose**: Simplified SIP client library for building VoIP applications
- **Status**: ✅ **Working** - Complete client functionality with examples
- **Features**: Call management, registration, media control, event handling
- **Use Cases**: Softphones, VoIP apps, client integrations

#### **api-server** - Management API
- **Purpose**: REST/WebSocket API for system management and control
- **Status**: 🚧 **Future** - Planned for system administration
- **Features**: System monitoring, configuration, provisioning
- **Use Cases**: Admin interfaces, system integration

### 🔗 Integration & Coordination Layer

#### **session-core** - Session Management Hub
- **Purpose**: Central coordination for SIP sessions, media, and call control
- **Status**: ✅ **Complete** - Core session management with comprehensive API
- **Features**: Session lifecycle, media coordination, bridge management, event system
- **Architecture Role**: Primary coordination layer that integrates all protocol layers

#### **client-core** - Client Library Framework
- **Purpose**: High-level client library with call management and media control
- **Status**: ✅ **Complete** - Production-ready client framework
- **Features**: Call lifecycle, media operations, event handling, configuration management
- **Use Cases**: VoIP client applications, mobile apps, desktop softphones

### ⚙️ Protocol & Processing Layer

#### **dialog-core** - SIP Dialog Management
- **Purpose**: RFC 3261 compliant SIP dialog state machine and message routing
- **Status**: ✅ **Complete** - Full dialog lifecycle management
- **Features**: Dialog state tracking, in-dialog requests, early/confirmed dialogs, recovery
- **Standards**: RFC 3261 compliant dialog layer

#### **transaction-core** - SIP Transaction Layer
- **Purpose**: Reliable SIP message delivery with retransmission and timeouts
- **Status**: ✅ **Complete** - Full client/server transaction support
- **Features**: Transaction state machines, timer management, message reliability
- **Standards**: RFC 3261 transaction layer

#### **media-core** - Media Processing Engine
- **Purpose**: Audio processing, codec management, and media session coordination
- **Status**: ✅ **Complete** - Advanced audio processing with quality monitoring
- **Features**: G.711/G.722/Opus codecs, AEC/AGC/VAD, format conversion, quality metrics
- **Performance**: Optimized for real-time processing with zero-copy optimization

### 🌐 Transport & Media Layer

#### **sip-transport** - SIP Transport Layer
- **Purpose**: Multi-protocol SIP transport (UDP/TCP/TLS/WebSocket)
- **Status**: ✅ **Working** - UDP/TCP implemented, TLS/WebSocket planned
- **Features**: Connection management, message routing, transport abstraction
- **Protocols**: UDP ✅, TCP ✅, TLS 🚧, WebSocket 🚧

#### **rtp-core** - RTP/RTCP Implementation
- **Purpose**: Real-time media transport with comprehensive RTP/RTCP support
- **Status**: ✅ **Complete** - Full-featured RTP stack with SRTP
- **Features**: RTP/RTCP processing, SRTP encryption, jitter buffering, statistics
- **Security**: SRTP with AES-CM encryption and HMAC-SHA1 authentication

#### **ice-core** - NAT Traversal
- **Purpose**: ICE/STUN/TURN implementation for NAT traversal
- **Status**: 🚧 **Partial** - Basic STUN client, full ICE implementation in progress
- **Features**: STUN client, candidate gathering, basic ICE state machine
- **Standards**: RFC 8445 ICE implementation

### 🔧 Foundation Layer

#### **sip-core** - SIP Protocol Foundation
- **Purpose**: Core SIP message parsing, serialization, and validation
- **Status**: ✅ **Complete** - Production-ready SIP protocol implementation
- **Features**: RFC-compliant parsing, strongly-typed headers, SDP support, builder patterns
- **Standards**: RFC 3261, RFC 4566 (SDP), RFC 4475 torture tests

#### **infra-common** - Infrastructure Services
- **Purpose**: Common infrastructure for event systems, configuration, and lifecycle management
- **Status**: ✅ **Complete** - High-performance event system with multiple implementation strategies
- **Features**: Zero-copy event bus, configuration management, component lifecycle
- **Performance**: 2M+ events/second with sub-millisecond latency

### 🎁 Higher-Level Abstractions

#### **rvoip-builder** - Flexible Composition Framework
- **Purpose**: Builder pattern for composing complex VoIP platforms
- **Status**: 🚧 **Experimental** - API design and component composition
- **Features**: Fluent API, platform composition, configuration management
- **Use Cases**: Custom VoIP platforms, enterprise deployments

#### **rvoip-presets** - Pre-configured Patterns
- **Purpose**: Pre-configured setups for common VoIP use cases
- **Status**: 🚧 **Experimental** - Common deployment patterns
- **Features**: Enterprise PBX, mobile apps, WebRTC platforms, contact centers
- **Use Cases**: Quick deployment, standard configurations

#### **rvoip-simple** - Simplified API
- **Purpose**: Beginner-friendly API for basic VoIP functionality
- **Status**: 🚧 **Experimental** - Simplified client interface
- **Features**: One-line clients, basic call operations, minimal configuration
- **Use Cases**: Learning, prototyping, simple applications

## 🔄 Event-Driven Architecture

The RVOIP stack uses a comprehensive event-driven architecture for loose coupling and real-time monitoring:

### Event System Features
- **Zero-Copy Events**: `Arc<T>` based events eliminate serialization overhead
- **Priority Handling**: Critical events processed first (sub-millisecond latency)
- **Filtering**: Content-based event filtering before delivery
- **Batch Processing**: High-throughput batch event processing
- **Performance**: 2M+ events/second with 5 subscribers

### Event Flow Example
```rust
use rvoip_session_core::prelude::*;
use rvoip_infra_common::events::*;

// Subscribe to call events
let mut events = coordinator.subscribe_events().await;

tokio::spawn(async move {
    while let Some(event) = events.recv().await {
        match event {
            SessionEvent::CallStarted { session_id, participants } => {
                info!("Call started: {} with {}", session_id, participants.len());
            }
            SessionEvent::CallEnded { session_id, duration, reason } => {
                info!("Call ended: {} after {:?} - {}", session_id, duration, reason);
            }
            SessionEvent::MediaQualityChanged { session_id, metrics } => {
                if metrics.mos_score < 3.0 {
                    warn!("Poor call quality: {} MOS={:.1}", session_id, metrics.mos_score);
                }
            }
            _ => {}
        }
    }
});
```

## 🚀 Getting Started

### Quick Start - Basic SIP Server

```rust
use rvoip_session_core::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Create session coordinator with auto-answer
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:server@192.168.1.100:5060")
        .with_handler(Arc::new(AutoAnswerHandler))
        .build()
        .await?;
    
    // Start the server
    SessionControl::start(&coordinator).await?;
    
    println!("✅ SIP server running on port 5060");
    
    // Handle shutdown gracefully
    tokio::signal::ctrl_c().await?;
    SessionControl::stop(&coordinator).await?;
    
    Ok(())
}
```

### Call Center Example

```rust
use rvoip_call_engine::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create call center with default configuration
    let engine = CallCenterEngine::new(CallCenterConfig::default()).await?;
    
    println!("🏢 Call Center Server starting...");
    
    // Start the call center (includes agent management, queuing, routing)
    engine.run().await?;
    
    Ok(())
}
```

### SIP Client Example

```rust
use rvoip_client_core::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ClientConfig {
        sip_uri: "sip:alice@example.com".to_string(),
        server_uri: "sip:server.example.com:5060".to_string(),
        local_port: 5070,
        ..Default::default()
    };
    
    let client = ClientManager::new(config).await?;
    
    // Register with server
    client.register().await?;
    
    // Make a call
    let call = client.make_call("sip:bob@example.com", None).await?;
    
    // Wait for answer
    call.wait_for_answer(Duration::from_secs(30)).await?;
    
    println!("✅ Call connected!");
    
    Ok(())
}
```

## 🔧 Current Development Status

### ✅ Production-Ready Components
- **sip-core**: Complete RFC 3261 implementation with torture tests
- **session-core**: Full session management with comprehensive API
- **dialog-core**: Complete dialog state machine
- **transaction-core**: Full transaction layer with reliability
- **media-core**: Advanced audio processing with quality monitoring
- **rtp-core**: Complete RTP/RTCP/SRTP implementation
- **client-core**: Production-ready client framework
- **call-engine**: Working call center with tested call flows
- **infra-common**: High-performance event system

### 🚧 In Progress
- **ice-core**: Full ICE/STUN/TURN implementation
- **sip-transport**: TLS and WebSocket transport protocols
- **API standardization**: Finalizing public APIs for 1.0 release

### 🔮 Planned Features
- **api-server**: REST/WebSocket management API
- **Advanced call features**: Transfer, hold, conference (3+ party)
- **Video support**: Video codecs and processing
- **WebRTC gateway**: Full WebRTC interoperability
- **Clustering**: High availability and load balancing
- **Monitoring**: Prometheus metrics and health checks

## 🏢 Production Deployment

### Enterprise Features
- **High Performance**: Designed for 100,000+ concurrent calls
- **Event-Driven Monitoring**: Real-time metrics and health monitoring
- **Security**: SRTP encryption, certificate-based authentication
- **Scalability**: Async-first design with tokio runtime
- **Reliability**: Comprehensive error handling and recovery

### Deployment Options
- **Standalone**: Single binary deployment
- **Containerized**: Docker/Kubernetes ready
- **Cloud Native**: AWS/GCP/Azure deployment patterns
- **On-Premises**: Traditional server deployment

## 🧪 Testing & Quality

### Comprehensive Test Suite
- **Unit Tests**: Every crate has extensive unit test coverage
- **Integration Tests**: End-to-end call flows with SIPp
- **RFC Compliance**: Torture tests based on RFC 4475
- **Performance Tests**: Benchmarks for critical paths
- **Interoperability**: Testing with commercial SIP systems

### Running Tests
```bash
# Run all tests
cargo test

# Run call center E2E test
cd crates/call-engine/examples/e2e_test
./run_e2e_test.sh

# Run SIPp interoperability tests
cd crates/session-core/examples/sipp_tests
./run_all_tests.sh
```

## 📋 Development Roadmap

### Phase 1: Core Stabilization (Current)
- API stabilization for 1.0 release
- Complete ICE implementation
- Transport layer completion (TLS/WebSocket)
- Performance optimization

### Phase 2: Advanced Features (Next 3-6 months)
- Video support and processing
- Advanced call features (transfer, conference)
- WebRTC gateway implementation
- REST/WebSocket management API

### Phase 3: Enterprise Features (6-12 months)
- High availability and clustering
- Advanced monitoring and metrics
- Database integration and persistence
- Load balancing and auto-scaling

### Phase 4: Ecosystem (12+ months)
- Language bindings (Python, Node.js, Go)
- Visual management interfaces
- Third-party integrations
- Performance optimization for extreme scale

## 🤝 Contributing

This project welcomes contributions! Key areas where help is needed:

1. **Protocol Implementation**: Complete TLS/WebSocket transports, full ICE support
2. **Advanced Features**: Video support, call transfer, advanced codecs
3. **Testing**: More interoperability tests, edge case coverage
4. **Documentation**: API documentation, tutorials, deployment guides
5. **Performance**: Optimization, benchmarking, scaling analysis

## 📄 License

Licensed under either of:
- MIT License
- Apache License 2.0

at your option.

---

**💡 Ready to get started?** Check out the [examples](examples/) directory for working code samples, or dive into the [session-core API documentation](crates/session-core/API_GUIDE.md) for detailed usage patterns.

**🏢 Enterprise users:** This library is being designed for production deployment. While currently in alpha, the architecture is stable and suitable for evaluation and development. Contact us for enterprise support and deployment guidance. 