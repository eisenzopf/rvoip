# SIP Client Library - Development Plan

## Overview

The `sip-client` library provides a unified, production-ready SIP client implementation that orchestrates audio device management with SIP protocol handling:
- **client-core**: High-level SIP protocol handling and session management (includes RTP via session-core/media-core)
- **audio-core**: Audio device management, PCM capture/playback, and format conversion
- **codec-core**: Not directly used - media-core will integrate codec-core for encoding/decoding

This library serves as the primary entry point for developers building VoIP applications, providing both simple and advanced APIs while handling the complexity of connecting audio devices to SIP/RTP streams.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    VoIP Application                             │
└─────────────────────┬───────────────────────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────────────────────┐
│                    sip-client                                   │
│ ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│ │   Simple API    │  │  Advanced API   │  │   Builder API   │ │
│ │                 │  │                 │  │                 │ │
│ │ • Quick setup   │  │ • Full control  │  │ • Flexible cfg  │ │
│ │ • Sane defaults │  │ • Custom pipes  │  │ • Feature flags │ │
│ └─────────────────┘  └─────────────────┘  └─────────────────┘ │
│                                                                 │
│ ┌─────────────────────────────────────────────────────────────┐ │
│ │                  Integration Layer                           │ │
│ │                                                              │ │
│ │  • Audio pipeline lifecycle management                       │ │
│ │  • PCM frame flow coordination                               │ │
│ │  • Event aggregation and translation                         │ │
│ │  • Resource management and cleanup                           │ │
│ └─────────────────────────────────────────────────────────────┘ │
└─────────────────────┬───────────────────────────────────────────┘
                      │
     ┌────────────────┼────────────────┐
     │                │                │
     ▼                ▼                ▼
┌──────────┐    ┌──────────┐    ┌────────────────────────┐
│client-core│    │audio-core│    │media-core (internal)   │
│           │    │          │    │ • Uses codec-core for  │
│ • SIP     │    │ • Device │    │   encoding/decoding    │
│ • RTP     │    │   mgmt   │    │ • Managed by           │
│ • SDP     │    │ • PCM    │    │   session-core         │
└──────────┘    └──────────┘    └────────────────────────┘
```

**Note**: codec-core is not directly used by sip-client. Instead, media-core (managed by session-core) uses codec-core as its codec provider. This maintains the established architecture where encoding/decoding happens at the RTP layer.

## Core Components

### 1. Simple API
Provides a streamlined interface for common VoIP use cases:
- One-line client creation with sensible defaults
- Automatic codec selection based on peer capabilities
- Built-in echo cancellation and noise suppression
- Event-driven architecture for UI integration

### 2. Advanced API
Offers fine-grained control for sophisticated applications:
- Custom audio pipeline configuration
- Manual codec selection and prioritization
- Direct access to audio frames for processing
- Advanced call control (transfer, conference, etc.)

### 3. Builder API
Flexible configuration system:
- Progressive disclosure of complexity
- Feature flag integration
- Runtime configuration validation
- Preset configurations for common scenarios

### 4. Integration Layer
Handles the orchestration between audio devices and SIP/RTP:
- **Audio Pipeline Management**: Connects audio devices to client-core's audio streaming API
- **PCM Frame Flow**: Coordinates bidirectional PCM audio between audio-core and client-core
- **Media Lifecycle**: Manages setup/teardown of audio pipelines when calls connect/disconnect
- **Event System**: Aggregates events from client-core and audio-core into unified stream

## Current Status

### ✅ Completed
- **Phase 1 Foundation**: All project setup, core types, and basic integration complete
- **Library Cleanup**: Successfully removed codec features from audio-core and audio features from client-core
- **Simple API**: Basic structure implemented with call operations
- **Tests**: All components compile and pass basic tests

### 🚧 What's Next (Priority Order)

1. **Complete Audio Pipeline Integration** (Phase 2)
   - Implement `setup_audio_pipeline()` and `cleanup_audio_pipeline()` in simple.rs
   - Connect audio-core PCM capture to client-core's `send_audio_frame()` API
   - Connect client-core's `subscribe_to_audio_frames()` to audio-core PCM playback
   - Handle bidirectional PCM frame flow with proper timing

2. **Media-Core Codec Integration** (Prerequisite - needs to be done in media-core)
   - Media-core needs to integrate codec-core as its codec provider
   - This work is outside sip-client scope but required for the architecture
   - Once complete, client-core will automatically use codec-core for encoding/decoding

3. **Event System Completion** (Phase 2)
   - Connect client-core events to sip-client event system
   - Add audio-core event forwarding
   - Implement proper event filtering and transformation

4. **Resource Management** (Phase 2)
   - Add proper start/stop lifecycle methods
   - Implement graceful shutdown
   - Handle resource cleanup on errors

5. **Testing Infrastructure** (Phase 2)
   - Create mock implementations for testing
   - Add integration tests for call flows
   - Test audio pipeline setup/teardown

## Missing Integration Pieces

### Core Integration Gaps
The following are the key pieces missing that sip-client needs to implement:

### 1. **Audio Device to RTP Stream Bridge**
**Problem**: Need to connect audio devices to client-core's audio streaming API
- audio-core produces/consumes PCM `AudioFrame` from devices
- client-core expects `session-core::AudioFrame` for its streaming API
- Need continuous bidirectional flow with proper timing

**Solution**: Create an audio bridge that:
- Converts between audio-core and session-core AudioFrame types
- Manages capture and playback tasks with proper timing
- Handles backpressure and buffer management

### 2. **Prerequisite: Media-Core Codec Integration**
**Status**: This needs to be implemented in media-core project
- media-core currently has its own G.711 implementation
- Need to update media-core to use codec-core as its codec provider
- This maintains the architecture where encoding happens at the RTP layer

**Impact on sip-client**: Once complete, encoding/decoding will happen automatically in media-core when we send/receive PCM frames through client-core

### 3. **Media Session Lifecycle Coordination**
**Problem**: Need to coordinate audio pipeline with call lifecycle
- When call connects: need to setup audio capture/playback pipelines
- When call disconnects: need to cleanup all audio resources
- Handle media state changes (hold/resume, etc.)

**Solution**: Extend the `Call` object with audio lifecycle methods that:
- Start audio pipelines when call is established
- Stop audio pipelines when call ends
- Handle hold/resume by pausing/resuming pipelines

### 4. **Audio Processing Loop Implementation**
**Problem**: Need continuous PCM audio flow between components

**Capture Direction** (Mic → Network):
```
Microphone → audio-core → PCM AudioFrame → client-core.send_audio_frame() → [media-core encodes] → RTP
```

**Playback Direction** (Network → Speaker):
```
RTP → [media-core decodes] → client-core.subscribe_to_audio_frames() → PCM AudioFrame → audio-core → Speaker
```

**Solution**: Use async streams with backpressure handling:
- Spawn capture task that reads from audio-core and sends to client-core
- Spawn playback task that receives from client-core and plays via audio-core
- Handle timing and buffering appropriately

### Recommended Implementation Approach

1. **Focus on PCM Frame Flow**: Connect audio devices to client-core's streaming API
2. **No Direct Codec Usage**: Let media-core handle encoding/decoding internally
3. **Simple Frame Conversion**: Convert between audio-core and session-core AudioFrame types
4. **Lifecycle Management**: Tie audio pipeline lifecycle to call state

## Development Phases

### Phase 1: Foundation (Week 1) ✅ COMPLETED
- [x] **Project Setup**
  - [x] Create Cargo.toml with dependencies on client-core, audio-core, codec-core
  - [x] Set up module structure
  - [x] Configure feature flags
  - [x] Create error types and result handling

- [x] **Core Types**
  - [x] Define SipClient struct with internal state management
  - [x] Create configuration types (SipClientConfig, AudioConfig, CodecConfig)
  - [x] Design event aggregation system
  - [x] Implement builder pattern foundation

- [x] **Basic Integration**
  - [x] Wire up client-core for SIP operations
  - [x] Connect audio-core for device management
  - [x] Integrate codec-core for encoding/decoding
  - [x] Create internal message passing system

### Phase 2: Simple API (Week 2) 🚧 IN PROGRESS
- [x] **Client Lifecycle**
  - [x] Implement `SipClient::new()` with defaults
  - [ ] Add `start()` and `stop()` methods
  - [ ] Handle resource cleanup and error recovery
  - [x] Create connection state management

- [x] **Basic Call Operations**
  - [x] Implement `make_call(uri)` with automatic setup
  - [x] Add `answer_call()` and `reject_call()`
  - [x] Create `hangup()` with proper cleanup
  - [x] Handle call state transitions

- [ ] **Audio Integration**
  - [ ] Automatic device selection
  - [ ] Default audio pipeline setup
  - [ ] Built-in echo cancellation
  - [x] Volume control and mute operations

### Phase 3: Advanced API (Week 3)
- [ ] **Custom Audio Pipelines**
  - [ ] Expose `AudioPipelineBuilder` integration
  - [ ] Allow custom audio processing chains
  - [ ] Support external audio sources/sinks
  - [ ] Frame-level audio access API

- [ ] **Media Preferences**
  - [ ] Configure preferred codecs for client-core
  - [ ] Set custom SDP attributes
  - [ ] Configure jitter buffer settings
  - [ ] Note: Actual codec selection happens in media-core

- [ ] **Advanced Call Control**
  - [ ] Call transfer implementation
  - [ ] Hold/resume with music on hold
  - [ ] DTMF generation and detection
  - [ ] Conference call support

### Phase 4: Production Features (Week 4)
- [ ] **Error Handling**
  - [ ] Comprehensive error recovery
  - [ ] Automatic reconnection logic
  - [ ] Graceful degradation
  - [ ] Detailed error reporting

- [ ] **Performance Optimization**
  - [ ] Zero-copy audio paths
  - [ ] Lazy initialization
  - [ ] Resource pooling
  - [ ] Benchmark suite

- [ ] **Monitoring & Metrics**
  - [ ] Call quality metrics (MOS, jitter, packet loss)
  - [ ] Audio level monitoring
  - [ ] Network statistics
  - [ ] Debug logging integration

### Phase 5: Documentation & Examples (Week 5)
- [ ] **Documentation**
  - [ ] API reference documentation
  - [ ] Architecture guide
  - [ ] Migration guide from individual crates
  - [ ] Troubleshooting guide

- [ ] **Examples**
  - [ ] Simple softphone example
  - [ ] Call center agent example
  - [ ] WebRTC gateway example
  - [ ] Custom audio processor example

- [ ] **Testing**
  - [ ] Unit tests for all components
  - [ ] Integration tests with mock servers
  - [ ] Performance benchmarks
  - [ ] Stress testing suite

## API Design

### Simple API Example
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
    
    // Talk for 30 seconds
    tokio::time::sleep(Duration::from_secs(30)).await;
    
    // Hangup
    call.hangup().await?;
    
    Ok(())
}
```

### Advanced API Example
```rust
use sip_client::{SipClientBuilder, AudioPipelineConfig, CodecPriority};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Advanced configuration
    let client = SipClientBuilder::new()
        .sip_identity("sip:alice@example.com")
        .sip_server("sip.example.com:5060")
        .audio_pipeline(
            AudioPipelineConfig::custom()
                .input_device("Microphone (USB)")
                .output_device("Headphones")
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
    
    // Access to raw audio frames
    let call = client.call("sip:bob@example.com").await?;
    let mut audio_stream = call.audio_stream().await?;
    
    while let Some(frame) = audio_stream.next().await {
        // Process audio frame
        let processed = custom_audio_processing(frame);
        audio_stream.send(processed).await?;
    }
    
    Ok(())
}
```

## Integration Details

### Audio Flow
```
Microphone → audio-core (capture) → PCM frames → client-core (streaming API) → media-core (encode) → RTP → Network
Network → RTP → media-core (decode) → PCM frames → client-core (streaming API) → audio-core (playback) → Speaker
```

### Architecture Notes
1. **PCM Throughout**: Audio flows as raw PCM frames through the entire application layer
2. **Encoding at RTP Layer**: media-core (managed by session-core) handles codec operations
3. **Clean Separation**: audio-core only handles device I/O, never touches encoded data
4. **Streaming API**: client-core provides `send_audio_frame()` and `subscribe_to_audio_frames()` for PCM data
5. **Future Enhancement**: media-core will be updated to use codec-core as its codec provider

### Event Aggregation
```rust
enum SipClientEvent {
    // From client-core
    IncomingCall { from: String, call_id: CallId },
    CallStateChanged { call_id: CallId, state: CallState },
    MediaStatisticsUpdate { call_id: CallId, stats: MediaStatistics },
    
    // From audio-core
    AudioDeviceChanged { device: AudioDevice },
    AudioLevelChanged { level: f32 },
    AudioPipelineError { error: String },
    
    // Aggregated events
    CallQualityReport { call_id: CallId, mos: f32, jitter: f32 },
    AudioStreamStarted { call_id: CallId },
    AudioStreamStopped { call_id: CallId },
}
```

## Testing Strategy

### Unit Tests
- Mock each underlying crate
- Test configuration validation
- Verify event aggregation logic
- Check error handling paths

### Integration Tests
- Use test fixtures from underlying crates
- End-to-end call flow testing
- Audio pipeline verification
- Codec negotiation scenarios

### Performance Tests
- Measure call setup time
- Audio latency benchmarks
- Memory usage profiling
- Concurrent call stress testing

## Migration Path

For users currently using individual crates:

### From client-core only:
```rust
// Before
let client = ClientBuilder::new()
    .local_address("127.0.0.1:5060".parse()?)
    .build()
    .await?;

// After
let client = SipClient::new("sip:user@example.com").await?;
```

### From client-core + audio-core:
```rust
// Before
let audio_manager = AudioDeviceManager::new().await?;
let client = ClientBuilder::new()...;
// Manual integration code

// After
let client = SipClientBuilder::new()
    .sip_identity("sip:user@example.com")
    .audio_defaults()
    .build()
    .await?;
```

## Success Criteria

### Functionality
- [ ] All basic call operations working
- [ ] Audio flows correctly in both directions
- [ ] Codec negotiation succeeds with common peers
- [ ] Events properly aggregated and delivered

### Performance
- [ ] Call setup < 1 second
- [ ] Audio latency < 150ms
- [ ] Memory usage < 50MB per call
- [ ] Supports 10+ concurrent calls

### Developer Experience
- [ ] Simple API requires < 10 lines for basic call
- [ ] Clear error messages with actionable fixes
- [ ] Comprehensive examples for common use cases
- [ ] Migration from individual crates is straightforward

## Future Enhancements

### Version 2.0
- Video call support
- Screen sharing
- Call recording
- WebRTC gateway mode

### Version 3.0
- Multi-party conferencing
- Call center features (queue, IVR)
- Voicemail integration
- Analytics dashboard

## Dependencies

```toml
[dependencies]
rvoip-client-core = { path = "../client-core" }
rvoip-audio-core = { path = "../audio-core" }
# Note: codec-core is NOT a direct dependency
# It will be used by media-core (inside session-core/client-core)

# Async runtime
tokio = { version = "1.0", features = ["full"] }
async-trait = "0.1"

# Error handling
thiserror = "1.0"
anyhow = "1.0"

# Logging
tracing = "0.1"

# Events
tokio-stream = "0.1"
futures = "0.3"
```

## Key Architectural Decisions

1. **Maintain Existing Architecture**: sip-client acts as a coordination layer, not a reimplementation
2. **PCM Frame Flow**: All audio flows as PCM frames between components, with encoding/decoding happening in media-core
3. **No Direct Codec Usage**: sip-client does not use codec-core directly; this is handled by media-core
4. **Focus on Integration**: Primary responsibility is connecting audio devices to SIP/RTP streams

This development plan provides a clear roadmap for creating a unified SIP client library that makes VoIP development in Rust accessible while respecting the established architecture of the RVOIP project.