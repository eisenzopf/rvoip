# Session Manager Integration Plan

**Goal**: Establish `session-core` as the central coordination layer that provides a unified API for SIP clients and servers, coordinating between SIP signaling and RTP media streams.

## Overview

The Session Manager will serve as the primary interface that:
- Manages SIP session lifecycle (creation, state transitions, termination)
- Coordinates SIP dialogs with RTP media streams
- Provides clean separation of concerns while maintaining integration
- Offers a unified event system for all session-related activities
- Exposes high-level APIs that hide protocol complexity

---

## Phase 1: Core Session Manager Infrastructure ✅ (MOSTLY COMPLETE)

**Status**: 90% Complete - Foundation is solid, needs enhancement

### 1.1 Session Manager Core (✅ COMPLETE)
- [x] SessionManager struct with async event processing
- [x] Session creation and lifecycle management
- [x] Integration with transaction-core and dialog management
- [x] Event-driven architecture with EventBus
- [x] Session-to-dialog mapping and coordination
- [x] Proper async runtime optimization with DashMap and efficient task management

### 1.2 Session State Management (✅ COMPLETE)
- [x] Session struct with state transitions
- [x] SessionState enum (Initializing, Dialing, Ringing, Connected, etc.)
- [x] Transaction tracking within sessions
- [x] Proper session termination and cleanup
- [x] Session recovery and error handling

### 1.3 Dialog Integration (✅ COMPLETE)
- [x] DialogManager integration within SessionManager
- [x] Dialog-to-session association and mapping
- [x] Dialog lifecycle coordination with session states
- [x] Event propagation between dialogs and sessions
- [x] Dialog recovery mechanisms

### 1.4 Basic Public API (✅ COMPLETE)
- [x] Helper functions in helpers.rs for common operations
- [x] make_call(), answer_call(), end_call() convenience functions
- [x] Dialog management helpers (create_dialog_from_invite, etc.)
- [x] Error handling with proper context and recovery actions

---

## Phase 2: Enhanced SIP + Media Coordination 🔄 (IN PROGRESS)

**Status**: 60% Complete - SDP basics done, needs media integration

### 2.1 SDP Negotiation Integration (✅ COMPLETE)
- [x] SdpContext integration in Dialog management
- [x] SDP offer/answer state machine (Initial, OfferSent, OfferReceived, Complete)
- [x] SDP generation for outgoing calls (create_audio_offer)
- [x] SDP answer generation for incoming calls (create_audio_answer)
- [x] SDP renegotiation support for re-INVITEs
- [x] Media configuration extraction (extract_media_config)

### 2.2 Media-Core Integration 🔄 (NEEDS WORK)
- [ ] **MediaManager** creation to coordinate RTP streams
  - [ ] Create MediaManager struct to bridge session-core and media-core
  - [ ] Implement session-to-media stream mapping
  - [ ] Add media stream lifecycle management (start/stop/pause)
  - [ ] Coordinate RTP stream setup based on SDP negotiation
  - [ ] Handle media stream cleanup on session termination

- [ ] **RTP Stream Coordination**
  - [ ] Extract RTP parameters from negotiated SDP
  - [ ] Configure rtp-core streams based on session requirements
  - [ ] Handle bidirectional RTP flow (send/receive streams)
  - [ ] Coordinate RTCP reporting with session state

- [ ] **Media Event Integration**
  - [ ] Subscribe to media-core events (stream status, quality metrics)
  - [ ] Propagate media events to session layer
  - [ ] Handle media failures and recovery
  - [ ] Coordinate media-driven session state changes

### 2.3 Advanced SDP Features (PARTIALLY COMPLETE)
- [x] Hold/resume operations (put_call_on_hold, resume_held_call)
- [x] SDP direction handling (sendrecv, sendonly, recvonly, inactive)
- [ ] **Codec Negotiation Enhancement**
  - [ ] Dynamic codec preference handling
  - [ ] Codec capability discovery from media-core
  - [ ] Advanced codec parameter negotiation
  - [ ] Fallback codec selection
- [ ] **Multi-media Support**
  - [ ] Video stream coordination (future)
  - [ ] Text stream support (future)

---

## Phase 3: Production-Ready Session API 🔜 (PLANNED)

**Status**: 0% Complete - High-level API design

### 3.1 Enhanced Session Manager API
- [ ] **Session Factory Pattern**
  - [ ] SessionBuilder for complex session configurations
  - [ ] Template-based session creation for common scenarios
  - [ ] Session configuration validation and constraints
  - [ ] Default configuration management

- [ ] **Session Discovery and Management**
  - [ ] Session search and filtering capabilities
  - [ ] Session grouping and batch operations
  - [ ] Session metrics and monitoring integration
  - [ ] Session persistence and recovery after restarts

### 3.2 Call Control Features
- [ ] **Advanced Call Operations**
  - [ ] Call transfer coordination (REFER method)
  - [ ] Call forwarding and redirection
  - [ ] Conference call management
  - [ ] Call parking and retrieval

- [ ] **Media Control Integration**
  - [ ] DTMF handling via SIP INFO or RTP events
  - [ ] Voice activity detection integration
  - [ ] Media quality monitoring and reporting
  - [ ] Echo cancellation coordination

### 3.3 Event System Enhancement
- [ ] **Structured Event Types**
  - [ ] Type-safe event definitions for all session activities
  - [ ] Event correlation and tracing across layers
  - [ ] Event filtering and subscription management
  - [ ] Event persistence for debugging and analytics

- [ ] **External Event Integration**
  - [ ] Webhook support for session events
  - [ ] REST API for session monitoring
  - [ ] WebSocket event streaming
  - [ ] Event bus integration with infra-common

---

## Phase 4: Advanced Integration Features 🔜 (PLANNED)

**Status**: 0% Complete - Advanced features

### 4.1 Security and Authentication
- [ ] **Session-Level Security**
  - [ ] Digest authentication integration
  - [ ] SRTP coordination for secure media
  - [ ] TLS transport selection and management
  - [ ] Security event monitoring and alerting

### 4.2 Scalability and Performance
- [ ] **High-Volume Session Management**
  - [ ] Session pooling and resource optimization
  - [ ] Load balancing integration
  - [ ] Session distribution across multiple instances
  - [ ] Memory usage optimization for large session counts

### 4.3 Monitoring and Observability
- [ ] **Session Analytics**
  - [ ] Call quality metrics collection
  - [ ] Session duration and success rate tracking
  - [ ] Performance monitoring integration
  - [ ] Distributed tracing support

---

## Implementation Tasks - Current Sprint

### Immediate Priority: Phase 2.2 - Media-Core Integration

#### Task 1: Create MediaManager (Week 1-2)
```rust
// Design the MediaManager interface
pub struct MediaManager {
    // Media stream management
    pub async fn create_media_session(&self, config: MediaConfig) -> Result<MediaSessionId, Error>;
    pub async fn start_media(&self, session_id: &MediaSessionId) -> Result<(), Error>;
    pub async fn stop_media(&self, session_id: &MediaSessionId) -> Result<(), Error>;
    
    // RTP stream coordination
    pub async fn setup_rtp_streams(&self, config: &MediaConfig) -> Result<RtpStreamInfo, Error>;
    pub async fn update_media_direction(&self, session_id: &MediaSessionId, direction: MediaDirection) -> Result<(), Error>;
    
    // Event subscription
    pub fn subscribe_to_media_events(&self) -> mpsc::Receiver<MediaEvent>;
}
```

**Checklist:**
- [ ] Create MediaManager struct in media.rs
- [ ] Implement basic media session lifecycle
- [ ] Add RTP stream setup integration with rtp-core
- [ ] Create MediaConfig to RTP parameter conversion
- [ ] Add media event subscription system
- [ ] Write unit tests for MediaManager
- [ ] Integration tests with mock media-core

#### Task 2: Session-to-Media Coordination (Week 2-3)
```rust
// Extend SessionManager with media coordination
impl SessionManager {
    pub async fn start_session_media(&self, session_id: &SessionId) -> Result<(), Error>;
    pub async fn stop_session_media(&self, session_id: &SessionId) -> Result<(), Error>;
    pub async fn update_session_media(&self, session_id: &SessionId, sdp: &SessionDescription) -> Result<(), Error>;
}

// Extend Session with media operations
impl Session {
    pub async fn media_status(&self) -> MediaStatus;
    pub async fn get_media_config(&self) -> Option<MediaConfig>;
    pub async fn update_media_config(&self, config: MediaConfig) -> Result<(), Error>;
}
```

**Checklist:**
- [ ] Add MediaManager to SessionManager
- [ ] Implement session-to-media session mapping
- [ ] Add media lifecycle coordination in session state transitions
- [ ] Create SDP-to-MediaConfig conversion utilities
- [ ] Add media status tracking in Session
- [ ] Handle media failures and recovery
- [ ] Update helper functions to include media coordination
- [ ] Write integration tests for session+media flows

#### Task 3: Event Integration (Week 3-4)
```rust
// Add media events to the session event system
pub enum SessionEvent {
    // Existing events...
    MediaStarted { session_id: SessionId, config: MediaConfig },
    MediaStopped { session_id: SessionId, reason: String },
    MediaQualityChanged { session_id: SessionId, metrics: QualityMetrics },
    MediaFailed { session_id: SessionId, error: String },
}
```

**Checklist:**
- [ ] Extend SessionEvent enum with media events
- [ ] Add media event handlers in SessionManager
- [ ] Propagate media events to external subscribers
- [ ] Add media quality monitoring integration
- [ ] Create media event correlation with SIP events
- [ ] Add media metrics collection
- [ ] Update documentation with new event types

---

## Integration Tasks from Basic SIP Requirements

### Priority A: Call-Engine Integration (Week 2-3)

Based on BASIC_SIP_TODO.md, session-core needs enhanced integration with call-engine for:

#### A1: Call Manager Integration
```rust
// Enhance session-core for call-engine integration
impl SessionManager {
    // Support for call-engine call routing
    pub async fn create_session_for_invite(&self, invite: Request, is_inbound: bool) -> Result<Arc<Session>, Error>;
    pub async fn find_session_for_dialog(&self, call_id: &str, from_tag: &str, to_tag: &str) -> Option<Arc<Session>>;
    
    // Call state coordination with call-engine
    pub async fn link_session_to_call(&self, session_id: &SessionId, call_id: &str) -> Result<(), Error>;
    pub async fn get_sessions_for_call(&self, call_id: &str) -> Vec<Arc<Session>>;
}
```

**Checklist:**
- [ ] **Enhanced Session Creation API** for call-engine integration
  - [ ] create_session_for_invite() method for INVITE processing
  - [ ] Support for inbound vs outbound session distinction
  - [ ] Integration with call-engine's call tracking
- [ ] **Call-to-Session Mapping**
  - [ ] link_session_to_call() for call-engine coordination
  - [ ] find_session_for_dialog() for routing responses
  - [ ] get_sessions_for_call() for multi-party scenarios
- [ ] **Dialog State Coordination with Call Engine**
  - [ ] Expose dialog state changes to call-engine
  - [ ] Handle call routing decisions based on dialog state
  - [ ] Support for call transfer and forwarding scenarios

#### A2: SIP Client Integration Support
```rust
// Enhanced helper functions for sip-client
pub async fn make_outbound_call(
    session_manager: &Arc<SessionManager>,
    destination: Uri,
    local_sdp: SessionDescription
) -> Result<(Arc<Session>, DialogId), Error>;

pub async fn handle_incoming_invite(
    session_manager: &Arc<SessionManager>,
    invite: Request,
    transaction_id: TransactionKey
) -> Result<(Arc<Session>, DialogId), Error>;

pub async fn send_call_progress_response(
    session_manager: &Arc<SessionManager>,
    session_id: &SessionId,
    status_code: StatusCode,
    reason_phrase: Option<&str>
) -> Result<(), Error>;
```

**Checklist:**
- [ ] **Enhanced Call Creation Helpers**
  - [ ] make_outbound_call() with automatic SDP generation
  - [ ] handle_incoming_invite() for server-side call handling
  - [ ] Proper transaction-to-session-to-dialog coordination
- [ ] **Call Progress Management**
  - [ ] send_call_progress_response() for 180, 183 responses
  - [ ] Automatic early dialog creation for provisional responses
  - [ ] SDP handling in provisional responses (early media)
- [ ] **Call Control Operations**
  - [ ] Enhanced hold/resume with proper SIP signaling
  - [ ] Call transfer preparation (REFER support foundation)
  - [ ] Call termination with BYE transaction coordination

### Priority B: Authentication Integration (Week 3-4)

#### B1: Session-Level Authentication
```rust
// Add authentication support to session management
impl SessionManager {
    pub async fn authenticate_session(&self, session_id: &SessionId, credentials: &Credentials) -> Result<bool, Error>;
    pub async fn challenge_session(&self, session_id: &SessionId, realm: &str) -> Result<Challenge, Error>;
    pub async fn require_authentication(&self, session_id: &SessionId) -> Result<(), Error>;
}

impl Session {
    pub async fn set_authentication_state(&self, state: AuthenticationState) -> Result<(), Error>;
    pub async fn get_authentication_state(&self) -> AuthenticationState;
    pub fn is_authenticated(&self) -> bool;
}
```

**Checklist:**
- [ ] **Session Authentication State**
  - [ ] Add AuthenticationState to Session struct
  - [ ] Track authentication status per session
  - [ ] Handle authentication state transitions
- [ ] **Authentication Challenge Generation**
  - [ ] challenge_session() for 401/407 responses
  - [ ] Integration with call-engine's credential store
  - [ ] Nonce tracking and validation
- [ ] **Authenticated Session Management**
  - [ ] require_authentication() for policy enforcement
  - [ ] Session creation with authentication requirements
  - [ ] Authentication bypass for testing scenarios

### Priority C: Media Session Integration (Week 4-5)

#### C1: Media Session Lifecycle
```rust
// Enhanced media integration beyond basic MediaManager
impl SessionManager {
    // Media session coordination
    pub async fn setup_media_for_dialog(&self, dialog_id: &DialogId, local_sdp: &SessionDescription, remote_sdp: &SessionDescription) -> Result<MediaSessionId, Error>;
    pub async fn teardown_media_for_session(&self, session_id: &SessionId) -> Result<(), Error>;
    
    // RTP relay support for call-engine
    pub async fn setup_rtp_relay(&self, session_a_id: &SessionId, session_b_id: &SessionId) -> Result<RelayId, Error>;
    pub async fn teardown_rtp_relay(&self, relay_id: &RelayId) -> Result<(), Error>;
}
```

**Checklist:**
- [ ] **Dialog-to-Media Coordination**
  - [ ] setup_media_for_dialog() using negotiated SDP
  - [ ] Automatic media setup on dialog establishment
  - [ ] Media teardown on dialog termination
- [ ] **RTP Relay Support for Proxy Scenarios**
  - [ ] setup_rtp_relay() for call routing through server
  - [ ] Media relay coordination between two sessions
  - [ ] Relay teardown and resource cleanup
- [ ] **Media Statistics and Quality**
  - [ ] Expose RTP statistics through session interface
  - [ ] Media quality metrics integration
  - [ ] Call quality reporting for call-engine

---

## Enhanced Testing Strategy

### Integration Testing with Other Components

#### Call-Engine Integration Tests
- [ ] **Session-Call Coordination Tests**
  - [ ] Session creation triggered by call-engine INVITE routing
  - [ ] Dialog state synchronization with call state
  - [ ] Multi-session call scenarios (transfer, conference)
- [ ] **Authentication Flow Tests**  
  - [ ] Session authentication with call-engine credential store
  - [ ] Challenge-response cycle with session management
  - [ ] Authenticated vs unauthenticated session handling

#### SIP-Client Integration Tests
- [ ] **Client Call Flow Tests**
  - [ ] Outbound call with session-core session management
  - [ ] Inbound call handling with automatic session creation
  - [ ] Call progress responses with early dialog management
- [ ] **Media Integration Tests**
  - [ ] End-to-end media setup through session-core
  - [ ] Hold/resume operations with SDP renegotiation
  - [ ] Call termination with proper media cleanup

#### Media-Core Integration Tests
- [ ] **RTP Stream Coordination Tests**
  - [ ] Automatic RTP stream setup from SDP negotiation
  - [ ] Bidirectional media flow through session coordination
  - [ ] Media relay for proxy scenarios
- [ ] **Media Event Integration Tests**
  - [ ] Media quality events propagated to session layer
  - [ ] Media failure handling and session recovery
  - [ ] Media statistics collection and reporting

---

## Updated Success Criteria

### Basic SIP Functionality Integration
1. 🔄 **Call-Engine Integration**: SessionManager provides session management for call routing
2. 🔄 **SIP-Client Integration**: Enhanced helper functions support complete call flows  
3. 🔄 **Authentication Integration**: Session-level authentication with call-engine
4. 🔄 **Media Relay Support**: RTP forwarding coordination for proxy scenarios
5. 🔄 **End-to-End Call Flows**: Registration → authentication → call setup → media → teardown

### Component Integration Success
1. **call-engine** can use SessionManager for call state management
2. **sip-client** can use enhanced helpers for simplified call handling
3. **media-core** integration provides automatic RTP stream management
4. **Basic SIP server** functionality works end-to-end
5. **Standard SIP clients** can interoperate successfully

These integration tasks ensure that session-core properly supports the basic SIP server and client requirements outlined in BASIC_SIP_TODO.md while maintaining its role as the central coordination layer.

---

## Migration Notes

### Existing Code Compatibility
- Current SessionManager API is preserved - this is purely additive
- Dialog management remains unchanged - MediaManager is a new layer
- Helper functions maintain backward compatibility
- Event system is extended, not replaced

### Future Integration Points
- **call-engine**: Will use enhanced SessionManager for high-level call management
- **sip-client**: Will leverage the unified API for simplified client implementation
- **api-server**: Will expose session management via REST/WebSocket APIs
- **media-recorder**: Will integrate with MediaManager for recording coordination

---

## Current Status Summary

**✅ COMPLETE (Phase 1)**: Strong foundation with session management, dialog integration, and SDP negotiation

**🔄 IN PROGRESS (Phase 2)**: Media integration is the current focus - MediaManager design and implementation

**🔜 PLANNED (Phase 3-4)**: Advanced features and production hardening

The session-core architecture is well-positioned to serve as the central coordination layer. The immediate focus should be on completing the media integration to provide the unified SIP+media API that will serve both client and server use cases. 