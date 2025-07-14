//! G.711 Audio Codec Implementation
//!
//! This module implements the G.711 codec with both μ-law (PCMU) and A-law (PCMA)
//! variants. G.711 is the fundamental codec for telephony systems worldwide.
//!
//! ## Performance Optimizations
//!
//! - Pre-computed lookup tables for O(1) conversion
//! - SIMD vectorization for batch processing
//! - Zero-allocation APIs with pre-allocated buffers
//! - Optimized scalar fallbacks for small frames

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecInfo, SampleRate};
use crate::utils::{encode_mulaw_optimized, encode_alaw_optimized, encode_mulaw_table, encode_alaw_table, decode_mulaw_table, decode_alaw_table, validate_g711_frame};
use tracing::{debug, trace};

/// G.711 codec variant
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum G711Variant {
    /// μ-law (PCMU) - Used primarily in North America and Japan
    MuLaw,
    /// A-law (PCMA) - Used primarily in Europe and rest of world
    ALaw,
}

/// G.711 codec implementation
pub struct G711Codec {
    /// Codec variant (μ-law or A-law)
    variant: G711Variant,
    /// Sample rate (typically 8000 Hz)
    sample_rate: u32,
    /// Number of channels (typically 1)
    channels: u8,
    /// Frame size in samples
    frame_size: usize,
}

impl G711Codec {
    /// Create a new G.711 μ-law (PCMU) codec
    pub fn new_pcmu(config: CodecConfig) -> Result<Self> {
        Self::new(config, G711Variant::MuLaw)
    }
    
    /// Create a new G.711 A-law (PCMA) codec
    pub fn new_pcma(config: CodecConfig) -> Result<Self> {
        Self::new(config, G711Variant::ALaw)
    }
    
    /// Create a new G.711 codec with specified variant
    fn new(config: CodecConfig, variant: G711Variant) -> Result<Self> {
        // Validate configuration
        let sample_rate = config.sample_rate.hz();
        
        // G.711 only supports 8kHz
        if sample_rate != 8000 {
            return Err(CodecError::InvalidSampleRate {
                rate: sample_rate,
                supported: vec![8000],
            });
        }
        
        // G.711 only supports mono
        if config.channels != 1 {
            return Err(CodecError::InvalidChannelCount {
                channels: config.channels,
                supported: vec![1],
            });
        }
        
        // Calculate frame size based on frame_size_ms or use default
        let frame_size = if let Some(frame_ms) = config.frame_size_ms {
            (sample_rate as f32 * frame_ms / 1000.0) as usize
        } else {
            160 // Default 20ms frame at 8kHz
        };
        
        // Validate frame size
        if ![80, 160, 240, 320].contains(&frame_size) {
            return Err(CodecError::InvalidFrameSize {
                expected: 160,
                actual: frame_size,
            });
        }
        
        debug!("Creating G.711 {:?} codec: {}Hz, {}ch, {} samples/frame", 
               variant, sample_rate, config.channels, frame_size);
        
        Ok(Self {
            variant,
            sample_rate,
            channels: config.channels,
            frame_size,
        })
    }
    
    /// Get the codec variant
    pub fn variant(&self) -> G711Variant {
        self.variant
    }
    
    /// Get the compression ratio (G.711 is 2:1, 16-bit to 8-bit)
    pub fn compression_ratio(&self) -> f32 {
        0.5
    }
}

impl AudioCodec for G711Codec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // Validate input
        validate_g711_frame(samples, self.frame_size)?;
        
        let mut output = vec![0u8; samples.len()];
        self.encode_to_buffer(samples, &mut output)?;
        
        trace!("G.711 {:?} encoded {} samples to {} bytes", 
               self.variant, samples.len(), output.len());
        
        Ok(output)
    }
    
    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }
        
        let mut output = vec![0i16; data.len()];
        self.decode_to_buffer(data, &mut output)?;
        
        trace!("G.711 {:?} decoded {} bytes to {} samples", 
               self.variant, data.len(), output.len());
        
        Ok(output)
    }
    
    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: match self.variant {
                G711Variant::MuLaw => "PCMU",
                G711Variant::ALaw => "PCMA",
            },
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: 64000, // 8kHz * 8 bits/sample
            frame_size: self.frame_size,
            payload_type: match self.variant {
                G711Variant::MuLaw => Some(0),
                G711Variant::ALaw => Some(8),
            },
        }
    }
    
    fn reset(&mut self) -> Result<()> {
        // G.711 is stateless, no reset needed
        debug!("G.711 {:?} codec reset (stateless)", self.variant);
        Ok(())
    }
    
    fn frame_size(&self) -> usize {
        self.frame_size
    }
    
    fn supports_variable_frame_size(&self) -> bool {
        true // G.711 supports various frame sizes
    }
}

impl AudioCodecExt for G711Codec {
    fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        // Validate input
        validate_g711_frame(samples, self.frame_size)?;
        
        if output.len() < samples.len() {
            return Err(CodecError::BufferTooSmall {
                needed: samples.len(),
                actual: output.len(),
            });
        }
        
        // Choose encoding method based on size and SIMD availability
        if samples.len() >= 16 && crate::utils::has_simd_support() {
            // Use SIMD-optimized encoding for larger frames
            match self.variant {
                G711Variant::MuLaw => encode_mulaw_optimized(samples, &mut output[..samples.len()]),
                G711Variant::ALaw => encode_alaw_optimized(samples, &mut output[..samples.len()]),
            }
        } else {
            // Use lookup table encoding for smaller frames or no SIMD
            match self.variant {
                G711Variant::MuLaw => {
                    for (i, &sample) in samples.iter().enumerate() {
                        output[i] = encode_mulaw_table(sample);
                    }
                }
                G711Variant::ALaw => {
                    for (i, &sample) in samples.iter().enumerate() {
                        output[i] = encode_alaw_table(sample);
                    }
                }
            }
        }
        
        trace!("G.711 {:?} encoded {} samples to {} bytes (zero-alloc)", 
               self.variant, samples.len(), samples.len());
        
        Ok(samples.len())
    }
    
    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }
        
        if output.len() < data.len() {
            return Err(CodecError::BufferTooSmall {
                needed: data.len(),
                actual: output.len(),
            });
        }
        
        // Use lookup table decoding (always fast)
        match self.variant {
            G711Variant::MuLaw => {
                for (i, &byte) in data.iter().enumerate() {
                    output[i] = decode_mulaw_table(byte);
                }
            }
            G711Variant::ALaw => {
                for (i, &byte) in data.iter().enumerate() {
                    output[i] = decode_alaw_table(byte);
                }
            }
        }
        
        trace!("G.711 {:?} decoded {} bytes to {} samples (zero-alloc)", 
               self.variant, data.len(), data.len());
        
        Ok(data.len())
    }
    
    fn max_encoded_size(&self, input_samples: usize) -> usize {
        // G.711 is 1:1 sample to byte ratio
        input_samples
    }
    
    fn max_decoded_size(&self, input_bytes: usize) -> usize {
        // G.711 is 1:1 byte to sample ratio
        input_bytes
    }
}

/// Initialize G.711 lookup tables
pub fn init_tables() {
    crate::utils::init_tables();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CodecConfig, CodecType, SampleRate};

    fn create_test_config(variant: G711Variant) -> CodecConfig {
        let codec_type = match variant {
            G711Variant::MuLaw => CodecType::G711Pcmu,
            G711Variant::ALaw => CodecType::G711Pcma,
        };
        
        CodecConfig::new(codec_type)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1)
            .with_frame_size_ms(20.0)
    }

    #[test]
    fn test_g711_creation() {
        let config = create_test_config(G711Variant::MuLaw);
        let codec = G711Codec::new_pcmu(config);
        assert!(codec.is_ok());
        
        let codec = codec.unwrap();
        assert_eq!(codec.variant(), G711Variant::MuLaw);
        assert_eq!(codec.frame_size(), 160);
        
        let info = codec.info();
        assert_eq!(info.name, "PCMU");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.payload_type, Some(0));
    }

    #[test]
    fn test_g711_pcma_creation() {
        let config = create_test_config(G711Variant::ALaw);
        let codec = G711Codec::new_pcma(config);
        assert!(codec.is_ok());
        
        let codec = codec.unwrap();
        assert_eq!(codec.variant(), G711Variant::ALaw);
        
        let info = codec.info();
        assert_eq!(info.name, "PCMA");
        assert_eq!(info.payload_type, Some(8));
    }

    #[test]
    fn test_invalid_sample_rate() {
        let mut config = create_test_config(G711Variant::MuLaw);
        config.sample_rate = SampleRate::Rate48000;
        
        let codec = G711Codec::new_pcmu(config);
        assert!(codec.is_err());
    }

    #[test]
    fn test_invalid_channels() {
        let mut config = create_test_config(G711Variant::MuLaw);
        config.channels = 2;
        
        let codec = G711Codec::new_pcmu(config);
        assert!(codec.is_err());
    }

    #[test]
    fn test_encoding_decoding_roundtrip() {
        let config = create_test_config(G711Variant::MuLaw);
        let mut codec = G711Codec::new_pcmu(config).unwrap();
        
        // Test with various sample values
        let test_samples = vec![
            0i16, 1000, -1000, 16000, -16000, 32000, -32000, 12345, -9876
        ];
        
        // Pad to frame size
        let mut samples = test_samples.clone();
        samples.resize(160, 0);
        
        // Encode
        let encoded = codec.encode(&samples).unwrap();
        assert_eq!(encoded.len(), samples.len());
        
        // Decode
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), samples.len());
        
        // Check quality (G.711 is lossy)
        for (original, decoded) in test_samples.iter().zip(decoded.iter()) {
            let error = (original - decoded).abs();
            assert!(error < 1000, "Error too large: {} vs {} (error: {})", 
                   original, decoded, error);
        }
    }

    #[test]
    fn test_alaw_encoding_decoding() {
        let config = create_test_config(G711Variant::ALaw);
        let mut codec = G711Codec::new_pcma(config).unwrap();
        
        let samples = vec![0i16; 160];
        let encoded = codec.encode(&samples).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        
        assert_eq!(samples.len(), encoded.len());
        assert_eq!(encoded.len(), decoded.len());
    }

    #[test]
    fn test_zero_copy_apis() {
        let config = create_test_config(G711Variant::MuLaw);
        let mut codec = G711Codec::new_pcmu(config).unwrap();
        
        let samples = vec![1000i16; 160];
        let mut encoded = vec![0u8; 160];
        let mut decoded = vec![0i16; 160];
        
        // Test zero-copy encoding
        let encoded_len = codec.encode_to_buffer(&samples, &mut encoded).unwrap();
        assert_eq!(encoded_len, 160);
        
        // Test zero-copy decoding
        let decoded_len = codec.decode_to_buffer(&encoded, &mut decoded).unwrap();
        assert_eq!(decoded_len, 160);
        
        // Verify roundtrip quality
        for (original, decoded) in samples.iter().zip(decoded.iter()) {
            let error = (original - decoded).abs();
            assert!(error < 1000, "Roundtrip error too large: {}", error);
        }
    }

    #[test]
    fn test_frame_size_validation() {
        let config = create_test_config(G711Variant::MuLaw);
        let mut codec = G711Codec::new_pcmu(config).unwrap();
        
        // Wrong frame size should fail
        let wrong_samples = vec![0i16; 100];
        assert!(codec.encode(&wrong_samples).is_err());
        
        // Empty samples should fail
        let empty_samples: Vec<i16> = vec![];
        assert!(codec.encode(&empty_samples).is_err());
    }

    #[test]
    fn test_buffer_size_validation() {
        let config = create_test_config(G711Variant::MuLaw);
        let mut codec = G711Codec::new_pcmu(config).unwrap();
        
        let samples = vec![0i16; 160];
        let mut small_buffer = vec![0u8; 80]; // Too small
        
        assert!(codec.encode_to_buffer(&samples, &mut small_buffer).is_err());
    }

    #[test]
    fn test_codec_reset() {
        let config = create_test_config(G711Variant::MuLaw);
        let mut codec = G711Codec::new_pcmu(config).unwrap();
        
        // Reset should always succeed for G.711 (stateless)
        assert!(codec.reset().is_ok());
    }

    #[test]
    fn test_codec_info() {
        let config = create_test_config(G711Variant::MuLaw);
        let codec = G711Codec::new_pcmu(config).unwrap();
        
        let info = codec.info();
        assert_eq!(info.name, "PCMU");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bitrate, 64000);
        assert_eq!(info.frame_size, 160);
        assert_eq!(info.payload_type, Some(0));
        
        assert!(codec.supports_variable_frame_size());
    }

    #[test]
    fn test_different_frame_sizes() {
        // Test 10ms frame
        let mut config = create_test_config(G711Variant::MuLaw);
        config.frame_size_ms = Some(10.0);
        let codec = G711Codec::new_pcmu(config).unwrap();
        assert_eq!(codec.frame_size(), 80);
        
        // Test 30ms frame
        let mut config = create_test_config(G711Variant::MuLaw);
        config.frame_size_ms = Some(30.0);
        let codec = G711Codec::new_pcmu(config).unwrap();
        assert_eq!(codec.frame_size(), 240);
    }

    #[test]
    fn test_compression_ratio() {
        let config = create_test_config(G711Variant::MuLaw);
        let codec = G711Codec::new_pcmu(config).unwrap();
        
        assert_eq!(codec.compression_ratio(), 0.5);
        assert_eq!(codec.max_encoded_size(160), 160);
        assert_eq!(codec.max_decoded_size(160), 160);
    }

    #[test]
    fn test_mulaw_vs_alaw_difference() {
        let samples = vec![12345i16; 160];
        
        let config_mu = create_test_config(G711Variant::MuLaw);
        let mut codec_mu = G711Codec::new_pcmu(config_mu).unwrap();
        
        let config_a = create_test_config(G711Variant::ALaw);
        let mut codec_a = G711Codec::new_pcma(config_a).unwrap();
        
        let encoded_mu = codec_mu.encode(&samples).unwrap();
        let encoded_a = codec_a.encode(&samples).unwrap();
        
        // μ-law and A-law should produce different encoded data
        assert_ne!(encoded_mu, encoded_a);
        
        // But both should decode to similar values
        let decoded_mu = codec_mu.decode(&encoded_mu).unwrap();
        let decoded_a = codec_a.decode(&encoded_a).unwrap();
        
        for (mu_sample, a_sample) in decoded_mu.iter().zip(decoded_a.iter()) {
            let diff = (mu_sample - a_sample).abs();
            // Difference should be small (both are G.711 variants)
            assert!(diff < 2000, "μ-law/A-law difference too large: {}", diff);
        }
    }
} 