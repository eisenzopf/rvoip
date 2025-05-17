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

Shows Opus codec configuration with different bandwidth modes, channels, and frame durations.

```
cargo run --example opus_payload
```

### 5. RTCP BYE Packet Handling (`rtcp_bye.rs`)

Demonstrates RTCP BYE packet sending and receiving when RTP sessions are closed.

```
cargo run --example rtcp_bye
```

### 6. RTCP APP Packet Handling (`rtcp_app.rs`)

Shows how RTCP Application-Defined (APP) packets can be sent and received between RTP sessions.

```
cargo run --example rtcp_app
```

### 7. Video Payload Formats (`video_payload.rs`)

Demonstrates VP8 and VP9 payload format handling for video over RTP.

```
cargo run --example video_payload
```

### 8. SSRC Demultiplexing (`ssrc_demultiplexing.rs`)

Shows how multiple RTP streams with different SSRCs can be handled within a single RTP session.

```
cargo run --example ssrc_demultiplexing
```

## Known Issues

- RTCP compound packets are not yet fully supported. 