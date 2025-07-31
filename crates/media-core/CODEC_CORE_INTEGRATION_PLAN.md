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
2. **Direct Integration**: Replace internal codec implementations directly with codec-core
3. **Performance**: Maintain zero-copy optimizations where possible
4. **Extensibility**: Easy to add new codecs from codec-core
5. **Error Context**: Preserve detailed error information from codec-core

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
│  │      G711Codec (UPDATED)                │    │
│  │                                          │    │
│  │  • Implements media-core AudioCodec      │    │
│  │  • Directly uses codec-core internally   │    │
│  │  • No intermediate adapter layer         │    │
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

### Phase 1: Update G711 Codec Implementation (Week 1)

#### 1.1 Add codec-core Dependency
```toml
# In media-core/Cargo.toml
[dependencies]
codec-core = { path = "../codec-core" }
```

#### 1.2 Replace G711 Implementation
Update `src/codec/audio/g711.rs` to use codec-core directly:

```rust
use codec_core::codecs::g711::{G711Decoder, G711Encoder, G711Law};
use crate::codec::audio::common::{AudioCodec, CodecInfo};
use crate::types::AudioFrame;
use crate::error::{Error, Result};

/// G.711 codec implementation using codec-core
pub struct G711Codec {
    encoder: G711Encoder,
    decoder: G711Decoder,
    law: G711Law,
    sample_rate: u32,
    channels: u16,
}

impl G711Codec {
    pub fn new(law: G711Law, sample_rate: u32, channels: u16) -> Result<Self> {
        Ok(Self {
            encoder: G711Encoder::new(law),
            decoder: G711Decoder::new(law),
            law,
            sample_rate,
            channels,
        })
    }
    
    pub fn mu_law(sample_rate: u32, channels: u16) -> Result<Self> {
        Self::new(G711Law::MuLaw, sample_rate, channels)
    }
    
    pub fn a_law(sample_rate: u32, channels: u16) -> Result<Self> {
        Self::new(G711Law::ALaw, sample_rate, channels)
    }
}

impl AudioCodec for G711Codec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        self.encoder.encode(&audio_frame.samples)
            .map_err(|e| Error::codec(format!("G.711 {} encoding failed: {}", 
                match self.law {
                    G711Law::MuLaw => "μ-law",
                    G711Law::ALaw => "A-law",
                }, e)))
    }
    
    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        let samples = self.decoder.decode(encoded_data)
            .map_err(|e| Error::codec(format!("G.711 {} decoding failed: {}", 
                match self.law {
                    G711Law::MuLaw => "μ-law",
                    G711Law::ALaw => "A-law",
                }, e)))?;
        
        Ok(AudioFrame::new(
            samples,
            self.sample_rate,
            self.channels,
            0, // timestamp will be set by caller
        ))
    }
    
    fn get_info(&self) -> CodecInfo {
        CodecInfo {
            name: match self.law {
                G711Law::MuLaw => "G.711 μ-law".to_string(),
                G711Law::ALaw => "G.711 A-law".to_string(),
            },
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: self.sample_rate * 8 * self.channels as u32, // 8 bits per sample
        }
    }
    
    fn reset(&mut self) {
        // G.711 is stateless, but we could reinitialize if needed
        self.encoder = G711Encoder::new(self.law);
        self.decoder = G711Decoder::new(self.law);
    }
}
```

### Phase 2: Update Codec Factory (Week 1)

#### 2.1 Create Codec Factory
Update `src/codec/factory.rs` (new file):

```rust
use codec_core::codecs::g711::G711Law;
use crate::codec::audio::g711::G711Codec;
use crate::codec::audio::common::AudioCodec;
use crate::error::{Error, Result};

pub struct CodecFactory;

impl CodecFactory {
    /// Create a codec instance with configurable parameters
    pub fn create_codec(
        payload_type: u8, 
        sample_rate: Option<u32>, 
        channels: Option<u16>
    ) -> Result<Box<dyn AudioCodec>> {
        // Default values if not specified
        let sample_rate = sample_rate.unwrap_or(8000);
        let channels = channels.unwrap_or(1);
        
        match payload_type {
            0 => Ok(Box::new(G711Codec::mu_law(sample_rate, channels)?)),
            8 => Ok(Box::new(G711Codec::a_law(sample_rate, channels)?)),
            _ => Err(Error::codec(format!("Unsupported payload type: {}", payload_type))),
        }
    }
    
    /// Create codec with standard RTP payload type defaults
    pub fn create_codec_default(payload_type: u8) -> Result<Box<dyn AudioCodec>> {
        Self::create_codec(payload_type, None, None)
    }
}
```

#### 2.2 Update Transcoder to Use Factory
Modify `src/codec/transcoding.rs`:

```rust
// Replace direct codec creation with factory
fn create_codec(&self, payload_type: PayloadType) -> Result<Box<dyn AudioCodec>> {
    CodecFactory::create_codec_default(payload_type)
}
```

### Phase 3: Update Media Session (Week 2)

#### 3.1 Replace Direct G711Codec Usage
In `src/session/media_session.rs`:

```rust
// Before:
// let codec = Box::new(G711Codec::mu_law(SampleRate::Rate8000, 1).unwrap());

// After:
let codec = CodecFactory::create_codec(
    params.audio_params.payload_type,
    Some(params.audio_params.sample_rate),
    Some(params.audio_params.channels)
)?;
```

### Phase 4: Maintain Zero-Copy Optimizations (Week 2)

#### 4.1 Extend G711Codec with Zero-Copy Methods
Add zero-copy methods to `src/codec/audio/g711.rs`:

```rust
impl G711Codec {
    /// Zero-copy encode directly to output buffer
    pub fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        self.encoder.encode_to_buffer(samples, output)
            .map_err(|e| Error::codec(format!("G.711 {} zero-copy encoding failed: {}", 
                match self.law {
                    G711Law::MuLaw => "μ-law",
                    G711Law::ALaw => "A-law",
                }, e)))
    }
    
    /// Zero-copy decode directly to output buffer
    pub fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        self.decoder.decode_to_buffer(data, output)
            .map_err(|e| Error::codec(format!("G.711 {} zero-copy decoding failed: {}", 
                match self.law {
                    G711Law::MuLaw => "μ-law",
                    G711Law::ALaw => "A-law",
                }, e)))
    }
}
```

#### 4.2 Update Zero-Copy Controller
In `src/relay/controller/zero_copy.rs`, update to use the new zero-copy methods directly.

### Phase 5: Remove Old Codec Implementations (Week 3)

#### 5.1 Cleanup Steps
1. Remove old internal G.711 implementation from `src/codec/g711.rs`
2. Update all imports and exports
3. Remove passthrough codec implementations

#### 5.2 Files to Update:
- Remove: `src/codec/g711.rs` (old internal implementation)
- Update: `src/codec/mod.rs` (remove old exports, add factory export)
- Update: `src/relay/mod.rs` (remove G711PcmuCodec, G711PcmaCodec)
- Keep: `src/codec/audio/g711.rs` (now uses codec-core internally)

### Phase 6: Testing and Validation (Week 3)

#### 6.1 Compatibility Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_codec_core_g711_compatibility() {
        // Test that codec-core G.711 produces same output as old implementation
        let mut codec = G711Codec::mu_law(8000, 1).unwrap();
        
        // Test standard patterns
        let test_patterns = vec![
            vec![0i16; 160],          // Silence
            vec![1000i16; 160],       // Constant tone
            (0..160).map(|i| (i * 100) as i16).collect(), // Linear ramp
        ];
        
        for samples in test_patterns {
            let frame = AudioFrame::new(samples.clone(), 8000, 1, 0);
            let encoded = codec.encode(&frame).unwrap();
            let decoded = codec.decode(&encoded).unwrap();
            
            // Verify round-trip
            assert_eq!(decoded.samples.len(), samples.len());
        }
    }
    
    #[test]
    fn test_g711_features() {
        // Test specific G.711 features like:
        // - Silence suppression patterns
        // - Maximum/minimum values
        // - Bit-exact compliance with ITU-T G.711
    }
    
    #[test]
    fn test_error_context() {
        let mut codec = G711Codec::mu_law(8000, 1).unwrap();
        
        // Test that errors contain proper context
        let result = codec.decode(&[]);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("G.711"));
        assert!(err_msg.contains("μ-law"));
    }
}
```

#### 6.2 Performance Benchmarks
- Compare encoding/decoding performance
- Verify zero-copy paths maintain performance
- Check memory allocation patterns
- Benchmark against reference G.711 implementations

#### 6.3 Integration Tests
- Test with real RTP streams
- Verify interoperability with external systems
- Test transcoding between μ-law and A-law

## Migration Strategy

### Backward Compatibility
1. Keep existing AudioCodec trait unchanged
2. Maintain same codec behavior and output
3. No changes to public media-core APIs

### Gradual Rollout
1. **Stage 1**: Update G711Codec to use codec-core internally
2. **Stage 2**: Add factory for codec creation
3. **Stage 3**: Migrate all code paths to use factory
4. **Stage 4**: Remove old internal G.711 implementation

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

- **Week 1**: Update G711Codec implementation and create factory
- **Week 2**: Update media session, transcoding, and zero-copy paths
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