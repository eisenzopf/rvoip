# RVOIP Media Core

`media-core` is the media handling library for the RVOIP project, providing audio processing, codec management, and media session coordination. It acts as the bridge between signaling (via session-core) and media transport (via rtp-core).

## Features

- **Codec Framework**: Support for audio codecs including G.711, G.722, and Opus
- **Audio Processing**: Components for echo cancellation, noise suppression, and voice activity detection
- **Session Management**: High-level media session abstraction for VoIP applications
- **RTP Integration**: Packetization, depacketization, and session management for RTP
- **Format Conversions**: Audio resampling and channel conversion utilities
- **Buffer Management**: Jitter buffers and adaptive media buffers
- **Secure Media**: SRTP and DTLS integration for secure communications

## Architecture

The media-core library follows a layered architecture:

```
┌─────────────────────────────────────────────────────────┐
│                Session Management Layer                  │
│       (Media sessions, negotiation, coordination)        │
└───────────┬─────────────────────────────────┬───────────┘
            │                                 │
┌───────────▼────────────┐   ┌───────────────▼────────────┐
│     Codec Framework    │   │       Media Processing      │
│   (Encoding/Decoding)  │◄──┤ (Echo, Noise, VAD, Format)  │
└───────────┬────────────┘   └───────────────┬────────────┘
            │                                │
┌───────────▼────────────┐   ┌───────────────▼────────────┐
│     Buffer Management  │   │       Security Layer        │
│  (Jitter, Adaptive)    │   │    (SRTP, DTLS, Crypto)     │
└───────────┬────────────┘   └───────────────┬────────────┘
            │                                │
┌───────────▼────────────────┬──────────────▼────────────┐
│       RTP Integration       │    Integration Layer       │
│ (Packet, Depacket, Session) │ (session-core, rtp-core)   │
└────────────────────────────┴───────────────────────────┘
```

## Module Structure

The library is organized into several core modules:

- **codec**: Framework for audio/video codecs
  - **registry**: Manages available codecs and creates instances
  - **audio**: Audio codec implementations (G.711, G.722, Opus)
  - **video**: Video codec implementations (future)

- **session**: Media session management
  - **media_session**: Core media session implementation
  - **config**: Session configuration
  - **events**: Media session events
  - **flow**: Media flow control (start/stop/pause)

- **processing**: Media signal processing
  - **audio**: Audio processing components
    - **aec**: Acoustic echo cancellation
    - **agc**: Automatic gain control
    - **vad**: Voice activity detection
    - **ns**: Noise suppression
    - **plc**: Packet loss concealment
  - **format**: Format conversion utilities

- **buffer**: Media buffer management
  - **jitter**: Jitter buffer implementation
  - **adaptive**: Adaptive buffer sizing

- **rtp**: RTP integration
  - **packetizer**: Converts media frames to RTP packets
  - **depacketizer**: Converts RTP packets to media frames
  - **session**: RTP session management

- **security**: Media security features
  - **srtp**: SRTP integration
  - **dtls**: DTLS key exchange

- **engine**: Audio/video processing engines
  - **audio**: Audio capture and playback
  - **mixer**: Audio mixing capabilities

- **integration**: Integration with other components
  - **session_core**: Session-core integration
  - **rtp_core**: RTP-core integration
  - **sdp**: SDP handling for media negotiation

## Implementation Status

### Completed Components

- ✅ Core library structure and error handling
- ✅ Audio format definitions and utilities
- ✅ RTP packetizer and depacketizer
- ✅ Basic codec registry framework
- ✅ Initial G.711 codec implementation
- ✅ Audio processing framework (VAD)
- ✅ Media session abstraction
- ✅ SDP integration for media negotiation
- ✅ RTP session management
- ✅ Format conversion utilities

### In Progress

- 🔄 Complete codec framework
- 🔄 SRTP and DTLS integration
- 🔄 Remaining audio processing components (AEC, AGC)
- 🔄 Jitter buffer implementation
- 🔄 Media flow control

### Planned Next

- 📝 Quality monitoring and metrics
- 📝 Media engine integration with audio devices
- 📝 Additional codec implementations
- 📝 Media synchronization
- 📝 Full integration with session-core

## Usage Example

```rust
use rvoip_media_core::prelude::*;
use rvoip_media_core::codec::registry::CodecRegistry;
use rvoip_media_core::rtp::create_audio_session;
use std::sync::Arc;

async fn create_call() -> Result<()> {
    // Create codec registry
    let registry = CodecRegistry::new();
    
    // Get a codec (PCMU/G.711)
    let codec = registry.create_codec_by_payload_type(0)?;
    
    // Create an RTP session
    let local_addr = "0.0.0.0:10000".parse()?;
    let (session, mut events) = create_audio_session(
        Arc::new(codec),
        local_addr,
        0, // PCMU payload type
        SampleRate::Rate8000
    ).await?;
    
    // Connect to remote party
    let remote_addr = "192.168.1.100:20000".parse()?;
    session.set_remote_addr(remote_addr).await?;
    
    // Process events
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                RtpSessionEvent::AudioReceived(buffer) => {
                    // Process received audio
                    println!("Received audio: {} bytes", buffer.data.len());
                },
                RtpSessionEvent::Connected(addr) => {
                    println!("Connected to {}", addr);
                },
                RtpSessionEvent::Disconnected => {
                    println!("Disconnected");
                    break;
                },
                RtpSessionEvent::Error(err) => {
                    println!("Error: {}", err);
                    break;
                },
                _ => {}
            }
        }
    });
    
    // Create audio buffer
    let pcm_data = vec![0i16; 160].into_iter()
        .map(|_| rand::random::<i16>())
        .collect::<Vec<_>>();
    
    let bytes_data = unsafe {
        std::slice::from_raw_parts(
            pcm_data.as_ptr() as *const u8,
            pcm_data.len() * 2
        )
    };
    
    let buffer = AudioBuffer::new(
        bytes::Bytes::copy_from_slice(bytes_data),
        AudioFormat::telephony()
    );
    
    // Send audio
    session.send_audio(&buffer).await?;
    
    Ok(())
}
```

## Integration with Other Crates

- **rvoip-rtp-core**: Provides RTP packet definitions and transport
- **rvoip-session-core**: Provides SIP signaling and session management
- **rvoip-ice-core**: Handles NAT traversal (planned)

## License

This project is licensed under the MIT License or Apache 2.0 License, at your option. 