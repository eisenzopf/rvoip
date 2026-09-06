# RVOIP RTP Core

[![Crates.io](https://img.shields.io/crates/v/rvoip-rtp-core.svg)](https://crates.io/crates/rvoip-rtp-core)
[![Documentation](https://docs.rs/rvoip-rtp-core/badge.svg)](https://docs.rs/rvoip-rtp-core)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> **rvoip 0.3.9 security notice:** direct AES-CM SRTP and SDES remain
> available. The `dtls-webrtc` feature adds the reviewed
> `rvoip_rtp_core::dtls_srtp` DTLS 1.2 handshake path used by `rvoip-sip`.
> Legacy DTLS constructors under `api::{client,server,common}`, every MIKEY
> mode, ZRTP, and AES-GCM remain unavailable and fail with typed errors.

## Overview

The `rtp-core` library provides RTP/RTCP packet processing, UDP media
transport, SRTP primitives, buffer management, and statistics collection for
the [rvoip](../../../README.md) VoIP stack. Some additional security and transport
modules exist in this crate, but they are not `rvoip-sip` beta claims unless
the beta compatibility matrix links test evidence for them.

## Architecture

The RTP Core sits at the foundation of the media transport stack, providing reliable and secure packet-level communication:

```
┌─────────────────────────────────────────┐
│            Application Layer            │
├─────────────────────────────────────────┤
│           rvoip-media-core              │
├─────────────────────────────────────────┤
│           rvoip-rtp-core   ⬅️ YOU ARE HERE
├─────────────────────────────────────────┤
│            Network Layer                │
└─────────────────────────────────────────┘
```

### Key Components

1. **RTP/RTCP Processing**: RFC 3550 packet processing with beta evidence requirements tracked by `rvoip-sip`
2. **Security Layer**: exact-suite direct SRTP and SDES are available; the feature-gated dedicated DTLS-SRTP handshake path is available; legacy DTLS constructors, MIKEY, ZRTP, and AES-GCM fail closed
3. **Transport Management**: UDP is the beta media transport; TCP transport is not a `rvoip-sip` beta claim
4. **Buffer Management**: Adaptive jitter buffer and high-performance memory pooling
5. **Statistics & Monitoring**: Comprehensive quality metrics and network analysis
6. **Payload Formats**: RTP payload metadata and helpers for audio/video payloads, including G.711 and static G.729/PT18

### Security Architecture

The library retains types for multiple security protocols. Only the paths shown
as available below may be selected in 0.3.9.

```
┌─────────────────────────────────────────────────────────────┐
│ Available: direct AES-CM SRTP | SDES | dedicated DTLS-SRTP │
├─────────────────────────────────────────────────────────────┤
│ Unavailable: legacy DTLS constructors | MIKEY | ZRTP        │
│              AES-GCM                                        │
│              (typed rejection, no fallback)                  │
└─────────────────────────────────────────────────────────────┘
```

## Features

### Implementation Inventory

#### **RTP/RTCP Implementation**
- ✅ Complete RFC 3550 compliant RTP/RTCP packet processing
- ✅ All RTCP packet types: SR, RR, SDES, BYE, APP, XR (RFC 3611)
- ✅ RTP header extensions support (RFC 8285, one-byte and two-byte formats)
- ✅ CSRC management for conferencing and mixing scenarios
- ✅ Sequence number tracking with reordering and duplicate detection
- ✅ Timestamp management and clock rate conversion
- ✅ SSRC collision detection and resolution

#### **Security Protocols**
- ✅ **SRTP**: AES-CM direct protection with explicitly provisioned keys
  - ✅ AES-CM (Counter Mode); AES-GCM is retained but unavailable
  - ✅ HMAC-SHA1 authentication (80-bit and 32-bit variants)
  - ✅ AES-CM key derivation and IV generation
  - ✅ Authentication and tamper detection
  - ⛔ Per-SSRC ROC/replay enforcement is deferred to the SRTP state repair
  - ✅ One exact AES-CM suite per direct PSK context
- ✅ **SRTCP**: AES-CM protection/unprotection with independent interop coverage
- ✅ **DTLS-SRTP**: `dtls_srtp` handshake functions behind `dtls-webrtc`,
  shared with RTP/RTCP on one UDP socket and fingerprint-checked by the SIP
  integration before context installation
- ⛔ **Legacy DTLS configuration constructors**: retained for source
  compatibility and still return typed unsupported errors
- ⛔ **ZRTP**: retained public configuration and packet types, typed unsupported result
- ⛔ **MIKEY**: PSK, PKE, and DH modes all return typed unsupported results
- ✅ **SDES-SRTP**: SDP-based key exchange for SIP compatibility

#### **Transport and Network**
- ✅ UDP transport with symmetric RTP support
- ⚠️ TCP transport implementation exists, but TCP RTP transport is not a `rvoip-sip` beta claim
- ✅ RTCP multiplexing (RFC 5761) on single port
- ✅ Smart port allocation strategies (Sequential, Random, Incremental)
- ✅ Cross-platform socket validation (Windows, macOS, Linux)
- ✅ IPv4/IPv6 dual-stack support
- ✅ Connection lifecycle management

#### **Buffer Management**
- ✅ High-performance adaptive jitter buffer
- ✅ Memory pooling to minimize allocations
- ✅ Priority-based transmit buffer with congestion control
- ✅ Global memory limits and resource management
- ✅ Buffer statistics and monitoring
- ✅ Tested with 500 concurrent streams (500,000+ packets)

#### **Payload Formats**
- ✅ Audio payload registry: G.711 (μ-law/A-law), static G.729/PT18, G.722, Opus
- ✅ Video codecs: VP8, VP9 with RFC 7741/8741 compliance
- ✅ Codec-specific timestamp handling
- ✅ Payload type negotiation and management
- ✅ Custom payload format extensibility

#### **Statistics and Quality Monitoring**
- ✅ Comprehensive packet loss and jitter tracking
- ✅ Round-trip time (RTT) measurement
- ✅ Bandwidth estimation and congestion detection
- ✅ MOS score estimation and R-factor calculation
- ✅ Quality metrics aggregation and reporting
- ✅ RTCP report generation and processing
- ✅ Network quality trend analysis

#### **Integration and API**
- ✅ Clean MediaTransport trait for media-core integration
- ✅ Event-driven architecture with comprehensive event system
- ✅ Client/Server API separation for different use cases
- ✅ Builder patterns for complex configurations
- ✅ Async/await support throughout

### 🚧 Planned Features

#### **Performance Optimizations**
- 🚧 Zero-copy packet processing optimizations
- 🚧 Hardware acceleration support (AES-NI, etc.)
- 🚧 SIMD optimizations for crypto operations
- 🚧 Lock-free data structures for high concurrency

#### **Advanced Security**
- 🚧 Hardware Security Module (HSM) integration
- 🚧 DTLS 1.3 support with 0-RTT handshakes
- 🚧 Post-quantum cryptography preparation
- 🚧 Advanced key rotation and management

#### **Enhanced Reliability**
- 🚧 Forward Error Correction (FEC) - RFC 5109
- 🚧 Redundant Encoding (RED) - RFC 2198
- 🚧 Transport-wide congestion control
- 🚧 Automatic quality adaptation

#### **Monitoring and Diagnostics**
- 🚧 Real-time performance monitoring
- 🚧 Packet capture and analysis tools
- 🚧 Network topology discovery
- 🚧 Quality degradation alerts

## Usage

### Basic RTP Session

```rust
use rvoip_rtp_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create RTP session configuration
    let config = RtpSessionConfig::builder()
        .local_addr("0.0.0.0:0".parse()?)
        .enable_rtcp_mux(true)
        .build();

    // Create RTP session
    let session = RtpSession::new(config).await?;
    let local_addr = session.local_addr()?;
    println!("RTP session listening on {}", local_addr);

    // Send RTP packet
    let packet = RtpPacket::builder()
        .payload_type(0) // G.711 μ-law
        .sequence_number(1234)
        .timestamp(160000)
        .ssrc(0x12345678)
        .payload(audio_data)
        .build();

    session.send_rtp_packet(packet, remote_addr).await?;
    
    // Receive packets
    while let Some(event) = session.receive_event().await {
        match event {
            RtpEvent::PacketReceived { packet, source } => {
                println!("Received RTP packet from {}", source);
                process_audio_packet(packet);
            }
            RtpEvent::RtcpReceived { packet, source } => {
                println!("Received RTCP packet from {}", source);
                process_rtcp_feedback(packet);
            }
            _ => {}
        }
    }

    Ok(())
}
```

### Legacy unavailable security configuration

The older DTLS-SRTP, MIKEY, and ZRTP constructors are retained for source
compatibility, but validation still rejects them. Use the feature-gated
`dtls_srtp` module (normally through `rvoip-sip`) for supported DTLS-SRTP.
Applications must handle every typed error and must not fall back to plaintext.

```rust
use rvoip_rtp_core::api::common::{SecurityConfig, SecurityError};

for config in [
    SecurityConfig::webrtc_compatible(),
    SecurityConfig::mikey_psk(),
    SecurityConfig::zrtp_p2p(),
] {
    assert!(matches!(
        config.validate(),
        Err(SecurityError::UnsupportedFeature(_))
    ));
}
```

### High-Performance Buffer Configuration

```rust
use rvoip_rtp_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Configure high-performance buffers
    let buffer_config = BufferConfig::builder()
        .jitter_buffer_size(50) // 50ms adaptive buffer
        .memory_pool_size(10 * 1024 * 1024) // 10MB pool
        .max_concurrent_streams(1000)
        .enable_priority_queue(true)
        .congestion_control_enabled(true)
        .build();

    let session = RtpSession::with_buffer_config(
        RtpSessionConfig::default(),
        buffer_config
    ).await?;

    // Session automatically uses optimized buffers
    // Tested with 500,000+ packets across 500 streams
    Ok(())
}
```

## SRTP availability in 0.3.9

The reviewed low-level RTP protection path supports four exact AES-CM/HMAC
suite identities. The dedicated DTLS-SRTP path derives those contexts after a
real handshake; several older public compatibility identities remain
unavailable and fail closed; see `MIGRATION_0.3.5.md`.

### Security Features

- **Encryption Algorithms**:
  - AES-CM with 128-bit or 256-bit master keys
  - AES-GCM, AES-f8, and NULL encryption identities are rejected

- **Authentication Algorithms**:
  - HMAC-SHA1 authentication with 80-bit and 32-bit output
  - NULL authentication and unreviewed hand-built combinations are rejected

- **Key Management**:
  - Session key derivation from master keys
  - Secure IV generation for encryption
  - Exact 14-byte master salts
  - Key rotation is unavailable and fails closed when requested

- **Tamper Detection**:
  - Authentication tag verification
  - Packet modification detection
  - Plaintext and authentication failures are dropped by secure transports

### Implementation Highlights

The implementation includes critical security improvements:

1. **Authentication Tag Handling**: Fixed authentication tag discarding vulnerability by introducing `ProtectedRtpPacket` struct
2. **Tamper Detection**: Comprehensive verification of authentication tags
3. **Key Derivation**: Standards-compliant key derivation following RFC 3711 Section 4.3
4. **Cipher Support**: AES-CM-128/HMAC-SHA1-80 and -32 are implemented; retained AES-GCM profiles are rejected

### Example Usage

```rust
// Create SRTP crypto context
let crypto_key = SrtpCryptoKey::new(master_key, master_salt);
let crypto = SrtpCrypto::new(SRTP_AES128_CM_SHA1_80, crypto_key)?;

// Encrypt RTP packet
let (encrypted_packet, auth_tag) = crypto.encrypt_rtp(&packet)?;
let protected = ProtectedRtpPacket::new(encrypted_packet, auth_tag);

// Serialize for transmission
let bytes = protected.serialize()?;

// On receiving side - automatically verifies auth tag
let decrypted_packet = crypto.decrypt_rtp(&bytes)?;
```

## Statistics and Quality Monitoring

The library provides comprehensive quality monitoring capabilities:

### Quality Metrics
- **Packet Loss**: Detection and percentage calculation
- **Jitter**: RFC 3550 compliant jitter calculation
- **Latency**: Round-trip time measurement
- **Bandwidth**: Usage estimation and congestion detection
- **MOS Score**: Voice quality estimation
- **R-Factor**: ITU-T G.107 quality rating

### RTCP Reports
- **Sender Reports (SR)**: Transmission statistics
- **Receiver Reports (RR)**: Reception quality feedback
- **Extended Reports (XR)**: Additional quality metrics
- **Source Description (SDES)**: Participant information

### Example Quality Monitoring

```rust
// Get comprehensive statistics
let stats = session.get_statistics().await?;
println!("Packet loss: {:.2}%", stats.packet_loss_percentage);
println!("Jitter: {:.1}ms", stats.jitter_ms);
println!("RTT: {:.1}ms", stats.round_trip_time_ms);
println!("MOS score: {:.1}", stats.mos_score);

// Configure quality alerts
session.set_quality_thresholds(QualityThresholds {
    max_packet_loss_percent: 1.0,
    max_jitter_ms: 30.0,
    min_mos_score: 3.5,
}).await?;
```

## Relationship to Other Crates

### Core Dependencies

- **`rvoip-sip-core`**: SIP message types and SDP handling
- **`tokio`**: Async runtime for network operations
- **`ring`**: Cryptographic operations for security
- **`rcgen`**: Certificate generation for DTLS

### Integration with rvoip Stack

The RTP Core provides the foundation for media transport in the rvoip stack:

- **Upward Interface**: Delivers media frames to media-core and call-engine
- **Downward Interface**: Handles network-level packet transmission/reception
- **Security Integration**: Provides exact-suite direct SRTP, SRTCP, SDES, and the feature-gated dedicated DTLS-SRTP handshake; unavailable compatibility methods fail closed
- **Event Propagation**: Notifies upper layers of transport events and quality changes

## Testing

Run the comprehensive test suite:

```bash
# Run all tests
cargo test -p rvoip-rtp-core

# Run the complete feature surface (unavailable protocols must still reject)
cargo test -p rvoip-rtp-core --all-features

# Run security-specific tests
cargo test -p rvoip-rtp-core srtp
cargo test -p rvoip-rtp-core dtls
cargo test -p rvoip-rtp-core security_fail_closed

# Run performance tests
cargo test -p rvoip-rtp-core --release buffer_performance
```

### Example Applications

The library includes working examples and explicit fail-closed availability examples:

```bash
# Basic RTP communication
cargo run --example api_basic

# Legacy DTLS, MIKEY, and ZRTP typed rejection
cargo run --example api_complete_security_showcase

cargo run --example api_zrtp_p2p

cargo run --example api_mikey_pke

# High-performance buffers
cargo run --example high_performance_buffers

# Quality monitoring
cargo run --example rtcp_reports

# Cross-platform compatibility
cargo run --example socket_validation
```

## Performance Characteristics

### Throughput
- **Packet Processing**: 100,000+ packets/second per core
- **Concurrent Streams**: Tested with 500+ simultaneous streams
- **Memory Usage**: ~2KB per active stream
- **Crypto Operations**: Hardware-accelerated when available

### Scalability Factors
- **Buffer Management**: Adaptive sizing based on network conditions
- **Memory Pooling**: Reduces GC pressure in high-throughput scenarios
- **Connection Management**: Efficient resource allocation
- **Security Context**: Minimal overhead for established sessions

### Optimization Recommendations
- **Security Protocol Selection**: use exact-suite direct SRTP/SDES, or enable
  `dtls-webrtc` and use the dedicated `dtls_srtp` path; legacy DTLS
  constructors, MIKEY, ZRTP, and AES-GCM remain unavailable
- **Buffer Configuration**: Tune based on network RTT and jitter characteristics
- **Memory Management**: Use memory pooling for high-volume applications
- **Transport Selection**: UDP for low latency, TCP for reliability

## Error Handling

The library provides comprehensive error handling with categorized error types:

```rust
use rvoip_rtp_core::Error;

match rtp_result {
    Err(Error::SecurityNegotiationFailed(details)) => {
        // Preserve the security requirement; never downgrade to plaintext.
        return Err(Error::SecurityNegotiationFailed(details));
    }
    Err(Error::PacketValidationFailed(reason)) => {
        // Handle malformed packets
        log::warn!("Invalid packet received: {}", reason);
        // Continue processing other packets
    }
    Err(Error::NetworkTimeout(addr)) => {
        // Handle network timeouts - often recoverable
        if error.is_recoverable() {
            retry_connection(addr).await?;
        }
    }
    Ok(result) => {
        // Handle success
    }
}
```

## Future Improvements

### Performance Enhancements
- Hardware Security Module (HSM) integration for private key operations
- Zero-copy packet processing with custom allocators
- SIMD optimizations for cryptographic operations
- Lock-free data structures for ultra-high concurrency

### Protocol Extensions
- DTLS 1.3 support with 0-RTT handshakes
- Post-quantum cryptography preparation
- Advanced ZRTP features (voice authentication, key continuity)
- MIKEY-DH hybrid mode for enterprise scenarios

### Advanced Features
- Forward Error Correction (FEC) for lossy networks
- Transport-wide congestion control
- Machine learning-based quality prediction
- Real-time network topology adaptation

## Contributing

Contributions are welcome! Please see the main [rvoip contributing guidelines](../../../README.md#contributing) for details.

For rtp-core specific contributions:
- Ensure RFC compliance for any protocol changes
- Add comprehensive tests for new security features
- Update documentation for any API changes
- Consider performance impact for high-throughput scenarios

## License

This project is licensed under the [MIT license](LICENSE).
