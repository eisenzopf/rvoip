# Registrar-Core Implementation Plan

## Phase 1: Foundation (Day 1-2)

### 1.1 Core Data Structures
- [ ] Define basic types (UserRegistration, ContactInfo, PresenceState)
- [ ] Implement error types
- [ ] Set up module structure

### 1.2 Registry Module
- [ ] Implement UserRegistry with DashMap
- [ ] Basic register/unregister operations
- [ ] Contact lookup functionality
- [ ] Expiry management (basic)

### 1.3 Basic Tests
- [ ] Unit tests for registry operations
- [ ] Concurrent access tests

## Phase 2: Presence Core (Day 3-4)

### 2.1 Presence Store
- [ ] PresenceState management
- [ ] Multi-device presence tracking
- [ ] Presence update operations

### 2.2 Subscription Manager
- [ ] Subscription data model
- [ ] Add/remove subscriptions
- [ ] Watcher/watching queries

### 2.3 PIDF Support
- [ ] Basic PIDF XML generation
- [ ] PIDF parsing
- [ ] Validation

## Phase 3: Event Integration (Day 5)

### 3.1 Event Definitions
- [ ] Define RegistrarEvent enum
- [ ] Implement Event trait from infra-common

### 3.2 Event Publishing
- [ ] Emit events on registration changes
- [ ] Emit events on presence updates
- [ ] Subscription events

### 3.3 Event Subscriptions
- [ ] Handle external events
- [ ] Event-driven notifications

## Phase 4: API Layer (Day 6)

### 4.1 RegistrarService API
- [ ] High-level registration methods
- [ ] Presence query/update methods
- [ ] Buddy list operations

### 4.2 Integration Helpers
- [ ] Session-core integration utilities
- [ ] Builder pattern for configuration

## Phase 5: Advanced Features (Day 7-8)

### 5.1 Automatic Buddy Lists
- [ ] Auto-subscribe on registration
- [ ] Buddy list management
- [ ] Bulk presence queries

### 5.2 Expiry Management
- [ ] Background expiry task
- [ ] Registration refresh
- [ ] Subscription expiry

### 5.3 Performance Optimizations
- [ ] PIDF caching
- [ ] Bulk operations
- [ ] Event batching

## Phase 6: Testing & Documentation (Day 9-10)

### 6.1 Integration Tests
- [ ] Full registration flow
- [ ] Presence update flow
- [ ] Multi-user scenarios

### 6.2 Performance Tests
- [ ] Benchmark concurrent operations
- [ ] Memory usage analysis
- [ ] Load testing

### 6.3 Documentation
- [ ] API documentation
- [ ] Integration guide
- [ ] Examples

## Success Criteria

1. **Functional Requirements**
   - [ ] User registration with multiple contacts
   - [ ] Presence state management
   - [ ] Subscription handling
   - [ ] Automatic buddy lists
   - [ ] Event-driven updates

2. **Performance Requirements**
   - [ ] Handle 10,000+ registered users
   - [ ] Sub-millisecond lookups
   - [ ] Concurrent access support

3. **Integration Requirements**
   - [ ] Clean API for session-core
   - [ ] Event bus integration
   - [ ] P2P and B2BUA modes

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