# RTP Core

This crate provides a complete implementation of the Real-time Transport Protocol (RTP) and Real-time Transport Control Protocol (RTCP) for the rVOIP project. It handles packet encoding/decoding, session management, statistics tracking, and secure transport.

## Features

- RTP packet encoding and decoding (RFC 3550)
- RTCP packet support (Sender Reports, Receiver Reports, SDES, etc.)
- RTP session management
- Jitter buffer implementation
- Packet loss and reception statistics
- SRTP encryption and authentication (RFC 3711)
- Transport-agnostic design with UDP implementation
- Media Transport interface for integration with media-core

## Key Components

### Packet Handling

The `packet` module provides encoding and decoding of RTP and RTCP packets, including:
- RTP headers with extension support
- CSRC list management
- RTCP compound packet handling
- RTCP report generation

### Session Management

The `session` module provides high-level RTP session management, including:
- Packet scheduling
- Sequence number and timestamp management
- Stream tracking and stats collection
- Jitter buffer implementation

### Transport Layer

The `transport` module provides network transport mechanisms:
- UDP transport implementation
- Event-based packet reception
- RTP/RTCP socket management
- Transport configuration options

### Secure RTP

The `srtp` module implements Secure RTP (SRTP):
- Encryption using AES-CM
- Authentication using HMAC-SHA1
- Key derivation
- Replay protection

### Statistics

The `stats` module provides comprehensive RTP statistics:
- Packet loss detection
- Jitter estimation
- Round-trip time calculation
- RTCP report generation

## Integration with Media Core

For integration with the media-core crate, this library provides the `MediaTransport` trait and an implementation in `RtpMediaTransport`. This allows the media-core to send and receive media data without concerning itself with the details of RTP.

## Example Usage

```rust
use rvoip_rtp_core::{RtpSession, RtpSessionConfig, MediaTransport, RtpMediaTransport};
use bytes::Bytes;

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    // Create RTP session
    let config = RtpSessionConfig {
        local_addr: "0.0.0.0:0".parse().unwrap(),
        // ... other config options
    };
    
    let session = RtpSession::new(config).await?;
    
    // Create MediaTransport adapter
    let transport = RtpMediaTransport::new(session);
    
    // Send media
    let payload = Bytes::from_static(b"media data");
    transport.send_media(0, 12345, payload, true).await?;
    
    Ok(())
}
```

See the `examples` directory for more complete examples.

## RFC Compliance

This library implements the following RFCs:
- RFC 3550: RTP/RTCP base protocol
- RFC 3551: RTP Audio/Video Profile
- RFC 3611: RTCP Extended Reports (XR)
- RFC 3711: Secure Real-time Transport Protocol (SRTP)

## Payload Format Handling

The RTP Core library provides a framework for handling different RTP payload formats. This allows for:

1. Properly interpreting media data carried in RTP packets
2. Converting between raw media frames and RTP payload formats
3. Calculating timing information based on packet size and codec properties

The framework is designed to be extensible, with a factory pattern that allows new codec payload formats to be added easily. Currently supported payload formats include:

- G.711 µ-law (PCMU, payload type 0)
- G.711 A-law (PCMA, payload type 8)
- G.722 wideband (payload type 9)
- Opus (dynamic payload type, typically 96-127)

Example usage:

```rust
use rvoip_rtp_core::{PayloadFormat, PayloadType, create_payload_format, OpusPayloadFormat, OpusBandwidth};

// Create a payload format handler for G.722
let g722_format = create_payload_format(PayloadType::G722, None).unwrap();

// Create a configurable Opus format for high-quality stereo audio
let opus_format = OpusPayloadFormat::new(101, 2) // PT 101, stereo
    .with_max_bitrate(128000)    // 128 kbit/s
    .with_bandwidth(OpusBandwidth::Fullband)
    .with_duration(20);          // 20ms frame size

// Pack encoded data into RTP payload
let payload = format.pack(&encoded_data, timestamp);

// Calculate timing
let packet_duration_ms = format.duration_from_packet_size(payload.len());
``` 