//! G.711 Audio Codec Implementation
//!
//! This module implements the G.711 codec with both μ-law (PCMU) and A-law (PCMA)
//! variants. G.711 is the standard codec for telephony systems.
//!
//! ## Features
//!
//! - ITU-T G.711 compliant implementation
//! - Both A-law and μ-law encoding/decoding  
//! - Simple single-sample functions
//! - Lookup table optimized for performance
//!
//! ## Usage
//!
//! ### Direct Function Calls
//!
//! ```rust
//! use codec_core::codecs::g711::{alaw_compress, alaw_expand, ulaw_compress, ulaw_expand};
//!
//! // Single sample processing
//! let sample = 1024i16;
//! let alaw_encoded = alaw_compress(sample);
//! let alaw_decoded = alaw_expand(alaw_encoded);
//!
//! let ulaw_encoded = ulaw_compress(sample);
//! let ulaw_decoded = ulaw_expand(ulaw_encoded);
//! ```
//!
//! ### Processing Multiple Samples
//!
//! ```rust
//! use codec_core::codecs::g711::{alaw_compress, alaw_expand};
//!
//! let samples = vec![0i16, 100, -100, 1000, -1000];
//! let encoded: Vec<u8> = samples.iter().map(|&s| alaw_compress(s)).collect();
//! let decoded: Vec<i16> = encoded.iter().map(|&e| alaw_expand(e)).collect();
//! ```
//!
//! ### Using the G711Codec Struct
//!
//! ```rust
//! use codec_core::codecs::g711::{G711Codec, G711Variant};
//! use codec_core::types::{AudioCodec, CodecConfig, CodecType, SampleRate};
//!
//! // Create μ-law codec
//! let config = CodecConfig::new(CodecType::G711Pcmu)
//!     .with_sample_rate(SampleRate::Rate8000)
//!     .with_channels(1);
//! let mut codec = G711Codec::new_pcmu(config)?;
//!
//! // Or create A-law codec directly
//! let mut alaw_codec = G711Codec::new(G711Variant::ALaw);
//!
//! // Encode/decode
//! let samples = vec![0i16; 160]; // 20ms at 8kHz
//! let encoded = codec.encode(&samples)?;
//! let decoded = codec.decode(&encoded)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::error::CodecError;

mod reference;
mod tables;

#[cfg(test)]
pub mod tests;

// Re-export the core functions
pub use reference::{
    alaw_compress, alaw_expand, ulaw_compress, ulaw_expand
};

/// G.711 codec implementation
pub struct G711Codec {
    variant: G711Variant,
    frame_size: usize,
}

/// G.711 codec variants
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum G711Variant {
    /// A-law (PCMA) - Used primarily in Europe
    ALaw,
    /// μ-law (PCMU) - Used primarily in North America and Japan
    MuLaw,
}

impl G711Codec {
    /// Create a new G.711 codec with the specified variant
    pub fn new(variant: G711Variant) -> Self {
        Self { 
            variant, 
            frame_size: 160, // Default 20ms at 8kHz
        }
    }
    
    /// Create a new G.711 codec with configuration
    pub fn new_with_config(variant: G711Variant, config: crate::types::CodecConfig) -> Result<Self, CodecError> {
        // Validate sample rate
        if config.sample_rate.hz() != 8000 {
            return Err(CodecError::InvalidSampleRate {
                rate: config.sample_rate.hz(),
                supported: vec![8000],
            });
        }
        
        // Validate channels
        if config.channels != 1 {
            return Err(CodecError::InvalidChannelCount {
                channels: config.channels,
                supported: vec![1],
            });
        }
        
        // Calculate frame size from config
        let frame_size = if let Some(frame_ms) = config.frame_size_ms {
            let samples_per_ms = 8000.0 / 1000.0; // 8 samples per ms at 8kHz
            (samples_per_ms * frame_ms) as usize
        } else {
            160 // Default 20ms
        };
        
        // Validate frame size
        let valid_sizes = [80, 160, 240, 320];
        if !valid_sizes.contains(&frame_size) {
            return Err(CodecError::InvalidFrameSize {
                expected: 160,
                actual: frame_size,
            });
        }
        
        Ok(Self { variant, frame_size })
    }
    
    /// Create a new G.711 μ-law (PCMU) codec
    pub fn new_pcmu(config: crate::types::CodecConfig) -> Result<Self, CodecError> {
        Self::new_with_config(G711Variant::MuLaw, config)
    }
    
    /// Create a new G.711 A-law (PCMA) codec
    pub fn new_pcma(config: crate::types::CodecConfig) -> Result<Self, CodecError> {
        Self::new_with_config(G711Variant::ALaw, config)
    }
    
    /// Get the codec variant
    pub fn variant(&self) -> G711Variant {
        self.variant
    }
    
    /// Compress samples using the configured variant
    pub fn compress(&self, samples: &[i16]) -> Result<Vec<u8>, CodecError> {
        match self.variant {
            G711Variant::ALaw => Ok(samples.iter().map(|&sample| alaw_compress(sample)).collect()),
            G711Variant::MuLaw => Ok(samples.iter().map(|&sample| ulaw_compress(sample)).collect()),
        }
    }
    
    /// Expand samples using the configured variant
    pub fn expand(&self, compressed: &[u8]) -> Result<Vec<i16>, CodecError> {
        match self.variant {
            G711Variant::ALaw => Ok(compressed.iter().map(|&sample| alaw_expand(sample)).collect()),
            G711Variant::MuLaw => Ok(compressed.iter().map(|&sample| ulaw_expand(sample)).collect()),
        }
    }
    
    /// Compress samples using A-law
    pub fn compress_alaw(&self, samples: &[i16]) -> Result<Vec<u8>, CodecError> {
        Ok(samples.iter().map(|&sample| alaw_compress(sample)).collect())
    }
    
    /// Expand A-law samples
    pub fn expand_alaw(&self, compressed: &[u8]) -> Result<Vec<i16>, CodecError> {
        Ok(compressed.iter().map(|&sample| alaw_expand(sample)).collect())
    }
    
    /// Compress samples using μ-law
    pub fn compress_ulaw(&self, samples: &[i16]) -> Result<Vec<u8>, CodecError> {
        Ok(samples.iter().map(|&sample| ulaw_compress(sample)).collect())
    }
    
    /// Expand μ-law samples
    pub fn expand_ulaw(&self, compressed: &[u8]) -> Result<Vec<i16>, CodecError> {
        Ok(compressed.iter().map(|&sample| ulaw_expand(sample)).collect())
    }
}

// Implement AudioCodec trait for backward compatibility
impl crate::types::AudioCodec for G711Codec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>, CodecError> {
        self.compress(samples)
    }

    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>, CodecError> {
        self.expand(data)
    }

    fn info(&self) -> crate::types::CodecInfo {
        let (name, payload_type) = match self.variant {
            G711Variant::ALaw => ("PCMA", Some(8)),
            G711Variant::MuLaw => ("PCMU", Some(0)),
        };
        
        crate::types::CodecInfo {
            name,
            sample_rate: 8000,
            channels: 1,
            bitrate: 64000,
            frame_size: self.frame_size,
            payload_type,
        }
    }

    fn reset(&mut self) -> Result<(), CodecError> {
        // G.711 is stateless, no reset needed
        Ok(())
    }

    fn frame_size(&self) -> usize {
        self.frame_size
    }

    fn supports_variable_frame_size(&self) -> bool {
        true
    }
}

// Implement AudioCodecExt trait for additional functionality
impl crate::types::AudioCodecExt for G711Codec {
    fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize, CodecError> {
        if output.len() < samples.len() {
            return Err(CodecError::BufferTooSmall { needed: samples.len(), actual: output.len() });
        }
        
        match self.variant {
            G711Variant::ALaw => {
                for (i, &sample) in samples.iter().enumerate() {
                    output[i] = alaw_compress(sample);
                }
            }
            G711Variant::MuLaw => {
                for (i, &sample) in samples.iter().enumerate() {
                    output[i] = ulaw_compress(sample);
                }
            }
        }
        
        Ok(samples.len())
    }

    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize, CodecError> {
        if output.len() < data.len() {
            return Err(CodecError::BufferTooSmall { needed: data.len(), actual: output.len() });
        }
        
        match self.variant {
            G711Variant::ALaw => {
                for (i, &encoded) in data.iter().enumerate() {
                    output[i] = alaw_expand(encoded);
                }
            }
            G711Variant::MuLaw => {
                for (i, &encoded) in data.iter().enumerate() {
                    output[i] = ulaw_expand(encoded);
                }
            }
        }
        
        Ok(data.len())
    }

    fn max_encoded_size(&self, input_samples: usize) -> usize {
        input_samples // G.711 has 1:1 sample to byte ratio
    }

    fn max_decoded_size(&self, input_bytes: usize) -> usize {
        input_bytes // G.711 has 1:1 byte to sample ratio
    }
}

/// Initialize G.711 lookup tables (stub for compatibility)
pub fn init_tables() {
    // No initialization needed with our implementation
}
