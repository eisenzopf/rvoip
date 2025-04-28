# Session Core - TODO List

This document tracks planned improvements and enhancements for the `rvoip-session-core` library.

## Priority Improvements

### 1. Early Dialog Management
- [ ] Enhance support for multiple simultaneous early dialogs
- [ ] Implement forking scenario handling per RFC 3261 Section 12.1.2
- [ ] Add proper PRACK support for reliable provisional responses (RFC 3262)
- [ ] Improve handling of dialog matching for requests with multiple candidates

### 2. SDP Negotiation
- [ ] Implement enhanced codec negotiation with proper offer/answer model (RFC 3264)
- [ ] Add support for media capability negotiation (RFC 5939)
- [ ] Implement bandwidth modifiers handling
- [ ] Add ICE candidates support (RFC 8445) for NAT traversal
- [ ] Support RTCP feedback mechanisms (RFC 4585)

### 3. Security Features
- [ ] Implement Digest Authentication (RFC 3261 Section 22.2)
- [ ] Add support for TLS transport in dialogs
- [ ] Integrate SRTP for secure media (RFC 3711)
- [ ] Support Identity header for call verification (RFC 8224)
- [ ] Implement SIPS URI scheme handling

## Additional Enhancements

### 4. Call Transfer Features
- [ ] Complete implementation of REFER handling (RFC 3515)
- [ ] Add NOTIFY generation for transfer progress updates
- [ ] Implement attended transfer scenarios
- [ ] Support Replaces header (RFC 3891) for transfer completion

### 5. Advanced Dialog Features
- [ ] Implement dialog refreshing mechanism
- [ ] Add support for dialog recovery from network failures
- [ ] Enhance Route header management for complex topologies
- [ ] Support Path header for registration scenarios (RFC 3327)

### 6. Performance Improvements
- [ ] Review and optimize dialog lookup mechanisms
- [ ] Improve thread safety in transaction-to-dialog mapping
- [ ] Add connection reuse optimizations
- [ ] Implement batched event processing

### 7. Testing & Compliance
- [ ] Add comprehensive test suite for all RFC-mandated behaviors
- [ ] Create interoperability tests with common SIP servers
- [ ] Document compliance status for each section of relevant RFCs
- [ ] Add benchmarks for critical performance metrics

### 8. Documentation
- [ ] Add detailed API documentation with examples
- [ ] Create sequence diagrams for common call flows
- [ ] Document integration patterns with client and server applications
- [ ] Add troubleshooting guide for common issues

## Future Considerations

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