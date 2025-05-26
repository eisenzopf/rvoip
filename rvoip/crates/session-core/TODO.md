# Session Core - TODO List

This document tracks planned improvements and enhancements for the `rvoip-session-core` library.

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
- [ ] **Create `src/api/server/config.rs`** - Server configuration types
  - [ ] ServerConfig struct with transport settings
  - [ ] Default implementations and validation
  - [ ] Transport protocol selection (UDP/TCP/TLS)
  - [ ] Binding address and port configuration

- [ ] **Create `src/api/server/transport.rs`** - Transport integration layer
  - [ ] Abstract transport creation from config
  - [ ] Transport event handling integration
  - [ ] Message routing to session manager
  - [ ] Transport lifecycle management

- [ ] **Create `src/api/server/manager.rs`** - ServerSessionManager
  - [ ] Incoming call handling (INVITE processing)
  - [ ] Session creation from incoming requests
  - [ ] Response generation and sending
  - [ ] Session lifecycle coordination

- [ ] **Create `src/api/server/operations.rs`** - Server operations
  - [ ] accept_call(), reject_call(), end_call()
  - [ ] hold_call(), resume_call()
  - [ ] Media coordination for server sessions
  - [ ] Event subscription and notification

### 1.2 Create Factory Functions
- [ ] **Create `src/api/factory.rs`** - High-level factory functions
  - [ ] `create_sip_server(config) -> ServerManager`
  - [ ] `create_sip_client(config) -> ClientManager`
  - [ ] Automatic transport setup and integration
  - [ ] Media manager initialization

### 1.3 Transport Integration Layer
- [ ] **Create `src/transport/integration.rs`** - Bridge to sip-transport
  - [ ] Transport trait implementation using sip-transport
  - [ ] Message parsing and routing
  - [ ] Error handling and conversion
  - [ ] Event propagation to session layer

- [ ] **Create `src/transport/factory.rs`** - Transport factory
  - [ ] Create transports from configuration
  - [ ] Protocol-specific transport creation
  - [ ] Transport lifecycle management

### 1.4 Update API Exports
- [ ] **Update `src/api/mod.rs`** - Clean public API exports
  - [ ] Export only high-level types and functions
  - [ ] Hide internal implementation details
  - [ ] Provide clear documentation

- [ ] **Update `src/lib.rs`** - Main library exports
  - [ ] Export API layer as primary interface
  - [ ] Maintain backward compatibility
  - [ ] Clear module organization

**Success Criteria for Phase 1:**
- [ ] `create_sip_server()` function works without external imports
- [ ] Server can bind to UDP port and receive messages
- [ ] Basic INVITE processing without media
- [ ] All files under 200 lines

---

## 🎵 PHASE 2: Media Manager Implementation (HIGH PRIORITY)

### 2.1 Create MediaManager Infrastructure
- [ ] **Create `src/media/manager.rs`** - MediaManager implementation
  - [ ] MediaSession creation and lifecycle
  - [ ] RTP stream coordination with rtp-core
  - [ ] Media state management
  - [ ] Session-to-media mapping

- [ ] **Create `src/media/session.rs`** - MediaSession implementation
  - [ ] Individual media session handling
  - [ ] RTP stream start/stop/pause operations
  - [ ] Media configuration management
  - [ ] Quality metrics collection

- [ ] **Create `src/media/config.rs`** - Media configuration
  - [ ] MediaConfig struct with codec preferences
  - [ ] RTP parameters and addressing
  - [ ] Media direction handling
  - [ ] Default configurations

- [ ] **Create `src/media/coordination.rs`** - Session-media coordination
  - [ ] Automatic media setup on session creation
  - [ ] SDP-to-media configuration mapping
  - [ ] Media state synchronization
  - [ ] Cleanup on session termination

### 2.2 Integrate MediaManager with Session Layer
- [ ] **Update `src/session/session/media.rs`** - Session media operations
  - [ ] Automatic MediaManager integration
  - [ ] Media state transitions
  - [ ] Hold/resume media coordination
  - [ ] Error handling and recovery

- [ ] **Update `src/session/manager.rs`** - SessionManager media integration
  - [ ] MediaManager initialization
  - [ ] Session-media lifecycle coordination
  - [ ] Media event handling
  - [ ] Resource cleanup

### 2.3 Update API Layer for Media
- [ ] **Update `src/api/server/operations.rs`** - Add media operations
  - [ ] Automatic media setup in accept_call()
  - [ ] Media coordination in hold/resume
  - [ ] Media cleanup in end_call()

- [ ] **Update `src/api/client/operations.rs`** - Add media operations
  - [ ] Automatic media setup in make_call()
  - [ ] Media coordination in hold/resume
  - [ ] Media cleanup in end_call()

**Success Criteria for Phase 2:**
- [ ] make_call() automatically sets up media
- [ ] hold_call() automatically pauses media
- [ ] resume_call() automatically resumes media
- [ ] end_call() automatically cleans up media
- [ ] No manual media state management required
- [ ] All files under 200 lines

---

## 🌐 PHASE 3: Complete SIPp Integration (VALIDATION)

### 3.1 Enhanced Server Operations
- [ ] **Update `src/api/server/manager.rs`** - Full INVITE handling
  - [ ] Complete INVITE/200 OK/ACK flow
  - [ ] SDP negotiation integration
  - [ ] Media setup coordination
  - [ ] Error response generation

- [ ] **Create `src/api/server/handlers.rs`** - Request handlers
  - [ ] INVITE request handler
  - [ ] BYE request handler
  - [ ] ACK request handler
  - [ ] Re-INVITE handler for hold/resume

### 3.2 SDP Integration
- [ ] **Create `src/api/common/sdp.rs`** - SDP handling
  - [ ] SDP generation from media config
  - [ ] SDP parsing and validation
  - [ ] Media parameter extraction
  - [ ] Direction attribute handling

### 3.3 Event System Enhancement
- [ ] **Update `src/api/common/events.rs`** - Complete event types
  - [ ] Call establishment events
  - [ ] Media state change events
  - [ ] Error and timeout events
  - [ ] Session termination events

### 3.4 Create SIPp Test Examples
- [ ] **Create `examples/sipp_server.rs`** - Production SIPp server
  - [ ] Uses only session-core API
  - [ ] Handles multiple concurrent calls
  - [ ] Complete call lifecycle support
  - [ ] Comprehensive logging

- [ ] **Create `examples/sipp_client.rs`** - SIPp client example
  - [ ] Outbound call generation
  - [ ] Media establishment
  - [ ] Call termination
  - [ ] Performance testing

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

---

## 📊 PROGRESS TRACKING

### Current Status: **Phase 1 - API Foundation**
- **Total Tasks**: 16
- **Completed**: 0
- **In Progress**: Planning
- **Next Milestone**: Self-contained server API structure

### File Count Monitoring
- **Current API files**: 8 (need to verify line counts)
- **Target API files**: 20+ (all under 200 lines)
- **Refactoring needed**: TBD after line count audit

---

## 🎯 IMMEDIATE NEXT STEPS

1. **Start Phase 1.1**: Create server API structure
2. **Audit existing files**: Check which files exceed 200 lines
3. **Refactor oversized files**: Split into focused modules
4. **Implement transport integration**: Bridge to sip-transport
5. **Create factory functions**: High-level API entry points

---

## Integration Notes

### Current Architecture Status
The session-core crate has a **very strong foundation** with complete session management, SIP dialog handling, SDP negotiation, and async runtime optimizations. The main work is creating a **self-contained API layer** that doesn't require users to import lower-level crates.

### Primary Goal: API Abstraction
The **main objective** is creating a clean API that internally uses transaction-core (which uses sip-transport) while providing a simple, high-level interface for SIP server and client operations.

### Success Metrics
- SIPp compatibility without external imports
- All library files under 200 lines
- Complete call lifecycle support
- Automatic media coordination
- Production-ready performance 