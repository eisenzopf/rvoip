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