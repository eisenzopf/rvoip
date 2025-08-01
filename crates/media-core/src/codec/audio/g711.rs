//! G.711 Audio Codec Implementation using codec-core
//!
//! This module implements the G.711 codec with both μ-law (PCMU) and A-law (PCMA)
//! variants by leveraging the codec-core library. G.711 is the fundamental codec 
//! for telephony systems worldwide.

use codec_core::codecs::g711::{G711Codec as CodecCoreG711, G711Variant};
use codec_core::types::{AudioCodec as CodecCoreAudioCodec, AudioCodecExt};
use crate::codec::audio::common::{AudioCodec, CodecInfo};
use crate::types::AudioFrame;
use crate::error::{Error, Result};

/// G.711 codec implementation using codec-core
pub struct G711Codec {
    inner: CodecCoreG711,
    variant: G711Variant,
    sample_rate: u32,
    channels: u16,
}

impl G711Codec {
    pub fn new(variant: G711Variant, sample_rate: u32, channels: u16) -> Result<Self> {
        Ok(Self {
            inner: CodecCoreG711::new(variant),
            variant,
            sample_rate,
            channels,
        })
    }
    
    pub fn mu_law(sample_rate: u32, channels: u16) -> Result<Self> {
        Self::new(G711Variant::MuLaw, sample_rate, channels)
    }
    
    pub fn a_law(sample_rate: u32, channels: u16) -> Result<Self> {
        Self::new(G711Variant::ALaw, sample_rate, channels)
    }

    /// Zero-copy encode directly to output buffer
    pub fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        // codec-core's encode_to_buffer returns Result<usize, CodecError>
        self.inner.encode_to_buffer(samples, output)
            .map_err(|e| Error::Codec(crate::error::CodecError::EncodingFailed {
                reason: format!("G.711 {} zero-copy encoding failed: {}", 
                    match self.variant {
                        G711Variant::MuLaw => "μ-law",
                        G711Variant::ALaw => "A-law",
                    }, e)
            }))
    }
    
    /// Zero-copy decode directly to output buffer
    pub fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        self.inner.decode_to_buffer(data, output)
            .map_err(|e| Error::Codec(crate::error::CodecError::DecodingFailed {
                reason: format!("G.711 {} zero-copy decoding failed: {}", 
                    match self.variant {
                        G711Variant::MuLaw => "μ-law",
                        G711Variant::ALaw => "A-law",
                    }, e)
            }))
    }
}

impl AudioCodec for G711Codec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        self.inner.encode(&audio_frame.samples)
            .map_err(|e| Error::Codec(crate::error::CodecError::EncodingFailed {
                reason: format!("G.711 {} encoding failed: {}", 
                    match self.variant {
                        G711Variant::MuLaw => "μ-law",
                        G711Variant::ALaw => "A-law",
                    }, e)
            }))
    }
    
    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        if encoded_data.is_empty() {
            return Err(Error::Codec(crate::error::CodecError::DecodingFailed {
                reason: format!("G.711 {} decoding failed: empty buffer", 
                    match self.variant {
                        G711Variant::MuLaw => "μ-law",
                        G711Variant::ALaw => "A-law",
                    })
            }));
        }
        
        let samples = self.inner.decode(encoded_data)
            .map_err(|e| Error::Codec(crate::error::CodecError::DecodingFailed {
                reason: format!("G.711 {} decoding failed: {}", 
                    match self.variant {
                        G711Variant::MuLaw => "μ-law",
                        G711Variant::ALaw => "A-law",
                    }, e)
            }))?;
        
        Ok(AudioFrame::new(
            samples,
            self.sample_rate,
            self.channels as u8,
            0, // timestamp will be set by caller
        ))
    }
    
    fn get_info(&self) -> CodecInfo {
        CodecInfo {
            name: match self.variant {
                G711Variant::MuLaw => "G.711 μ-law".to_string(),
                G711Variant::ALaw => "G.711 A-law".to_string(),
            },
            sample_rate: self.sample_rate,
            channels: self.channels as u8,
            bitrate: self.sample_rate * 8 * self.channels as u32, // 8 bits per sample
        }
    }
    
    fn reset(&mut self) {
        // G.711 is stateless, but we could reinitialize if needed
        self.inner = CodecCoreG711::new(self.variant);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_codec_core_g711_compatibility() {
        // Test that codec-core G.711 produces expected output
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
        // Test specific G.711 features
        let mut mu_codec = G711Codec::mu_law(8000, 1).unwrap();
        let mut a_codec = G711Codec::a_law(8000, 1).unwrap();
        
        // Test maximum values
        let max_samples = vec![i16::MAX; 160];
        let frame = AudioFrame::new(max_samples, 8000, 1, 0);
        
        // Both codecs should handle max values without panic
        let mu_encoded = mu_codec.encode(&frame).unwrap();
        let a_encoded = a_codec.encode(&frame).unwrap();
        
        assert_eq!(mu_encoded.len(), 160);
        assert_eq!(a_encoded.len(), 160);
        
        // Test minimum values
        let min_samples = vec![i16::MIN; 160];
        let frame = AudioFrame::new(min_samples, 8000, 1, 0);
        
        let mu_encoded = mu_codec.encode(&frame).unwrap();
        let a_encoded = a_codec.encode(&frame).unwrap();
        
        assert_eq!(mu_encoded.len(), 160);
        assert_eq!(a_encoded.len(), 160);
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
    
    #[test]
    fn test_zero_copy_methods() {
        let mut codec = G711Codec::mu_law(8000, 1).unwrap();
        
        let samples = vec![1000i16; 160];
        let mut encoded = vec![0u8; 160];
        
        // Test zero-copy encode
        let encoded_len = codec.encode_to_buffer(&samples, &mut encoded).unwrap();
        assert_eq!(encoded_len, 160);
        
        // Test zero-copy decode
        let mut decoded = vec![0i16; 160];
        let decoded_len = codec.decode_to_buffer(&encoded, &mut decoded).unwrap();
        assert_eq!(decoded_len, 160);
    }
    
    #[test]
    fn test_codec_info() {
        let mu_codec = G711Codec::mu_law(8000, 1).unwrap();
        let info = mu_codec.get_info();
        assert_eq!(info.name, "G.711 μ-law");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bitrate, 64000); // 8000 * 8 * 1
        
        let a_codec = G711Codec::a_law(16000, 2).unwrap();
        let info = a_codec.get_info();
        assert_eq!(info.name, "G.711 A-law");
        assert_eq!(info.sample_rate, 16000);
        assert_eq!(info.channels, 2);
        assert_eq!(info.bitrate, 256000); // 16000 * 8 * 2
    }
}