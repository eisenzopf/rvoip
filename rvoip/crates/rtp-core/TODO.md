# RTP Core Implementation TODO

This document outlines the implementation plan for the rtp-core crate, which handles RTP/RTCP packet processing and media transport.

## Directory Structure

The rtp-core crate follows this directory structure to maintain clean organization and separation of concerns:

```
rtp-core/
├── src/
│   ├── lib.rs                 # Main library exports and documentation
│   ├── error.rs               # Error types and handling
│   ├── packet/                # Packet processing
│   │   ├── mod.rs             # Packet module exports
│   │   ├── rtp.rs             # RTP packet implementation
│   │   ├── rtcp/              # RTCP packet implementations
│   │   │   ├── mod.rs         # RTCP module exports
│   │   │   ├── sr.rs          # Sender Report
│   │   │   ├── rr.rs          # Receiver Report
│   │   │   ├── sdes.rs        # Source Description
│   │   │   ├── bye.rs         # Goodbye
│   │   │   ├── app.rs         # Application-defined
│   │   │   └── xr.rs          # Extended Reports
│   │   └── header.rs          # Common header functionality
│   ├── session/               # RTP session management
│   │   ├── mod.rs             # Session module exports
│   │   ├── stream.rs          # RTP stream implementation
│   │   └── scheduling.rs      # Packet scheduling
│   ├── transport/             # Network transport
│   │   ├── mod.rs             # Transport module exports
│   │   ├── socket.rs          # Socket abstraction
│   │   ├── udp.rs             # UDP implementation
│   │   └── tcp.rs             # TCP implementation (if needed)
│   ├── srtp/                  # Secure RTP implementation
│   │   ├── mod.rs             # SRTP module exports
│   │   ├── crypto.rs          # Encryption/decryption
│   │   ├── key_derivation.rs  # Key management
│   │   └── auth.rs            # Authentication
│   ├── stats/                 # Statistics tracking
│   │   ├── mod.rs             # Stats module exports
│   │   ├── jitter.rs          # Jitter calculation
│   │   ├── loss.rs            # Packet loss tracking
│   │   ├── rtt.rs             # Round-trip time
│   │   └── reports.rs         # RTCP report generation
│   ├── time/                  # Timing and clock functionality
│   │   ├── mod.rs             # Time module exports
│   │   ├── ntp.rs             # NTP timestamp handling
│   │   └── clock.rs           # Clock rate conversions
│   ├── buffer/                # Buffer management
│   │   ├── mod.rs             # Buffer module exports
│   │   ├── pool.rs            # Memory pooling
│   │   ├── jitter.rs          # Adaptive jitter buffer
│   │   └── transmit.rs        # Priority-based transmit buffer
│   └── traits/                # Public traits for integration
│       ├── mod.rs             # Traits module exports
│       ├── media_transport.rs # Interface for media-core integration
│       └── events.rs          # Event system
├── examples/                  # Example implementations
├── tests/                     # Integration tests
└── benches/                   # Performance benchmarks
```

## Phase 1: Packet Processing (2 weeks)

### RTP Packet Structure
- [x] Implement RFC 3550 compliant RTP packet parsing
- [x] Create RtpPacket struct with all required fields
- [x] Implement header extension support
- [x] Add CSRC list handling
- [ ] Implement payload format identification
- [x] Create efficient serialization/deserialization
- [x] Add verification for RTP header validity

### RTCP Implementation
- [x] Create RTCP packet base structure
- [x] Implement Sender Report (SR) packets
- [x] Implement Receiver Report (RR) packets
- [x] Add Source Description (SDES) packets
- [x] Implement Goodbye (BYE) packets
- [x] Add Application-Defined (APP) packets
- [x] Implement Extended Report (XR) packets (RFC 3611)
- [x] Create RTCP compound packet handling

### Sequence and Timing Management
- [x] Implement sequence number tracking
- [x] Add detection of packet reordering
- [x] Create duplicate packet detection
- [x] Implement timestamp management
- [x] Add clock rate conversion utilities
- [x] Implement synchronization source (SSRC) handling
- [x] Add contributing source (CSRC) management

## Phase 2: Media Transport (2-3 weeks)

### Socket Management
- [x] Create RtpSocket abstraction
- [x] Implement separate RTP/RTCP sockets
- [x] Add support for symmetric RTP
- [ ] Implement port allocation strategy
- [x] Create socket binding with appropriate options
- [ ] Add IPv4/IPv6 dual-stack support
- [ ] Implement connection-oriented RTP (if needed)

### Packet Reception
- [x] Create async receiver for RTP packets
- [x] Implement separate RTCP packet receiver
- [x] Add incoming packet validation
- [x] Create packet demultiplexing based on SSRC
- [x] Implement buffer management for received packets
- [x] Add pipelining for packet processing
- [x] Create event system for received packets

### Packet Transmission
- [x] Implement RTP packet sender
- [ ] Create RTCP packet transmission logic
- [ ] Add rate limiting for RTCP (5% bandwidth rule)
- [x] Implement packet scheduling
- [x] Add transmission buffer management
- [x] Create burst mitigation logic
- [x] Implement congestion control indicators

### Secure RTP (SRTP)
- [ ] Integrate DTLS for key exchange
  - [x] Implement DTLS 1.2 protocol
  - [x] Create handshake protocol (ClientHello, ServerHello, certificates, etc.)
  - [x] Implement record layer protocol
  - [ ] Add alert protocol for error handling
  - [x] Support cryptographic operations leveraging existing Rust crates
  - [x] Implement SRTP profile negotiation
  - [x] Extract keying material for SRTP key derivation
  - [ ] Add certificate and fingerprint validation
- [x] Implement SRTP/SRTCP encryption
- [x] Add authentication tag handling
- [x] Implement replay protection
- [x] Create key derivation functions
- [x] Add support for multiple crypto suites
- [x] Implement crypto context management

## Phase 3: Statistics and Reporting (1-2 weeks)

### Metrics Collection
- [x] Implement packet loss detection
- [x] Add jitter calculation per RFC 3550
- [x] Create round-trip time estimation
- [x] Implement throughput measurement
- [x] Add bandwidth estimation
- [x] Create statistics aggregation
- [x] Implement NTP timestamp conversion

### RTCP Report Generation
- [x] Implement sender report generation logic
- [x] Create receiver report generation
- [x] Add extended reports for additional metrics
- [x] Implement RTCP interval calculation
- [x] Add SDES information generation
- [x] Create BYE packet generation logic
- [x] Implement RTCP transmission scheduling

### Quality Monitoring
- [x] Create MOS score estimation
- [x] Implement R-factor calculation
- [x] Add burst/gap metrics
- [ ] Create concealment metrics
- [x] Implement network congestion detection
- [ ] Add event-based quality alerts
- [ ] Create quality trend analysis

## Phase 4: Testing and Validation (Ongoing)

### Unit Tests
- [x] Create comprehensive test suite for RTP packet handling
- [x] Add tests for RTCP packet processing
- [x] Implement socket and transport tests
- [x] Add encryption/authentication testing
- [x] Create performance benchmarks
- [ ] Implement fuzzing for packet parsing robustness

### Integration Tests
- [ ] Test media transport with session layer
- [ ] Implement interoperability testing with standard clients
- [ ] Add cross-platform socket validation
- [x] Create timing and synchronization tests
- [x] Implement load testing for packet processing

### RFC Compliance
- [x] Verify RFC 3550 (RTP) compliance
- [x] Test RFC 3551 (RTP A/V Profile) compatibility
- [x] Validate RFC 3611 (RTCP XR) implementation
- [x] Verify RFC 3711 (SRTP) compliance
- [x] Test RFC 8285 (RTP Header Extensions) support
- [x] Implemented RFC 5761 (Multiplexing RTP and RTCP) support

## Integration with Media Core

- [x] Create clean interfaces for media-core integration
- [x] Implement MediaTransport trait
- [x] Add event system for media-core communication
- [x] Create codec payload format handlers
  - [x] Implement G.711 μ-law and A-law payload formats
  - [x] Implement G.722 payload format
  - [x] Implement Opus payload format
  - [x] Implement VP8/VP9 payload formats
- [ ] Implement media synchronization mechanisms
- [x] Add RTP session management for media-core use

## Immediate Fixes Needed

- [x] Fix socket sharing issue in RtpSession (currently creates new socket in receive_task)
- [x] Fix missing channel close detection in transports
- [x] Improve error handling in transport layer
- [x] Add proper documentation for MediaTransport integration 

## Recently Completed Work

- [x] Created MediaTransport trait adapter for media-core integration
- [x] Enhanced UdpRtpTransport with event-based packet reception
- [x] Added receive_packet method to RtpTransport
- [x] Fixed socket reuse issues in RtpSession
- [x] Implemented proper remote address handling
- [x] Improved error handling for broadcast channel operation
- [x] Created working bidirectional packet exchange example between RTP sessions
- [x] Fixed remaining socket binding conflicts
- [x] Implemented payload format framework with G.711 support
- [x] Created PayloadType enum with standard RTP payload types from RFC 3551
- [x] Added G.722 payload format implementation with special timestamp handling
- [x] Implemented Opus payload format with configurable bandwidth and bitrate
- [x] Added RTCP BYE packet handling for clean session termination
- [x] Integrated RTCP APP packet parsing and serialization
- [x] Created example demonstrating RTCP APP packet usage
- [x] Fixed RTCP packet detection in UdpRtpTransport (packets with payload types 200-204 were misidentified)
- [x] Implemented VP8 and VP9 video payload formats with RFC 7741/8741 compliant header handling
- [x] Created packet demultiplexing based on SSRC with stream tracking in RtpSession
- [x] Developed high-performance buffer management system with:
  - [x] Memory pooling to minimize allocations and reduce GC pressure
  - [x] Adaptive jitter buffer with RFC-compliant jitter calculations
  - [x] Priority-based transmit buffer with congestion control
  - [x] Global memory limits to prevent OOM conditions
  - [x] Comprehensive statistics and monitoring capabilities
- [x] Successfully tested buffer system with 500 concurrent streams (500,000 total packets)
- [x] Implemented Extended Report (XR) packets for additional metrics
- [x] Created RTCP compound packet handling
- [x] Added proper PayloadType enum variants for Opus, VP8, and VP9
- [x] Completed RFC 3551 (RTP A/V Profile) compatibility testing
- [x] Implemented MOS score estimation and R-factor calculation for voice quality metrics
- [x] Implemented RFC 8285 header extensions support with one-byte and two-byte formats
- [x] Created comprehensive example for RTP header extensions usage
- [x] Added contributing source (CSRC) management with support for mixed streams
- [x] Implemented CsrcManager for SSRC to CSRC mappings in mixer scenarios
- [x] Created helper methods in RtpHeader for easy CSRC manipulation
- [x] Developed comprehensive example simulating an RTP mixer with CSRC attribution
- [x] Integrated CSRC information with RTCP SDES packets
- [x] Implemented core DTLS 1.2 protocol with:
  - [x] Complete handshake protocol with cookie exchange for DoS protection
  - [x] Record layer implementation with epoch handling
  - [x] ChangeCipherSpec and Finished message verification
  - [x] SRTP key derivation from DTLS handshake
  - [x] ECDHE key exchange using P-256 curve
  - [x] TLS PRF implementation for key derivation
- [x] Implemented RFC 5761 (Multiplexing RTP and RTCP) support:
  - [x] Added RTCP-MUX configuration option
  - [x] Updated transport layer to handle multiplexed packets
  - [x] Implemented correct packet type detection per the RFC
  - [x] Created rtcp_mux.rs example demonstrating the functionality
  - [x] Fixed RTCP serialization in UdpRtpTransport
  - [x] Improved detection of RTCP packets in multiplexed streams
  - [x] Added timeouts to prevent examples from hanging
- [x] Completed RTCP implementation for quality metrics:
  - [x] Added methods to send Sender Reports (SR) and Receiver Reports (RR)
  - [x] Implemented round-trip time (RTT) calculation
  - [x] Created packet loss and jitter tracking and reporting
  - [x] Added quality statistics gathering and processing
  - [x] Created a comprehensive example demonstrating RTCP reports
- [x] Implemented media synchronization mechanisms:
  - [x] Added NTP to media clock conversion utilities
  - [x] Created timestamp synchronization between streams
  - [x] Added clock drift detection and compensation
  - [x] Implemented MediaSync and TimestampMapper interfaces
  - [x] Created comprehensive example demonstrating lip sync correction
- [x] Added cross-platform socket validation:
  - [x] Implemented detection for Windows, macOS, and Linux platforms
  - [x] Created platform-specific socket configuration strategies
  - [x] Added automatic testing and fallback mechanisms
  - [x] Improved socket reuse behavior across platforms
  - [x] Created comprehensive example demonstrating cross-platform compatibility

## Next Priorities (Updated)

### CRITICAL for media-core integration
- There are no remaining CRITICAL items for media-core integration

### Important for production use
- [ ] Enhance DTLS for full WebRTC compliance
  - [ ] Implement message fragmentation support for certificates
  - [ ] Expand cipher suite support for WebRTC compatibility 
  - [ ] Enhance certificate handling for proper identity verification
- [ ] Improve SRTP integration with DTLS
  - [ ] Expand SRTP protection profile support
  - [ ] Create proper profile negotiation logic
- [ ] Add IPv4/IPv6 dual-stack support
  - [ ] Create address family detection and selection logic
  - [ ] Implement fallback mechanisms for transport creation

### Future improvements
- [ ] Implement concealment metrics
- [ ] Add event-based quality alerts
- [ ] Create quality trend analysis
- [ ] Implement port allocation strategy

## DTLS Implementation Plan

### Directory Structure
```
src/dtls/
├── mod.rs               # Main module exports
├── connection.rs        # DTLS connection state management
├── handshake.rs         # Handshake protocol implementation
├── record.rs            # Record layer protocol
├── alert.rs             # Alert protocol
├── crypto/
│   ├── mod.rs           # Crypto module exports
│   ├── cipher.rs        # Cipher suite implementations
│   ├── keys.rs          # Key derivation and management
│   └── verify.rs        # Certificate verification
├── message/
│   ├── mod.rs           # Message module exports
│   ├── handshake.rs     # Handshake message types
│   ├── content.rs       # Content type definitions
│   └── extension.rs     # Extension handling (incl. SRTP profiles)
├── transport/
│   ├── mod.rs           # Transport layer exports
│   └── udp.rs           # UDP transport implementation
└── srtp/
    ├── mod.rs           # SRTP integration exports
    └── extractor.rs     # SRTP key material extraction
```

### Implementation Phases

#### Phase 1: Core DTLS Structure and Transport
- [ ] Define basic types and constants
  - [ ] DTLS record types, handshake message types
  - [ ] DTLS version constants
  - [ ] Error types
- [ ] Implement record layer
  - [ ] DTLS record format
  - [ ] Record parsing and serialization
  - [ ] Sequence numbers and replay protection
- [ ] Create transport integration
  - [ ] UDP-based transport for DTLS packets
  - [ ] Retransmission logic for lost packets
  - [ ] MTU handling

#### Phase 2: Handshake Protocol
- [ ] Implement basic handshake
  - [ ] ClientHello/ServerHello messages
  - [ ] Certificate messaging
  - [ ] Integration with existing crypto libraries
- [ ] Add key exchange
  - [ ] ECDHE using elliptic-curve crate
  - [ ] Key derivation using ring or RustCrypto HKDF
- [ ] Handle certificates
  - [ ] Generate certificates using rcgen
  - [ ] Parse certificates with x509-parser
  - [ ] Implement fingerprint validation for SDP

#### Phase 3: SRTP Integration
- [ ] Implement SRTP profile negotiation
  - [ ] Add use_srtp extension
  - [ ] Create profile selection logic
- [ ] Extract keys for SRTP
  - [ ] Extract keying material from handshake
  - [ ] Implement RFC 5764 key derivation
  - [ ] Integrate with existing SRTP code
- [ ] Manage connections
  - [ ] Handle session lifecycle
  - [ ] Implement rekeying and teardown

#### Phase 4: Testing and Security
- [ ] Create comprehensive tests
  - [ ] Test against RFC test vectors
  - [ ] Test interoperability
  - [ ] Implement stress testing and fuzzing
- [ ] Conduct security review
  - [ ] Review crypto operations
  - [ ] Prevent timing attacks
  - [ ] Ensure proper key handling

### DTLS Implementation Improvements
Following a review of the current implementation, several weaknesses were identified that need to be addressed:

- [x] Strengthen cookie validation
  - [x] Implement cryptographically secure cookie generation with server secret
  - [x] Add MAC to bind cookies to client IP addresses
  - [x] Implement proper cookie verification logic
- [x] Complete the handshake implementation
  - [x] Add ChangeCipherSpec handling
  - [x] Implement Finished message exchange with proper verification
  - [x] Create full handshake state machine with complete message flows
- [ ] Expand cipher suite support
  - [ ] Add support for modern AEAD ciphers (AES-GCM, ChaCha20-Poly1305)
  - [ ] Implement proper cipher suite negotiation
  - [ ] Phase out older, less secure cipher suites
- [ ] Improve error handling
  - [ ] Implement proper recovery mechanisms for all error paths
  - [ ] Add handling for out-of-order packets in UDP transport
  - [ ] Create comprehensive alert message system
- [ ] Enhance certificate handling
  - [ ] Implement proper certificate chain validation
  - [ ] Add certificate revocation checking
  - [ ] Support identity verification beyond fingerprints
- [ ] Expand SRTP protection profile support
  - [ ] Add AEAD GCM profile implementations (AES-128-GCM, AES-256-GCM)
  - [ ] Implement newer, more secure hash algorithms
  - [ ] Create proper profile negotiation logic
- [ ] Add message fragmentation support
  - [ ] Implement message fragmentation for large handshake messages
  - [ ] Create proper fragment reassembly with timeout handling
  - [ ] Add MTU discovery to optimize fragmentation
- [ ] Implement session resumption
  - [ ] Add session ticket support
  - [ ] Implement pre-shared key (PSK) mode
  - [ ] Create session caching mechanism
- [ ] Improve state machine robustness
  - [ ] Redesign state transitions to be more resilient
  - [ ] Add comprehensive state validation
  - [ ] Implement proper timeout handling for all states
- [ ] Add DTLS 1.3 support
  - [ ] Implement TLS 1.3 handshake protocol adaptations
  - [ ] Add 0-RTT support for faster connections
  - [ ] Implement required cryptographic primitives
- [ ] WebRTC-Specific Requirements
  - [ ] Implement recommended cipher suites for WebRTC compatibility
  - [ ] Add support for all WebRTC-required TLS extensions
  - [ ] Implement ICE/STUN integration for connectivity checks
  - [ ] Add secure random number generation for all crypto operations
  - [ ] Create comprehensive testing against browser WebRTC implementations
  - [ ] Implement renegotiation handling and security measures

## Next Priorities (Updated)

- [ ] Enhance DTLS for full WebRTC compliance
  - [ ] Implement message fragmentation support for certificates
  - [ ] Expand cipher suite support for WebRTC compatibility 
  - [ ] Enhance certificate handling for proper identity verification
- [ ] Improve SRTP integration with DTLS
  - [ ] Expand SRTP protection profile support
  - [ ] Create proper profile negotiation logic
- [ ] Test RFC 5761 (Multiplexing RTP and RTCP) support
- [ ] Add cross-platform socket validation
- [ ] Create RTCP BYE packet generation logic
- [ ] Implement concealment metrics
- [ ] Add event-based quality alerts
- [ ] Create quality trend analysis
- [ ] Implement port allocation strategy
- [ ] Add IPv4/IPv6 dual-stack support
- [ ] Create media synchronization mechanisms 