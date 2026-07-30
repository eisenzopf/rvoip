# RTP-Core Examples

This directory contains comprehensive examples demonstrating the functionality of the `rtp-core` crate. Examples are organized into two main categories:

- **🏗️ API Examples** - Use the high-level API layer for common use cases
- **⚙️ Core Examples** - Use low-level core libraries directly for maximum control

## 🏗️ API Examples (High-Level API Layer)

*These examples use the simplified API layer and are recommended for most applications.*

### 📱 Basic API Usage

#### [`api_basic.rs`](api_basic.rs)
Basic client/server setup demonstrating fundamental RTP communication.
```bash
cargo run --example api_basic
```

#### [`media_api_usage.rs`](media_api_usage.rs)
Integration between media-core and rtp-core using the new API with broadcast channels.
```bash
cargo run --example media_api_usage
```

### 🔐 Security API Features

#### [`api_srtp.rs`](api_srtp.rs)
SRTP encryption with pre-shared keys (most common SRTP deployment scenario).
```bash
cargo run --example api_srtp
```

#### [`api_srtp_simple.rs`](api_srtp_simple.rs)
Simplified SRTP usage for quick setup.
```bash
cargo run --example api_srtp_simple
```

#### [`api_sdes_srtp.rs`](api_sdes_srtp.rs)
SDES-based SRTP key exchange mechanism.
```bash
cargo run --example api_sdes_srtp
```

#### [`api_mikey_pke.rs`](api_mikey_pke.rs)
Demonstrates typed rejection of the unavailable MIKEY public-key exchange and
its incomplete trust-path helpers.
```bash
cargo run --example api_mikey_pke
```

#### [`api_mikey_srtp.rs`](api_mikey_srtp.rs)
Demonstrates typed rejection of every unavailable MIKEY mode. It does not
substitute static SRTP keys or simulate a successful exchange.
```bash
cargo run --example api_mikey_srtp
```

#### [`api_unified_security.rs`](api_unified_security.rs)
Shows that the unified manager advertises only implemented, provisioned methods
and rejects retained DTLS, MIKEY, and ZRTP configurations.
```bash
cargo run --example api_unified_security
```

#### [`api_zrtp_p2p.rs`](api_zrtp_p2p.rs)
Demonstrates typed rejection of the unavailable ZRTP state machine. No SAS or
secure call is simulated.
```bash
cargo run --example api_zrtp_p2p
```

#### [`api_advanced_security.rs`](api_advanced_security.rs)
Demonstrates that placeholder key rotation/derivation fails closed and that
built-in recovery policies contain only implemented methods.
```bash
cargo run --example api_advanced_security
```

#### [`api_complete_security_showcase.rs`](api_complete_security_showcase.rs)
Shows working direct SRTP/SDES availability and typed rejection of unavailable
DTLS, MIKEY, and ZRTP configurations.
```bash
cargo run --example api_complete_security_showcase
```

### ⚡ Advanced API Features

#### [`api_ssrc_demultiplexing.rs`](api_ssrc_demultiplexing.rs)
SSRC-based stream separation for handling multiple concurrent streams.
```bash
cargo run --example api_ssrc_demultiplexing
```

#### [`api_ssrc_demux_test.rs`](api_ssrc_demux_test.rs)
Testing and validation of SSRC demultiplexing functionality.
```bash
cargo run --example api_ssrc_demux_test
```

#### [`api_media_sync.rs`](api_media_sync.rs)
Media synchronization API for precise timing control.
```bash
cargo run --example api_media_sync
```

#### [`api_high_performance_buffers.rs`](api_high_performance_buffers.rs)
High-performance buffer management for large-scale deployments.
```bash
cargo run --example api_high_performance_buffers
```

#### [`api_header_extensions.rs`](api_header_extensions.rs)
RTP header extensions (RFC 8285) for carrying additional metadata.
```bash
cargo run --example api_header_extensions
```

#### [`api_header_extensions_simple.rs`](api_header_extensions_simple.rs)
Simplified header extensions usage.
```bash
cargo run --example api_header_extensions_simple
```

#### [`api_csrc_management_test.rs`](api_csrc_management_test.rs)
CSRC management for mixed streams in conferencing scenarios.
```bash
cargo run --example api_csrc_management_test
```

### 📡 RTCP API Features

#### [`api_rtcp_mux.rs`](api_rtcp_mux.rs)
RFC 5761 RTCP multiplexing on a single port.
```bash
cargo run --example api_rtcp_mux
```

#### [`api_rtcp_reports.rs`](api_rtcp_reports.rs)
RTCP report generation and handling.
```bash
cargo run --example api_rtcp_reports
```

#### [`api_rtcp_app_bye_xr.rs`](api_rtcp_app_bye_xr.rs)
RTCP APP, BYE, and Extended Reports functionality.
```bash
cargo run --example api_rtcp_app_bye_xr
```

## ⚙️ Core Examples (Low-Level Libraries)

*These examples use core libraries directly and are suitable for advanced use cases requiring fine-grained control.*

### 🔧 Basic Core Usage

#### [`minimal_connection_test.rs`](minimal_connection_test.rs)
Minimal UDP connection test for diagnosing frame reception issues.
```bash
cargo run --example minimal_connection_test
```

#### [`media_transport.rs`](media_transport.rs)
Bidirectional RTP packet exchange between two RTP sessions.
```bash
cargo run --example media_transport
```

#### [`media_sync.rs`](media_sync.rs)
Core media synchronization implementation.
```bash
cargo run --example media_sync
```

### 📡 Protocol Implementation

#### [`rtcp_mux.rs`](rtcp_mux.rs)
RFC 5761 RTCP multiplexing implementation at the core level.
```bash
cargo run --example rtcp_mux
```

#### [`rtcp_bye.rs`](rtcp_bye.rs)
RTCP BYE packet sending and receiving when RTP sessions are closed.
```bash
cargo run --example rtcp_bye
```

#### [`rtcp_app.rs`](rtcp_app.rs)
RTCP APP packets for application-specific functions.
```bash
cargo run --example rtcp_app
```

#### [`rtcp_reports.rs`](rtcp_reports.rs)
RTCP report generation and processing.
```bash
cargo run --example rtcp_reports
```

#### [`rtcp_xr_example.rs`](rtcp_xr_example.rs)
RTCP XR (Extended Reports) for detailed media quality metrics.
```bash
cargo run --example rtcp_xr_example
```

#### [`rtcp_rate_limiting.rs`](rtcp_rate_limiting.rs)
RTCP rate limiting and bandwidth management.
```bash
cargo run --example rtcp_rate_limiting
```

#### [`rtp_broadcast_fix.rs`](rtp_broadcast_fix.rs)
RTP broadcast functionality fixes and improvements.
```bash
cargo run --example rtp_broadcast_fix
```

### 🎵 Payload Format Handling

#### [`payload_format.rs`](payload_format.rs)
Different payload formats for various codecs, focusing on G.711.
```bash
cargo run --example payload_format
```

#### [`payload_type_demo.rs`](payload_type_demo.rs)
Comprehensive demonstration of RFC 3551-compliant payload type handling.
```bash
cargo run --example payload_type_demo
```

#### [`g722_payload.rs`](g722_payload.rs)
G.722's special timestamp handling with 16kHz sampling but 8kHz RTP timestamps.
```bash
cargo run --example g722_payload
```

#### [`opus_payload.rs`](opus_payload.rs)
Opus codec configuration with different bandwidth modes and channels.
```bash
cargo run --example opus_payload
```

#### [`video_payload.rs`](video_payload.rs)
VP8 and VP9 video payload formats with scalability features.
```bash
cargo run --example video_payload
```

### 🔒 SRTP Examples and Unavailable-Feature Checks

#### [`srtp_crypto.rs`](srtp_crypto.rs)
Core SRTP cryptographic operations and key management.
```bash
cargo run --example srtp_crypto
```

#### [`srtp_protected.rs`](srtp_protected.rs)
SRTP-protected RTP session implementation.
```bash
cargo run --example srtp_protected
```

#### [`debug_srtp.rs`](debug_srtp.rs)
SRTP debugging utilities and diagnostics.
```bash
cargo run --example debug_srtp
```

#### [`dtls_test.rs`](dtls_test.rs)
Availability check only: asserts typed rejection of the retained, incomplete
DTLS-SRTP constructor. It does not demonstrate a DTLS handshake.
```bash
cargo run --example dtls_test
```

#### [`direct_dtls_media_streaming.rs`](direct_dtls_media_streaming.rs)
Availability check only: asserts that direct DTLS media streaming is rejected
before opening a media path. It is not a streaming example in 0.3.5.
```bash
cargo run --example direct_dtls_media_streaming
```

#### [`generate_certificates.rs`](generate_certificates.rs)
Standalone certificate generation utility. Generated files do not make the
unavailable DTLS-SRTP implementation usable.
```bash
cargo run --example generate_certificates
```

### 🏗️ Core Advanced Features

#### [`ssrc_demultiplexing.rs`](ssrc_demultiplexing.rs)
Core packet demultiplexing based on SSRC for multiple streams.
```bash
cargo run --example ssrc_demultiplexing
```

#### [`high_performance_buffers.rs`](high_performance_buffers.rs)
Core high-performance buffer management with memory pooling.
```bash
cargo run --example high_performance_buffers
```

#### [`header_extensions.rs`](header_extensions.rs)
Core RTP header extensions (RFC 8285) implementation.
```bash
cargo run --example header_extensions
```

#### [`csrc_management.rs`](csrc_management.rs)
Core CSRC management for mixed streams in conferencing.
```bash
cargo run --example csrc_management
```

#### [`port_allocation.rs`](port_allocation.rs)
Port allocation and management for RTP sessions.
```bash
cargo run --example port_allocation
```

#### [`port_allocation_demo.rs`](port_allocation_demo.rs)
Port allocation demonstration and testing.
```bash
cargo run --example port_allocation_demo
```

### 🧪 Testing & Debugging

#### [`udp_test.rs`](udp_test.rs)
Basic UDP transport testing.
```bash
cargo run --example udp_test
```

#### [`udp_raw_test.rs`](udp_raw_test.rs)
Raw UDP packet testing and validation.
```bash
cargo run --example udp_raw_test
```

#### [`udp_to_broadcast_test.rs`](udp_to_broadcast_test.rs)
UDP to broadcast channel testing.
```bash
cargo run --example udp_to_broadcast_test
```

#### [`non_rtp_packet_test.rs`](non_rtp_packet_test.rs)
Testing handling of non-RTP packets.
```bash
cargo run --example non_rtp_packet_test
```

#### [`socket_validation.rs`](socket_validation.rs)
Socket validation and connection testing.
```bash
cargo run --example socket_validation.rs
```

#### [`broadcast_channel_test.rs`](broadcast_channel_test.rs)
Broadcast channel functionality testing.
```bash
cargo run --example broadcast_channel_test
```

#### [`rfc3551_compatibility.rs`](rfc3551_compatibility.rs)
RFC 3551 (RTP A/V Profile) compatibility testing.
```bash
cargo run --example rfc3551_compatibility
```

## 🚀 Quick Start

For new users, we recommend starting with these examples:

1. **[`api_basic.rs`](api_basic.rs)** - Learn the fundamental API
2. **[`api_srtp.rs`](api_srtp.rs)** - Add security to your application  
3. **[`api_ssrc_demultiplexing.rs`](api_ssrc_demultiplexing.rs)** - Handle multiple streams
4. **[`payload_type_demo.rs`](payload_type_demo.rs)** - Understand payload types

## 🔧 Running Examples

### Basic Usage
```bash
cargo run --example api_basic
```

### With Logging
Control the log level using the `RUST_LOG` environment variable:
```bash
RUST_LOG=debug cargo run --example api_srtp
RUST_LOG=trace cargo run --example rtcp_mux
```

### Running All Examples
Test all examples to ensure functionality:
```bash
# API examples
for example in api_*; do cargo run --example ${example%.rs}; done

# Core examples  
for example in !(api_)*; do cargo run --example ${example%.rs}; done
```

## 📋 Troubleshooting

### Common Issues

**Connection Timeouts**: Some examples may show connection timeouts - this is often expected behavior for demonstration purposes.

**Security Warnings**: Security examples may show intentional warnings to demonstrate error recovery mechanisms.

**Port Conflicts**: If you encounter port binding errors, ensure no other RTP applications are running.

### Getting Help

1. Check the example's source code for detailed comments
2. Run with `RUST_LOG=debug` for detailed logging
3. Review the main [rtp-core documentation](../README.md)
4. Check [TODO.md](TODO.md) for known issues and planned improvements

## 📚 Documentation

Each example includes detailed inline documentation explaining:
- Purpose and use case
- Key concepts demonstrated  
- Expected output and behavior
- Configuration options and variations

For API reference documentation, run:
```bash
cargo doc --open
```
