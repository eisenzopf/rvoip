# Media-Core Codec-Core Integration Plan

## Executive Summary

This plan outlines the integration of `codec-core` into `media-core` to replace the current internal codec implementations. This change will provide a clean separation of concerns where `codec-core` handles all codec operations while `media-core` focuses on media session management and RTP processing.

## Current State Analysis

### Existing Codec Implementation in Media-Core

1. **Internal G.711 Implementation**:
   - `src/codec/g711.rs` - Basic G.711 implementation
   - `src/codec/audio/g711.rs` - Full G.711 codec with AudioCodec trait
   - `src/relay/mod.rs` - G711PcmuCodec and G711PcmaCodec for passthrough

2. **Codec Framework**:
   - `AudioCodec` trait in `src/codec/audio/common.rs`
   - `CodecRegistry` in `src/codec/mod.rs`
   - Codec mapping utilities in `src/codec/mapping.rs`
   - Transcoding support in `src/codec/transcoding.rs`

3. **Usage Points**:
   - `MediaSession` uses `G711Codec` for encoding/decoding
   - `Transcoder` creates codec instances for format conversion
   - Zero-copy processing in `relay/controller/zero_copy.rs`

## Integration Architecture

### Design Principles

1. **Minimal Disruption**: Keep existing media-core APIs intact
2. **Clean Abstraction**: Create an adapter layer between media-core and codec-core
3. **Performance**: Maintain zero-copy optimizations where possible
4. **Extensibility**: Easy to add new codecs from codec-core

### Architectural Approach

```
┌─────────────────────────────────────────────────┐
│                 media-core                       │
│                                                  │
│  ┌─────────────────────────────────────────┐    │
│  │         Existing AudioCodec Trait        │    │
│  └─────────────────────┬───────────────────┘    │
│                        │                         │
│  ┌─────────────────────▼───────────────────┐    │
│  │      CodecCoreAdapter (NEW)             │    │
│  │                                          │    │
│  │  • Implements media-core AudioCodec      │    │
│  │  • Wraps codec-core codecs               │    │
│  │  • Handles format conversions            │    │
│  └─────────────────────┬───────────────────┘    │
│                        │                         │
└────────────────────────┼─────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────┐
│                 codec-core                       │
│                                                  │
│  • G.711 (PCMU/PCMA)                            │
│  • Future: Opus, G.722, etc.                    │
└──────────────────────────────────────────────────┘
```

## Implementation Steps

### Phase 1: Create Codec Adapter Layer (Week 1)

#### 1.1 Add codec-core Dependency
```toml
# In media-core/Cargo.toml
[dependencies]
codec-core = { path = "../codec-core" }
```

#### 1.2 Create Adapter Module
Create `src/codec/codec_core_adapter.rs`:

```rust
use codec_core::types::{AudioCodec as CodecCoreAudioCodec, CodecInfo as CoreCodecInfo};
use crate::codec::audio::common::{AudioCodec, CodecInfo};
use crate::types::AudioFrame;
use crate::error::{Error, Result};

/// Adapter that wraps codec-core codecs to implement media-core's AudioCodec trait
pub struct CodecCoreAdapter {
    inner: Box<dyn CodecCoreAudioCodec>,
    name: String,
}

impl CodecCoreAdapter {
    pub fn new(codec: Box<dyn CodecCoreAudioCodec>) -> Self {
        let name = codec.info().name.to_string();
        Self { inner: codec, name }
    }
}

impl AudioCodec for CodecCoreAdapter {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        // Convert media-core AudioFrame to codec-core format
        self.inner.encode(&audio_frame.samples)
            .map_err(|e| Error::codec(format!("Encoding failed: {}", e)))
    }
    
    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        // Decode and convert to media-core AudioFrame
        let samples = self.inner.decode(encoded_data)
            .map_err(|e| Error::codec(format!("Decoding failed: {}", e)))?;
        
        let info = self.inner.info();
        Ok(AudioFrame::new(
            samples,
            info.sample_rate,
            info.channels,
            0, // timestamp will be set by caller
        ))
    }
    
    fn get_info(&self) -> CodecInfo {
        let core_info = self.inner.info();
        CodecInfo {
            name: core_info.name.to_string(),
            sample_rate: core_info.sample_rate,
            channels: core_info.channels,
            bitrate: core_info.bitrate,
        }
    }
    
    fn reset(&mut self) {
        let _ = self.inner.reset();
    }
}
```

### Phase 2: Update Codec Factory (Week 1)

#### 2.1 Create Codec Factory Using codec-core
Update `src/codec/factory.rs` (new file):

```rust
use codec_core::{CodecFactory as CoreFactory, CodecConfig, CodecType, SampleRate};
use crate::codec::codec_core_adapter::CodecCoreAdapter;
use crate::codec::audio::common::AudioCodec;
use crate::error::Result;

pub struct CodecFactory;

impl CodecFactory {
    /// Create a codec instance using codec-core
    pub fn create_codec(payload_type: u8) -> Result<Box<dyn AudioCodec>> {
        let (codec_type, sample_rate) = match payload_type {
            0 => (CodecType::G711Pcmu, SampleRate::Rate8000),
            8 => (CodecType::G711Pcma, SampleRate::Rate8000),
            _ => return Err(Error::codec(format!("Unsupported payload type: {}", payload_type))),
        };
        
        let config = CodecConfig::new(codec_type)
            .with_sample_rate(sample_rate)
            .with_channels(1);
        
        let codec_core_codec = CoreFactory::create(config)
            .map_err(|e| Error::codec(format!("Failed to create codec: {}", e)))?;
        
        Ok(Box::new(CodecCoreAdapter::new(codec_core_codec)))
    }
}
```

#### 2.2 Update Transcoder to Use Factory
Modify `src/codec/transcoding.rs`:

```rust
// Replace direct codec creation with factory
fn create_codec(&self, payload_type: PayloadType) -> Result<Box<dyn AudioCodec>> {
    CodecFactory::create_codec(payload_type)
}
```

### Phase 3: Update Media Session (Week 2)

#### 3.1 Replace Direct G711Codec Usage
In `src/session/media_session.rs`:

```rust
// Before:
// let codec = Box::new(G711Codec::mu_law(SampleRate::Rate8000, 1).unwrap());

// After:
let codec = CodecFactory::create_codec(params.audio_params.payload_type)?;
```

### Phase 4: Maintain Zero-Copy Optimizations (Week 2)

#### 4.1 Create Zero-Copy Adapter
For performance-critical paths, create a zero-copy variant:

```rust
/// Zero-copy adapter for codec-core codecs that support AudioCodecExt
pub struct ZeroCopyCodecAdapter {
    inner: Box<dyn codec_core::types::AudioCodecExt>,
}

impl ZeroCopyCodecAdapter {
    pub fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        self.inner.encode_to_buffer(samples, output)
            .map_err(|e| Error::codec(format!("Zero-copy encoding failed: {}", e)))
    }
    
    pub fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        self.inner.decode_to_buffer(data, output)
            .map_err(|e| Error::codec(format!("Zero-copy decoding failed: {}", e)))
    }
}
```

### Phase 5: Remove Old Codec Implementations (Week 3)

#### 5.1 Deprecate Internal Codecs
1. Mark old G.711 implementations as deprecated
2. Update all references to use the new factory
3. Remove old implementations after verification

#### 5.2 Files to Remove/Update:
- Remove: `src/codec/g711.rs` (old implementation)
- Remove: `src/codec/audio/g711.rs`
- Update: `src/codec/mod.rs` (remove old exports, add new ones)
- Update: `src/relay/mod.rs` (remove G711PcmuCodec, G711PcmaCodec)

### Phase 6: Testing and Validation (Week 3)

#### 6.1 Compatibility Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_codec_core_g711_compatibility() {
        // Test that codec-core G.711 produces same output as old implementation
        let mut old_codec = G711Codec::mu_law(SampleRate::Rate8000, 1).unwrap();
        let mut new_codec = CodecFactory::create_codec(0).unwrap();
        
        let test_samples = vec![0i16; 160]; // 20ms at 8kHz
        let old_encoded = old_codec.encode(&test_frame).unwrap();
        let new_encoded = new_codec.encode(&test_frame).unwrap();
        
        assert_eq!(old_encoded, new_encoded);
    }
}
```

#### 6.2 Performance Benchmarks
- Compare encoding/decoding performance
- Verify zero-copy paths maintain performance
- Check memory allocation patterns

## Migration Strategy

### Backward Compatibility
1. Keep existing AudioCodec trait unchanged
2. Maintain same codec behavior and output
3. No changes to public media-core APIs

### Gradual Rollout
1. **Stage 1**: Add adapter layer alongside existing codecs
2. **Stage 2**: Switch to codec-core for new sessions
3. **Stage 3**: Migrate existing code paths
4. **Stage 4**: Remove old implementations

### Risk Mitigation
1. **Feature Flag**: Add `use-codec-core` feature flag for gradual adoption
2. **Parallel Testing**: Run both implementations in test environments
3. **Rollback Plan**: Keep old code available for quick reversion

## Benefits

1. **Separation of Concerns**: Codec logic isolated in codec-core
2. **Consistency**: Same codec implementation across all RVOIP components
3. **Maintainability**: Single place to fix codec bugs
4. **Extensibility**: Easy to add new codecs to codec-core
5. **Quality**: codec-core has comprehensive testing with real audio

## Success Criteria

1. ✅ All existing media-core tests pass with codec-core
2. ✅ Performance benchmarks show no regression
3. ✅ Zero-copy optimizations maintained
4. ✅ RTP streaming works correctly end-to-end
5. ✅ Seamless upgrade path for existing deployments

## Timeline

- **Week 1**: Implement adapter layer and factory
- **Week 2**: Update media session and transcoding
- **Week 3**: Testing, benchmarking, and cleanup
- **Week 4**: Documentation and migration guide

## Dependencies

- codec-core must be stable and well-tested
- No breaking changes to media-core public API
- Coordination with session-core team for testing

## Future Enhancements

Once the initial integration is complete:
1. Add support for new codecs (Opus, G.722) from codec-core
2. Implement codec negotiation improvements
3. Add codec-specific optimizations
4. Support for video codecs when available