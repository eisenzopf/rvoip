//! Codec Factory for creating codec instances
//!
//! This module provides a factory for creating audio codec instances based on
//! payload types and configuration parameters.

use codec_core::codecs::g711::G711Variant;
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
            _ => Err(Error::unsupported_payload_type(payload_type)),
        }
    }
    
    /// Create codec with standard RTP payload type defaults
    pub fn create_codec_default(payload_type: u8) -> Result<Box<dyn AudioCodec>> {
        Self::create_codec(payload_type, None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_create_mulaw_codec() {
        let codec = CodecFactory::create_codec_default(0).unwrap();
        let info = codec.get_info();
        assert!(info.name.contains("μ-law"));
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
    }
    
    #[test]
    fn test_create_alaw_codec() {
        let codec = CodecFactory::create_codec_default(8).unwrap();
        let info = codec.get_info();
        assert!(info.name.contains("A-law"));
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
    }
    
    #[test]
    fn test_create_codec_with_params() {
        let codec = CodecFactory::create_codec(0, Some(16000), Some(2)).unwrap();
        let info = codec.get_info();
        assert!(info.name.contains("μ-law"));
        assert_eq!(info.sample_rate, 16000);
        assert_eq!(info.channels, 2);
    }
    
    #[test]
    fn test_unsupported_payload_type() {
        let result = CodecFactory::create_codec_default(99);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported payload type: 99"));
    }
}