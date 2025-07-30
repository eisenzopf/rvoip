# RVOIP SIP Client

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip-client.svg)](https://crates.io/crates/rvoip-sip-client)
[![Documentation](https://docs.rs/rvoip-sip-client/badge.svg)](https://docs.rs/rvoip-sip-client)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

A unified, production-ready SIP client library that orchestrates the RVOIP stack components to provide a complete VoIP solution.

## Overview

The `sip-client` library integrates three core components:
- **client-core**: High-level SIP protocol handling and session management
- **audio-core**: Audio device management, format conversion, and pipeline processing
- **codec-core**: Audio codec encoding/decoding (G.711, etc.)

## Features

- 🚀 **Simple API** - Get started with just 3 lines of code
- 🎛️ **Advanced Control** - Full access to audio pipeline and codec configuration
- 🔊 **Audio Processing** - Built-in echo cancellation, noise suppression, and AGC
- 📞 **Complete Call Control** - Make, receive, transfer, hold, and conference calls
- 🎯 **Automatic Codec Negotiation** - Seamless interoperability with any SIP endpoint
- 📊 **Real-time Metrics** - Call quality monitoring with MOS scores
- 🔄 **Event-driven Architecture** - Perfect for modern UI frameworks

## Quick Start

```rust
use sip_client::SipClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One-line setup with defaults
    let client = SipClient::new("sip:alice@example.com").await?;
    
    // Make a call
    let call = client.call("sip:bob@example.com").await?;
    
    // Wait for answer
    call.wait_for_answer().await?;
    
    println!("Call connected! Press Ctrl+C to hangup");
    tokio::signal::ctrl_c().await?;
    
    // Hangup
    call.hangup().await?;
    
    Ok(())
}
```

## Advanced Usage

```rust
use sip_client::{SipClientBuilder, AudioPipelineConfig, CodecPriority};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Advanced configuration with full control
    let client = SipClientBuilder::new()
        .sip_identity("sip:alice@example.com")
        .sip_server("sip.example.com:5060")
        .audio_pipeline(
            AudioPipelineConfig::custom()
                .input_device("Blue Yeti Microphone")
                .output_device("AirPods Pro")
                .echo_cancellation(true)
                .noise_suppression(true)
                .auto_gain_control(true)
        )
        .codecs(vec![
            CodecPriority::new("opus", 100),
            CodecPriority::new("G722", 90),
            CodecPriority::new("PCMU", 80),
        ])
        .build()
        .await?;
    
    // Subscribe to events
    let mut events = client.events();
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            match event {
                SipClientEvent::CallQualityReport { mos, .. } => {
                    println!("Call quality: {:.1} MOS", mos);
                }
                _ => {}
            }
        }
    });
    
    // Make a call with custom audio processing
    let call = client.call("sip:bob@example.com").await?;
    let mut audio_stream = call.audio_stream().await?;
    
    // Process audio frames directly
    while let Some(frame) = audio_stream.next().await {
        // Apply custom processing
        let processed = my_audio_filter(frame);
        audio_stream.send(processed).await?;
    }
    
    Ok(())
}
```

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
rvoip-sip-client = "0.1"
tokio = { version = "1.0", features = ["full"] }
```

## Examples

Check out the `examples/` directory for complete working examples:

- `simple_softphone` - Basic softphone implementation
- `advanced_client` - Advanced features demonstration
- `call_center_agent` - Call center agent console

Run examples with:

```bash
cargo run --example simple_softphone
```

## Architecture

```
Your Application
        │
        ▼
   SIP Client (this crate)
        │
   ┌────┴────┬──────────┐
   ▼         ▼          ▼
client-core  audio-core  codec-core
```

The SIP Client handles all the complex integration between components, providing you with a clean, unified API.

## License

This project is licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.