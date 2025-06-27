# RVOIP Media Core

`media-core` is the media processing engine for the RVOIP project. It focuses exclusively on media processing, codec management, and media session coordination while integrating cleanly with `session-core` (SIP signaling) and `rtp-core` (RTP transport).

## Vision & Scope

**media-core** handles the "smart" media processing while delegating transport and signaling to specialized crates:

### ✅ **Core Responsibilities**
- **Media Processing**: Codec encode/decode, audio processing (AEC, AGC, VAD, NS)
- **Media Session Management**: Coordinate media flows for SIP dialogs  
- **Quality Management**: Monitor and adapt media quality in real-time
- **Format Conversion**: Sample rate conversion, channel mixing
- **Codec Management**: Registry, negotiation, transcoding

### ❌ **Delegated Responsibilities**
- **RTP Transport**: Handled by `rtp-core`
- **SIP Signaling**: Handled by `session-core`  
- **Network I/O**: Handled by `rtp-core`
- **SDP Negotiation**: Handled by `session-core` (media-core provides capabilities)

## Architecture

Clean separation of concerns across three specialized crates:

```
┌─────────────────┐    Media Capabilities    ┌─────────────────┐
│                 │ ◄──────────────────────── │                 │
│  session-core   │                           │   media-core    │
│ (SIP Signaling) │ ──────────────────────► │ (Processing)    │
│                 │    Media Session Mgmt     │                 │
└─────────────────┘                           └─────────────────┘
                                                       │
                                                       │ Media Streams
                                                       ▼
                                              ┌─────────────────┐
                                              │    rtp-core     │
                                              │  (Transport)    │
                                              └─────────────────┘
```

### Integration Flow
1. **session-core → media-core**: Request capabilities, create/destroy media sessions
2. **media-core → session-core**: Provide codec capabilities, report events  
3. **rtp-core → media-core**: Deliver incoming media packets for processing
4. **media-core → rtp-core**: Send processed media packets for transmission

## Core Components

### MediaEngine - Central Orchestrator
```rust
pub struct MediaEngine {
    codec_manager: Arc<CodecManager>,
    session_manager: Arc<SessionManager>,
    quality_monitor: Arc<QualityMonitor>,
    audio_processor: Arc<AudioProcessor>,
    // ...
}
```

### MediaSession - Per-Dialog Management
```rust
pub struct MediaSession {
    dialog_id: DialogId,
    audio_codec: RwLock<Option<Box<dyn AudioCodec>>>,
    jitter_buffer: Arc<JitterBuffer>,
    quality_metrics: Arc<RwLock<QualityMetrics>>,
    // ...
}
```

## Module Structure

```
src/
├── engine/                    # Core Media Engine
│   ├── media_engine.rs        # Central MediaEngine orchestrator
│   ├── config.rs              # Engine configuration
│   └── lifecycle.rs           # Startup/shutdown management
│
├── session/                   # Media Session Management  
│   ├── media_session.rs       # MediaSession per SIP dialog
│   ├── session_manager.rs     # Manages multiple MediaSessions
│   └── events.rs              # Media session events
│
├── codec/                     # Codec Framework
│   ├── manager.rs             # CodecManager orchestration
│   ├── registry.rs            # Available codecs
│   ├── negotiation.rs         # Capability matching
│   ├── audio/                 # Audio codec implementations
│   │   ├── g711.rs            # G.711 μ-law/A-law (PCMU/PCMA)
│   │   ├── opus.rs            # Opus codec
│   │   └── g722.rs            # G.722 wideband
│   └── video/                 # Video codecs (future)
│
├── processing/                # Signal Processing
│   ├── audio/                 # Audio processing
│   │   ├── processor.rs       # Main audio processor
│   │   ├── aec.rs             # Echo cancellation
│   │   ├── agc.rs             # Gain control
│   │   ├── vad.rs             # Voice activity detection
│   │   └── ns.rs              # Noise suppression
│   └── format/                # Format conversion
│       ├── resampler.rs       # Sample rate conversion
│       └── channel_mixer.rs   # Channel conversion
│
├── quality/                   # Quality Management
│   ├── monitor.rs             # Real-time monitoring
│   ├── metrics.rs             # Quality metrics
│   └── adaptation.rs          # Quality adaptation
│
├── buffer/                    # Media Buffering
│   ├── jitter.rs              # Adaptive jitter buffering
│   └── adaptive.rs            # Dynamic buffer sizing
│
└── integration/               # Cross-Crate Bridges
    ├── rtp_bridge.rs          # rtp-core integration
    └── session_bridge.rs      # session-core integration
```

## Implementation Status

This is a **clean rewrite** following SIP best practices and modern Rust patterns.

### ✅ **Phase 1: Foundation** (In Progress)
- [ ] Core types and error handling
- [ ] MediaEngine structure
- [ ] Basic MediaSession
- [ ] G.711 codec implementation
- [ ] Integration bridges

### 📋 **Phase 2: Processing Pipeline**
- [ ] AudioProcessor framework
- [ ] Voice Activity Detection (VAD)
- [ ] Format conversion
- [ ] Jitter buffering
- [ ] Quality monitoring

### 🚀 **Phase 3: Advanced Features**
- [ ] Acoustic Echo Cancellation (AEC)
- [ ] Automatic Gain Control (AGC)
- [ ] Opus codec
- [ ] Codec transcoding
- [ ] Quality adaptation

## Usage Example

```rust
use rvoip_media_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create media engine
    let config = MediaEngineConfig::default();
    let engine = MediaEngine::new(config).await?;
    
    // Start the engine
    engine.start().await?;
    
    // Create media session for SIP dialog
    let dialog_id = DialogId::new("call-123");
    let params = MediaSessionParams::audio_only()
        .with_preferred_codec(PayloadType::PCMU)
        .with_processing_enabled(true);
    
    let session = engine.create_media_session(dialog_id, params).await?;
    
    // Get codec capabilities for SDP negotiation
    let capabilities = engine.get_supported_codecs();
    println!("Supported codecs: {:?}", capabilities);
    
    // Process incoming media (called by rtp-core)
    session.process_incoming_media(media_packet).await?;
    
    // Send outgoing media (to rtp-core)
    session.send_outgoing_media(audio_frame).await?;
    
    // Monitor quality
    let metrics = session.get_quality_metrics().await;
    println!("Call quality: {:?}", metrics);
    
    // Clean shutdown
    engine.destroy_media_session(dialog_id).await?;
    engine.stop().await?;
    
    Ok(())
}
```

## Features

- 🎵 **Advanced Audio Processing**: AEC, AGC, VAD, noise suppression
- 🔊 **Multiple Codecs**: G.711, Opus, G.722, DTMF support
- 📊 **Quality Monitoring**: Real-time quality metrics and adaptation
- 🔄 **Format Conversion**: Sample rate and channel conversion
- 📦 **Jitter Buffering**: Adaptive buffering for smooth playback
- ⚡ **High Performance**: Optimized for real-time media processing
- 🧩 **Clean Integration**: Works seamlessly with session-core and rtp-core

## Integration with Other Crates

- **rvoip-session-core**: Provides SIP signaling and dialog management
- **rvoip-rtp-core**: Provides RTP transport and packet handling

## License

This project is licensed under the MIT License or Apache 2.0 License, at your option. 