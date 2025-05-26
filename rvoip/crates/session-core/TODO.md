# Session Core - TODO List

This document tracks planned improvements and enhancements for the `rvoip-session-core` library.

## 🚨 CRITICAL ARCHITECTURAL REFACTORING REQUIRED

**Current Status**: Architecture violation discovered - session-core is handling SIP protocol details instead of acting as pure coordinator.

### 🔍 **ISSUE ANALYSIS**

**What We Discovered**:
1. **session-core** is manually sending SIP responses (180 Ringing, 200 OK) - this violates separation of concerns
2. **MediaManager** uses simplified mock implementation instead of media-core's MediaEngine
3. **ServerManager** handles SIP protocol details instead of coordinating between layers
4. **Architecture** doesn't follow README.md design where session-core is "Central Coordinator"

**Why This Matters**:
- **SIP Compliance**: transaction-core should handle all SIP protocol details
- **Scalability**: session-core doing too much creates bottlenecks
- **Maintainability**: Mixed concerns make code harder to maintain
- **Integration**: media-core capabilities not properly utilized

### 🎯 **REFACTORING STRATEGY**

**Phase 4 Priority**: Fix architectural violations before continuing with SIPp integration

1. **Complete media-core integration** - Replace MediaManager mock with real MediaEngine usage
2. **Remove SIP protocol handling** - session-core should NEVER send SIP responses directly  
3. **Implement event coordination** - Proper event-driven architecture between layers
4. **Test separation of concerns** - Validate each layer handles only its responsibilities

**Expected Outcome**: Clean architecture where session-core coordinates between transaction-core (SIP) and media-core (media) without handling protocol details directly.

## 📏 CODE ORGANIZATION CONSTRAINT

**CRITICAL RULE**: No library file (excluding examples, tests, and documentation) may exceed **200 lines**.
- When a file approaches 200 lines, it MUST be refactored into smaller, focused modules
- This ensures maintainability, readability, and proper separation of concerns
- Examples and tests are exempt from this constraint
- Documentation files (README.md, TODO.md, etc.) are exempt

---

## 🎯 MASTER GOAL: Self-Contained Session-Core Server API

**Objective**: Create a session-core API that can handle real SIPp connections without requiring users to import sip-core, transaction-core, or sip-transport directly.

### Target Directory Structure
```
src/
├── api/                           # Public API layer (self-contained)
│   ├── mod.rs                     # API module exports (<200 lines)
│   ├── client/                    # Client API
│   │   ├── mod.rs                 # Client exports (<200 lines)
│   │   ├── config.rs              # Client configuration (<200 lines)
│   │   ├── manager.rs             # ClientSessionManager (<200 lines)
│   │   └── operations.rs          # Client operations (<200 lines)
│   ├── server/                    # Server API  
│   │   ├── mod.rs                 # Server exports (<200 lines)
│   │   ├── config.rs              # Server configuration (<200 lines)
│   │   ├── manager.rs             # ServerSessionManager (<200 lines)
│   │   ├── operations.rs          # Server operations (<200 lines)
│   │   └── transport.rs           # Transport integration (<200 lines)
│   ├── common/                    # Shared API components
│   │   ├── mod.rs                 # Common exports (<200 lines)
│   │   ├── session.rs             # Session interface (<200 lines)
│   │   ├── events.rs              # Event types (<200 lines)
│   │   └── errors.rs              # API error types (<200 lines)
│   └── factory.rs                 # Factory functions (<200 lines)
├── session/                       # Core session management
│   ├── mod.rs                     # Session exports (<200 lines)
│   ├── manager.rs                 # SessionManager (<200 lines)
│   ├── session/                   # Session implementation
│   │   ├── mod.rs                 # Session exports (<200 lines)
│   │   ├── core.rs                # Core Session struct (<200 lines)
│   │   ├── media.rs               # Media coordination (<200 lines)
│   │   ├── state.rs               # State management (<200 lines)
│   │   └── operations.rs          # Session operations (<200 lines)
│   └── events.rs                  # Session events (<200 lines)
├── media/                         # Media coordination layer
│   ├── mod.rs                     # Media exports (<200 lines)
│   ├── manager.rs                 # MediaManager (<200 lines)
│   ├── session.rs                 # MediaSession (<200 lines)
│   ├── config.rs                  # Media configuration (<200 lines)
│   └── coordination.rs            # Session-media coordination (<200 lines)
├── transport/                     # Transport integration
│   ├── mod.rs                     # Transport exports (<200 lines)
│   ├── integration.rs             # Transport integration (<200 lines)
│   └── factory.rs                 # Transport factory (<200 lines)
└── lib.rs                         # Main library exports (<200 lines)
```

---

## 🚀 PHASE 1: API Foundation & Transport Integration (IMMEDIATE)

### 1.1 Create Self-Contained Server API Structure
- [x] **Create `src/api/server/config.rs`** - Server configuration types
  - [x] ServerConfig struct with transport settings
  - [x] Default implementations and validation
  - [x] Transport protocol selection (UDP/TCP/TLS)
  - [x] Binding address and port configuration

- [x] **Create `src/api/server/transport.rs`** - Transport integration layer
  - [x] Abstract transport creation from config
  - [x] Transport event handling integration
  - [x] Message routing to session manager
  - [x] Transport lifecycle management

- [x] **Create `src/api/server/manager.rs`** - ServerSessionManager
  - [x] Incoming call handling (INVITE processing)
  - [x] Session creation from incoming requests
  - [x] Response generation and sending
  - [x] Session lifecycle coordination

- [x] **Create `src/api/server/operations.rs`** - Server operations
  - [x] accept_call(), reject_call(), end_call()
  - [x] hold_call(), resume_call()
  - [x] Media coordination for server sessions
  - [x] Event subscription and notification

### 1.2 Create Factory Functions
- [x] **Create `src/api/factory.rs`** - High-level factory functions
  - [x] `create_sip_server(config) -> ServerManager`
  - [x] `create_sip_client(config) -> ClientManager`
  - [x] Automatic transport setup and integration
  - [x] Media manager initialization

### 1.3 Transport Integration Layer
- [x] **Create `src/transport/integration.rs`** - Bridge to sip-transport
  - [x] Transport trait implementation using sip-transport
  - [x] Message parsing and routing
  - [x] Error handling and conversion
  - [x] Event propagation to session layer

- [x] **Create `src/transport/factory.rs`** - Transport factory
  - [x] Create transports from configuration
  - [x] Protocol-specific transport creation
  - [x] Transport lifecycle management

### 1.4 Update API Exports
- [x] **Update `src/api/mod.rs`** - Clean public API exports
  - [x] Export only high-level types and functions
  - [x] Hide internal implementation details
  - [x] Provide clear documentation

- [x] **Update `src/lib.rs`** - Main library exports
  - [x] Export API layer as primary interface
  - [x] Maintain backward compatibility
  - [x] Clear module organization

**Success Criteria for Phase 1:**
- [x] `create_sip_server()` function works without external imports
- [x] Server can bind to UDP port and receive messages
- [x] Basic INVITE processing without media
- [x] All files under 200 lines

**Current Status**: 
- ✅ **Phase 1**: ✅ COMPLETE - Self-contained API foundation (16/16 tasks)
- ✅ **Phase 2**: ✅ COMPLETE - Automatic media coordination (4/4 tasks)
- ✅ **Phase 3.1**: ✅ COMPLETE - Enhanced Server Operations with Transaction-Core Integration (4/4 tasks)
- 🔄 **Phase 3.2**: 🚀 READY - SIPp Integration Testing

**✅ COMPLETED (24/24 tasks)**:

**PHASE 3.1: ENHANCED SERVER OPERATIONS WITH TRANSACTION-CORE INTEGRATION (NEW)**

21. **Transaction-Core Integration Architecture** - Single shared transport
    - Fixed factory to use one transport shared between session-core and transaction-core
    - Eliminated port conflicts by removing duplicate transport creation
    - Clean integration where transaction-core handles SIP protocol details

22. **Enhanced ServerManager with Transaction-Core** - Complete SIP flow
    - accept_call() sends 200 OK responses via transaction-core with automatic media setup
    - reject_call() sends error responses via transaction-core with proper session cleanup
    - end_call() handles BYE requests and responses with automatic media cleanup
    - Transaction-core handles all SIP protocol details (180 Ringing, 200 OK, ACK, BYE)

23. **API Export Enhancement** - User convenience
    - Added TransportProtocol to API module re-exports
    - Users can now import directly from rvoip_session_core::api
    - Clean API surface without requiring deep imports

24. **Transaction Integration Testing** - Comprehensive validation
    - api_transaction_integration_test.rs demonstrates working integration
    - All server operations accessible through clean API
    - Verified transaction-core integration working properly
    - Single transport eliminates architecture complexity

**Phase 3.1 Success Criteria Met**:
- ✅ Transaction-core handles all SIP protocol details automatically
- ✅ ServerManager focuses on session management and media coordination
- ✅ Single shared transport eliminates port conflicts
- ✅ Clean API surface - users only need create_sip_server() and operations
- ✅ Complete INVITE/200 OK/ACK flow working
- ✅ Automatic media coordination in all server operations
- ✅ All compilation and runtime tests passing

---

## 🎵 PHASE 2: Media Manager Implementation ✅ COMPLETE

### 2.1 Create MediaManager Infrastructure ✅ COMPLETE
- [x] **Enhanced Session Media Operations** - Automatic media coordination
  - [x] MediaSession creation and lifecycle via existing session.media operations
  - [x] Media state management (Negotiating, Active, Paused, Stopped)
  - [x] Session-to-media mapping through session.media_session_id
  - [x] Automatic media setup/cleanup coordination

### 2.2 Integrate MediaManager with Session Layer ✅ COMPLETE
- [x] **Update Session Media Operations** - Session media operations
  - [x] Automatic MediaManager integration via existing session.start_media()
  - [x] Media state transitions via session.set_media_negotiating()
  - [x] Hold/resume media coordination via session.pause_media()/resume_media()
  - [x] Error handling and recovery in all media operations

### 2.3 Update API Layer for Media ✅ COMPLETE
- [x] **Enhanced Server Operations** - Add automatic media operations
  - [x] Automatic media setup in accept_call() - calls session.start_media()
  - [x] Media coordination in hold_call() - calls session.pause_media()
  - [x] Media coordination in resume_call() - calls session.resume_media()
  - [x] Media cleanup in end_call() - calls session.stop_media()

### 2.4 API Integration and Testing ✅ COMPLETE
- [x] **SipServer API Enhancement** - Complete server operations
  - [x] Added hold_call() and resume_call() to SipServer
  - [x] All operations available through single API
  - [x] No manual media state management required by users

**Success Criteria for Phase 2:** ✅ ALL MET
- [x] accept_call() automatically sets up media
- [x] hold_call() automatically pauses media
- [x] resume_call() automatically resumes media
- [x] end_call() automatically cleans up media
- [x] No manual media state management required
- [x] All files under 200 lines

---

## 🌐 PHASE 3: Complete SIPp Integration (VALIDATION)

### 3.1 Enhanced Server Operations ✅ COMPLETE
- [x] **Update `src/api/server/manager.rs`** - Full INVITE handling
  - [x] Complete INVITE/200 OK/ACK flow via transaction-core
  - [x] SDP negotiation integration
  - [x] Media setup coordination
  - [x] Error response generation

- [x] **Transaction-Core Integration** - Single shared transport
  - [x] Fixed factory to use one transport for both session-core and transaction-core
  - [x] Eliminated port conflicts and architecture complexity
  - [x] Transaction-core handles all SIP protocol details automatically

- [x] **API Export Enhancement** - User convenience
  - [x] Added TransportProtocol to API module re-exports
  - [x] Clean API surface without requiring deep imports

- [x] **Integration Testing** - Comprehensive validation
  - [x] Created api_transaction_integration_test.rs
  - [x] Verified transaction-core integration working properly
  - [x] All server operations accessible through clean API

### 3.2 SIPp Integration Testing 🔄 IN PROGRESS
- [x] **Create `examples/sipp_server.rs`** - Production SIPp server ✅ COMPLETE
  - [x] Uses only session-core API
  - [x] Handles multiple concurrent calls (up to 1000)
  - [x] Complete call lifecycle support (auto-accept incoming calls)
  - [x] Comprehensive logging with DEBUG level tracing
  - [x] Graceful shutdown with Ctrl+C
  - [x] Production-ready configuration

- [ ] **Create SIPp test scenarios** - Real SIP traffic validation
  - [ ] SIPp UAC scenario against our server
  - [ ] INVITE/200 OK/ACK flow with real SIP messages
  - [ ] BYE request handling
  - [ ] Error response scenarios

- [ ] **SDP Integration Enhancement** - Real media negotiation
  - [ ] SDP generation from media config
  - [ ] SDP parsing and validation
  - [ ] Media parameter extraction
  - [ ] Direction attribute handling

- [ ] **Event System Enhancement** - Complete event types
  - [ ] Call establishment events
  - [ ] Media state change events
  - [ ] Error and timeout events
  - [ ] Session termination events

**Success Criteria for Phase 3:**
- [ ] SIPp UAC scenario works against our server
- [ ] SIPp UAS scenario works with our client
- [ ] Multiple concurrent calls supported
- [ ] Media flows established and terminated
- [ ] All operations use only session-core API
- [ ] All files under 200 lines

---

## 🔧 PHASE 4: API Refinement & Production Features (ENHANCEMENT)

### 4.1 Configuration Enhancement
- [ ] **Create `src/api/common/config.rs`** - Advanced configuration
  - [ ] Codec preferences and capabilities
  - [ ] Transport protocol selection
  - [ ] Security settings (TLS/SRTP)
  - [ ] Performance tuning parameters

### 4.2 Error Handling Enhancement
- [ ] **Update `src/api/common/errors.rs`** - Comprehensive error types
  - [ ] Transport-specific errors
  - [ ] Media-specific errors
  - [ ] Protocol violation errors
  - [ ] Configuration errors

### 4.3 Monitoring and Observability
- [ ] **Create `src/api/common/metrics.rs`** - Metrics collection
  - [ ] Call success/failure rates
  - [ ] Media quality metrics
  - [ ] Performance counters
  - [ ] Resource usage tracking

### 4.4 Advanced Features
- [ ] **Authentication integration** - Digest auth support
- [ ] **REFER handling** - Call transfer support
- [ ] **Security enhancements** - TLS and SRTP
- [ ] **Performance optimization** - Connection pooling

**Success Criteria for Phase 4:**
- [ ] Production-ready configuration options
- [ ] Comprehensive error handling
- [ ] Performance monitoring capabilities
- [ ] Advanced SIP features working
- [ ] All files under 200 lines

---

## 🏗️ PHASE 4: ARCHITECTURAL REFACTORING - PROPER SEPARATION OF CONCERNS (CRITICAL)

### 🚨 **ARCHITECTURE VIOLATION DISCOVERED**

**Current Issue**: session-core is violating separation of concerns by manually handling SIP responses (180 Ringing, 200 OK) which should be transaction-core's responsibility.

**Root Cause**: According to README.md architecture, session-core should be a "Central Coordinator" that bridges SIP signaling (via transaction-core) with media processing (via media-core), NOT a SIP protocol handler.

### 🎯 **CORRECT ARCHITECTURE DESIGN**

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
├─────────────────────────────────────────────────────────────┤
│                 *** session-core ***                        │
│           (Session Manager - Central Coordinator)           │
│      • Session Lifecycle Management  • Media Coordination   │
│      • Dialog State Coordination     • Event Orchestration  │  
│      • Reacts to Transaction Events  • Coordinates Media    │
├─────────────────────────────────────────────────────────────┤
│         Processing Layer                                    │
│  transaction-core              │  media-core               │
│  (SIP Protocol Handler)        │  (Media Processing)       │
│  • Sends SIP Responses         │  • Codec Management       │
│  • Manages SIP State Machine   │  • Audio Processing       │
│  • Handles Retransmissions     │  • RTP Stream Management  │
├─────────────────────────────────────────────────────────────┤
│              Transport Layer                                │
│  sip-transport    │  rtp-core    │  ice-core               │
└─────────────────────────────────────────────────────────────┘
```

### 🔧 **REFACTORING PLAN**

#### 4.1 Media-Core Integration Completion ⚠️ CRITICAL
- [ ] **Fix MediaManager Implementation** - Complete media-core integration
  - [ ] Remove simplified MediaStream and use media-core's MediaSession directly
  - [ ] Implement proper MediaSessionParams conversion from session-core config
  - [ ] Add real pause/resume operations through media-core API
  - [ ] Implement SDP-to-MediaConfig conversion for codec negotiation
  - [ ] Add media quality monitoring and event propagation

- [ ] **Create Media Coordination Bridge** - `src/media/coordination.rs` (<200 lines)
  - [ ] SessionMediaCoordinator that maps SessionId -> MediaSessionId
  - [ ] Automatic media lifecycle management (create/start/pause/resume/stop)
  - [ ] SDP negotiation integration with media-core capabilities
  - [ ] Media event propagation to session layer

- [ ] **Refactor Media Configuration** - `src/media/config.rs` (<200 lines)
  - [ ] Convert session-core MediaConfig to media-core MediaSessionParams
  - [ ] Codec preference mapping (AudioCodecType -> PayloadType)
  - [ ] Media direction handling (sendrecv, sendonly, recvonly, inactive)
  - [ ] RTP stream configuration extraction

#### 4.2 Transaction-Core Integration Refactoring ⚠️ CRITICAL
- [ ] **Remove SIP Response Handling from ServerManager** - Architecture fix
  - [ ] Remove manual 180 Ringing response sending
  - [ ] Remove manual 200 OK response creation and sending
  - [ ] Remove manual error response handling
  - [ ] session-core should ONLY react to transaction events, not send responses

- [ ] **Create Transaction Event Coordinator** - `src/transaction/coordinator.rs` (<200 lines)
  - [ ] TransactionEventCoordinator that reacts to transaction-core events
  - [ ] Maps transaction events to session lifecycle changes
  - [ ] Coordinates session state with transaction state
  - [ ] Propagates transaction events to media coordination layer

- [ ] **Implement Proper Session Coordination** - Refactor ServerManager
  - [ ] React to TransactionEvent::InviteReceived -> create session, coordinate media
  - [ ] React to TransactionEvent::ResponseSent -> update session state
  - [ ] React to TransactionEvent::AckReceived -> confirm session establishment
  - [ ] React to TransactionEvent::ByeReceived -> coordinate session termination

#### 4.3 Session-Core as Pure Coordinator ⚠️ CRITICAL
- [ ] **Refactor Session Operations** - Remove SIP protocol handling
  - [ ] accept_call() should coordinate media setup and notify transaction-core to send 200 OK
  - [ ] reject_call() should coordinate cleanup and notify transaction-core to send error
  - [ ] hold_call() should coordinate media pause and notify transaction-core for re-INVITE
  - [ ] end_call() should coordinate media cleanup and notify transaction-core for BYE

- [ ] **Create Session-Transaction Bridge** - `src/session/transaction_bridge.rs` (<200 lines)
  - [ ] SessionTransactionBridge that coordinates between session and transaction layers
  - [ ] Session state changes trigger appropriate transaction-core notifications
  - [ ] Transaction events trigger appropriate session state changes
  - [ ] Clean separation of concerns with proper event flow

- [ ] **Implement Event-Driven Architecture** - Pure coordination
  - [ ] Session operations emit events that transaction-core subscribes to
  - [ ] Transaction events trigger session state changes and media coordination
  - [ ] Media events trigger session state updates and transaction notifications
  - [ ] No direct SIP protocol handling in session-core

#### 4.4 API Layer Simplification 🔄 ENHANCEMENT
- [ ] **Simplify Server API** - Remove SIP protocol complexity
  - [ ] SipServer operations focus on session management only
  - [ ] Hide transaction-core and media-core complexity from users
  - [ ] Provide simple accept_call(), reject_call(), hold_call() operations
  - [ ] All SIP protocol details handled automatically by transaction-core

- [ ] **Update Factory Functions** - Clean integration
  - [ ] create_sip_server() sets up proper event coordination between layers
  - [ ] Automatic transaction-core and media-core integration
  - [ ] Event bus setup for proper layer communication
  - [ ] Configuration validation and default setup

### 🎯 **SUCCESS CRITERIA FOR PHASE 4**

#### Architecture Compliance
- [ ] session-core NEVER sends SIP responses directly
- [ ] session-core ONLY reacts to transaction events and coordinates media
- [ ] transaction-core handles ALL SIP protocol details (responses, retransmissions, timers)
- [ ] media-core handles ALL media processing (codecs, RTP, quality monitoring)

#### Integration Quality
- [ ] Complete media-core integration with real MediaEngine usage
- [ ] Proper SDP negotiation through media-core capabilities
- [ ] Real media pause/resume operations through media-core API
- [ ] Media quality monitoring and event propagation

#### API Simplicity
- [ ] Users only need session-core API imports
- [ ] SIPp compatibility without protocol complexity
- [ ] All operations work through simple accept_call(), reject_call(), etc.
- [ ] Complete call lifecycle support with automatic coordination

#### Code Quality
- [ ] All files under 200 lines
- [ ] Clear separation of concerns across modules
- [ ] Comprehensive error handling and logging
- [ ] Production-ready performance and reliability

### 🚨 **IMMEDIATE PRIORITY**

**Phase 4.1 and 4.2 are CRITICAL** - The current architecture violates the design principles and will cause issues with:
- SIP compliance (transaction-core should handle protocol details)
- Scalability (session-core doing too much)
- Maintainability (mixed concerns)
- Integration (media-core not properly utilized)

**Next Steps**:
1. Complete media-core integration in MediaManager
2. Remove SIP response handling from ServerManager  
3. Implement proper event-driven coordination
4. Test with SIPp to ensure compliance

---

## 📊 PROGRESS TRACKING

### Current Status: **Phase 4 - Architectural Refactoring (CRITICAL)**
- **Phase 1 - API Foundation**: ✅ COMPLETE (16/16 tasks)
- **Phase 2 - Media Coordination**: ✅ COMPLETE (4/4 tasks)  
- **Phase 3.1 - Enhanced Server Operations**: ✅ COMPLETE (4/4 tasks)
- **Phase 3.2 - SIPp Integration**: ⏸️ PAUSED (1/4 tasks) - Architecture issues discovered
- **Phase 4 - Architectural Refactoring**: 🚨 CRITICAL (0/16 tasks) - **IMMEDIATE PRIORITY**
- **Total Completed**: 24/44 tasks (55%)
- **Next Milestone**: Complete media-core integration and remove SIP protocol handling

### File Count Monitoring
- **Current API files**: 12 (all under 200 lines ✅)
- **Target API files**: 25+ (all under 200 lines)
- **Refactoring needed**: Major - architecture violation fixes required

### Recent Discoveries
- 🚨 **Architecture Violation**: session-core manually sending SIP responses (180 Ringing, 200 OK)
- 🚨 **Incomplete Integration**: MediaManager not using media-core's MediaEngine properly
- 🚨 **Mixed Concerns**: session-core handling SIP protocol details instead of coordinating
- 🚨 **Design Violation**: Not following README.md architecture where session-core is pure coordinator

### Critical Issues to Address
1. **SIP Protocol Handling**: session-core should NEVER send SIP responses directly
2. **Media Integration**: MediaManager should use media-core's MediaEngine, not simplified mock
3. **Event Coordination**: Need proper event-driven architecture between layers
4. **Separation of Concerns**: Each layer should handle only its designated responsibilities

---

## 🎯 IMMEDIATE NEXT STEPS

1. **Start Phase 4.1**: Complete media-core integration in MediaManager
2. **Fix MediaManager**: Replace simplified implementation with real media-core usage
3. **Remove SIP Handling**: Remove all SIP response sending from ServerManager
4. **Implement Coordination**: Create proper event-driven coordination between layers
5. **Test Architecture**: Validate proper separation of concerns with comprehensive tests

---

## ✅ COMPLETED - Core Infrastructure Foundation

### Session Manager & Dialog Integration
- [x] SessionManager with async event processing
- [x] Session creation and lifecycle management  
- [x] Integration with transaction-core and dialog management
- [x] Event-driven architecture with EventBus
- [x] Session-to-dialog mapping and coordination
- [x] DialogManager integration within SessionManager
- [x] Dialog-to-session association and mapping
- [x] Dialog lifecycle coordination with session states
- [x] Event propagation between dialogs and sessions
- [x] Dialog recovery mechanisms

### SDP Negotiation & Media Coordination
- [x] SdpContext integration in Dialog management
- [x] SDP offer/answer state machine (Initial, OfferSent, OfferReceived, Complete)
- [x] SDP generation for outgoing calls (create_audio_offer)
- [x] SDP answer generation for incoming calls (create_audio_answer)
- [x] SDP renegotiation support for re-INVITEs
- [x] Media configuration extraction (extract_media_config)
- [x] Hold/resume operations (put_call_on_hold, resume_held_call)
- [x] SDP direction handling (sendrecv, sendonly, recvonly, inactive)

### Transaction Layer Integration
- [x] Transaction event subscription in SessionManager
- [x] Transaction event processing loop for session management
- [x] Transaction-to-dialog mapping for proper event routing
- [x] Transaction state timeouts and error handling
- [x] Transaction cancellation for INVITE requests
- [x] Forked INVITE transaction handling
- [x] Transaction termination cleanup
- [x] Retransmission handling coordination with transaction layer
- [x] ACK handling for non-2xx responses (auto-generated by transaction layer)
- [x] Transaction timer events handling (Timer A-K)
- [x] Transaction state synchronization with session/dialog states

### Request Generation and Processing
- [x] Request generation for all SIP methods
- [x] Proper header generation (Via, Contact, CSeq, etc.)
- [x] Incoming request handling via transactions
- [x] Response creation and sending through transactions
- [x] ACK handling for INVITE transactions
- [x] ACK for 2xx responses (TU responsibility)
- [x] Response handling for different transaction types

### Error Handling & Robustness
- [x] Detailed error types with specific categorization (network, protocol, application)
- [x] Retry mechanisms for recoverable errors
- [x] Error propagation with context through the stack
- [x] Graceful fallback for non-critical failures
- [x] Timeout handling for all operations
- [x] Boundary checking for user inputs

### Early Dialog Management
- [x] Support for multiple simultaneous early dialogs
- [x] Forking scenario handling per RFC 3261 Section 12.1.2

### Async Runtime Optimizations
- [x] Event-driven mechanisms replacing polling-based subscription tracking
- [x] Efficient task management for event handling
- [x] DashMap for concurrent access to transaction subscriptions
- [x] Proper backpressure handling in event channels
- [x] tokio::select! for efficient multiplexing of event sources
- [x] Reduced number of spawned tasks by consolidating related functionality
- [x] Channel buffer size tuning based on expected transaction volume
- [x] Dead task cleanup for orphaned subscriptions
- [x] Benchmarks for async runtime performance
- [x] Lock contention fixes in high-volume scenarios

### Public API & Helper Functions
- [x] High-level client API for common call scenarios
- [x] Server API for registration, proxy, and B2BUA use cases
- [x] Session modification API (hold, resume, transfer)
- [x] Media control interface (mute, codec switching)
- [x] Quality metrics reporting API
- [x] Event subscription model for asynchronous operations
- [x] Logging and tracing interfaces
- [x] Configuration management API
- [x] Transport abstraction for protocol flexibility
- [x] Helper functions for dialog operations:
  - [x] put_call_on_hold, resume_held_call
  - [x] verify_dialog_active, update_codec_preferences
  - [x] create_dialog_from_invite, send_dialog_request
  - [x] update_dialog_media, get_dialog_media_config 