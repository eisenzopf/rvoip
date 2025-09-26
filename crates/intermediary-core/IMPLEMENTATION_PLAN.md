# Intermediary-Core Implementation Plan

## Executive Summary

The `intermediary-core` crate will provide a flexible foundation for building various types of SIP intermediaries including proxies, B2BUAs, SBCs, and gateways. It will leverage existing RVoIP components (particularly `session-core-v2`) while maintaining clean architectural boundaries.

## Goals and Non-Goals

### Goals
- Provide a unified library for building any type of SIP intermediary
- Support multiple operating modes (proxy, B2BUA, gateway, SBC)
- Enable flexible routing and policy enforcement
- Maintain clean separation between proxy and B2BUA logic
- Leverage `session-core-v2` for individual session management
- Support high-performance concurrent operations
- Provide simple APIs for common use cases

### Non-Goals
- Not replacing `session-core-v2` for single-session management
- Not implementing low-level SIP protocol handling (use `sip-core`)
- Not implementing media processing (use `media-core`)
- Not providing a complete server implementation (that's for applications)

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                   Application Layer                      │
│         (SIP Proxy, B2BUA, SBC, Gateway, etc.)          │
└────────────────────────┬────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────┐
│                  Intermediary-Core API                   │
│  ┌──────────────────────────────────────────────────┐  │
│  │                  Simple API                       │  │
│  │         (IntermediaryBuilder, Intermediary)      │  │
│  └──────────────────────────────────────────────────┘  │
└────────────────────────┬────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────┐
│                    Core Modules                          │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌──────────┐  │
│  │  Proxy  │  │  B2BUA  │  │ Routing │  │  Policy  │  │
│  │  Mode   │  │  Mode   │  │ Engine  │  │  Engine  │  │
│  └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘  │
└───────┼────────────┼────────────┼────────────┼─────────┘
        │            │            │            │
┌───────▼────────────▼────────────▼────────────▼─────────┐
│                 Session Management Layer                 │
│  ┌──────────────────────────────────────────────────┐  │
│  │            session-core-v2 (for B2BUA)           │  │
│  └──────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────┐  │
│  │          dialog-core (for Proxy mode)            │  │
│  └──────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

## Implementation Phases

### Phase 1: Foundation (Week 1-2)
**Status: COMPLETED ✓**

- [x] Create crate structure and Cargo.toml
- [x] Define core types and traits
- [x] Implement basic routing engine interface
- [x] Implement basic policy engine interface
- [x] Create simple API builder pattern

### Phase 2: Proxy Mode (Week 3-4)

#### 2.1 Stateless Proxy
- [ ] Implement request forwarding without state
- [ ] Add Via header manipulation
- [ ] Implement response routing
- [ ] Add Record-Route handling

#### 2.2 Stateful Proxy
- [ ] Add transaction state management
- [ ] Implement forking proxy support
- [ ] Add timer management (Timer C, etc.)
- [ ] Implement CANCELs and retransmissions

#### 2.3 Call-Stateful Proxy
- [ ] Add dialog state tracking
- [ ] Implement mid-dialog request routing
- [ ] Add dialog event notifications

### Phase 3: B2BUA Mode (Week 5-6)

#### 3.1 Basic B2BUA
- [ ] Integrate with session-core-v2 for leg management
- [ ] Implement leg coordination logic
- [ ] Add SDP manipulation between legs
- [ ] Implement call establishment flow

#### 3.2 Advanced B2BUA Features
- [ ] Add media bridging coordination
- [ ] Implement hold/resume across legs
- [ ] Add transfer support (blind/attended)
- [ ] Implement conferencing support

#### 3.3 Session Management
- [ ] Add session pair tracking
- [ ] Implement cleanup and teardown
- [ ] Add monitoring and metrics
- [ ] Implement failover handling

### Phase 4: Routing Engine (Week 7)

#### 4.1 Routing Rules
- [ ] Implement rule-based routing
- [ ] Add DNS-based routing (NAPTR, SRV, A)
- [ ] Implement least-cost routing
- [ ] Add location service integration

#### 4.2 Load Balancing
- [ ] Implement round-robin
- [ ] Add weighted distribution
- [ ] Implement health checking
- [ ] Add failover mechanisms

#### 4.3 Number Translation
- [ ] Add E.164 manipulation
- [ ] Implement prefix/suffix handling
- [ ] Add regex-based transformations

### Phase 5: Policy Engine (Week 8)

#### 5.1 Access Control
- [ ] Implement IP-based ACLs
- [ ] Add user/domain whitelisting/blacklisting
- [ ] Implement time-based restrictions

#### 5.2 Rate Limiting
- [ ] Add per-user rate limiting
- [ ] Implement global rate limiting
- [ ] Add burst handling

#### 5.3 Security Policies
- [ ] Add authentication requirements
- [ ] Implement TLS/SRTP enforcement
- [ ] Add topology hiding

### Phase 6: Gateway Features (Week 9)

#### 6.1 Protocol Translation
- [ ] Design protocol adapter interface
- [ ] Add SIP normalization
- [ ] Implement header translation

#### 6.2 Codec Management
- [ ] Add codec policy enforcement
- [ ] Implement transcoding coordination
- [ ] Add bandwidth management

### Phase 7: SBC Features (Week 10)

#### 7.1 Security Features
- [ ] Implement topology hiding
- [ ] Add NAT traversal helpers
- [ ] Implement media pinholing

#### 7.2 QoS and Monitoring
- [ ] Add call admission control
- [ ] Implement QoS marking
- [ ] Add CDR generation

### Phase 8: High-Level APIs (Week 11)

#### 8.1 Simple APIs
- [ ] Create proxy server builder
- [ ] Create B2BUA server builder
- [ ] Add gateway configuration helpers
- [ ] Implement SBC presets

#### 8.2 Integration APIs
- [ ] Add event streaming interface
- [ ] Implement plugin system
- [ ] Add REST API helpers
- [ ] Create WebSocket interface

### Phase 9: Testing and Documentation (Week 12)

#### 9.1 Testing
- [ ] Unit tests for each module
- [ ] Integration tests for modes
- [ ] Performance benchmarks
- [ ] Load testing

#### 9.2 Documentation
- [ ] API documentation
- [ ] Architecture guide
- [ ] Usage examples
- [ ] Migration guide from other solutions

## Key Design Decisions

### 1. Modular Architecture
- Each mode (proxy, B2BUA) is a separate module
- Can be compiled conditionally with feature flags
- Shared routing and policy engines

### 2. Session Management Strategy
- **Proxy mode**: Direct dialog-core integration
- **B2BUA mode**: Use session-core-v2 for individual legs
- **Coordination**: Intermediary-core manages the relationship

### 3. Async-First Design
- All operations are async using Tokio
- Non-blocking I/O throughout
- Support for concurrent operations

### 4. Configuration Philosophy
- Builder pattern for easy configuration
- Sensible defaults for common cases
- Full control when needed

## Integration Points

### With session-core-v2
- B2BUA mode creates SimplePeer instances for each leg
- Coordinates media bridging through session-core-v2 APIs
- Handles events from individual sessions

### With dialog-core
- Proxy mode uses dialog-core directly
- Stateless forwarding with minimal overhead
- Transaction and dialog state when needed

### With media-core
- Coordinate media operations through session-core-v2
- Policy enforcement for codecs and bandwidth
- QoS management

### With call-engine
- Call-engine can use intermediary-core for B2BUA operations
- Replaces ad-hoc B2BUA implementation
- Provides consistent intermediary behavior

## Usage Examples

### Simple Proxy Server
```rust
use rvoip_intermediary_core::{
    IntermediaryBuilder,
    IntermediaryMode,
    routing::BasicRoutingEngine,
};

let intermediary = IntermediaryBuilder::new()
    .mode(IntermediaryMode::StatefulProxy)
    .routing_engine(Arc::new(BasicRoutingEngine::new()))
    .build()
    .await?;

// Process incoming requests
let decision = intermediary.process_request(from, to, "INVITE", headers).await?;
```

### B2BUA with Custom Routing
```rust
use rvoip_intermediary_core::{
    IntermediaryMode,
    b2bua::B2BUACoordinator,
};

let coordinator = B2BUACoordinator::new(
    custom_routing_engine,
    custom_policy_engine,
);

// Handle incoming call
let session_id = coordinator.handle_incoming_call(from, to, sdp).await?;
```

### Gateway with Protocol Translation
```rust
let gateway = IntermediaryBuilder::new()
    .mode(IntermediaryMode::Gateway)
    .add_protocol_adapter(h323_adapter)
    .add_codec_policy(transcode_policy)
    .build()
    .await?;
```

## Success Metrics

1. **Functionality**: All planned features implemented
2. **Performance**: < 10ms processing latency for proxy mode
3. **Scalability**: Support 10K+ concurrent sessions
4. **Reliability**: 99.99% uptime in production
5. **Adoption**: Used by at least 3 other crates in RVoIP

## Risks and Mitigations

### Risk 1: Complexity
**Mitigation**: Start with simple cases, iterate based on needs

### Risk 2: Performance
**Mitigation**: Benchmark early, optimize critical paths

### Risk 3: Integration Issues
**Mitigation**: Work closely with session-core-v2 team

### Risk 4: Scope Creep
**Mitigation**: Strict adherence to phases, defer nice-to-haves

## Open Questions

1. Should we support SIP over WebSocket in intermediary-core or leave it to transport layer?
2. How deep should codec policy go - just filtering or active transcoding coordination?
3. Should we build a registration/location service or integrate with external ones?
4. What level of clustering/HA support should be built-in vs external?

## Next Steps

1. Review and approve this plan
2. Begin Phase 2 implementation (Proxy Mode)
3. Set up CI/CD pipeline
4. Create initial documentation structure
5. Establish testing framework