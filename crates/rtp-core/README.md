# RVOIP RTP Core

This crate provides core RTP functionality for the RVOIP project:

- RTP/RTCP packet parsing and serialization
- RTP session management
- SRTP encryption/decryption
- DTLS handshake for key exchange
- High-level API for media transport

## Features

- **RTP/RTCP Implementation**: Full RFC 3550 implementation of RTP and RTCP
- **SRTP Support**: Secure RTP implementation with AES-CM encryption and HMAC-SHA1 authentication
- **Cross-Platform**: Works on all major platforms (Linux, macOS, Windows)
- **Flexible Transport**: UDP transport with configurable RTCP multiplexing
- **Port Allocation**: Smart port allocation with various strategies (Sequential, Random, Incremental)
- **Payload Formats**: Support for multiple codecs (G.711, G.722, Opus, VP8, VP9)
- **Jitter Buffer**: Adaptive jitter buffer for handling network variation

## SRTP Implementation

The SRTP implementation follows RFC 3711 and includes:

- AES-CM (Counter Mode) encryption
- HMAC-SHA1 authentication in both 80-bit and 32-bit variants
- Key derivation functions
- IV generation
- Proper authentication tag handling with tamper detection

The implementation is tested with various examples in the `examples/` directory:
- `srtp_crypto.rs`: Tests all cipher combinations
- `srtp_protected.rs`: Demonstrates authentication handling and tamper resistance

## Usage

```rust
// Create a secure RTP session
let crypto_key = SrtpCryptoKey::new(master_key, master_salt);
let crypto = SrtpCrypto::new(SRTP_AES128_CM_SHA1_80, crypto_key)?;

// Encrypt an RTP packet
let (encrypted_packet, auth_tag) = crypto.encrypt_rtp(&packet)?;

// Create a protected packet with auth tag
let protected = ProtectedRtpPacket::new(encrypted_packet, auth_tag);

// Serialize for transmission
let bytes = protected.serialize()?;

// On receiving side
let decrypted_packet = crypto.decrypt_rtp(&bytes)?;
```

## Notes

The library has been extensively tested and most SRTP functionality is working correctly. However, there are some remaining test failures in other parts of the library:

1. Buffer/jitter tests: Some timing and sequence number issues in tests
2. Payload format tests for VP8/VP9: Byte size calculation discrepancies
3. RTCP XR tests: Structure size mismatches

These issues are not related to the core SRTP functionality and can be addressed separately.

## Examples

### Basic RTP API Usage

The basic RTP API example demonstrates how to use the high-level API to create a server and client, and exchange RTP packets:

```bash
cargo run --example api_basic
```

### High-Performance Buffer Configuration

The high-performance buffer example demonstrates how to configure and use the tunable buffer options:

```bash
cargo run --example api_high_performance_buffers
```

## License

This project is dual-licensed under either:

* MIT License
* Apache License, Version 2.0

at your option. 