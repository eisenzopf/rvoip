# Media Core Implementation Plan

This document outlines the implementation plan for the `media-core` crate, which serves as the Media Engine layer in the rvoip architecture. It handles audio/video processing, codec management, media session coordination, and interfaces with both `session-core` (signaling) and `rtp-core` (transport).

## Directory Structure

```
media-core/
├── src/
│   ├── lib.rs               # Main library exports and documentation
│   ├── error.rs             # Error types and handling
│   ├── session/             # Media session management
│   │   ├── mod.rs           # Session module exports
│   │   ├── media_session.rs # Core media session implementation
│   │   ├── config.rs        # Session configuration
│   │   ├── events.rs        # Media session events
│   │   └── flow.rs          # Media flow control (start/stop/pause)
│   ├── codec/               # Codec framework
│   │   ├── mod.rs           # Codec module exports
│   │   ├── registry.rs      # Codec registry and factory
│   │   ├── traits.rs        # Codec interface definitions
│   │   ├── audio/           # Audio codec implementations
│   │   │   ├── mod.rs       # Audio codec exports
│   │   │   ├── opus.rs      # Opus codec implementation
│   │   │   ├── g711.rs      # G.711 µ-law and A-law implementation
│   │   │   ├── g722.rs      # G.722 wideband implementation
│   │   │   └── ilbc.rs      # iLBC narrowband implementation
│   │   └── video/           # Video codec implementations (future)
│   │       ├── mod.rs       # Video codec exports
│   │       ├── h264.rs      # H.264/AVC implementation
│   │       └── vp8.rs       # VP8 implementation
│   ├── engine/              # Media processing engines
│   │   ├── mod.rs           # Engine module exports
│   │   ├── audio/           # Audio engine
│   │   │   ├── mod.rs       # Audio engine exports
│   │   │   ├── device.rs    # Audio device abstraction
│   │   │   ├── capture.rs   # Audio capture pipeline
│   │   │   ├── playback.rs  # Audio playback pipeline
│   │   │   └── mixer.rs     # Audio mixing capabilities
│   │   └── video/           # Video engine (future)
│   │       ├── mod.rs       # Video engine exports
│   │       ├── device.rs    # Video device abstraction
│   │       ├── capture.rs   # Video capture
│   │       └── render.rs    # Video rendering
│   ├── processing/          # Media signal processing
│   │   ├── mod.rs           # Processing module exports
│   │   ├── audio/           # Audio processing
│   │   │   ├── mod.rs       # Audio processing exports
│   │   │   ├── aec.rs       # Acoustic echo cancellation
│   │   │   ├── agc.rs       # Automatic gain control
│   │   │   ├── vad.rs       # Voice activity detection
│   │   │   ├── ns.rs        # Noise suppression
│   │   │   ├── plc.rs       # Packet loss concealment
│   │   │   └── dtmf.rs      # DTMF generation and detection
│   │   └── format/          # Format conversion
│   │       ├── mod.rs       # Format conversion exports
│   │       ├── resampler.rs # Sample rate conversion
│   │       └── channels.rs  # Channel conversion (mono/stereo)
│   ├── buffer/              # Media buffer management
│   │   ├── mod.rs           # Buffer module exports
│   │   ├── jitter.rs        # Jitter buffer implementation
│   │   └── adaptive.rs      # Adaptive buffer sizing
│   ├── quality/             # Media quality monitoring and adaptation
│   │   ├── mod.rs           # Quality module exports
│   │   ├── metrics.rs       # Quality metrics collection
│   │   ├── estimation.rs    # MOS and quality estimation
│   │   └── adaptation.rs    # Quality-based adaptation
│   ├── rtp/                 # RTP integration
│   │   ├── mod.rs           # RTP module exports
│   │   ├── packetizer.rs    # RTP packetization
│   │   ├── depacketizer.rs  # RTP depacketization
│   │   └── session.rs       # RTP session management
│   ├── security/            # Media security
│   │   ├── mod.rs           # Security module exports
│   │   ├── srtp.rs          # SRTP integration
│   │   └── dtls.rs          # DTLS key exchange coordination
│   ├── sync/                # Media synchronization
│   │   ├── mod.rs           # Sync module exports
│   │   ├── clock.rs         # Media clock implementation
│   │   └── lipsync.rs       # A/V synchronization
│   └── integration/         # Integration with other components
│       ├── mod.rs           # Integration module exports
│       ├── session_core.rs  # Session-core integration
│       ├── rtp_core.rs      # RTP-core integration
│       └── sdp.rs           # SDP handling for media negotiation
├── examples/                # Example implementations
├── tests/                   # Integration tests
└── benches/                 # Performance benchmarks
```

## Implementation Phases

### Phase 1: Codec Framework and Core Infrastructure (2-3 weeks)

#### 1.1 Project Setup and Core Architecture
- [ ] Create initial crate structure and dependencies
- [ ] Establish error handling types and patterns
- [ ] Define core traits and interfaces
- [ ] Setup test framework and CI integration

#### 1.2 Codec Framework
- [ ] Implement `Codec` trait with core functionality
- [ ] Create `AudioCodec` and `VideoCodec` specializations
- [ ] Implement codec registry and factory pattern
- [ ] Create capability description system for codec negotiation
- [ ] Implement codec parameter handling
- [ ] Create payload type management system
- [ ] Implement dynamic payload type assignment
- [ ] Add benchmarking for codec performance

#### 1.3 Audio Codec Implementation
- [ ] Implement Opus codec (high-quality default codec)
  - [ ] Variable bitrate support
  - [ ] Forward error correction
  - [ ] Voice/music mode switching
  - [ ] Bandwidth control
- [ ] Implement G.711 (PCM µ-law, A-law)
  - [ ] Basic encoding/decoding
  - [ ] PLC integration
- [ ] Implement G.722 wideband codec
  - [ ] 16kHz sampling support
  - [ ] Bitrate modes
- [ ] Implement iLBC narrowband codec
  - [ ] 20ms/30ms frame modes
  - [ ] Enhanced PLC

#### 1.4 Format Conversion
- [ ] Create audio format type definitions
- [ ] Implement sample format converters (S16, F32, etc.)
- [ ] Add resampling for different sample rates
- [ ] Implement channel conversion (mono/stereo)
- [ ] Create optimized audio frame allocation system
- [ ] Implement audio buffer chain for processing
- [ ] Add benchmarks for conversion operations

### Phase 2: RTP Integration and Buffer Management (2 weeks)

#### 2.1 RTP Adaptation Layer
- [ ] Create MediaTransport implementation for RTP
- [ ] Implement codec-to-RTP packetization
- [ ] Add RTP-to-codec depacketization
- [ ] Create payload format handlers for codecs
- [ ] Implement SSRC management and tracking
- [ ] Add RTCP report processing
- [ ] Create RTP session binding for media sessions
- [ ] Implement RTCP feedback mechanisms

#### 2.2 Media Buffer Management
- [ ] Implement jitter buffer for audio streams
- [ ] Create adaptive buffer sizing logic
- [ ] Add late packet handling policies
- [ ] Implement packet prioritization for mixed media
- [ ] Create comprehensive buffer statistics
- [ ] Add configurable buffer strategies (fixed vs. adaptive)
- [ ] Implement diagnostic tools for buffer analysis
- [ ] Create stream synchronization for multiple sources

#### 2.3 Security Integration
- [ ] Implement SRTP context management
- [ ] Create DTLS-SRTP integration
- [ ] Add key material handling from DTLS
- [ ] Implement secure policy enforcement

### Phase 3: Media Session Management (2 weeks)

#### 3.1 Media Session Framework
- [ ] Create `MediaSession` abstraction
- [ ] Implement session state machine
- [ ] Add media attribute negotiation
- [ ] Create session modification support
- [ ] Implement session events system
- [ ] Add multi-stream session support
- [ ] Create session configuration API

#### 3.2 Media Flow Control
- [ ] Implement media start/stop controls
- [ ] Create pause/resume functionality
- [ ] Add hold/unhold support with session-core integration
- [ ] Implement mute/unmute controls
- [ ] Create media direction control (sendrecv, sendonly, recvonly)
- [ ] Add media forking for recording or processing
- [ ] Implement DTMF sending via RFC 4733

#### 3.3 SDP Integration
- [ ] Create SDP media capability extraction
- [ ] Implement codec parameter parsing from SDP
- [ ] Add SDP media line generation
- [ ] Create ICE candidate handling
- [ ] Implement DTLS fingerprint management
- [ ] Add RTCP-FB parameter negotiation
- [ ] Create extmap attribute handling for RTP extensions

### Phase 4: Audio Processing and Quality (2-3 weeks)

#### 4.1 Audio Engine
- [ ] Create audio device abstraction
- [ ] Implement audio capture pipeline
- [ ] Add audio playback pipeline
- [ ] Create audio mixing capabilities
- [ ] Implement level monitoring and metering
- [ ] Add configurable audio buffering
- [ ] Create audio device hotplug support
- [ ] Implement device enumeration across platforms

#### 4.2 Audio Processing
- [ ] Implement acoustic echo cancellation (AEC)
- [ ] Create noise suppression algorithms
- [ ] Add automatic gain control (AGC)
- [ ] Implement voice activity detection (VAD)
- [ ] Create comfort noise generation (CNG)
- [ ] Add packet loss concealment (PLC)
- [ ] Implement audio processing pipeline
- [ ] Create audio effects framework (optional)

#### 4.3 Media Quality Management
- [ ] Create comprehensive audio quality metrics
- [ ] Implement MOS estimation algorithm
- [ ] Add audio clipping and distortion detection
- [ ] Create network quality indicators
- [ ] Implement quality event notifications
- [ ] Add quality adaptation framework
- [ ] Create quality monitoring dashboard (debug)
- [ ] Implement periodic quality reporting

### Phase 5: Network Adaptation and Advanced Features (2 weeks)

#### 5.1 Network Adaptation
- [ ] Create bandwidth estimation integration
- [ ] Implement codec bitrate adaptation
- [ ] Add packet loss resilience mechanisms
- [ ] Create FEC integration for critical streams
- [ ] Implement redundant transmission (RED)
- [ ] Add congestion control response
- [ ] Create network quality event system
- [ ] Implement adaptive encoding parameters

#### 5.2 Media Synchronization
- [ ] Implement audio/video sync framework
- [ ] Create timestamp synchronization utilities
- [ ] Add NTP to media clock conversion
- [ ] Implement lip sync buffer
- [ ] Create multi-stream synchronization
- [ ] Add drift detection and correction
- [ ] Implement reference clock selection and distribution

#### 5.3 Advanced Features
- [ ] Implement call recording capabilities
- [ ] Create conference mixing support
- [ ] Add media relay functionality
- [ ] Implement advanced DTMF handling (events + audio)
- [ ] Create diagnostics and monitoring API
- [ ] Add WebRTC compatibility layer (future)
- [ ] Implement voice activity based optimizations

### Phase 6: Integration with Session Core (1-2 weeks)

#### 6.1 Session Core Integration
- [ ] Create `MediaManager` as main integration point
- [ ] Implement mapping between SIP dialogs and media sessions
- [ ] Add media event propagation to signaling layer
- [ ] Create dialog-to-media session binding
- [ ] Implement early media handling
- [ ] Add reinvite media renegotiation support
- [ ] Create session refresh handling

#### 6.2 Production Hardening
- [ ] Implement proper resource cleanup
- [ ] Add graceful error handling throughout stack
- [ ] Create recovery mechanisms for failed sessions
- [ ] Implement dead session detection
- [ ] Add comprehensive metrics and monitoring
- [ ] Create production logging strategy
- [ ] Implement security audit and hardening

### Phase 7: Testing and Validation (Ongoing)

#### 7.1 Unit Testing
- [ ] Create comprehensive codec tests
- [ ] Implement audio processing tests
- [ ] Add media session tests
- [ ] Create integration tests with RTP
- [ ] Implement performance benchmarks
- [ ] Add stress testing suite
- [ ] Create compatibility validation

#### 7.2 Integration Testing
- [ ] Test with session-core integration
- [ ] Create RTP-core integration tests
- [ ] Add end-to-end media flow tests
- [ ] Implement security integration tests
- [ ] Create load testing framework
- [ ] Add regression test suite

#### 7.3 Interoperability Testing
- [ ] Test with common SIP clients
- [ ] Implement WebRTC compatibility tests
- [ ] Add PBX system interoperability
- [ ] Create SIP trunk compatibility tests
- [ ] Implement PSTN gateway testing
- [ ] Add automation for regression testing

## Next Immediate Steps

1. Create initial directory structure
2. Implement base codec framework (traits and interfaces)
3. Build first codec implementation (G.711)
4. Create RTP integration for basic transport
5. Implement simple MediaSession abstraction

## Future Considerations

- **Video Support**: While the initial focus is on audio, the architecture should be designed to accommodate video in the future
- **WebRTC Gateway**: Future extension for WebRTC compatibility
- **Hardware Acceleration**: Consider interfaces for hardware-accelerated media processing
- **Machine Learning**: Possible integration of ML-based audio enhancements
- **Cloud Deployment**: Considerations for containerized deployment in cloud environments 