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

    /// Every codec entry point (vector, per-variant and buffer APIs) gives
    /// the reference encoder's code for all 65,536 PCM samples and the
    /// reference decoder's sample for all 256 wire bytes, and the buffer
    /// APIs leave bytes past the input untouched.
    #[test]
    fn table_backed_codec_matches_reference_over_entire_domains() {
        let samples: Vec<i16> = (0..=u16::MAX).map(u16::cast_signed).collect();
        let bytes: Vec<u8> = (0..=u8::MAX).collect();
        for variant in [G711Variant::MuLaw, G711Variant::ALaw] {
            let (expected_codes, expected_samples): (Vec<u8>, Vec<i16>) = match variant {
                G711Variant::MuLaw => (
                    samples.iter().map(|&s| ulaw_compress(s)).collect(),
                    bytes.iter().map(|&b| ulaw_expand(b)).collect(),
                ),
                G711Variant::ALaw => (
                    samples.iter().map(|&s| alaw_compress(s)).collect(),
                    bytes.iter().map(|&b| alaw_expand(b)).collect(),
                ),
            };
            let mut codec = G711Codec::new(variant);

            assert_eq!(codec.compress(&samples).unwrap(), expected_codes);
            assert_eq!(codec.encode(&samples).unwrap(), expected_codes);
            assert_eq!(codec.expand(&bytes).unwrap(), expected_samples);
            assert_eq!(codec.decode(&bytes).unwrap(), expected_samples);
            let (per_variant_codes, per_variant_samples) = match variant {
                G711Variant::MuLaw => (
                    codec.compress_ulaw(&samples).unwrap(),
                    codec.expand_ulaw(&bytes).unwrap(),
                ),
                G711Variant::ALaw => (
                    codec.compress_alaw(&samples).unwrap(),
                    codec.expand_alaw(&bytes).unwrap(),
                ),
            };
            assert_eq!(per_variant_codes, expected_codes);
            assert_eq!(per_variant_samples, expected_samples);

            let mut encoded = vec![0x5a; samples.len() + 7];
            assert_eq!(
                codec.encode_to_buffer(&samples, &mut encoded).unwrap(),
                samples.len()
            );
            assert_eq!(&encoded[..samples.len()], expected_codes.as_slice());
            assert_eq!(&encoded[samples.len()..], &[0x5a; 7]);
            let mut decoded = [123_i16; 263];
            assert_eq!(codec.decode_to_buffer(&bytes, &mut decoded).unwrap(), 256);
            assert_eq!(&decoded[..256], expected_samples.as_slice());
            assert_eq!(&decoded[256..], &[123; 7]);
            assert!(codec.encode_to_buffer(&samples[..2], &mut [0; 1]).is_err());
            assert!(codec.decode_to_buffer(&bytes[..2], &mut [0; 1]).is_err());
        }
    }
}
