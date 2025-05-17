# RTP Core

This crate provides a complete implementation of the Real-time Transport Protocol (RTP) and Real-time Transport Control Protocol (RTCP) for the rVOIP project. It handles packet encoding/decoding, session management, statistics tracking, and secure transport.

## Features

- RTP packet encoding and decoding (RFC 3550)
- RTCP packet support (Sender Reports, Receiver Reports, SDES, etc.)
- RTCP Extended Reports (XR) with VoIP metrics for call quality monitoring
- RTCP compound packet handling for efficient statistics reporting
- RTP session management
- Jitter buffer implementation with adaptive sizing
- High-performance buffer management for large-scale deployments
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
- RTCP Extended Reports (XR) with VoIP quality metrics

### Session Management

The `session` module provides high-level RTP session management, including:
- Packet scheduling
- Sequence number and timestamp management
- Stream tracking and stats collection
- SSRC demultiplexing
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
- VoIP quality metrics (R-factor and MOS scores)

### Buffer Management

The `buffer` module provides high-performance buffer management for:
- Memory pooling to reduce allocations and GC pressure
- Adaptive jitter buffer with RFC-compliant jitter calculations
- Priority-based transmit buffer with congestion control
- Global memory limits to prevent OOM conditions

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
- RFC 7741: RTP Payload Format for VP8 Video
- RFC 8241: RTP Payload Format for VP9 Video

## RTCP Extended Reports

The implementation includes comprehensive RTCP XR (Extended Reports) as per RFC 3611, providing:

- Receiver Reference Time reporting for precise synchronization
- VoIP metrics reporting with:
  - Loss/discard rates and burst metrics
  - End-to-end and round-trip delay measurements
  - Signal and noise levels
  - R-factor calculation according to ITU-T G.107 E-model
  - MOS (Mean Opinion Score) derivation for listening and conversational quality
- RTCP compound packet handling for efficient reporting
- Statistics summaries for detailed stream analysis

Example of creating VoIP metrics reports:

```rust
use rvoip_rtp_core::{RtcpExtendedReport, VoipMetricsBlock, RtcpCompoundPacket};

// Create an XR packet with VoIP metrics
let mut xr = RtcpExtendedReport::new(ssrc);
let mut metrics = VoipMetricsBlock::new(stream_ssrc);

// Configure metrics
metrics.loss_rate = 5; // 5% packet loss
metrics.round_trip_delay = 150; // 150ms RTT

// Calculate R-factor and MOS scores
metrics.calculate_r_factor(5.0, 150, 30.0);

// Add to XR packet and create compound packet
xr.add_voip_metrics(metrics);
let mut compound = RtcpCompoundPacket::new_with_sr(sender_report);
compound.add_xr(xr);
```

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
- VP8 and VP9 video (dynamic payload types)

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