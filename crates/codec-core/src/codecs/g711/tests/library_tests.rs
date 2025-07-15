//! G.711 Library Unit Tests
//!
//! Tests for basic library functionality including:
//! - Codec creation and configuration
//! - Error handling and validation
//! - Frame size validation
//! - Memory management
//! - API consistency

use crate::codecs::g711::*;
use crate::error::CodecError;
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecType, SampleRate};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_g711_pcmu_codec_creation() {
        let config = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1)
            .with_frame_size_ms(20.0);

        let codec = G711Codec::new_pcmu(config);
        assert!(codec.is_ok());

        let codec = codec.unwrap();
        assert_eq!(codec.variant(), G711Variant::MuLaw);
        assert_eq!(codec.frame_size(), 160);
        
        let info = codec.info();
        assert_eq!(info.name, "PCMU");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.payload_type, Some(0));
    }

    #[test]
    fn test_g711_pcma_codec_creation() {
        let config = CodecConfig::new(CodecType::G711Pcma)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1)
            .with_frame_size_ms(20.0);

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
        let config = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate48000)
            .with_channels(1);

        let result = G711Codec::new_pcmu(config);
        assert!(result.is_err());
        
        if let Err(CodecError::InvalidSampleRate { rate, .. }) = result {
            assert_eq!(rate, 48000);
        } else {
            panic!("Expected InvalidSampleRate error");
        }
    }

    #[test]
    fn test_invalid_channels() {
        let config = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(2);

        let result = G711Codec::new_pcmu(config);
        assert!(result.is_err());
        
        if let Err(CodecError::InvalidChannelCount { channels, .. }) = result {
            assert_eq!(channels, 2);
        } else {
            panic!("Expected InvalidChannelCount error");
        }
    }

    #[test]
    fn test_frame_size_validation() {
        // Test valid frame sizes
        let valid_sizes = [80, 160, 240, 320];
        for &size in &valid_sizes {
            let frame_ms = (size as f32 * 1000.0) / 8000.0;
            let config = CodecConfig::new(CodecType::G711Pcmu)
                .with_sample_rate(SampleRate::Rate8000)
                .with_channels(1)
                .with_frame_size_ms(frame_ms);

            let codec = G711Codec::new_pcmu(config);
            assert!(codec.is_ok(), "Frame size {} should be valid", size);
            assert_eq!(codec.unwrap().frame_size(), size);
        }

        // Test invalid frame size
        let config = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1)
            .with_frame_size_ms(15.0); // 120 samples - invalid

        let result = G711Codec::new_pcmu(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_codec_reset() {
        let config = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);

        let mut codec = G711Codec::new_pcmu(config).unwrap();
        
        // G.711 is stateless, so reset should always succeed
        assert!(codec.reset().is_ok());
    }

    #[test]
    fn test_codec_info_consistency() {
        let config = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);

        let codec = G711Codec::new_pcmu(config).unwrap();
        let info = codec.info();

        // Test all info fields are consistent
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bitrate, 64000); // 8kHz * 8 bits/sample
        assert_eq!(info.frame_size, 160);
        assert!(codec.supports_variable_frame_size());
    }

    #[test]
    fn test_max_sizes() {
        let config = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);

        let codec = G711Codec::new_pcmu(config).unwrap();
        
        // G.711 has 1:1 sample to byte ratio
        assert_eq!(codec.max_encoded_size(160), 160);
        assert_eq!(codec.max_decoded_size(160), 160);
    }

    #[test]
    fn test_variant_differences() {
        let config = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);

        let codec_mu = G711Codec::new_pcmu(config.clone()).unwrap();
        let codec_a = G711Codec::new_pcma(config).unwrap();

        // Test variants are different
        assert_ne!(codec_mu.variant(), codec_a.variant());
        assert_eq!(codec_mu.variant(), G711Variant::MuLaw);
        assert_eq!(codec_a.variant(), G711Variant::ALaw);

        // Test payload types are different
        assert_eq!(codec_mu.info().payload_type, Some(0));
        assert_eq!(codec_a.info().payload_type, Some(8));
    }
} 