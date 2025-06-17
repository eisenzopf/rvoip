# Call-Engine Session-Core Integration Update Plan

## 🚨 **CRITICAL ISSUE: API Mismatch Between Call-Engine and Session-Core**

### **Current Problems:**

1. **Non-existent APIs**: Call-engine uses these APIs that don't exist in session-core:
   - `ServerSessionManager`, `ServerConfig`, `create_full_server_manager`  
   - `IncomingCallEvent`, `CallerInfo`, `CallDecision`, `IncomingCallNotification`
   - Bridge management APIs that aren't exposed

2. **Architectural Mismatch**: 
   - Call-engine expects server-oriented APIs
   - Session-core actually provides `SessionCoordinator` with `SessionControl` and `MediaControl` traits
   - Session-core uses `CallHandler` trait for incoming calls, not `IncomingCallNotification`

3. **Recent Session-Core Changes**:
   - Moved 2,400+ lines of business logic out to basic primitives
   - Simplified API to focus on session coordination only
   - Preparing for dialog-core unified DialogManager integration

## 📋 **COMPREHENSIVE UPDATE PLAN FOR CALL-ENGINE**

### **Phase 1: Fix Import and Type Mismatches** 🚨 **URGENT**

#### 1.1 Update Core Types and Imports
- [ ] Update `src/lib.rs` imports
  - [ ] Remove non-existent session-core API imports
  - [ ] Add actual session-core API imports
- [ ] Update `src/orchestrator/core.rs` imports
  - [ ] Replace ServerSessionManager imports with SessionCoordinator
  - [ ] Update bridge-related imports
- [ ] Fix type mismatches throughout codebase

```rust
// REMOVE these non-existent imports:
use rvoip_session_core::api::{
    ServerSessionManager, ServerConfig, create_full_server_manager,
    IncomingCallEvent, CallerInfo, CallDecision, IncomingCallNotification,
    BridgeId, BridgeConfig, BridgeInfo, BridgeEvent, BridgeEventType,
};

// REPLACE with actual session-core API:
use rvoip_session_core::api::{
    // Core types
    SessionId, CallSession, CallState, IncomingCall, CallDecision,
    SessionStats, MediaInfo,
    // Handlers and control
    CallHandler, SessionControl, MediaControl,
    // Builder
    SessionManagerBuilder, SessionManagerConfig,
    // Main coordinator
    SessionCoordinator,
};
```

#### 1.2 Replace ServerSessionManager with SessionCoordinator
- [ ] Update `CallCenterEngine` struct
  - [ ] Change `server_manager: Arc<ServerSessionManager>` to `coordinator: Arc<SessionCoordinator>`
  - [ ] Update all field references
- [ ] Update all method implementations
  - [ ] Replace `self.server_manager` with `self.coordinator`
  - [ ] Update method calls to use SessionControl trait
- [ ] Fix compilation errors from type changes

#### 1.3 Implement CallHandler Instead of IncomingCallNotification
- [ ] Remove `CallCenterNotificationHandler` with `IncomingCallNotification`
- [ ] Create new `CallCenterHandler` implementing `CallHandler`
- [ ] Update callback signatures
  - [ ] `on_incoming_call(call: IncomingCall) -> CallDecision`
  - [ ] `on_call_ended(session: CallSession, reason: &str)`
- [ ] Handle CallDecision enum properly

### **Phase 2: Redesign Call Center Architecture**

#### 2.1 Create CallCenterHandler Implementation
- [ ] Create new handler structure
  ```rust
  #[derive(Clone)]
  struct CallCenterHandler {
      engine: Arc<CallCenterEngine>,
  }
  ```
- [ ] Implement CallHandler trait
  - [ ] Implement `on_incoming_call` with routing logic
  - [ ] Implement `on_call_ended` with cleanup logic
- [ ] Handle deferred decisions for async processing
- [ ] Add error handling and logging

#### 2.2 Update CallCenterEngine Creation
- [ ] Rewrite `CallCenterEngine::new()` method
  - [ ] Remove TransactionManager parameter
  - [ ] Use SessionManagerBuilder instead of create_full_server_manager
  - [ ] Create handler with engine reference
  - [ ] Handle circular reference between engine and handler
- [ ] Update configuration mapping
  - [ ] Map CallCenterConfig to SessionManagerConfig
  - [ ] Set appropriate SIP settings
- [ ] Initialize with CallHandler
- [ ] Start SessionCoordinator

#### 2.3 Fix Agent Registration
- [ ] Update `register_agent()` method
  - [ ] Remove session pre-creation
  - [ ] Track agents in available pool only
  - [ ] Return tracking ID instead of session ID
- [ ] Update agent availability tracking
- [ ] Fix agent status management

### **Phase 3: Rework Bridge Management**

#### 3.1 Investigate Available Bridge Functionality
- [ ] Check session-core API for conference/bridge features
- [ ] Review session-core examples for bridge patterns
- [ ] Determine if media-core integration needed
- [ ] Document findings

#### 3.2 Implement Local Bridge Management
- [ ] Create `BridgeManager` struct
  ```rust
  struct BridgeManager {
      bridges: HashMap<String, CallBridge>,
  }
  ```
- [ ] Implement bridge lifecycle
  - [ ] Create bridge between sessions
  - [ ] Track active bridges
  - [ ] Handle bridge termination
- [ ] Replace session-core bridge API calls
- [ ] Add bridge event emulation if needed

#### 3.3 Update Bridge-Related Methods
- [ ] Fix `create_conference()` method
- [ ] Update `transfer_call()` implementation
- [ ] Handle bridge monitoring without events
- [ ] Implement local bridge statistics

### **Phase 4: Update Call Processing**

#### 4.1 Rewrite Incoming Call Processing
- [ ] Update `process_incoming_call_event()` method
  - [ ] Use IncomingCall instead of IncomingCallEvent
  - [ ] Map call information correctly
  - [ ] Return proper CallDecision
- [ ] Fix customer analysis logic
- [ ] Update routing decision logic
- [ ] Handle call acceptance/rejection

#### 4.2 Fix Call State Management
- [ ] Update call tracking to use CallSession
- [ ] Fix state transitions
- [ ] Handle call lifecycle events
- [ ] Update statistics collection

#### 4.3 Implement Deferred Call Handling
- [ ] Create async call processing queue
- [ ] Implement deferred decision handling
- [ ] Add call accept/reject methods
- [ ] Handle timeout scenarios

### **Phase 5: Adapt Business Logic Integration**

#### 5.1 Use Session-Core Basic Primitives
- [ ] Import basic primitives from session-core
  - [ ] BasicSessionGroup for conferences
  - [ ] BasicResourceAllocation for resources
  - [ ] BasicSessionPriority for QoS
  - [ ] BasicEventBus for events
- [ ] Update existing code to use primitives
- [ ] Remove dependencies on removed APIs

#### 5.2 Prepare for Business Logic Migration
- [ ] Create directory structure
  - [ ] `src/conference/` for conference management
  - [ ] `src/policy/` for policy engine
  - [ ] `src/priority/` for QoS management
- [ ] Create placeholder modules
- [ ] Plan integration points

#### 5.3 Implement Missing Business Logic
- [ ] Conference management (from session-core)
  - [ ] Port SessionGroupManager logic
  - [ ] Adapt for call center use
  - [ ] Integrate with existing code
- [ ] Policy engine (from session-core)
  - [ ] Port SessionPolicyManager logic
  - [ ] Connect to routing decisions
  - [ ] Add call center policies
- [ ] QoS management (from session-core)
  - [ ] Port SessionPriorityManager logic
  - [ ] Integrate with agent assignment
  - [ ] Add priority queuing

### **Phase 6: Testing and Validation**

#### 6.1 Fix Compilation Errors
- [ ] Run `cargo build` and fix all errors
- [ ] Update all examples to compile
- [ ] Fix all test compilation issues
- [ ] Ensure clean build

#### 6.2 Update Examples
- [ ] Fix `examples/call_center_with_database.rs`
  - [ ] Remove ServerSessionManager usage
  - [ ] Use SessionCoordinator
  - [ ] Update to working example
- [ ] Create new examples
  - [ ] Basic call routing example
  - [ ] Agent management example
  - [ ] Queue management example

#### 6.3 Create Integration Tests
- [ ] Test agent registration
- [ ] Test incoming call routing
- [ ] Test call state management
- [ ] Test queue functionality
- [ ] Test database integration

#### 6.4 Validate Functionality
- [ ] Manual testing with SIP clients
- [ ] Performance testing
- [ ] Load testing with multiple agents
- [ ] Edge case testing

### **Phase 7: Documentation and Cleanup**

#### 7.1 Update Documentation
- [ ] Update README.md with new architecture
- [ ] Document API changes
- [ ] Create migration guide
- [ ] Update inline documentation

#### 7.2 Clean Up Code
- [ ] Remove dead code
- [ ] Remove unused imports
- [ ] Fix all warnings
- [ ] Run clippy and fix issues

#### 7.3 Final Integration
- [ ] Ensure all features working
- [ ] Performance optimization
- [ ] Security review
- [ ] Prepare for production use

## 🚀 **RECOMMENDED IMPLEMENTATION ORDER**

1. **IMMEDIATE (Day 1-2)**: Fix compilation errors
   - Update imports to use actual session-core API
   - Replace ServerSessionManager with SessionCoordinator
   - Implement CallHandler trait

2. **SHORT TERM (Day 3-5)**: Restore basic functionality
   - Get CallCenterEngine creation working
   - Fix agent registration
   - Handle incoming calls through CallHandler

3. **MEDIUM TERM (Week 2)**: Implement missing features
   - Design bridge management solution
   - Integrate with media-core if needed
   - Update examples and tests

4. **LONG TERM (Week 3-4)**: Complete business logic integration
   - Integrate conference management from session-core
   - Add policy engine and QoS management
   - Complete Phase 2.5 objectives

## ⚠️ **RISKS AND MITIGATIONS**

1. **Risk**: Bridge functionality might not exist in session-core
   - **Mitigation**: Implement bridge tracking in call-engine, use media-core for RTP

2. **Risk**: CallHandler pattern might not fit call center needs
   - **Mitigation**: Use deferred decision pattern with async processing

3. **Risk**: Missing transaction manager integration
   - **Mitigation**: Check if SessionCoordinator provides transaction access

4. **Risk**: Circular dependency between engine and handler
   - **Mitigation**: Use weak references or initialization pattern

## 📊 **ESTIMATED EFFORT**

- **Phase 1**: 1-2 days (urgent compilation fixes)
- **Phase 2**: 2-3 days (architecture redesign)
- **Phase 3**: 3-4 days (bridge management)
- **Phase 4**: 2-3 days (call processing)
- **Phase 5**: 1 week (business logic)
- **Phase 6**: 2-3 days (testing)
- **Phase 7**: 1-2 days (documentation)

**Total**: ~3-4 weeks for complete integration

## ✅ **SUCCESS CRITERIA**

1. **Compilation Success**
   - [ ] Call-engine compiles without errors
   - [ ] All examples compile
   - [ ] All tests compile

2. **Basic Functionality**
   - [ ] CallCenterEngine can be created
   - [ ] Agents can be registered
   - [ ] Incoming calls are received
   - [ ] Call routing works

3. **Advanced Features**
   - [ ] Queue management works
   - [ ] Call statistics collected
   - [ ] Database integration works
   - [ ] Performance acceptable

4. **Business Logic Integration**
   - [ ] Conference management integrated
   - [ ] Policy engine working
   - [ ] QoS management active
   - [ ] Event system functional

5. **Production Readiness**
   - [ ] All tests passing
   - [ ] Examples working
   - [ ] Documentation complete
   - [ ] Performance validated

## 📝 **NOTES**

- This plan addresses fundamental API mismatches between call-engine and session-core
- The session-core API is simpler than what call-engine expected
- Some features (like bridges) may need to be implemented locally
- Business logic from session-core Phase 12 will enhance call-engine significantly
- The final result will be a more robust and properly architected system 