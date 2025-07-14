# Codec-Core Implementation Plan

## Overview

The `codec-core` library is a high-performance, unified audio codec implementation that consolidates and improves upon the existing codec implementations scattered across the RVOIP codebase. This library serves as the single source of truth for audio codec operations across all RVOIP components.

## Problem Statement

Currently, the RVOIP codebase has multiple implementations of the same codecs:
- **G.711**: 3 different implementations (basic, advanced, mock)
- **G.722**: 2 different implementations (media-core, audio-core)
- **G.729**: 2 different implementations (media-core, audio-core)
- **Opus**: 2 different implementations (media-core, audio-core)

This leads to:
- ❌ Code duplication and maintenance overhead
- ❌ Inconsistent behavior between components
- ❌ Difficulty in ensuring quality and performance
- ❌ Architectural confusion and tight coupling

## Solution

Create a dedicated `codec-core` library that:
- ✅ Consolidates all codec implementations
- ✅ Provides consistent, high-performance APIs
- ✅ Eliminates code duplication
- ✅ Enables shared improvements and optimizations
- ✅ Offers both real and simulation modes for testing

## Architecture

### Core Design Principles

1. **Performance First**: SIMD optimizations, lookup tables, zero-copy APIs
2. **Flexibility**: Support both real and simulation modes
3. **Testability**: Comprehensive test coverage with property-based testing
4. **Maintainability**: Clean APIs, clear documentation, modular design
5. **Compatibility**: Easy integration with existing media-core and audio-core

### Module Structure

```
codec-core/
├── src/
│   ├── lib.rs                 # Public API and re-exports
│   ├── types.rs               # Common types and traits
│   ├── error.rs               # Error handling
│   ├── codecs/
│   │   ├── mod.rs            # Codec registry and factory
│   │   ├── g711/
│   │   │   ├── mod.rs        # G.711 public API
│   │   │   ├── encoder.rs    # μ-law/A-law encoding
│   │   │   ├── decoder.rs    # μ-law/A-law decoding
│   │   │   ├── tables.rs     # Pre-computed lookup tables
│   │   │   └── simd.rs       # SIMD optimizations
│   │   ├── g722/
│   │   │   ├── mod.rs        # G.722 public API
│   │   │   ├── encoder.rs    # Sub-band ADPCM encoding
│   │   │   ├── decoder.rs    # Sub-band ADPCM decoding
│   │   │   ├── qmf.rs        # QMF analysis/synthesis
│   │   │   └── adpcm.rs      # ADPCM implementation
│   │   ├── g729/
│   │   │   ├── mod.rs        # G.729 public API
│   │   │   ├── encoder.rs    # ACELP encoding
│   │   │   ├── decoder.rs    # ACELP decoding
│   │   │   ├── lpc.rs        # LPC analysis/synthesis
│   │   │   ├── pitch.rs      # Pitch analysis
│   │   │   ├── codebook.rs   # Algebraic codebook
│   │   │   └── simulation.rs # Simulation mode
│   │   └── opus/
│   │       ├── mod.rs        # Opus public API
│   │       ├── encoder.rs    # Real Opus encoding
│   │       ├── decoder.rs    # Real Opus decoding
│   │       └── simulation.rs # Simulation mode
│   └── utils/
│       ├── simd.rs           # SIMD utilities
│       ├── tables.rs         # Table generation utilities
│       └── validation.rs     # Input validation
├── tests/
│   ├── integration/
│   │   ├── g711_tests.rs     # G.711 integration tests
│   │   ├── g722_tests.rs     # G.722 integration tests
│   │   ├── g729_tests.rs     # G.729 integration tests
│   │   ├── opus_tests.rs     # Opus integration tests
│   │   └── interop_tests.rs  # Cross-codec tests
│   ├── property/
│   │   ├── roundtrip.rs      # Property-based roundtrip tests
│   │   ├── quality.rs        # Quality preservation tests
│   │   └── performance.rs    # Performance regression tests
│   └── compatibility/
│       ├── media_core.rs     # Media-core compatibility
│       └── audio_core.rs     # Audio-core compatibility
└── benches/
    └── codec_benchmarks.rs   # Performance benchmarks
```

## Codec Implementations

### G.711 (PCMU/PCMA)
**Source**: Best features from `media-core/src/codec/audio/g711.rs`

#### Features:
- ✅ **High Performance**: Pre-computed lookup tables for O(1) conversion
- ✅ **SIMD Optimizations**: x86_64 SSE2 and AArch64 NEON support
- ✅ **Zero-Copy APIs**: Pre-allocated buffer support
- ✅ **ITU-T Compliance**: Fully compliant μ-law and A-law implementations
- ✅ **Quality Validation**: Comprehensive SNR and distortion testing

#### Key Optimizations:
- Pre-computed 65536-element lookup tables for encoding
- Pre-computed 256-element lookup tables for decoding
- SIMD processing for 8-16 samples at once
- Automatic fallback to scalar implementation

### G.722 (Wideband)
**Source**: Enhanced version of `audio-core/src/codec/g722.rs`

#### Features:
- ✅ **Sub-band Coding**: Proper QMF analysis and synthesis
- ✅ **ADPCM Implementation**: ITU-T G.722 compliant ADPCM for each band
- ✅ **16kHz Support**: Wideband audio at 64kbps
- ✅ **Quality Optimization**: Improved quantization and prediction

#### Key Components:
- QMF filter bank with proper coefficients
- ADPCM encoders/decoders for low and high bands
- Optimized bit packing/unpacking
- Proper state management for continuous processing

### G.729 (Low-bitrate)
**Source**: Enhanced version of `media-core/src/codec/audio/g729.rs`

#### Features:
- ✅ **ACELP Implementation**: Algebraic Code Excited Linear Prediction
- ✅ **Dual Mode**: Real G.729 (with external library) and simulation
- ✅ **Annex Support**: Annex A (reduced complexity) and Annex B (VAD/CNG)
- ✅ **8kbps Compression**: Excellent voice quality at low bitrate

#### Key Components:
- LPC analysis with windowing and autocorrelation
- Pitch analysis (open-loop and closed-loop)
- Algebraic codebook search
- Gain quantization and LSP conversion
- Voice Activity Detection (VAD)
- Comfort Noise Generation (CNG)

### Opus (Modern)
**Source**: Enhanced version of `media-core/src/codec/audio/opus.rs`

#### Features:
- ✅ **Flexible Bitrate**: 6-510 kbps with VBR/CBR support
- ✅ **Wide Sample Rate**: 8-48kHz support
- ✅ **Low Latency**: Optimized for real-time applications
- ✅ **Quality Modes**: Voice and audio application modes
- ✅ **Dual Mode**: Real Opus (with external library) and simulation

#### Key Components:
- Proper Opus encoder/decoder initialization
- Bitrate adaptation and complexity control
- Frame size flexibility (2.5-60ms)
- FEC (Forward Error Correction) support
- Packet loss concealment simulation

## API Design

### Core Traits

```rust
/// Primary codec trait for encoding/decoding operations
pub trait AudioCodec: Send + Sync {
    type Config: Clone + Send + Sync;
    type Error: std::error::Error + Send + Sync + 'static;
    
    /// Create a new codec instance
    fn new(config: Self::Config) -> Result<Self, Self::Error> where Self: Sized;
    
    /// Encode audio samples to compressed data
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>, Self::Error>;
    
    /// Decode compressed data to audio samples
    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>, Self::Error>;
    
    /// Get codec information
    fn info(&self) -> CodecInfo;
    
    /// Reset codec state
    fn reset(&mut self) -> Result<(), Self::Error>;
}

/// Codec capability information
#[derive(Debug, Clone)]
pub struct CodecInfo {
    pub name: &'static str,
    pub sample_rate: u32,
    pub channels: u8,
    pub bitrate: u32,
    pub frame_size: usize,
}

/// Audio frame for processing
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u8,
    pub timestamp: u64,
}
```

### Codec Factory

```rust
/// Codec factory for creating codec instances
pub struct CodecFactory;

impl CodecFactory {
    /// Create a codec by name
    pub fn create_by_name(name: &str, config: CodecConfig) -> Result<Box<dyn AudioCodec>, CodecError>;
    
    /// Create a codec by payload type
    pub fn create_by_payload_type(pt: u8, config: CodecConfig) -> Result<Box<dyn AudioCodec>, CodecError>;
    
    /// Get all supported codecs
    pub fn supported_codecs() -> Vec<CodecInfo>;
}
```

## Performance Targets

### Benchmark Goals
- **G.711**: < 100ns per sample (encode/decode)
- **G.722**: < 500ns per sample (encode/decode)
- **G.729**: < 2μs per frame (80 samples, 10ms)
- **Opus**: < 5μs per frame (variable size)

### Memory Usage
- **G.711**: < 1KB state per codec instance
- **G.722**: < 2KB state per codec instance
- **G.729**: < 8KB state per codec instance
- **Opus**: < 16KB state per codec instance

### Quality Targets
- **G.711**: SNR > 20dB for sine waves
- **G.722**: SNR > 25dB for wideband content
- **G.729**: MOS > 3.9 for voice content
- **Opus**: MOS > 4.2 for voice content

## Testing Strategy

### Test Categories

1. **Unit Tests**: Individual component testing
2. **Integration Tests**: Full codec pipeline testing
3. **Property Tests**: Roundtrip and invariant testing
4. **Compatibility Tests**: Integration with existing codebase
5. **Performance Tests**: Benchmark regression testing
6. **Quality Tests**: Audio quality validation

### Test Coverage Goals
- **Line Coverage**: > 90%
- **Branch Coverage**: > 85%
- **Integration Coverage**: 100% of public APIs

### Property-Based Testing
- **Roundtrip Properties**: encode(decode(x)) ≈ x
- **Monotonicity**: Consistent behavior with similar inputs
- **Boundary Testing**: Edge cases and error conditions

## Integration Plan

### Phase 1: Core Implementation (Week 1)
- ✅ Create library structure and build system
- ✅ Implement core traits and types
- ✅ Implement G.711 with full optimizations
- ✅ Basic test suite for G.711

### Phase 2: Extended Codecs (Week 2)
- ✅ Implement G.722 with sub-band coding
- ✅ Implement G.729 with simulation mode
- ✅ Implement Opus with simulation mode
- ✅ Comprehensive test suite for all codecs

### Phase 3: Performance Optimization (Week 3)
- ✅ SIMD optimizations for all codecs
- ✅ Lookup table optimizations
- ✅ Zero-copy API implementations
- ✅ Performance benchmarking and validation

### Phase 4: Integration and Migration (Week 4)
- ✅ Update media-core to use codec-core
- ✅ Update audio-core to use codec-core
- ✅ Remove duplicate codec implementations
- ✅ Final testing and validation

## Migration Strategy

### Backward Compatibility
- Maintain existing API compatibility where possible
- Provide migration guides for breaking changes
- Gradual migration with feature flags

### Rollout Plan
1. **Parallel Implementation**: codec-core alongside existing codecs
2. **Feature Flag Migration**: Optional use of codec-core
3. **Default Switch**: Make codec-core the default
4. **Cleanup**: Remove old implementations

## Success Metrics

### Performance Metrics
- **Encoding Speed**: > 100x real-time for all codecs
- **Memory Usage**: < 50MB for 1000 concurrent codec instances
- **Latency**: < 1ms additional latency over existing implementations

### Quality Metrics
- **No Regression**: Audio quality equal or better than existing
- **Consistency**: Identical behavior across all consumers
- **Robustness**: Handle edge cases and error conditions gracefully

### Maintainability Metrics
- **Code Reduction**: > 50% reduction in codec-related code
- **Test Coverage**: > 90% line coverage
- **Documentation**: 100% of public APIs documented

## Risk Mitigation

### Technical Risks
- **Performance Regression**: Comprehensive benchmarking
- **Quality Degradation**: Extensive A/B testing
- **Integration Issues**: Gradual migration with feature flags

### Licensing Risks
- **G.729**: Use simulation mode by default
- **Opus**: MIT licensed, no issues
- **Patents**: Stick to well-established, patent-free implementations

### Timeline Risks
- **Scope Creep**: Focus on core functionality first
- **Quality Issues**: Prioritize correctness over performance
- **Integration Complexity**: Maintain backward compatibility

## Conclusion

The `codec-core` library represents a significant architectural improvement to the RVOIP codebase. By consolidating codec implementations, we achieve:

1. **Reduced Complexity**: Single source of truth for all codecs
2. **Improved Performance**: SIMD optimizations and lookup tables
3. **Better Quality**: Comprehensive testing and validation
4. **Enhanced Maintainability**: Clean APIs and documentation
5. **Future Flexibility**: Easy addition of new codecs

This implementation plan provides a roadmap for creating a production-ready, high-performance codec library that serves as the foundation for all RVOIP audio processing needs. 