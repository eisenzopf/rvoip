# RVOIP SIP Client

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip-client.svg)](https://crates.io/crates/rvoip-sip-client)
[![Documentation](https://docs.rs/rvoip-sip-client/badge.svg)](https://docs.rs/rvoip-sip-client)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

A simple, batteries-included SIP client library for making and receiving VoIP calls in Rust.

## What can you do with this?

- ✅ **Make voice calls** to any SIP address
- ✅ **Receive incoming calls** from other SIP clients
- ✅ **Connect to SIP servers** (like Asterisk, FreeSWITCH)
- ✅ **Direct peer-to-peer calls** using IP addresses
- ✅ **Automatic audio handling** - just plug in your mic and speakers
- ✅ **Production-ready** with error recovery and reconnection

## Quick Start

### 1. Add to your `Cargo.toml`:

```toml
[dependencies]
rvoip-sip-client = "0.1"
tokio = { version = "1.0", features = ["full"] }
```

### 2. Make your first call:

```rust
use rvoip_sip_client::SipClient;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a SIP client with your SIP address
    let client = SipClient::new("sip:alice@example.com").await?;
    
    // Start the client
    client.start().await?;
    
    // Make a call
    let call = client.call("sip:bob@example.com").await?;
    
    // Wait for the other person to answer
    call.wait_for_answer().await?;
    println!("🎉 Call connected!");
    
    // Let them talk for 30 seconds
    tokio::time::sleep(Duration::from_secs(30)).await;
    
    // Hang up
    client.hangup(&call.id).await?;
    println!("📞 Call ended");
    
    Ok(())
}
```

### 3. Receive incoming calls:

```rust
use rvoip_sip_client::{SipClient, SipClientEvent};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = SipClient::new("sip:alice@example.com").await?;
    client.start().await?;
    
    // Subscribe to events
    let mut events = client.events();
    
    println!("📞 Waiting for calls...");
    
    while let Some(event) = events.next().await {
        match event {
            SipClientEvent::IncomingCall { call, from, .. } => {
                println!("📞 Incoming call from {}", from);
                
                // Answer the call
                client.answer(&call.id).await?;
                println!("✅ Call answered!");
            }
            SipClientEvent::CallEnded { call } => {
                println!("📞 Call ended: {}", call.id);
            }
            _ => {}
        }
    }
    
    Ok(())
}
```

## Common Use Cases

### Connect to a SIP Server (PBX)

```rust
use rvoip_sip_client::SipClientBuilder;

// Connect to your company's PBX
let client = SipClientBuilder::new()
    .sip_identity("sip:alice@company.com")
    .sip_server("pbx.company.com:5060")
    .build()
    .await?;
```

### Direct Peer-to-Peer Call

```rust
// Call someone directly by IP address (no server needed!)
let call = client.call("sip:bob@192.168.1.100:5060").await?;
```

### Mute/Unmute During Call

```rust
// Mute your microphone
client.set_mute(&call.id, true).await?;

// Unmute
client.set_mute(&call.id, false).await?;
```

### List Audio Devices

```rust
use rvoip_audio_core::AudioDirection;

// List available microphones
let mics = client.list_audio_devices(AudioDirection::Input).await?;
for mic in mics {
    println!("🎤 {}", mic.name);
}

// List available speakers
let speakers = client.list_audio_devices(AudioDirection::Output).await?;
for speaker in speakers {
    println!("🔊 {}", speaker.name);
}
```

## Error Handling

The library provides user-friendly error messages:

```rust
match client.call("sip:invalid@nowhere").await {
    Ok(call) => println!("Call started"),
    Err(e) => {
        // You'll get helpful messages like:
        // "Network connectivity issue detected"
        // "Check your internet connection"
        // "Verify firewall settings allow SIP traffic"
        eprintln!("Call failed: {}", e);
    }
}
```

## Events You Can Listen For

```rust
use rvoip_sip_client::SipClientEvent;

match event {
    SipClientEvent::IncomingCall { call, from, .. } => {
        // Someone is calling you
    }
    SipClientEvent::CallConnected { call_id, .. } => {
        // Call was answered
    }
    SipClientEvent::CallEnded { call } => {
        // Call finished
    }
    SipClientEvent::AudioLevelChanged { level, .. } => {
        // Audio volume changed (useful for UI meters)
    }
    _ => {}
}
```

## Supported Features

✅ **What Works Now:**
- G.711 μ-law and A-law codecs (standard telephony codecs)
- Echo cancellation and noise suppression
- Automatic audio device selection
- Error recovery and reconnection
- Both server-based and peer-to-peer calls

⏳ **Coming Soon:**
- Additional codecs (Opus, G.722)
- Call recording
- Conference calls
- Video calls

## Testing

The sip-client crate includes comprehensive tests, including integration tests that simulate full audio roundtrips between SIP clients.

### Running All Tests

Some tests require the `test-audio` feature to be enabled. To run all tests including the full roundtrip test:

```bash
# Using cargo alias (recommended)
cargo test-all

# Or explicitly with features
cargo test --features test-audio

# Or use the provided script
./run-all-tests.sh
```

### Running Specific Tests

```bash
# Run just the full roundtrip test
cargo test-roundtrip

# Run tests with all features
cargo test-everything
```

The full roundtrip test (`tests/full_roundtrip.rs`) creates two SIP clients that exchange audio through WAV files, providing end-to-end validation of the audio pipeline.

## Troubleshooting

### "Not receiving audio"
- Check your firewall allows UDP ports 5060 (SIP) and 10000-20000 (RTP)
- Ensure your audio devices have proper permissions

### "Registration failed"
- Verify your SIP credentials
- Check the server address and port
- Ensure you're connected to the internet

### "No audio devices found"
- On macOS: Check System Preferences > Security & Privacy > Microphone
- On Windows: Check Settings > Privacy > Microphone
- On Linux: Ensure PulseAudio/ALSA is running

## License

This project is licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Need Help?

- 📖 [API Documentation](https://docs.rs/rvoip-sip-client)
- 💬 [GitHub Issues](https://github.com/rvoip/rvoip/issues)
- 📧 [Email Support](mailto:support@rvoip.io)