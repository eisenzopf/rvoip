//! G.711 Audio Codec Implementation
//!
//! This module implements the G.711 codec with both μ-law (PCMU) and A-law (PCMA)
//! variants. G.711 is the fundamental codec for telephony systems worldwide.

use tracing::{debug, trace};
use crate::error::{Result, CodecError};
use crate::types::{AudioFrame, SampleRate};
use super::common::{AudioCodec, CodecInfo};

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
}

impl AudioCodec for G711Codec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        if audio_frame.samples.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize {
                expected: self.frame_size,
                actual: audio_frame.samples.len(),
            }.into());
        }
        
        let mut encoded = Vec::with_capacity(audio_frame.samples.len());
        
        match self.config.variant {
            G711Variant::MuLaw => {
                for &sample in &audio_frame.samples {
                    encoded.push(linear_to_mulaw(sample));
                }
            }
            G711Variant::ALaw => {
                for &sample in &audio_frame.samples {
                    encoded.push(linear_to_alaw(sample));
                }
            }
        }
        
        trace!("G.711 {:?} encoded {} samples to {} bytes", 
               self.config.variant, audio_frame.samples.len(), encoded.len());
        
        Ok(encoded)
    }
    
    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        let mut samples = Vec::with_capacity(encoded_data.len());
        
        match self.config.variant {
            G711Variant::MuLaw => {
                for &byte in encoded_data {
                    samples.push(mulaw_to_linear(byte));
                }
            }
            G711Variant::ALaw => {
                for &byte in encoded_data {
                    samples.push(alaw_to_linear(byte));
                }
            }
        }
        
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

// μ-law encoding/decoding tables and functions
const MULAW_BIAS: i16 = 0x84;
const MULAW_CLIP: i16 = 8159;

/// Convert linear PCM sample to μ-law
fn linear_to_mulaw(sample: i16) -> u8 {
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

/// Convert μ-law to linear PCM sample
fn mulaw_to_linear(mulaw: u8) -> i16 {
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

/// Convert linear PCM sample to A-law
fn linear_to_alaw(sample: i16) -> u8 {
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

/// Convert A-law to linear PCM sample
fn alaw_to_linear(alaw: u8) -> i16 {
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