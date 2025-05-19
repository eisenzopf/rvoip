# Media Core Implementation Plan

This document outlines the implementation plan for the `media-core` crate, which serves as the Media Engine layer in the rvoip architecture. It handles audio/video processing, codec management, media session coordination, and interfaces with both `session-core` (signaling) and `rtp-core` (transport).

## Layer Responsibility Clarification

The `media-core` crate focuses on media processing and session management, delegating transport and packet-level concerns to `rtp-core`. Here are the clear responsibilities:

1. **Media Session Management**
   - SDP media negotiation
   - Codec selection and configuration
   - Media state management (start/stop/pause)
   - Session events and signaling integration

2. **Codec Framework**
   - Codec registration and discovery
   - Audio/video encoding/decoding
   - Format conversion and transcoding
   - Bitrate and quality adaptation

3. **Media Processing**
   - Audio/video signal processing (AEC, AGC, NS)
   - Voice activity detection
   - Media mixing for conferencing
   - Audio/video effects and filters

4. **Device Management**
   - Audio/video device enumeration
   - Capture and playback setup
   - Device hotplug handling
   - Platform-specific device APIs

5. **Quality Management**
   - High-level quality measurement
   - User experience metrics
   - Adaptation strategy selection
   - Session quality reporting

## Standardized Event Bus Implementation

Integrate with the infra-common high-performance event bus using a specialized approach for media processing:

### Media Event Categorization

1. **Static Event Implementation (High-Throughput Protocol Events)**
   - [ ] Implement `StaticEvent` trait for all media control protocol messages
   - [ ] Create specialized event types for high-frequency media commands
   - [ ] Implement `MediaFrameEvent` with StaticEvent optimizations
   - [ ] Optimize codec control messages with StaticEvent fast path

2. **Priority-Based Processing**
   - [ ] Use `EventPriority::High` for media session state changes
     - [ ] Media start/stop events
     - [ ] Codec parameter changes
     - [ ] Format switches
   - [ ] Use `EventPriority::Normal` for regular media processing events
     - [ ] Audio level changes
     - [ ] Processing state updates
   - [ ] Use `EventPriority::Low` for metrics and statistics
     - [ ] Quality metrics
     - [ ] Device statistics
     - [ ] Resource usage reporting

3. **Batch Processing for Media Frames**
   - [ ] Implement batch processing for audio frame events
   - [ ] Create optimized batch handling for video frames
   - [ ] Add metrics collection batching for performance analysis
   - [ ] Implement batch handlers for device events

### Integration Implementation

1. **Publishers & Subscribers**
   - [ ] Create `MediaEventPublisher` for encapsulating media event publishing
   - [ ] Implement specialized `CodecEventPublisher` for codec-related events
   - [ ] Add `DeviceEventPublisher` for device management events
   - [ ] Create typed subscribers for different media event categories

2. **Event Bus Configuration**
   - [ ] Configure event bus with optimal settings for media processing:
     ```rust
     EventBusConfig {
         max_concurrent_dispatches: 10000,
         broadcast_capacity: 16384,
         enable_priority: true,
         enable_zero_copy: true,
         batch_size: 100, // Optimal for audio/video frame batching
         shard_count: 32,
     }
     ```
   - [ ] Tune channel capacities for audio/video frame scenarios
   - [ ] Implement monitoring for event bus performance in media context

3. **Media-Specific Optimizations**
   - [ ] Add memory pooling for media frame events
   - [ ] Implement zero-copy frame passing between processing stages
   - [ ] Create specialized event types for different media formats
   - [ ] Use Arc wrapping for media buffers to eliminate copying

## Integration with RTP-Core

`media-core` should delegate these responsibilities to `rtp-core`:
- Low-level packet processing
- Transport socket management
- DTLS/SRTP implementation
- Jitter buffer management at packet level
- Network statistics collection

## Directory Structure

```
media-core/
├── src/
│   ├── lib.rs               # Main library exports and documentation
│   ├── error.rs             # Error handling types
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
│   │   └── video/           # Video codec implementations 
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
│   │   └── video/           # Video engine
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
│   ├── quality/             # Media quality monitoring and adaptation
│   │   ├── mod.rs           # Quality module exports
│   │   ├── metrics.rs       # Quality metrics collection
│   │   ├── estimation.rs    # MOS and quality estimation
│   │   └── adaptation.rs    # Quality-based adaptation
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

## RTP-Core Integration Plan

To address the duplication and properly integrate with `rtp-core`'s new API, we'll:

1. **Remove Duplicate Functionality**
   - [x] Replace DTLS/SRTP implementation with rtp-core integration
   - [ ] Remove packet-level jitter buffer implementation
   - [ ] Remove low-level RTP packet handling
   - [ ] Replace network transport code with rtp-core API calls

2. **Create Adapter Layer**
   - [ ] Implement `MediaTransportAdapter` for rtp-core integration
     - [ ] Create bi-directional mapping between media frames and RTP packets
     - [ ] Implement codec payload format selection
     - [ ] Add timestamp management for media synchronization
     - [ ] Create SSRC mapping and tracking for streams
   - [ ] Design clean media frame abstractions
     - [ ] Audio frame structure with format metadata
     - [ ] Video frame structure with timing information
     - [ ] Frame sequence tracking independent of RTP
   - [ ] Add configuration adapters for security and buffer settings
     - [ ] Map media-core security requirements to rtp-core SecurityConfig
     - [ ] Create buffer configuration based on media requirements
   - [ ] Implement event handlers for network quality changes
     - [ ] React to bandwidth estimation updates
     - [ ] Handle transport state changes (connected, disconnected)
     - [ ] Process quality metrics for adaptation

3. **SDP Integration**
   - [ ] Create unified SDP handling for media negotiation
     - [ ] Extract codec capabilities from rtp-core's payload formats
     - [ ] Map DTLS fingerprints from rtp-core to SDP
     - [ ] Handle ICE candidates via ICE integration
   - [ ] Implement codec parameter extraction and configuration
     - [ ] Map SDP parameters to codec configurations
     - [ ] Generate SDP that reflects rtp-core capabilities
     - [ ] Handle format-specific SDP attributes

4. **Quality Monitoring**
   - [ ] Use rtp-core's statistics API for network metrics
     - [ ] Subscribe to network quality events
     - [ ] Map network statistics to media quality impact
   - [ ] Focus on media-specific quality measurements in media-core
     - [ ] Audio signal quality metrics
     - [ ] Video quality assessment
   - [ ] Create unified quality model combining network and media metrics
     - [ ] Develop scoring system that incorporates both layers
     - [ ] Generate user experience metrics from combined data

## Session-Core Integration Plan

To maintain proper separation of concerns between media-core (media processing) and session-core (signaling/session management), we need to:

1. **Delegate SDP Handling to Session-Core**
   - [ ] Remove direct SDP parsing/generation from media-core
   - [ ] Create codec capability descriptors for session-core to use in SDP
   - [ ] Implement parameter extraction interfaces for session-core to pass negotiated parameters
   - [ ] Design media configuration interfaces for session-core to control media setup

2. **Establish Clean Interfaces for Session Control**
   - [ ] Create `MediaSessionController` as the primary integration point
     - [ ] Implement start/stop/pause methods for session-core to invoke
     - [ ] Add hold/resume functionality that session-core can control
     - [ ] Create media state notifications back to session-core
     - [ ] Implement clean configuration update methods
   - [ ] Design capability discovery API
     - [ ] Provide codec capabilities (audio/video formats, parameters)
     - [ ] Export security capabilities (SRTP profiles, encryption modes)
     - [ ] Report device capabilities (audio/video devices, supported features)
   - [ ] Create media address management interface
     - [ ] Accept remote transport addresses from session-core
     - [ ] Provide local transport addresses to session-core
     - [ ] Handle transport creation based on negotiated parameters

3. **Design Event Propagation System**
   - [ ] Create media event notification channel to session-core
     - [ ] Media status events (started, stopped, failed)
     - [ ] Quality alert events (poor audio, video degradation)
     - [ ] DTMF and other media-specific events
   - [ ] Implement session event listeners from session-core
     - [ ] Session state changes (early, confirmed, terminated)
     - [ ] SDP renegotiation notifications
     - [ ] Transport changes (ICE candidates, address updates)

4. **Implement Configuration Interface**
   - [ ] Create `MediaSessionConfig` for session-core to provide
     - [ ] Codec parameters extracted from SDP
     - [ ] Transport parameters (addresses, ports, protocols)
     - [ ] Security parameters (keys, fingerprints, profiles)
     - [ ] Session information (call-id, participants)
   - [ ] Design builder pattern for configuration
     - [ ] Make it easy for session-core to create valid configs
     - [ ] Create validation logic to catch misconfigurations
     - [ ] Add helper methods for common session patterns

## Implementation Changes Based on Integration

The integration with session-core affects our media-core implementation in these ways:

1. **Focus Media-Core on Media Processing**
   - [x] Remove SDP handling (delegate to session-core)
   - [x] Remove signaling state management (delegate to session-core)
   - [ ] Create API abstractions that don't depend on SIP/SDP concepts

2. **Revise Media Session Management**
   - [ ] Make media sessions responsive to external control
   - [ ] Remove direct SIP dialog dependencies
   - [ ] Create state machines that session-core can drive
   - [ ] Add flexible media parameter update systems
   - [ ] Design clean restart/reconfiguration capabilities

3. **Enhance Codec Framework for Integration**
   - [ ] Add capability description mechanism for SDP generation
   - [ ] Create parameter extraction for SDP negotiation results
   - [ ] Implement dynamic codec configuration from negotiated parameters
   - [ ] Add format mapping to standard SDP payload types

4. **Create Session-Core Integration Tests**
   - [ ] Test media session control from session-core
   - [ ] Verify codec negotiation works through the integration layer
   - [ ] Ensure events propagate correctly between layers
   - [ ] Validate clean separation and proper delegation

## Implementation Phases (Updated)

### Phase 1: Cleanup and Core Architecture (2 weeks)

#### 1.1 Project Structure and Dependency Audit
- [ ] Remove duplicate functionality currently in rtp-core
- [ ] Reorganize security integration to use rtp-core exclusively
- [ ] Create proper dependency management with rtp-core
- [ ] Document integration points and responsibilities

#### 1.2 Codec Framework
- [ ] Implement `Codec` trait with core functionality
- [ ] Create `AudioCodec` and `VideoCodec` specializations
- [ ] Implement codec registry and factory pattern
- [ ] Create capability description system for codec negotiation

#### 1.3 RTP-Core Integration Layer
- [ ] Create adapters for rtp-core's new API
  - [ ] Implement MediaTransportAdapter with frame conversion
  - [ ] Create security context integration
  - [ ] Develop buffer configuration mapping
  - [ ] Build statistics and monitoring integration
- [ ] Implement high-level media frame processing
  - [ ] Design media frame structure compatible with rtp-core
  - [ ] Create frame pool for efficient memory management
  - [ ] Implement frame metadata and timing information
- [ ] Add configuration mapping to rtp-core settings
  - [ ] Map codec parameters to payload format settings
  - [ ] Create transport configuration builder
  - [ ] Design security profile mapping
- [ ] Create event handling for rtp-core notifications
  - [ ] Implement bandwidth adaptation callbacks
  - [ ] Add connectivity state change handling
  - [ ] Process quality alert notifications

### Phase 2: Audio Codec Implementation (2-3 weeks)

#### 2.1 Opus Codec
- [ ] Implement Opus codec (high-quality default codec)
  - [ ] Variable bitrate support
  - [ ] Forward error correction
  - [ ] Voice/music mode switching
  - [ ] Bandwidth control
- [ ] Add Opus-RTP integration through rtp-core

#### 2.2 Other Audio Codecs
- [ ] Implement G.711 (PCM µ-law, A-law)
  - [ ] Basic encoding/decoding
  - [ ] PLC integration
- [ ] Implement G.722 wideband codec
  - [ ] 16kHz sampling support
  - [ ] Bitrate modes
- [ ] Implement iLBC narrowband codec
  - [ ] 20ms/30ms frame modes
  - [ ] Enhanced PLC

#### 2.3 Format Conversion
- [ ] Create audio format type definitions
- [ ] Implement sample format converters (S16, F32, etc.)
- [ ] Add resampling for different sample rates
- [ ] Implement channel conversion (mono/stereo)

### Phase 3: Media Session Management (3 weeks)

#### 3.1 Media Session Framework
- [ ] Create `MediaSession` abstraction
- [ ] Implement session state machine
- [ ] Add media attribute negotiation
- [ ] Create session configuration API

#### 3.2 Media Flow Control
- [ ] Implement media start/stop controls
- [ ] Create pause/resume functionality
- [ ] Add hold/unhold support with session-core integration
- [ ] Implement mute/unmute controls

#### 3.3 SDP Integration
- [ ] Create SDP media capability extraction
- [ ] Implement codec parameter parsing from SDP
- [ ] Add SDP media line generation
- [ ] Create ICE candidate handling with rtp-core integration

### Phase 4: Audio Processing (2 weeks)

#### 4.1 Audio Engine
- [ ] Create audio device abstraction
- [ ] Implement audio capture pipeline
- [ ] Add audio playback pipeline
- [ ] Create audio mixing capabilities

#### 4.2 Audio Processing
- [ ] Implement acoustic echo cancellation (AEC)
- [ ] Create noise suppression algorithms
- [ ] Add automatic gain control (AGC)
- [ ] Implement voice activity detection (VAD)

### Phase 5: Video Support and Integration (3 weeks)

#### 5.1 Video Codec Implementation
- [ ] Implement H.264/AVC video codec integration
- [ ] Add VP8 video codec support
- [ ] Create video frame handling

#### 5.2 Video Engine
- [ ] Implement video device management
- [ ] Create video capture pipeline
- [ ] Add video rendering
- [ ] Implement resolution adaptation

#### 5.3 Media Synchronization
- [ ] Implement audio/video sync framework
- [ ] Create timestamp synchronization utilities
- [ ] Add lip sync buffer with rtp-core integration
- [ ] Implement drift detection and correction

#### 5.4 Media Quality Management
- [ ] Create comprehensive audio quality metrics
- [ ] Implement MOS estimation for perceived quality (not network MOS)
- [ ] Add audio clipping and distortion detection
- [ ] Create media quality indicators distinct from network quality
- [ ] Implement quality event notifications
- [ ] Add media adaptation framework
  - [ ] Integrate with rtp-core bandwidth estimation
  - [ ] Implement codec bitrate adaptation based on network feedback
  - [ ] Create resolution switching for video
  - [ ] Add frame rate adaptation
- [ ] Create quality monitoring dashboard (debug)

### Phase 6: Advanced Media Features (2-3 weeks)

#### 6.1 Media Adaptation and Resilience
- [ ] Implement dynamic codec parameter adaptation
- [ ] Create automatic quality preset selection
- [ ] Add integration with rtp-core's FEC when available
- [ ] Implement integration with rtp-core's RED when available
- [ ] Create codec-specific loss concealment strategies
- [ ] Add voice activity based optimizations

#### 6.2 Media Recording and Processing
- [ ] Implement call recording capabilities
- [ ] Create conference mixing support
- [ ] Add transcoding support
- [ ] Implement advanced DTMF handling (events + audio)
- [ ] Create diagnostics and monitoring API

## Next Immediate Steps

1. **Architecture Cleanup**
   - [ ] Audit all modules for duplication with rtp-core
   - [ ] Remove security implementation in favor of rtp-core
   - [ ] Remove buffer implementation in favor of rtp-core
   - [ ] Create clear integration points with rtp-core API

2. **API Integration**
   - [ ] Implement adapters for the new rtp-core API
   - [ ] Create proper event handling from rtp-core
   - [ ] Add quality metric integration
   - [ ] Update security handling to use rtp-core API

3. **Media Processing Focus**
   - [ ] Improve codec implementations
   - [ ] Enhance audio/video processing capabilities
   - [ ] Create better device management abstraction
   - [ ] Focus on media-specific quality adaptations

4. **Documentation and Testing**
   - [ ] Document clear layer responsibilities
   - [ ] Create examples of proper rtp-core integration
   - [ ] Add comprehensive tests for codec functionality
   - [ ] Test end-to-end media flow with rtp-core

## Future Considerations

- **Video Support**: While the initial focus is on audio, the architecture should be designed to accommodate video in the future
- **WebRTC Gateway**: Future extension for WebRTC compatibility
- **Hardware Acceleration**: Consider interfaces for hardware-accelerated media processing
- **Machine Learning**: Possible integration of ML-based audio enhancements
- **Cloud Deployment**: Considerations for containerized deployment in cloud environments

## Component Lifecycle Management

- [ ] Implement comprehensive lifecycle management
  - [ ] Create initialization sequence with dependencies
    - [ ] Add device discovery and initialization
    - [ ] Implement codec registration and validation
    - [ ] Create session state initialization
  - [ ] Add graceful shutdown handling
    - [ ] Implement clean media session termination
    - [ ] Create resource release sequence
    - [ ] Add completion of in-progress operations
  - [ ] Create status reporting for lifecycle stages
    - [ ] Implement initialization progress tracking
    - [ ] Add component health reporting
    - [ ] Create dependency status monitoring
  - [ ] Add recovery mechanisms
    - [ ] Implement device failure recovery
    - [ ] Add codec fallback mechanisms
    - [ ] Create session recovery procedures

## Cross-Component Configuration

- [ ] Create unified configuration system
  - [ ] Implement configuration validation against requirements
    - [ ] Add codec capability validation
    - [ ] Create device capability checking
    - [ ] Implement network requirement validation
  - [ ] Add dependency declaration for configurations
    - [ ] Create explicit dependency specification
    - [ ] Implement version compatibility checking
    - [ ] Add feature requirement declaration
  - [ ] Create configuration change management
    - [ ] Implement safe configuration updates
    - [ ] Add configuration versioning
    - [ ] Create change notification system

## Standardized Event System

- [ ] Design standardized event architecture
  - [ ] Create event type hierarchy
    - [ ] Define media events (started, stopped, failed)
    - [ ] Add quality events (degraded, improved)
    - [ ] Create device events (added, removed, failed)
  - [ ] Implement event propagation system
    - [ ] Add event priority handling
    - [ ] Create event filtering mechanisms
    - [ ] Implement correlation ID tracking
  - [ ] Create event serialization and persistence
    - [ ] Add event logging integration
    - [ ] Implement event history tracking
    - [ ] Create event replay capabilities

## Call Engine Integration

- [ ] Create Call Engine API adapter
  - [ ] Implement high-level media session control
    - [ ] Add simplified session creation interface
    - [ ] Create feature-based configuration
    - [ ] Implement call-specific media operations
  - [ ] Design comprehensive event notification system
    - [ ] Create call-level media events
    - [ ] Add quality notification interfaces
    - [ ] Implement device status updates
  - [ ] Add call quality management
    - [ ] Create call-specific quality metrics
    - [ ] Implement quality adaptation strategies
    - [ ] Add quality prediction and proactive adjustment

- [ ] Implement feature coordination with Call Engine
  - [ ] Create call hold/resume integration
    - [ ] Add media state synchronization
    - [ ] Implement resource management during hold
    - [ ] Create resume optimization for faster switching
  - [ ] Add call transfer media handling
    - [ ] Implement media session transfer procedures
    - [ ] Create media state preservation during transfer
    - [ ] Add optimization for minimizing disruption
  - [ ] Implement conference support
    - [ ] Create dynamic mixing configuration
    - [ ] Add participant management integration
    - [ ] Implement conference-specific optimizations

- [ ] Create diagnostics interface for Call Engine
  - [ ] Implement call-specific diagnostic tools
    - [ ] Add media quality analysis
    - [ ] Create codec performance reporting
    - [ ] Implement network impact assessment
  - [ ] Add troubleshooting utilities
    - [ ] Create media logging with correlation IDs
    - [ ] Implement media sample capture for analysis
    - [ ] Add quality issue diagnosis tools 