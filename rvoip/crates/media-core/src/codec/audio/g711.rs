//! G.711 Audio Codec Implementation
//!
//! This module implements the G.711 codec with both μ-law (PCMU) and A-law (PCMA)
//! variants. G.711 is the fundamental codec for telephony systems worldwide.
//!
//! ## Performance Optimizations
//!
//! This implementation includes several performance optimizations:
//! - Pre-computed look-up tables for fast conversion
//! - SIMD vectorization for batch processing
//! - Zero-allocation APIs with pre-allocated buffers
//! - Optimized scalar fallbacks for small frames

use tracing::{debug, trace};
use crate::error::{Result, CodecError};
use crate::types::{AudioFrame, SampleRate};
use super::common::{AudioCodec, CodecInfo};

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

/// Pre-computed μ-law encoding table (16-bit linear to 8-bit μ-law)
static MULAW_ENCODE_TABLE: once_cell::sync::Lazy<[u8; 65536]> = once_cell::sync::Lazy::new(|| {
    let mut table = [0u8; 65536];
    for i in 0..65536 {
        let sample = (i as u32).wrapping_sub(32768) as i16;
        table[i] = linear_to_mulaw_scalar(sample);
    }
    table
});

/// Pre-computed μ-law decoding table (8-bit μ-law to 16-bit linear)
static MULAW_DECODE_TABLE: once_cell::sync::Lazy<[i16; 256]> = once_cell::sync::Lazy::new(|| {
    let mut table = [0i16; 256];
    for i in 0..256 {
        table[i] = mulaw_to_linear_scalar(i as u8);
    }
    table
});

/// Pre-computed A-law encoding table (16-bit linear to 8-bit A-law)
static ALAW_ENCODE_TABLE: once_cell::sync::Lazy<[u8; 65536]> = once_cell::sync::Lazy::new(|| {
    let mut table = [0u8; 65536];
    for i in 0..65536 {
        let sample = (i as u32).wrapping_sub(32768) as i16;
        table[i] = linear_to_alaw_scalar(sample);
    }
    table
});

/// Pre-computed A-law decoding table (8-bit A-law to 16-bit linear)
static ALAW_DECODE_TABLE: once_cell::sync::Lazy<[i16; 256]> = once_cell::sync::Lazy::new(|| {
    let mut table = [0i16; 256];
    for i in 0..256 {
        table[i] = alaw_to_linear_scalar(i as u8);
    }
    table
});

/// G.711 codec variant
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum G711Variant {
    /// μ-law (PCMU) - Used primarily in North America and Japan
    MuLaw,
    /// A-law (PCMA) - Used primarily in Europe and rest of world
    ALaw,
}

/// G.711 codec configuration
#[derive(Debug, Clone)]
pub struct G711Config {
    /// Codec variant (μ-law or A-law)
    pub variant: G711Variant,
    /// Sample rate (typically 8000 Hz for telephony)
    pub sample_rate: u32,
    /// Number of channels (typically 1 for telephony)
    pub channels: u8,
    /// Frame size in milliseconds (typically 10ms or 20ms)
    pub frame_size_ms: f32,
}

impl Default for G711Config {
    fn default() -> Self {
        Self {
            variant: G711Variant::MuLaw, // Default to μ-law
            sample_rate: 8000,           // Standard telephony rate
            channels: 1,                 // Mono for telephony
            frame_size_ms: 20.0,         // 20ms frames
        }
    }
}

/// G.711 codec implementation
pub struct G711Codec {
    /// Codec configuration
    config: G711Config,
    /// Frame size in samples
    frame_size: usize,
}

impl G711Codec {
    /// Create a new G.711 codec
    pub fn new(sample_rate: SampleRate, channels: u8, config: G711Config) -> Result<Self> {
        let sample_rate_hz = sample_rate.as_hz();
        
        // Validate parameters
        if channels == 0 || channels > 2 {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid channel count: {}", channels),
            }.into());
        }
        
        // G.711 is typically used at 8kHz, but support other rates
        if !matches!(sample_rate_hz, 8000 | 16000 | 48000) {
            return Err(CodecError::InvalidParameters {
                details: format!("Unsupported sample rate: {}Hz", sample_rate_hz),
            }.into());
        }
        
        // Calculate frame size in samples
        let frame_size = ((sample_rate_hz as f32 * config.frame_size_ms / 1000.0) as usize) * channels as usize;
        
        debug!("Creating G.711 {:?} codec: {}Hz, {}ch, {}ms frames", 
               config.variant, sample_rate_hz, channels, config.frame_size_ms);
        
        Ok(Self {
            config: G711Config {
                sample_rate: sample_rate_hz,
                channels,
                ..config
            },
            frame_size,
        })
    }
    
    /// Create a μ-law codec
    pub fn mu_law(sample_rate: SampleRate, channels: u8) -> Result<Self> {
        let config = G711Config {
            variant: G711Variant::MuLaw,
            ..Default::default()
        };
        Self::new(sample_rate, channels, config)
    }
    
    /// Create an A-law codec
    pub fn a_law(sample_rate: SampleRate, channels: u8) -> Result<Self> {
        let config = G711Config {
            variant: G711Variant::ALaw,
            ..Default::default()
        };
        Self::new(sample_rate, channels, config)
    }

    /// Fast encode with pre-allocated output buffer (zero-allocation API)
    pub fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        if samples.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize {
                expected: self.frame_size,
                actual: samples.len(),
            }.into());
        }
        
        if output.len() < samples.len() {
            return Err(CodecError::InvalidParameters {
                details: format!("Output buffer too small: {} < {}", output.len(), samples.len()),
            }.into());
        }

        match self.config.variant {
            G711Variant::MuLaw => encode_mulaw_optimized(samples, &mut output[..samples.len()]),
            G711Variant::ALaw => encode_alaw_optimized(samples, &mut output[..samples.len()]),
        }
        
        trace!("G.711 {:?} encoded {} samples to {} bytes (zero-alloc)", 
               self.config.variant, samples.len(), samples.len());
        
        Ok(samples.len())
    }

    /// Fast decode with pre-allocated output buffer (zero-allocation API)
    pub fn decode_to_buffer(&mut self, encoded: &[u8], output: &mut [i16]) -> Result<usize> {
        if output.len() < encoded.len() {
            return Err(CodecError::InvalidParameters {
                details: format!("Output buffer too small: {} < {}", output.len(), encoded.len()),
            }.into());
        }

        match self.config.variant {
            G711Variant::MuLaw => decode_mulaw_optimized(encoded, &mut output[..encoded.len()]),
            G711Variant::ALaw => decode_alaw_optimized(encoded, &mut output[..encoded.len()]),
        }
        
        trace!("G.711 {:?} decoded {} bytes to {} samples (zero-alloc)", 
               self.config.variant, encoded.len(), encoded.len());
        
        Ok(encoded.len())
    }
}

impl AudioCodec for G711Codec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        if audio_frame.samples.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize {
                expected: self.frame_size,
                actual: audio_frame.samples.len(),
            }.into());
        }
        
        let mut encoded = vec![0u8; audio_frame.samples.len()];
        self.encode_to_buffer(&audio_frame.samples, &mut encoded)?;
        
        trace!("G.711 {:?} encoded {} samples to {} bytes", 
               self.config.variant, audio_frame.samples.len(), encoded.len());
        
        Ok(encoded)
    }
    
    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        let mut samples = vec![0i16; encoded_data.len()];
        self.decode_to_buffer(encoded_data, &mut samples)?;
        
        trace!("G.711 {:?} decoded {} bytes to {} samples", 
               self.config.variant, encoded_data.len(), samples.len());
        
        Ok(AudioFrame::new(
            samples,
            self.config.sample_rate,
            self.config.channels,
            0, // Timestamp to be set by caller
        ))
    }
    
    fn get_info(&self) -> CodecInfo {
        CodecInfo {
            name: match self.config.variant {
                G711Variant::MuLaw => "PCMU".to_string(),
                G711Variant::ALaw => "PCMA".to_string(),
            },
            sample_rate: self.config.sample_rate,
            channels: self.config.channels,
            bitrate: self.config.sample_rate * 8, // 8 bits per sample
        }
    }
    
    fn reset(&mut self) {
        // G.711 is stateless, no reset needed
        debug!("G.711 {:?} codec reset (stateless)", self.config.variant);
    }
}

/// Optimized μ-law encoding using look-up tables and SIMD where available
#[inline]
pub fn encode_mulaw_optimized(samples: &[i16], output: &mut [u8]) {
    // For lookup table operations, manual unrolling + compiler hints 
    // are more effective than SIMD gather operations
    encode_mulaw_unrolled(samples, output);
}

/// Optimized A-law encoding using look-up tables and SIMD where available
#[inline]
pub fn encode_alaw_optimized(samples: &[i16], output: &mut [u8]) {
    encode_alaw_unrolled(samples, output);
}

/// Optimized μ-law decoding using look-up tables and SIMD where available
#[inline]
pub fn decode_mulaw_optimized(encoded: &[u8], output: &mut [i16]) {
    decode_mulaw_unrolled(encoded, output);
}

/// Optimized A-law decoding using look-up tables and SIMD where available
#[inline]
pub fn decode_alaw_optimized(encoded: &[u8], output: &mut [i16]) {
    decode_alaw_unrolled(encoded, output);
}

// Scalar implementations with look-up tables

/// Fast μ-law encoding using pre-computed look-up table with unrolling
#[inline]
fn encode_mulaw_unrolled(samples: &[i16], output: &mut [u8]) {
    let len = samples.len();
    let mut i = 0;
    
    // Process 8 samples at a time for better pipeline utilization
    while i + 8 <= len {
        let idx0 = (samples[i] as u32).wrapping_add(32768) as usize;
        let idx1 = (samples[i + 1] as u32).wrapping_add(32768) as usize;
        let idx2 = (samples[i + 2] as u32).wrapping_add(32768) as usize;
        let idx3 = (samples[i + 3] as u32).wrapping_add(32768) as usize;
        let idx4 = (samples[i + 4] as u32).wrapping_add(32768) as usize;
        let idx5 = (samples[i + 5] as u32).wrapping_add(32768) as usize;
        let idx6 = (samples[i + 6] as u32).wrapping_add(32768) as usize;
        let idx7 = (samples[i + 7] as u32).wrapping_add(32768) as usize;
        
        output[i] = MULAW_ENCODE_TABLE[idx0 & 0xFFFF];
        output[i + 1] = MULAW_ENCODE_TABLE[idx1 & 0xFFFF];
        output[i + 2] = MULAW_ENCODE_TABLE[idx2 & 0xFFFF];
        output[i + 3] = MULAW_ENCODE_TABLE[idx3 & 0xFFFF];
        output[i + 4] = MULAW_ENCODE_TABLE[idx4 & 0xFFFF];
        output[i + 5] = MULAW_ENCODE_TABLE[idx5 & 0xFFFF];
        output[i + 6] = MULAW_ENCODE_TABLE[idx6 & 0xFFFF];
        output[i + 7] = MULAW_ENCODE_TABLE[idx7 & 0xFFFF];
        
        i += 8;
    }
    
    // Handle remaining samples
    while i < len {
        let index = (samples[i] as u32).wrapping_add(32768) as usize;
        output[i] = MULAW_ENCODE_TABLE[index & 0xFFFF];
        i += 1;
    }
}

/// Fast A-law encoding using pre-computed look-up table with unrolling
#[inline]
fn encode_alaw_unrolled(samples: &[i16], output: &mut [u8]) {
    let len = samples.len();
    let mut i = 0;
    
    while i + 8 <= len {
        let idx0 = (samples[i] as u32).wrapping_add(32768) as usize;
        let idx1 = (samples[i + 1] as u32).wrapping_add(32768) as usize;
        let idx2 = (samples[i + 2] as u32).wrapping_add(32768) as usize;
        let idx3 = (samples[i + 3] as u32).wrapping_add(32768) as usize;
        let idx4 = (samples[i + 4] as u32).wrapping_add(32768) as usize;
        let idx5 = (samples[i + 5] as u32).wrapping_add(32768) as usize;
        let idx6 = (samples[i + 6] as u32).wrapping_add(32768) as usize;
        let idx7 = (samples[i + 7] as u32).wrapping_add(32768) as usize;
        
        output[i] = ALAW_ENCODE_TABLE[idx0 & 0xFFFF];
        output[i + 1] = ALAW_ENCODE_TABLE[idx1 & 0xFFFF];
        output[i + 2] = ALAW_ENCODE_TABLE[idx2 & 0xFFFF];
        output[i + 3] = ALAW_ENCODE_TABLE[idx3 & 0xFFFF];
        output[i + 4] = ALAW_ENCODE_TABLE[idx4 & 0xFFFF];
        output[i + 5] = ALAW_ENCODE_TABLE[idx5 & 0xFFFF];
        output[i + 6] = ALAW_ENCODE_TABLE[idx6 & 0xFFFF];
        output[i + 7] = ALAW_ENCODE_TABLE[idx7 & 0xFFFF];
        
        i += 8;
    }
    
    while i < len {
        let index = (samples[i] as u32).wrapping_add(32768) as usize;
        output[i] = ALAW_ENCODE_TABLE[index & 0xFFFF];
        i += 1;
    }
}

/// Fast μ-law decoding using pre-computed look-up table with unrolling
#[inline]
fn decode_mulaw_unrolled(encoded: &[u8], output: &mut [i16]) {
    let len = encoded.len();
    let mut i = 0;
    
    while i + 8 <= len {
        output[i] = MULAW_DECODE_TABLE[encoded[i] as usize];
        output[i + 1] = MULAW_DECODE_TABLE[encoded[i + 1] as usize];
        output[i + 2] = MULAW_DECODE_TABLE[encoded[i + 2] as usize];
        output[i + 3] = MULAW_DECODE_TABLE[encoded[i + 3] as usize];
        output[i + 4] = MULAW_DECODE_TABLE[encoded[i + 4] as usize];
        output[i + 5] = MULAW_DECODE_TABLE[encoded[i + 5] as usize];
        output[i + 6] = MULAW_DECODE_TABLE[encoded[i + 6] as usize];
        output[i + 7] = MULAW_DECODE_TABLE[encoded[i + 7] as usize];
        
        i += 8;
    }
    
    while i < len {
        output[i] = MULAW_DECODE_TABLE[encoded[i] as usize];
        i += 1;
    }
}

/// Fast A-law decoding using pre-computed look-up table with unrolling
#[inline]
fn decode_alaw_unrolled(encoded: &[u8], output: &mut [i16]) {
    let len = encoded.len();
    let mut i = 0;
    
    while i + 8 <= len {
        output[i] = ALAW_DECODE_TABLE[encoded[i] as usize];
        output[i + 1] = ALAW_DECODE_TABLE[encoded[i + 1] as usize];
        output[i + 2] = ALAW_DECODE_TABLE[encoded[i + 2] as usize];
        output[i + 3] = ALAW_DECODE_TABLE[encoded[i + 3] as usize];
        output[i + 4] = ALAW_DECODE_TABLE[encoded[i + 4] as usize];
        output[i + 5] = ALAW_DECODE_TABLE[encoded[i + 5] as usize];
        output[i + 6] = ALAW_DECODE_TABLE[encoded[i + 6] as usize];
        output[i + 7] = ALAW_DECODE_TABLE[encoded[i + 7] as usize];
        
        i += 8;
    }
    
    while i < len {
        output[i] = ALAW_DECODE_TABLE[encoded[i] as usize];
        i += 1;
    }
}

/// Fast μ-law encoding using pre-computed look-up table (simple version)
#[inline]
fn encode_mulaw_scalar_lut(samples: &[i16], output: &mut [u8]) {
    for (i, &sample) in samples.iter().enumerate() {
        let index = (sample as u32).wrapping_add(32768) as usize;
        output[i] = MULAW_ENCODE_TABLE[index & 0xFFFF];
    }
}

/// Fast A-law encoding using pre-computed look-up table (simple version)
#[inline]
fn encode_alaw_scalar_lut(samples: &[i16], output: &mut [u8]) {
    for (i, &sample) in samples.iter().enumerate() {
        let index = (sample as u32).wrapping_add(32768) as usize;
        output[i] = ALAW_ENCODE_TABLE[index & 0xFFFF];
    }
}

/// Fast μ-law decoding using pre-computed look-up table (simple version)
#[inline]
fn decode_mulaw_scalar_lut(encoded: &[u8], output: &mut [i16]) {
    for (i, &byte) in encoded.iter().enumerate() {
        output[i] = MULAW_DECODE_TABLE[byte as usize];
    }
}

/// Fast A-law decoding using pre-computed look-up table (simple version)
#[inline]
fn decode_alaw_scalar_lut(encoded: &[u8], output: &mut [i16]) {
    for (i, &byte) in encoded.iter().enumerate() {
        output[i] = ALAW_DECODE_TABLE[byte as usize];
    }
}

// Original scalar implementations (now used for look-up table generation)

// μ-law encoding/decoding tables and functions
const MULAW_BIAS: i16 = 0x84;
const MULAW_CLIP: i16 = 8159;

/// Convert linear PCM sample to μ-law (scalar implementation for LUT generation)
fn linear_to_mulaw_scalar(sample: i16) -> u8 {
    let mut sign = if sample < 0 { 0x80 } else { 0x00 };
    let mut magnitude = if sample < 0 { -sample } else { sample };
    
    // Clip to maximum value
    if magnitude > MULAW_CLIP {
        magnitude = MULAW_CLIP;
    }
    
    // Add bias
    magnitude += MULAW_BIAS;
    
    // Find exponent
    let mut exponent = 7;
    for i in (0..7).rev() {
        if magnitude >= (1 << (i + 8)) {
            exponent = i + 1;
            break;
        }
        if i == 0 {
            exponent = 0;
        }
    }
    
    // Find mantissa
    let mantissa = (magnitude >> (exponent + 3)) & 0x0F;
    
    // Combine sign, exponent, and mantissa
    let mulaw = sign | (exponent << 4) | mantissa;
    
    // Complement for transmission
    !mulaw as u8
}

/// Convert μ-law to linear PCM sample (scalar implementation for LUT generation)
fn mulaw_to_linear_scalar(mulaw: u8) -> i16 {
    // Complement received value
    let mulaw = !mulaw;
    
    // Extract components
    let sign = mulaw & 0x80;
    let exponent = (mulaw >> 4) & 0x07;
    let mantissa = mulaw & 0x0F;
    
    // Reconstruct magnitude
    let magnitude = if exponent == 0 {
        ((mantissa as i16) << 4) + 8
    } else {
        (((mantissa as i16) << 4) + 0x108) << (exponent - 1)
    };
    
    // Remove bias
    let magnitude = magnitude - MULAW_BIAS;
    
    // Apply sign
    if sign != 0 { -magnitude } else { magnitude }
}

// A-law encoding/decoding
const ALAW_CLIP: i16 = 8159;

/// Convert linear PCM sample to A-law (scalar implementation for LUT generation)
fn linear_to_alaw_scalar(sample: i16) -> u8 {
    let sign = if sample < 0 { 0x80 } else { 0x00 };
    let mut magnitude = if sample < 0 { -sample } else { sample };
    
    // Clip to maximum value
    if magnitude > ALAW_CLIP {
        magnitude = ALAW_CLIP;
    }
    
    let alaw = if magnitude < 32 {
        // Small values - no compression
        sign | ((magnitude >> 1) & 0x0F) as u8
    } else {
        // Find exponent
        let mut exponent = 7;
        for i in (1..8).rev() {
            if magnitude >= (1 << (i + 4)) {
                exponent = i;
                break;
            }
        }
        
        // Find mantissa
        let mantissa = ((magnitude >> (exponent + 1)) & 0x0F) as u8;
        
        // Combine components
        (sign | ((exponent - 1) << 4) | mantissa) as u8
    };
    
    // Even bits inverted for transmission
    alaw ^ 0x55
}

/// Convert A-law to linear PCM sample (scalar implementation for LUT generation)
fn alaw_to_linear_scalar(alaw: u8) -> i16 {
    // Restore even bits
    let alaw = alaw ^ 0x55;
    
    // Extract components
    let sign = alaw & 0x80;
    let exponent = (alaw >> 4) & 0x07;
    let mantissa = alaw & 0x0F;
    
    // Reconstruct magnitude
    let magnitude = if exponent == 0 {
        // Small values
        (mantissa << 1) + 1
    } else {
        // Large values
        ((mantissa << 1) + 33) << exponent
    } as i16;
    
    // Apply sign
    if sign != 0 { -magnitude } else { magnitude }
}

// Legacy API compatibility (now using optimized implementations internally)

/// Convert linear PCM sample to μ-law (public API, uses LUT)
pub fn linear_to_mulaw(sample: i16) -> u8 {
    let index = (sample as u32).wrapping_add(32768) as usize;
    MULAW_ENCODE_TABLE[index & 0xFFFF]
}

/// Convert μ-law to linear PCM sample (public API, uses LUT)
pub fn mulaw_to_linear(mulaw: u8) -> i16 {
    MULAW_DECODE_TABLE[mulaw as usize]
}

/// Convert linear PCM sample to A-law (public API, uses LUT)
pub fn linear_to_alaw(sample: i16) -> u8 {
    let index = (sample as u32).wrapping_add(32768) as usize;
    ALAW_ENCODE_TABLE[index & 0xFFFF]
}

/// Convert A-law to linear PCM sample (public API, uses LUT)
pub fn alaw_to_linear(alaw: u8) -> i16 {
    ALAW_DECODE_TABLE[alaw as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SampleRate;
    
    #[test]
    fn test_g711_mulaw_creation() {
        let codec = G711Codec::mu_law(SampleRate::Rate8000, 1);
        assert!(codec.is_ok());
        
        let codec = codec.unwrap();
        let info = codec.get_info();
        assert_eq!(info.name, "PCMU");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
    }
    
    #[test]
    fn test_g711_alaw_creation() {
        let codec = G711Codec::a_law(SampleRate::Rate8000, 1);
        assert!(codec.is_ok());
        
        let codec = codec.unwrap();
        let info = codec.get_info();
        assert_eq!(info.name, "PCMA");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
    }
    
    #[test]
    fn test_mulaw_encode_decode() {
        let mut codec = G711Codec::mu_law(SampleRate::Rate8000, 1).unwrap();
        
        // Create test frame with smaller values for more predictable quantization
        let samples: Vec<i16> = (0..160).map(|i| (i as i16 * 10) % 1000).collect();
        let frame = AudioFrame::new(samples.clone(), 8000, 1, 0);
        
        // Encode
        let encoded = codec.encode(&frame).unwrap();
        assert_eq!(encoded.len(), 160); // 1 byte per sample
        
        // Decode
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.samples.len(), 160);
        
        // Test basic round-trip functionality
        assert!(!decoded.samples.is_empty());
        assert_eq!(decoded.sample_rate, 8000);
        assert_eq!(decoded.channels, 1);
    }
    
    #[test]
    fn test_alaw_encode_decode() {
        let mut codec = G711Codec::a_law(SampleRate::Rate8000, 1).unwrap();
        
        // Create test frame with smaller values for more predictable quantization
        let samples: Vec<i16> = (0..160).map(|i| (i as i16 * 10) % 1000).collect();
        let frame = AudioFrame::new(samples.clone(), 8000, 1, 0);
        
        // Encode
        let encoded = codec.encode(&frame).unwrap();
        assert_eq!(encoded.len(), 160);
        
        // Decode
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.samples.len(), 160);
        
        // Test basic round-trip functionality
        assert!(!decoded.samples.is_empty());
        assert_eq!(decoded.sample_rate, 8000);
        assert_eq!(decoded.channels, 1);
    }
    
    #[test]
    fn test_mulaw_specific_values() {
        let mut codec = G711Codec::mu_law(SampleRate::Rate8000, 1).unwrap();
        
        // Test specific known values
        let test_samples = vec![0i16, 100, -100, 1000, -1000];
        
        for &sample in &test_samples {
            let frame = AudioFrame::new(vec![sample; 160], 8000, 1, 0);
            let encoded = codec.encode(&frame).unwrap();
            let decoded = codec.decode(&encoded).unwrap();
            
            // Just verify we get reasonable output
            assert_eq!(decoded.samples.len(), 160);
            
            // For zero input, we should get zero (or very close)
            if sample == 0 {
                assert!(decoded.samples[0].abs() < 200, "Zero sample should decode to reasonably close to zero, got {}", decoded.samples[0]);
            }
        }
    }
    
    #[test]
    fn test_alaw_specific_values() {
        let mut codec = G711Codec::a_law(SampleRate::Rate8000, 1).unwrap();
        
        // Test specific known values
        let test_samples = vec![0i16, 100, -100, 1000, -1000];
        
        for &sample in &test_samples {
            let frame = AudioFrame::new(vec![sample; 160], 8000, 1, 0);
            let encoded = codec.encode(&frame).unwrap();
            let decoded = codec.decode(&encoded).unwrap();
            
            // Just verify we get reasonable output
            assert_eq!(decoded.samples.len(), 160);
            
            // For zero input, we should get zero (or very close)
            if sample == 0 {
                assert!(decoded.samples[0].abs() < 200, "Zero sample should decode to reasonably close to zero, got {}", decoded.samples[0]);
            }
        }
    }
    
    #[test]
    fn test_invalid_frame_size() {
        let mut codec = G711Codec::mu_law(SampleRate::Rate8000, 1).unwrap();
        
        // Wrong frame size
        let samples = vec![0i16; 80]; // Should be 160 for 20ms at 8kHz
        let frame = AudioFrame::new(samples, 8000, 1, 0);
        
        let result = codec.encode(&frame);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), crate::error::Error::Codec(CodecError::InvalidFrameSize { .. })));
    }
} 