# Session Core - TODO List

This document tracks planned improvements and enhancements for the `rvoip-session-core` library.

## Critical Path for Real-World SIP Deployments

### 1. Authentication and Security
- [ ] Implement Digest Authentication according to RFC 3261 Section 22.2
- [ ] Create challenge-response handling for 401/407 responses
- [ ] Implement nonce tracking and expiration handling
- [ ] Add authentication caching for subsequent requests
- [ ] Implement quality of protection (qop) support
- [ ] Add TLS transport support integration for secure signaling
- [ ] Create high-level authentication API for both client and server usage
- [ ] Add helper functions for common authentication scenarios
- [ ] Implement credential storage and management

### 2. Dialog Lifecycle Management
- [x] Add support for re-INVITEs and session refreshes
- [x] Implement dialog refresh mechanism for long-running sessions
- [x] Add dialog recovery after network failures
- [x] Implement UPDATE method handling per RFC 3311
- [ ] Implement tag generation and management for dialogs
- [ ] Add correct CSeq handling for transactions
- [ ] Complete Route and Record-Route header processing
- [ ] Add automatic Route/Record-Route header handling
- [ ] Support Path header for registration scenarios (RFC 3327)
- [ ] Add proper PRACK support for reliable provisional responses (RFC 3262)
- [ ] Improve handling of dialog matching for requests with multiple candidates
- [x] Create additional helper functions for re-INVITE scenarios
- [ ] Add session modification abstractions (hold, resume, codec change)
- [x] Complete the create_dialog() function for direct dialog creation
- [x] Implement session refresh helper functions

### 3. Media Session Management and SDP Negotiation
- [x] Replace current sdp.rs with sip-core's SdpBuilder integration
  - [x] Update Cargo.toml to properly import sip-core's sdp module
  - [x] Create proper integration between session-core and sip-core's SDP implementation
  - [x] Port fallback RTP port extraction utility to work with sip-core's types
  - [x] Remove duplicate functionality in session-core's sdp.rs
- [x] Add SDP generation and processing functions
  - [x] Create function to generate SDP offers for outgoing calls
  - [x] Implement function to process SDP answers in responses
  - [x] Add function to create SDP answers for incoming calls
  - [x] Add support for generating re-INVITE SDPs with media updates
- [x] Implement core SDP negotiation mechanisms
  - [x] Create SdpContext to store and track local and remote SDP state
  - [x] Implement SDP offer/answer state machine per RFC 3264
  - [x] Add media capability negotiation for codec selection
- [x] Complete dialog-level SDP integration
  - [x] Integrate SdpContext with dialog state
  - [x] Handle early media scenarios properly
  - [x] Add automatic SDP state updates on dialog events
- [x] Add helper functions for SDP operations
  - [x] Create high-level functions for common SDP tasks
  - [x] Update helpers.rs with SDP-focused convenience methods
  - [x] Ensure proper error handling for SDP operations
- [x] Complete SDP body handling in SIP messages
  - [x] Add functions to attach SDP to requests
  - [x] Implement automatic extraction and processing of SDP from responses
  - [x] Ensure proper Content-Type handling
- [ ] Enhance session-level media management
  - [x] Implement SDP-to-MediaConfig conversion
  - [ ] Add automatic media stream setup based on negotiated parameters
  - [ ] Implement media state tracking in session objects
  - [ ] Create helper functions for media operations (mute, codec change)
- [ ] Add advanced media features
  - [ ] Add proper ICE integration for NAT traversal (RFC 8445)
  - [ ] Support RTCP feedback mechanisms (RFC 4585)
  - [ ] Implement DTMF handling via RTP events (RFC 4733)
  - [ ] Integrate SRTP for secure media (RFC 3711)

### 4. Error Handling and Reliability
- [x] Create comprehensive error documentation for API users
- [x] Implement transaction recovery mechanisms for network failures
- [ ] Add comprehensive logging for transaction events
- [ ] Implement circuit breakers for external systems
- [ ] Add panic recovery in critical paths
- [ ] Add soak testing for memory leaks detection
- [x] Create error recovery helper functions for common failure scenarios
- [ ] Implement guided recovery procedures for network and protocol errors
- [ ] Add retry management with backoff strategies

### 5. Feature Extensions
- [ ] Complete implementation of REFER handling (RFC 3515)
- [ ] Add NOTIFY generation for transfer progress updates
- [ ] Implement attended transfer scenarios
- [ ] Support Replaces header (RFC 3891) for transfer completion
- [ ] Implement voice quality metrics reporting

## Completed Critical Tasks

### Transaction-Core Integration
- [x] Implement transaction event subscription in SessionManager
- [x] Create dedicated event processing loop for transaction events
- [x] Map transaction events to the correct dialog/session
- [x] Implement proper error handling for transaction failures
- [x] Add support for transaction state timeouts
- [x] Enhance the transaction_to_dialog mapping in DialogManager
- [x] Implement transaction cancellation for INVITE requests
- [x] Handle special cases like forked INVITE transactions
- [x] Implement proper transaction termination cleanup
- [x] Add retransmission handling coordination with transaction layer
- [x] Ensure proper ACK handling for non-2xx responses (auto-generated by transaction layer)
- [x] Add proper handling for transaction timer events (Timer A-K)
- [x] Sync transaction states with session/dialog states

### Session State Management
- [x] Improve state transitions based on transaction events
- [x] Handle transaction failures in session state machine
- [x] Implement proper dialog termination via BYE transactions
- [x] Handle INVITE transaction transitions to dialog states per RFC 3261 Section 13
- [x] Implement proper early dialog management with multiple transactions
- [x] Implement proper CANCEL handling as described in RFC 3261 Section 9
- [x] Add full support for multi-device forking scenarios

### Request Generation and Processing
- [x] Enhance request generation for all SIP methods
- [x] Implement proper header generation (Via, Contact, etc.)
- [x] Add support for handling incoming requests via transactions
- [x] Improve response creation and sending through transactions
- [x] Implement proper ACK handling for INVITE transactions
- [x] Add correct handling of ACK for 2xx responses (TU responsibility)
- [x] Implement proper response handling for different transaction types

### Error Handling & Robustness
- [x] Replace generic anyhow errors with specific error types
- [x] Implement detailed error categorization (network, protocol, application)
- [x] Add retry mechanisms for recoverable errors
- [x] Implement error propagation with context through the stack
- [x] Add graceful fallback for non-critical failures
- [x] Implement timeout handling for all operations
- [x] Add boundary checking for user inputs

### Early Dialog Management
- [x] Enhance support for multiple simultaneous early dialogs
- [x] Implement forking scenario handling per RFC 3261 Section 12.1.2

### Async Runtime Optimizations
- [x] Replace polling-based subscription tracking with event-driven mechanisms
- [x] Use more efficient task management for event handling
- [x] Replace standard Mutex with DashMap for concurrent access to transaction subscriptions
- [x] Implement proper backpressure handling in event channels
- [x] Use tokio::select! for efficient multiplexing of event sources
- [x] Reduce number of spawned tasks by consolidating related functionality
- [x] Add channel buffer size tuning based on expected transaction volume
- [x] Implement dead task cleanup for orphaned subscriptions
- [x] Add benchmarks specific to async runtime performance
- [x] Fix remaining lock contention issues in high-volume scenarios

## Additional Enhancements for Production Environments

### Performance and Scalability
- [ ] Implement session pooling for high-volume environments
- [ ] Add connection reuse optimizations
- [x] Create comprehensive benchmarking suite
- [ ] Optimize memory usage for large session counts
- [x] Add configurable limits for resource management
- [ ] Implement adaptive throttling mechanisms
- [ ] Create performance profiling tools
- [ ] Support for distributed session management
- [x] Add metrics collection for operational monitoring
- [x] Implement resource usage reporting

### Public API Improvements
- [x] Create high-level client API for common call scenarios
- [x] Add server API for registration, proxy, and B2BUA use cases
- [x] Implement session modification API (hold, resume, transfer)
- [x] Add media control interface (mute, codec switching)
- [ ] Create high-level authentication handling
- [x] Add quality metrics reporting API
- [x] Implement event subscription model for asynchronous operations
- [ ] Improve event subscription interface with type-safe callbacks
- [ ] Add helper methods for common event handling scenarios
- [ ] Create standardized event filtering and prioritization
- [x] Create logging and tracing interfaces
- [x] Add configuration management API
- [x] Create transport abstraction for protocol flexibility
- [x] Add missing helper functions for dialog operations:
  - [x] Implement `put_call_on_hold` helper function for the helpers.rs API
  - [x] Implement `resume_held_call` helper function for the helpers.rs API
  - [x] Implement `verify_dialog_active` helper function for the helpers.rs API  
  - [x] Implement `update_codec_preferences` helper function for the helpers.rs API

### Testing & Compliance
- [x] Create test suite for transaction-to-session integration
- [x] Test dialog creation and management
- [x] Test session state transitions based on transaction events
- [x] Test integration with transaction-core using mock transport
- [x] Add comprehensive test suite for all RFC-mandated behaviors
- [ ] Create interoperability tests with common SIP servers
- [x] Document compliance status for each section of relevant RFCs
- [x] Add benchmarks for critical performance metrics
- [ ] Implement continuous performance regression testing

### Recent Improvements

1. **Network Transport Abstraction**
   - [x] Created the Transport trait to abstract network operations
   - [x] Implemented UDP transport with send/receive capabilities
   - [x] Added automatic address resolution for SIP URIs
   - [x] Improved error handling for network failures

2. **Transaction Layer**
   - [x] Implemented INVITE client transaction state machine
   - [x] Implemented INVITE server transaction state machine
   - [x] Added Non-INVITE client and server transactions
   - [x] Created a transaction manager for handling all transactions
   - [x] Added proper timer support for retransmissions
   - [x] Implemented reliable provisional responses (PRACK support)
   - [x] Improved error handling and propagation in transactions
   - [x] Added transaction failure detection and recovery

3. **Dialog Management**
   - [x] Implemented dialog creation from 2xx responses
   - [x] Added dialog state management
   - [x] Created dialog ID generation and lookup
   - [x] Implemented route set manipulation
   - [x] Added full support for To/From/Call-ID headers
   - [x] Implemented early dialog support
   - [x] Added proper dialog termination handling
   - [x] Implemented dialog-based request creation
   - [x] Implemented in-dialog ACK generation
   - [x] Added support for re-INVITEs for dialog refresh
   - [x] Added dialog recovery mechanism for network failures
   - [x] Implemented UPDATE method support (RFC 3311)

4. **SDP Handling**
   - [x] Implemented basic SDP parsing and generation
   - [x] Added support for audio codecs
   - [x] Implemented proper SDP negotiation
   - [x] Added SDP offer/answer model
   - [x] Implemented SDP version handling
   - [x] Added handling for SDP in INVITEs
   - [x] Added early media SDP support
   - [x] Added SDP renegotiation for session updates
   - [x] Added session refreshes with SDP
   - [x] Added SDP support for UPDATE method

5. **Tokio Async Runtime Optimizations**
   - [x] Replaced polling-based subscription tracking with event-driven approach
   - [x] Optimized task usage with StreamExt and FuturesUnordered
   - [x] Implemented efficient multiplexing with tokio::select!
   - [x] Added proper backpressure handling in event channels
   - [x] Reduced the number of spawned tasks for better performance
   - [x] Used DashMap for efficient concurrent access to transaction state
   - [x] Added constants for optimal channel sizing
   - [x] Improved error handling for async task failures
   - [x] Added proper resource cleanup in terminate_all() method 
   - [x] Fixed session state transition issues during termination

6. **Session Manager Improvements**
   - [x] Optimized session event processing with dedicated channels
   - [x] Implemented more efficient task tracking for session operations
   - [x] Added proper cleanup routines with timeout handling
   - [x] Improved dialog-to-session mapping with DashMap
   - [x] Added better error handling for session termination
   - [x] Implemented asynchronous session cleanup
   - [x] Added session batch operations with FuturesUnordered
   - [x] Optimized transaction event processing for sessions
   - [x] Fixed transaction resource management with shutdown() method

## Transaction Integration Issues Discovered in Benchmark Testing

These issues were identified through benchmark testing and have been fixed:

### 1. Transaction-to-Session Mapping Issues
- [x] Fix transaction-to-session mapping to ensure sessions only receive events for their own transactions
- [x] Implement proper filtering of transaction events at the session layer
- [x] Add transaction ownership tracking to prevent cross-session interference
- [x] Implement transaction reference counting to prevent premature transaction termination

### 2. Event Handling Issues
- [x] Fix global event distribution that causes all sessions to process events for all transactions
- [x] Implement transaction ID-based event routing to target specific sessions
- [x] Add transaction context to events to facilitate proper routing
- [x] Create session-specific event queues to prevent interference between sessions

### 3. Message Processing Issues
- [x] Improve handling of retransmissions at the session layer
- [x] Add proper coordination between transaction state and session state
- [x] Fix race conditions in concurrent event processing
- [x] Add robust error handling for transaction failures

### 4. Dialog Integration Issues
- [x] Ensure dialog state properly transitions based on transaction events
- [x] Fix race conditions in dialog creation and update operations
- [x] Improve coordination between dialog and transaction lifecycle management
- [x] Add proper handling of dialog-related issues in SIP transaction processing

### 5. Test Improvements
- [x] Create more realistic end-to-end session tests
- [x] Add benchmarks with different concurrent session counts
- [x] Implement tests that verify proper transaction-to-session event routing
- [x] Add tests for handling various failure scenarios (network errors, timeouts, etc.)
- [x] Fix session termination issues in benchmarks

## Integration with Improved Transport Layer

Following the integration of transaction-core with sip-transport, these tasks will ensure session-core properly leverages the improved transport capabilities while maintaining appropriate layer separation:

### 1. Transport Information Access
- [ ] Add methods to get transport capabilities through transaction-core API
- [ ] Ensure SDP generation uses accurate network information from transaction-core
- [ ] Add transport status reporting in dialog recovery mechanisms
- [ ] Implement connection status awareness for long-running dialogs

### 2. Transport-aware Routing
- [ ] Update URI handling to properly select transport based on scheme (sip:, sips:, ws:, wss:)
- [ ] Enhance dialog route set processing to respect transport parameters
- [ ] Add support for failover between transport types when primary transport fails
- [ ] Implement RFC 3263 DNS-based SIP server location support

### 3. WebSocket Support
- [ ] Add session-level logic for WebSocket connection handling
- [ ] Implement proper connection lifecycle management for persistent connections
- [ ] Add reconnection logic with backoff for WebSocket transport
- [ ] Handle WebSocket-specific SIP behaviors (e.g., connection correlation headers)

### 4. Testing and Validation
- [ ] Update test suite to use real transport implementations instead of mocks
- [ ] Create tests for transport failover scenarios
- [ ] Test WebSocket connection handling in session-core
- [ ] Verify proper SDP generation with real network interfaces

### 5. Examples and Documentation
- [ ] Update the integrated_call.rs example to use real UDP/TCP/WebSocket transports
- [ ] Add examples demonstrating transport failover
- [ ] Document best practices for transport selection in session-core
- [ ] Create advanced examples showing WebSocket-based calls

## Media Integration

Tasks to integrate the session-core with rtp-core and media-core for full media functionality:

### 1. Media Session Management
- [ ] Create MediaManager to coordinate between SIP dialogs and media sessions
- [ ] Implement mapping of SIP dialogs to media sessions
- [ ] Add lifecycle management for media sessions based on dialog events
- [ ] Create error handling and recovery for media session failures
- [ ] Implement proper cleanup of media resources when dialogs terminate
- [ ] Add support for multiple media streams per dialog (audio+video)

### 2. SDP and Media Negotiation
- [ ] Enhance SDP handling to extract and utilize codec information
- [ ] Implement mapping of SDP media descriptions to media-core codecs
- [ ] Add support for ICE candidate negotiation in SDP
- [ ] Implement DTLS-SRTP fingerprint exchange via SDP
- [ ] Create helpers for RTCP feedback parameter negotiation
- [ ] Add bandwidth and quality parameter extraction from SDP
- [ ] Implement RTP payload type mapping and management

### 3. Media Control Interface
- [ ] Create high-level API for common media operations (mute, codec change)
- [ ] Implement events for media state changes (active, inactive, hold)
- [ ] Add quality metrics reporting from media-core to session layer
- [ ] Create volume control and audio level monitoring interface
- [ ] Implement DTMF sending via RTP events from session layer
- [ ] Add media failure notification and recovery mechanism
- [ ] Create diagnostic interfaces for media troubleshooting

### 4. Media Feature Negotiation
- [ ] Implement SIP feature negotiation for media capabilities
- [ ] Add support for early media scenarios
- [ ] Create proper handling of media direction attributes (sendonly, recvonly)
- [ ] Implement hold/resume media state synchronization
- [ ] Add codec renegotiation during active sessions
- [ ] Create support for media security negotiation
- [ ] Implement bandwidth adaptation mechanism based on network conditions

### 5. Testing and Examples
- [ ] Create comprehensive test suite for media integration
- [ ] Add examples demonstrating basic audio call functionality
- [ ] Create advanced examples with media feature negotiation
- [ ] Implement interoperability tests with common SIP clients
- [ ] Add performance benchmarks for media processing
- [ ] Create media flow visualization tools for debugging
- [ ] Implement test cases for various network conditions

## Future Scope

### Advanced Media Features
- [ ] Support for video sessions
- [ ] Implementation of WebRTC integration
- [ ] Advanced audio processing capabilities
- [ ] Multi-party conferencing support

### Standards Extensions
- [ ] Support for SIP extensions (INFO, MESSAGE, etc.)
- [ ] Implementation of presence and events framework (RFC 3856, RFC 3265)
- [ ] Support for advanced SIP routing features
- [ ] Integration with IMS/VoLTE standards 