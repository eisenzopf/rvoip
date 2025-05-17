# RTP Core Examples

This directory contains examples demonstrating the functionality of the `rtp-core` crate.

## Available Examples

### 1. Basic Media Transport (`media_transport.rs`)

This example demonstrates bidirectional RTP packet exchange between two RTP sessions.

```
cargo run --example media_transport
```

### 2. Payload Format Handling (`payload_format.rs`)

Shows how different payload formats are handled for various codecs, focusing on G.711.

```
cargo run --example payload_format
```

### 3. G.722 Payload Format (`g722_payload.rs`)

Demonstrates G.722's special timestamp handling with 16kHz sampling but 8kHz RTP timestamps.

```
cargo run --example g722_payload
```

### 4. Opus Payload Format (`opus_payload.rs`)

Shows Opus codec configuration with different bandwidth modes, channels, and framerates.

```
cargo run --example opus_payload
```

### 5. Video Payload Formats (`video_payload.rs`)

Demonstrates VP8 and VP9 video payload formats with scalability and multiple resolutions.

```
cargo run --example video_payload
```

### 6. SSRC Demultiplexing (`ssrc_demultiplexing.rs`)

Shows packet demultiplexing based on SSRC, handling multiple streams in a single session.

```
cargo run --example ssrc_demultiplexing
```

### 7. High-Performance Buffer Management (`high_performance_buffers.rs`)

Demonstrates the high-performance buffer management system designed for large-scale 
deployments with tens of thousands of concurrent connections. Features include:

- Memory pooling to reduce allocations and GC pressure
- Adaptive jitter buffering that responds to network conditions
- Priority-based transmit buffering with congestion control
- Global memory management to prevent OOM errors
- Stream prioritization and bandwidth management

```
cargo run --example high_performance_buffers
```

### 8. RTCP BYE Packet Handling (`rtcp_bye.rs`)

Demonstrates RTCP BYE packet sending and receiving when RTP sessions are closed.

```
cargo run --example rtcp_bye
```

### 9. RTCP APP Packet Handling (`rtcp_app.rs`)

Shows how to use RTCP APP packets for application-specific functions.

```
cargo run --example rtcp_app
```

### 10. RTCP Extended Reports (`rtcp_xr_example.rs`)

Demonstrates RTCP XR (Extended Reports) for detailed media quality metrics.

```
cargo run --example rtcp_xr_example
```

### 11. RFC 3551 Compatibility (`rfc3551_compatibility.rs`)

Tests compatibility with RFC 3551 (RTP A/V Profile) including payload types.

```
cargo run --example rfc3551_compatibility
```

### 12. Header Extensions (`header_extensions.rs`)

Demonstrates RTP header extensions (RFC 8285) for carrying additional metadata.

```
cargo run --example header_extensions
```

### 13. CSRC Management for Mixed Streams (`csrc_management.rs`)

Demonstrates how to use CSRC management capabilities in mixed RTP streams for conferencing.
This example shows:

- Creating and managing mappings between SSRCs and CSRCs
- Adding contributing sources to RTP headers
- Generating RTCP SDES packets that describe all sources
- Simulating a conferencing mixer that combines multiple audio sources
- Properly attributing media to active participants using CSRCs

```
cargo run --example csrc_management
```

## Running with Logging

You can control the log level using the `RUST_LOG` environment variable:

```
RUST_LOG=debug cargo run --example high_performance_buffers
```

## Known Issues

- RTCP compound packets are not yet fully supported. 