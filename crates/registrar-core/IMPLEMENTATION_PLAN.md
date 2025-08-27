# Registrar-Core Implementation Plan

## Current Status Summary

**Overall Progress: ~85% Complete**

### ✅ Fully Implemented
- Complete registrar-core crate with all core functionality
- Full PIDF XML support (RFC 3863 compliant)
- Event system with infra-common integration
- High-level RegistrarService API
- Session-core integration module (RegistrarIntegration)
- Presence coordinator in session-core
- OAuth 2.0 authentication support
- REGISTER, PUBLISH, SUBSCRIBE/NOTIFY request handling
- P2P and B2BUA dual-mode architecture

### 🔶 Partially Complete
- Expiry management (basic implementation, needs background tasks)
- Testing (unit tests exist, integration tests needed)
- Documentation (inline docs complete, guides needed)

### ❌ Not Yet Implemented
- Persistent storage (uses in-memory only)
- Background expiry cleanup tasks
- Performance benchmarks and load testing
- Integration test suites
- Example applications

### 🎯 Next Priority Tasks
1. **Complete SimplePeer Integration** - Wire up register() and unregister() methods to use RegistrarService
2. **Background Tasks** - Implement expiry cleanup for registrations and subscriptions
3. **Integration Tests** - Full end-to-end testing of registration and presence flows
4. **Performance Testing** - Verify 10,000+ user capacity claims

## Phase 1: Foundation (Day 1-2) ✅ COMPLETED

### 1.1 Core Data Structures
- [x] Define basic types (UserRegistration, ContactInfo, PresenceState)
- [x] Implement error types
- [x] Set up module structure

### 1.2 Registry Module
- [x] Implement UserRegistry with DashMap
- [x] Basic register/unregister operations
- [x] Contact lookup functionality
- [x] Expiry management (basic)

### 1.3 Basic Tests
- [x] Unit tests for registry operations
- [x] Concurrent access tests

## Phase 2: Presence Core (Day 3-4) ✅ COMPLETED

### 2.1 Presence Store
- [x] PresenceState management
- [x] Multi-device presence tracking
- [x] Presence update operations

### 2.2 Subscription Manager
- [x] Subscription data model
- [x] Add/remove subscriptions
- [x] Watcher/watching queries

### 2.3 PIDF Support
- [x] Basic PIDF XML generation
- [x] PIDF parsing
- [x] Validation

## Phase 3: Event Integration (Day 5) ✅ COMPLETED

### 3.1 Event Definitions
- [x] Define RegistrarEvent enum
- [x] Implement Event trait from infra-common

### 3.2 Event Publishing
- [x] Emit events on registration changes
- [x] Emit events on presence updates
- [x] Subscription events

### 3.3 Event Subscriptions
- [x] Handle external events
- [x] Event-driven notifications

## Phase 4: API Layer (Day 6) ✅ COMPLETED

### 4.1 RegistrarService API
- [x] High-level registration methods
- [x] Presence query/update methods
- [x] Buddy list operations

### 4.2 Integration Helpers
- [x] Session-core integration utilities
- [x] Builder pattern for configuration

## Phase 5: Advanced Features (Day 7-8) 🔶 PARTIALLY COMPLETED

### 5.1 Automatic Buddy Lists
- [x] Auto-subscribe on registration (B2BUA mode)
- [x] Buddy list management
- [ ] Bulk presence queries

### 5.2 Expiry Management
- [ ] Background expiry task
- [x] Registration refresh (basic)
- [ ] Subscription expiry

### 5.3 Performance Optimizations
- [ ] PIDF caching
- [ ] Bulk operations
- [ ] Event batching

## Phase 6: Testing & Documentation (Day 9-10) 🔶 PARTIALLY COMPLETED

### 6.1 Integration Tests
- [ ] Full registration flow
- [ ] Presence update flow
- [ ] Multi-user scenarios

### 6.2 Performance Tests
- [ ] Benchmark concurrent operations
- [ ] Memory usage analysis
- [ ] Load testing

### 6.3 Documentation
- [x] API documentation (inline)
- [ ] Integration guide
- [ ] Examples

## Success Criteria

1. **Functional Requirements**
   - [x] User registration with multiple contacts
   - [x] Presence state management
   - [x] Subscription handling
   - [x] Automatic buddy lists (B2BUA mode)
   - [x] Event-driven updates

2. **Performance Requirements**
   - [ ] Handle 10,000+ registered users (not tested)
   - [x] Sub-millisecond lookups (DashMap provides this)
   - [x] Concurrent access support

3. **Integration Requirements**
   - [x] Clean API for session-core
   - [x] Event bus integration
   - [x] P2P and B2BUA modes

## Risk Mitigation

### Technical Risks
1. **PIDF XML Complexity**
   - Mitigation: Start with basic PIDF, enhance later
   
2. **Event Bus Performance**
   - Mitigation: Implement batching early

3. **Memory Usage**
   - Mitigation: Implement expiry from the start

### Integration Risks
1. **Session-core coupling**
   - Mitigation: Clear interface boundaries
   
2. **Event ordering**
   - Mitigation: Event sequence numbers

## Dependencies

- `infra-common`: Event bus system
- `sip-core`: SIP types and parsing
- `quick-xml`: PIDF XML handling
- `dashmap`: Concurrent hashmaps
- `tokio`: Async runtime

## Testing Strategy

### Unit Tests
- Test each module in isolation
- Mock dependencies
- Property-based testing for concurrent ops

### Integration Tests
- Full user journey tests
- Multi-user interaction tests
- Event flow verification

### Performance Tests
- Concurrent registration benchmark
- Presence update throughput
- Memory usage under load

## Delivery Milestones

1. **Week 1**: Core functionality (Phases 1-3)
   - Basic registration
   - Simple presence
   - Event publishing

2. **Week 2**: Full features (Phases 4-6)
   - Complete API
   - Automatic buddy lists
   - Full test coverage

## Open Questions

1. Should we support persistent storage in v1?
2. How many devices per user should we support?
3. Should presence policies be in v1 or v2?
4. Do we need presence aggregation for multiple devices?

## Next Steps

1. Review and approve this plan
2. Set up the crate structure
3. Begin Phase 1 implementation
4. Daily progress updates