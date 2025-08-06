# RVOIP Media Core

[![Crates.io](https://img.shields.io/crates/v/rvoip-media-core.svg)](https://crates.io/crates/rvoip-media-core)
[![Documentation](https://docs.rs/rvoip-media-core/badge.svg)](https://docs.rs/rvoip-media-core)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

## Overview

The `media-core` library provides comprehensive media processing and audio management capabilities for the [rvoip](../../README.md) VoIP stack. It handles all media-level operations including codec management, advanced audio processing, quality monitoring, and multi-party conference mixing while integrating seamlessly with `session-core` (SIP signaling) and `rtp-core` (RTP transport).

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

The Media Core sits at the heart of the media processing stack, providing intelligent audio processing and session coordination:

```
┌─────────────────────────────────────────┐
│          Application Layer              │
├─────────────────────────────────────────┤
│         rvoip-session-core              │
├─────────────────────────────────────────┤
│         rvoip-media-core   ⬅️ YOU ARE HERE
├─────────────────────────────────────────┤
│           rvoip-rtp-core                │
├─────────────────────────────────────────┤
│            Network Layer                │
└─────────────────────────────────────────┘
```

### Key Components

1. **Media Processing Engine**: Advanced audio processing with AEC, AGC, VAD, and noise suppression
2. **Codec Management**: Multi-codec support with real-time transcoding (G.711, Opus, G.729)
3. **Session Coordination**: Per-dialog media session management with SIP integration
4. **Conference Mixing**: N-way audio mixing for multi-party conferences
5. **Quality Monitoring**: Real-time quality metrics and adaptive processing
6. **Zero-Copy Pipeline**: High-performance memory management with SIMD optimizations

### Integration Architecture

Clean separation of concerns across the rvoip stack:

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

## Features
### ✅ Completed Features

#### **Advanced Audio Processing**
- ✅ **Acoustic Echo Cancellation (AEC v2)**: Frequency-domain NLMS with 16.4 dB ERLE improvement
  - ✅ Multi-partition filtering for echo delays up to 200ms
  - ✅ Coherence-based double-talk detection
  - ✅ Wiener filter residual echo suppression
  - ✅ 3.9x speed improvement over basic implementation
- ✅ **Automatic Gain Control (AGC v2)**: Multi-band processing with broadcast quality
  - ✅ Linkwitz-Riley crossover filters for 3-band processing
  - ✅ Look-ahead peak limiting with 8ms preview
  - ✅ LUFS loudness measurement (ITU-R BS.1770-4)
  - ✅ 2.6x consistency improvement in gain control
- ✅ **Voice Activity Detection (VAD v2)**: Spectral analysis with ensemble voting
  - ✅ FFT-based spectral analysis with Hanning windowing
  - ✅ Multiple feature extraction (energy, ZCR, spectral centroid, rolloff, flux)
  - ✅ Fundamental frequency detection with harmonic analysis
  - ✅ Adaptive noise floor estimation

#### **High-Performance Architecture**
- ✅ **Zero-Copy Media Pipeline**: Dramatic performance improvements
  - ✅ Arc-based shared ownership with 1.7-2.1x speedup
  - ✅ 67% reduction in memory allocations
  - ✅ 1.88x faster audio processing pipelines
- ✅ **Object Pooling**: Memory optimization with exceptional results
  - ✅ AudioFramePool with 4.67x allocation speedup
  - ✅ 100% pool hit rate in steady-state operation
  - ✅ Sub-microsecond frame operations (42ns pooled processing)
- ✅ **SIMD Optimizations**: Cross-platform performance
  - ✅ x86_64 SSE2 and AArch64 NEON support
  - ✅ Automatic fallback to scalar implementations
  - ✅ 8-sample parallel processing for audio operations

#### **Codec Support and Transcoding**
- ✅ **Multi-Codec Support**: Complete telephony codec suite
  - ✅ **G.711**: μ-law/A-law (PCMU/PCMA) with ITU-T compliance
  - ✅ **Opus**: Modern wideband/fullband codec with VBR/CBR
  - ✅ **G.729**: Low-bitrate 8 kbps codec with Annex A/B support
- ✅ **Real-Time Transcoding**: Seamless format conversion
  - ✅ PCMU ↔ PCMA ↔ Opus ↔ G.729 transcoding matrix
  - ✅ Session management with performance statistics
  - ✅ Format conversion with sample rate adaptation

#### **Session Management and Integration**
- ✅ **MediaSession**: Complete per-dialog media management
  - ✅ Lifecycle management (create, start, pause, resume, stop)
  - ✅ Codec negotiation and dynamic switching
  - ✅ Quality monitoring integration
- ✅ **Session Coordination**: Seamless integration bridges
  - ✅ RtpBridge for rtp-core integration
  - ✅ SessionBridge for session-core coordination
  - ✅ Event-driven architecture with comprehensive events

#### **Conference Audio Mixing**
- ✅ **N-Way Audio Mixing**: Professional conference capabilities
  - ✅ Dynamic participant management (add/remove streams)
  - ✅ Real-time audio processing with frame buffering
  - ✅ N-1 mixing for each participant (exclude own voice)
  - ✅ Voice activity detection for selective mixing
- ✅ **Conference Integration**: Production-ready implementation
  - ✅ AudioMixer with MediaSessionController integration
  - ✅ Event system for conference monitoring
  - ✅ Quality monitoring for mixed audio streams
  - ✅ 7/7 integration tests passing (100% success rate)

#### **Quality Monitoring and Statistics**
- ✅ **Real-Time Quality Metrics**: Comprehensive monitoring
  - ✅ MOS score calculation and R-factor quality rating
  - ✅ Packet loss, jitter, and latency tracking
  - ✅ Network quality assessment and adaptation
- ✅ **RTCP Statistics Integration**: Complete implementation
  - ✅ RTP/RTCP statistics exposure to session-core
  - ✅ Quality degradation alerts and monitoring
  - ✅ Performance metrics collection and reporting

#### **Production-Ready Infrastructure**
- ✅ **Comprehensive Testing**: Extensive validation
  - ✅ 80+ tests passing (66 unit + 7 conference + 6 RTP + 1 doc)
  - ✅ Performance benchmarking and regression detection
  - ✅ Integration test coverage for all major components
- ✅ **Error Handling**: Robust production deployment
  - ✅ Comprehensive error types and handling
  - ✅ Graceful degradation scenarios
  - ✅ Resource cleanup and lifecycle management

### 🚧 Planned Features

#### **Advanced Processing**
- 🚧 Machine learning-based VAD for enhanced accuracy
- 🚧 Multi-band noise suppression
- 🚧 Packet loss concealment (PLC) improvements
- 🚧 Dynamic range compression and audio effects

#### **Performance Enhancements**
- 🚧 Hardware acceleration support (AES-NI, etc.)
- 🚧 Custom memory allocators for specific workloads
- 🚧 Advanced SIMD optimizations (AVX-512, ARM SVE)
- 🚧 Lock-free data structures for ultra-high concurrency

#### **Extended Codec Support**
- 🚧 Video codec support (H.264, VP8, VP9)
- 🚧 Additional audio codecs (G.722, SILK)
- 🚧 Hardware codec acceleration
- 🚧 Codec-specific optimizations

## Usage

### Basic Media Session

```rust
use rvoip_media_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create media engine configuration
    let config = MediaEngineConfig::builder()
        .enable_advanced_processing(true)
        .enable_performance_optimizations(true)
        .build();

    // Create and start media engine
    let engine = MediaEngine::new(config).await?;
    engine.start().await?;

    // Create media session for SIP dialog
    let dialog_id = DialogId::new("call-123");
    let params = MediaSessionParams::builder()
        .audio_only()
        .preferred_codec(PayloadType::PCMU)
        .enable_processing(true)
        .advanced_aec_config(AdvancedAecConfig::default())
        .advanced_agc_config(AdvancedAgcConfig::default())
        .build();

    let session = engine.create_media_session(dialog_id, params).await?;

    // Get codec capabilities for SDP negotiation
    let capabilities = engine.get_supported_codecs();
    println!("Supported codecs: {:?}", capabilities);

    // Process audio with advanced algorithms
    let audio_frame = AudioFrame::new(samples, 16000, 1, timestamp);
    let processed = session.process_audio(audio_frame).await?;

    // Monitor quality metrics
    let metrics = session.get_quality_metrics().await;
    println!("MOS score: {:.1}, Packet loss: {:.1}%", 
             metrics.mos_score, metrics.packet_loss_percent);

    Ok(())
}
```

### Advanced Audio Processing

```rust
use rvoip_media_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Configure advanced audio processing
    let aec_config = AdvancedAecConfig::builder()
        .filter_length(512)
        .adaptation_rate(0.1)
        .enable_comfort_noise(true)
        .build();

    let agc_config = AdvancedAgcConfig::builder()
        .target_level_dbfs(-23.0)
        .compression_ratio(3.0)
        .enable_multiband(true)
        .build();

    let vad_config = AdvancedVadConfig::builder()
        .enable_spectral_analysis(true)
        .adaptive_threshold(true)
        .build();

    let processing_config = AudioProcessingConfig::builder()
        .advanced_aec_config(aec_config)
        .advanced_agc_config(agc_config)
        .advanced_vad_config(vad_config)
        .enable_simd_optimizations(true)
        .build();

    // Create processor with advanced algorithms
    let processor = AudioProcessor::new(processing_config)?;

    // Process audio with professional-grade algorithms
    let input_frame = AudioFrame::new(samples, 16000, 1, timestamp);
    let output_frame = processor.process_capture_audio(&input_frame)?;

    // Advanced processing provides:
    // - 16.4 dB better echo cancellation
    // - 2.6x more consistent gain control
    // - Spectral voice activity detection
    
    Ok(())
}
```

### Multi-Party Conference Mixing

```rust
use rvoip_media_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create media session controller with conference support
    let controller = MediaSessionController::with_conference_support().await?;

    // Create media sessions for participants
    let alice_dialog = DialogId::new("alice");
    let bob_dialog = DialogId::new("bob");
    let charlie_dialog = DialogId::new("charlie");

    // Start media sessions
    controller.start_media(alice_dialog.clone(), media_config()).await?;
    controller.start_media(bob_dialog.clone(), media_config()).await?;
    controller.start_media(charlie_dialog.clone(), media_config()).await?;

    // Add participants to conference
    controller.add_to_conference(alice_dialog.clone()).await?;
    controller.add_to_conference(bob_dialog.clone()).await?;
    controller.add_to_conference(charlie_dialog.clone()).await?;

    // Process conference audio (N-1 mixing for each participant)
    let alice_audio = AudioFrame::new(alice_samples, 8000, 1, timestamp);
    controller.process_conference_audio(alice_dialog, alice_audio).await?;

    // Conference automatically mixes audio from Bob and Charlie for Alice
    // (excluding Alice's own voice to prevent echo)

    // Monitor conference statistics
    let stats = controller.get_conference_stats().await?;
    println!("Active participants: {}, Total mixes: {}", 
             stats.active_participants, stats.total_mixes);

    Ok(())
}
```

### Zero-Copy Performance Optimization

```rust
use rvoip_media_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Configure high-performance processing
    let performance_config = PerformanceConfig::builder()
        .enable_zero_copy(true)
        .enable_simd_optimizations(true)
        .enable_frame_pooling(true)
        .frame_pool_size(64)
        .build();

    let config = MediaEngineConfig::builder()
        .performance(performance_config)
        .build();

    let engine = MediaEngine::new(config).await?;

    // Create session with performance optimizations
    let session = engine.create_media_session(dialog_id, params).await?;

    // Process with zero-copy architecture
    // - 1.7-2.1x speedup from zero-copy operations
    // - 4.2-12.6x speedup from object pooling
    // - Sub-microsecond frame operations
    let processed = session.process_audio_zero_copy(audio_frame).await?;

    // Monitor performance metrics
    let metrics = session.get_performance_metrics().await;
    println!("Processing time: {}ns, Pool efficiency: {:.1}%",
             metrics.avg_processing_time_ns, metrics.pool_efficiency * 100.0);

    Ok(())
}
```

### Real-Time Codec Transcoding

```rust
use rvoip_media_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create transcoding session
    let transcoder = Transcoder::new().await?;

    // Configure transcoding between different codecs
    let session_config = TranscodingSessionConfig::builder()
        .input_codec(CodecType::PCMU)
        .output_codec(CodecType::Opus)
        .enable_format_conversion(true)
        .build();

    let session_id = transcoder.create_session(session_config).await?;

    // Transcode audio in real-time
    let pcmu_frame = AudioFrame::new(pcmu_samples, 8000, 1, timestamp);
    let opus_frame = transcoder.transcode(session_id, pcmu_frame).await?;

    // Supports all codec combinations:
    // G.711 (PCMU/PCMA) ↔ Opus ↔ G.729
    // with automatic format conversion

    // Monitor transcoding performance
    let stats = transcoder.get_session_stats(session_id).await?;
    println!("Transcoding latency: {}μs, Quality: MOS {:.1}",
             stats.avg_latency_us, stats.output_quality_mos);

    Ok(())
}
```

## Advanced Audio Processing

The library provides cutting-edge audio processing algorithms competitive with commercial solutions:

### Echo Cancellation (AEC v2)

- **Frequency-Domain Processing**: 512-point FFT with overlap-add
- **Multi-Partition Filtering**: Handles echo delays up to 200ms
- **Coherence Detection**: Advanced double-talk detection
- **Performance**: 16.4 dB ERLE improvement, 3.9x speed increase

### Automatic Gain Control (AGC v2)

- **Multi-Band Processing**: 3-band Linkwitz-Riley crossover filters
- **Look-Ahead Limiting**: 8ms preview for transient protection
- **Broadcast Standards**: LUFS measurement (ITU-R BS.1770-4)
- **Performance**: 2.6x consistency improvement

### Voice Activity Detection (VAD v2)

- **Spectral Analysis**: FFT-based with multiple feature extraction
- **Ensemble Voting**: 5 different detection algorithms combined
- **Adaptive Thresholds**: Self-tuning based on acoustic environment
- **Features**: Energy, ZCR, spectral centroid, rolloff, flux

## Performance Characteristics

### Zero-Copy Pipeline Performance

- **Small Frames (160 samples)**: 231ns → 55ns (4.20x speedup with pooling)
- **Large Frames (320 samples)**: 530ns → 42ns (12.62x speedup with pooling)
- **Pipeline Throughput**: 1.88x improvement in multi-stage processing
- **Memory Efficiency**: 67% reduction in allocations

### Real-Time Processing

- **Audio Processing**: Sub-microsecond frame operations
- **Echo Cancellation**: 42x real-time factor (process 42s in 1s)
- **Conference Mixing**: <5ms latency per participant
- **Codec Transcoding**: Real-time performance for all supported codecs

### Scalability Factors

- **Concurrent Sessions**: Tested with 100+ simultaneous sessions
- **Memory Usage**: ~2KB per active session
- **CPU Efficiency**: 0.7% usage on Apple Silicon for typical workloads
- **Pool Efficiency**: 100% hit rate in steady-state operation

## Quality and Testing

### Comprehensive Test Coverage

- **Unit Tests**: 66 tests covering all core functionality
- **Integration Tests**: 7 conference tests + 6 RTP integration tests
- **Performance Tests**: 8 benchmark tests validating optimizations
- **Audio Quality Tests**: 4 comparison tests for advanced processing

### Quality Improvements Achieved

- **Echo Cancellation**: 16.4 dB ERLE improvement over basic implementation
- **Gain Control**: 2.6x more consistent level control
- **Processing Speed**: 3.9x faster advanced AEC with better quality
- **Memory Efficiency**: 4.67x faster allocation with object pooling

### Production Validation

- **All Examples Working**: 6/6 examples run successfully
- **Performance Validation**: Debug vs release builds tested
- **Cross-Platform**: Tested on x86_64, AArch64 with SIMD optimizations
- **Long-Running Stability**: Memory leak detection and resource cleanup

## Codec Implementation

### Supported Codecs

- **G.711 (PCMU/PCMA)**: ITU-T compliant μ-law/A-law implementation
- **Opus**: Modern wideband codec with VBR/CBR, 6-510 kbps
- **G.729**: Low-bitrate 8 kbps with Annex A/B (VAD/CNG) support

### Transcoding Capabilities

- **Real-Time Transcoding**: All codec combinations supported
- **Format Conversion**: Automatic sample rate and channel conversion
- **Session Management**: Performance statistics and caching
- **Quality Preservation**: Optimal transcoding paths to minimize quality loss

## Integration with Other Crates

### Session-Core Integration

- **MediaControl Trait**: Complete statistics and control API
- **SIP Dialog Coordination**: Per-dialog media session management
- **Codec Negotiation**: SDP capability exchange and matching
- **Event Propagation**: Media events to SIP layer

### Audio Muting Implementation

The media-core library implements production-ready audio muting that maintains RTP flow by sending silence packets instead of dropping RTP transmission. This approach ensures compatibility with NAT traversal, firewalls, and all SIP endpoints.

```rust
use rvoip_media_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let controller = MediaSessionController::new();
    let dialog_id = DialogId::new("call-123");
    
    // Start media session
    controller.start_media(dialog_id.clone(), config).await?;
    
    // Mute audio - RTP continues with silence packets
    controller.set_audio_muted(&dialog_id, true).await?;
    
    // Audio frames are now replaced with silence before encoding
    // This maintains:
    // - Continuous RTP sequence numbers and timestamps
    // - NAT binding keepalive (prevents timeout)
    // - Remote endpoint connectivity
    // - Codec state consistency
    
    // Check mute status
    let is_muted = controller.is_audio_muted(&dialog_id).await?;
    
    // Unmute to resume normal audio
    controller.set_audio_muted(&dialog_id, false).await?;
    
    Ok(())
}
```

**Technical Implementation:**
- **Silence Generation**: PCM samples replaced with zeros before codec encoding
- **Codec Compatibility**: Works with all codecs (G.711, Opus, G.729)
- **State Tracking**: Per-session mute state in `RtpSessionWrapper`
- **Processing Pipeline**: Muting occurs in `encode_and_send_audio_frame()`

**Key Benefits:**
- **NAT Traversal**: Prevents binding timeouts by maintaining packet flow
- **Compatibility**: Works with all SIP endpoints and middleboxes
- **Instant Toggle**: No renegotiation required for mute/unmute
- **Professional Quality**: Follows VoIP industry best practices
- **No Packet Loss**: Remote endpoint sees continuous RTP stream

### RTP-Core Integration

- **MediaTransport**: Seamless RTP packet handling
- **Statistics Forwarding**: RTCP metrics to quality monitoring
- **Secure Transport**: Integration with SRTP/DTLS security
- **Packet Processing**: Zero-copy RTP ↔ Audio frame conversion

## Testing

Run the comprehensive test suite:

```bash
# Run all tests
cargo test -p rvoip-media-core

# Run with advanced processing features
cargo test -p rvoip-media-core --features "advanced-processing"

# Run performance benchmarks
cargo test -p rvoip-media-core --release -- --ignored benchmark

# Run specific test suites
cargo test -p rvoip-media-core audio_processing
cargo test -p rvoip-media-core conference_mixing
cargo test -p rvoip-media-core zero_copy_performance
```

### Example Applications

The library includes comprehensive examples demonstrating all features:

```bash
# Basic media engine usage
cargo run --example basic_usage

# Advanced audio processing demonstration
cargo run --example processing_demo

# Echo cancellation showcase
cargo run --example aec_demo

# Quality monitoring example
cargo run --example quality_demo

# Conference mixing demonstration
RUST_LOG=info cargo run --example conference_demo

# Performance validation
cargo run --release --example performance_comparison
```

## Error Handling

The library provides comprehensive error handling with categorized error types:

```rust
use rvoip_media_core::Error;

match media_result {
    Err(Error::CodecNotSupported(codec)) => {
        log::error!("Unsupported codec: {}", codec);
        attempt_codec_fallback().await?;
    }
    Err(Error::ProcessingFailed(details)) => {
        log::warn!("Audio processing failed: {}", details);
        if error.is_recoverable() {
            retry_with_basic_processing().await?;
        }
    }
    Err(Error::SessionNotFound(session_id)) => {
        log::info!("Session {} not found, creating new", session_id);
        create_new_session(session_id).await?;
    }
    Ok(result) => {
        // Handle success
    }
}
```

## Future Improvements

### Advanced Features
- Machine learning-based audio enhancement
- Multi-room acoustic modeling
- Advanced packet loss concealment
- Real-time audio effects and spatial processing

### Performance Enhancements
- Hardware Security Module (HSM) integration
- Custom SIMD kernels for audio processing
- GPU acceleration for conference mixing
- Distributed processing for large conferences

### Protocol Extensions
- Video codec support and processing
- Advanced RTCP feedback mechanisms
- WebRTC compatibility enhancements
- Low-latency streaming protocols

## Contributing

Contributions are welcome! Please see the main [rvoip contributing guidelines](../../README.md#contributing) for details.

For media-core specific contributions:
- Ensure ITU-T compliance for any codec changes
- Add comprehensive audio quality tests for new processing features
- Update documentation for any API changes
- Consider real-time performance impact for all changes

## License

This project is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option. 