# Call Engine TODO

## Overview
The Call Engine is responsible for managing call center operations, including agent management, call queuing, routing, and session management. It builds on top of session-core to provide call center-specific functionality.

## Architecture
- **Session Management**: Uses SessionCoordinator from session-core
- **Agent Management**: Tracks agent states and availability
- **Queue Management**: Manages call queues with various strategies
- **Routing Engine**: Routes calls based on skills, availability, and business rules
- **Bridge Management**: Uses session-core's bridge API for agent-customer connections

## ✅ COMPLETED: Session-Core Integration

The integration with session-core has been successfully completed:

### What Was Done:
1. **Updated Imports**: Now correctly imports SessionCoordinator and related types
2. **Bridge Management**: Uses session-core's bridge API for connecting calls
3. **Event System**: Monitors bridge events for real-time updates
4. **Clean Architecture**: Call-engine focuses on call center logic, not SIP details

### Key Components:
- `CallCenterEngine`: Main orchestrator using SessionCoordinator
- `AgentManager`: Manages agent states and sessions
- `QueueManager`: Handles call queuing logic
- `RoutingEngine`: Implements call distribution algorithms

## Current Status

### Phase 0: Basic Call Delivery (Prerequisite) 🚨

**Critical**: Without this foundation, agents cannot receive calls and the system is non-functional.

#### 0.1 Fix Call-Engine Integration with Session-Core
- [ ] Remove references to non-existent types (IncomingCallNotificationTrait, ServerSessionManager, etc.)
- [ ] Replace with correct session-core types (SessionCoordinator, CallHandler, etc.)
- [ ] Update imports in `src/orchestrator/core.rs` to use actual session-core API

#### 0.2 Implement CallHandler for Call-Engine
- [ ] Create `CallCenterCallHandler` struct that implements session-core's CallHandler trait
- [ ] Implement `on_incoming_call()` to route calls through call-engine's routing logic
- [ ] Implement `on_call_ended()` to clean up call state
- [ ] Implement `on_call_established()` to track active calls

#### 0.3 Update CallCenterEngine Creation
- [ ] Use SessionManagerBuilder with the CallCenterCallHandler
- [ ] Remove complex notification handler setup code
- [ ] Store reference to SessionCoordinator for making outbound calls
- [ ] Test that incoming calls reach the CallHandler

#### 0.4 Agent Registration & Call Delivery
- [ ] Design how agents register their SIP endpoints (store in database)
- [ ] When routing decision selects an agent, create outbound call to agent's SIP URI
- [ ] Use session-core's bridge functionality to connect customer and agent
- [ ] Handle agent busy/no-answer scenarios

#### 0.5 End-to-End Testing
- [ ] Create test scenario: customer calls → CallHandler receives it → routes to agent
- [ ] Test call bridging between customer and agent
- [ ] Verify media path establishment
- [ ] Test multiple concurrent calls
- [ ] Validate call teardown and cleanup

**Estimated Time**: 1 week (much simpler than original estimate)
**Priority**: MUST COMPLETE before any other phases

**Key Insight**: No session-core changes needed - just use the existing CallHandler API correctly!

### Phase 1: IVR System Implementation (Critical) 🎯

#### 1.1 Core IVR Module
- [ ] Create `src/ivr/mod.rs` with IVR menu system
- [ ] Define `IvrMenu` structure with prompts and options
- [ ] Implement `IvrAction` enum (TransferToQueue, PlayPrompt, SubMenu, etc.)
- [ ] Create `IvrSession` to track caller's menu state
- [ ] Build menu configuration loader (JSON/YAML support)

#### 1.2 DTMF Integration
- [ ] Integrate with session-core's DTMF handling
- [ ] Create DTMF event listener in CallCenterEngine
- [ ] Implement menu navigation state machine
- [ ] Add timeout handling for menu options
- [ ] Support retry logic with configurable attempts

#### 1.3 Audio Prompt Management
- [ ] Define `AudioPrompt` structure for menu prompts
- [ ] Support multiple audio formats (wav, mp3, g711)
- [ ] Implement prompt caching system
- [ ] Add multi-language prompt support
- [ ] Create prompt recording management API

#### 1.4 IVR Flow Builder
- [ ] Visual IVR designer data model
- [ ] Support conditional branching
- [ ] Integration with external data sources
- [ ] A/B testing support for menu flows

### Phase 2: Enhanced Routing Engine 🚦

#### 2.1 Advanced Routing Rules
- [ ] Create rule-based routing engine
- [ ] Support custom routing scripts (Lua/JavaScript)
- [ ] Implement routing strategies:
  - [ ] Round-robin
  - [ ] Least-busy
  - [ ] Sticky sessions
  - [ ] Skills-based with weights
- [ ] Add routing fallback chains

#### 2.2 Business Logic
- [ ] Business hours configuration per queue
- [ ] Holiday calendar support
- [ ] Geographic/timezone-based routing
- [ ] Language preference routing
- [ ] Customer history-based routing

#### 2.3 Load Balancing
- [ ] Agent capacity scoring algorithm
- [ ] Queue overflow thresholds
- [ ] Dynamic rebalancing
- [ ] Predictive routing based on call patterns

### Phase 3: Core Call Center Features 📞

#### 3.1 Call Recording
- [ ] Integration with media-core for recording
- [ ] Configurable recording policies
- [ ] On-demand recording start/stop
- [ ] Recording storage management
- [ ] Compliance features (PCI, GDPR)

#### 3.2 Call Transfer
- [ ] Implement blind transfer
- [ ] Implement attended transfer
- [ ] Warm transfer with consultation
- [ ] Transfer to external numbers
- [ ] Transfer history tracking

#### 3.3 Conference Support
- [ ] Multi-party conference bridges
- [ ] Dynamic participant management
- [ ] Conference recording
- [ ] Moderator controls
- [ ] Scheduled conferences

#### 3.4 Supervisor Features
- [ ] Call monitoring (listen-only)
- [ ] Whisper mode (agent-only audio)
- [ ] Barge-in capability
- [ ] Real-time coaching
- [ ] Quality scoring interface

### Phase 4: API & Integration Layer 🔌

#### 4.1 REST API
- [ ] Design OpenAPI specification
- [ ] Implement with Axum:
  - [ ] Agent management endpoints
  - [ ] Queue management endpoints
  - [ ] Call control endpoints
  - [ ] Statistics endpoints
  - [ ] IVR configuration endpoints
- [ ] Authentication & authorization
- [ ] Rate limiting
- [ ] API versioning

#### 4.2 WebSocket API
- [ ] Real-time event streaming
- [ ] Call state notifications
- [ ] Agent status updates
- [ ] Queue statistics feed
- [ ] Custom event subscriptions

#### 4.3 Webhooks
- [ ] Configurable webhook endpoints
- [ ] Event filtering
- [ ] Retry mechanism
- [ ] Webhook security (HMAC)
- [ ] Event batching

#### 4.4 External Integrations
- [ ] CRM integration framework
- [ ] Ticketing system adapters
- [ ] Analytics platform connectors
- [ ] Cloud storage for recordings
- [ ] SMS/Email notification service

### Phase 5: Production Readiness 🚀

#### 5.1 High Availability
- [ ] State replication across nodes
- [ ] Automatic failover
- [ ] Load distribution
- [ ] Health monitoring
- [ ] Graceful degradation

#### 5.2 Performance Optimization
- [ ] Connection pooling optimization
- [ ] Caching strategies
- [ ] Database query optimization
- [ ] Memory usage profiling
- [ ] Benchmark suite

#### 5.3 Monitoring & Observability
- [ ] Prometheus metrics export
- [ ] Distributed tracing (OpenTelemetry)
- [ ] Custom dashboards
- [ ] Alerting rules
- [ ] SLA tracking

#### 5.4 Security
- [ ] SIP security hardening
- [ ] Encryption for recordings
- [ ] Access control lists
- [ ] Audit logging
- [ ] Penetration testing

### Phase 6: Testing & Documentation 📚

#### 6.1 Testing Suite
- [ ] Unit tests for IVR system
- [ ] Integration tests for call flows
- [ ] Load testing scenarios
- [ ] Chaos engineering tests
- [ ] End-to-end test automation

#### 6.2 Documentation
- [ ] IVR configuration guide
- [ ] API documentation with examples
- [ ] Deployment best practices
- [ ] Troubleshooting guide
- [ ] Performance tuning guide

#### 6.3 Examples & Tutorials
- [ ] Complete IVR setup example
- [ ] Multi-tenant configuration
- [ ] CRM integration example
- [ ] Custom routing rules
- [ ] Monitoring setup

### 📅 Estimated Timeline

- **Phase 0 (Basic Call Delivery)**: 1 week - Critical for basic operation
- **Phase 1 (IVR)**: 4-6 weeks - Critical for basic operation
- **Phase 2 (Routing)**: 3-4 weeks - Enhanced functionality
- **Phase 3 (Features)**: 6-8 weeks - Production features
- **Phase 4 (API)**: 4-5 weeks - Integration capabilities
- **Phase 5 (Production)**: 4-6 weeks - Reliability & scale
- **Phase 6 (Testing)**: Ongoing throughout all phases

**Total Estimate**: 5-6 months for full production readiness

### 🎯 Quick Wins (Can be done in parallel)

1. [ ] Add basic DTMF handling (1 week)
2. [ ] Simple audio prompt playback (1 week)
3. [ ] REST API skeleton (3 days)
4. [ ] Basic call transfer (1 week)
5. [ ] Prometheus metrics (3 days)

### 📊 Success Metrics

- IVR menu completion rate > 80%
- Average routing time < 100ms
- Agent utilization > 70%
- Call setup time < 2 seconds
- System uptime > 99.9%
- API response time < 50ms p95

### 🚧 Technical Debt to Address

1. [ ] Refactor routing engine for extensibility
2. [ ] Improve error handling consistency
3. [ ] Add comprehensive logging
4. [ ] Optimize database queries
5. [ ] Memory leak investigation
6. [ ] Code coverage > 80%

### 🔗 Dependencies to Add

```toml
# For IVR support
symphonia = "0.5"  # Audio decoding
rubato = "0.14"    # Sample rate conversion

# For API development  
axum = "0.7"
tower = "0.4"
tower-http = "0.5"

# For external integrations
reqwest = "0.11"
aws-sdk-s3 = "1.0"  # For recording storage

# For monitoring
prometheus = "0.13"
opentelemetry = "0.21"
```

### 💡 Architecture Decisions Needed

1. **IVR State Storage**: In-memory vs Redis vs Database
2. **Recording Storage**: Local vs S3 vs dedicated solution
3. **Multi-tenancy**: Shared vs isolated resources
4. **Scaling Strategy**: Horizontal vs vertical
5. **Configuration Management**: File-based vs API-based vs hybrid

---

**Next Step**: Start with Phase 1.1 - Create the core IVR module structure 