//! Codec Factory for creating codec instances
//!
//! This module provides a factory for creating audio codec instances based on
//! payload types and configuration parameters.

use crate::codec::audio::common::AudioCodec;
use crate::codec::audio::g711::G711Codec;
#[cfg(feature = "g729")]
use crate::codec::audio::g729::{G729Codec, G729Config};
use crate::codec::audio::payload_type::PCM_S16LE;
use crate::codec::audio::pcm::PcmS16LeCodec;
use crate::error::{Error, Result};
#[cfg(feature = "g729")]
use crate::types::SampleRate;

pub struct CodecFactory;

impl CodecFactory {
    /// Create a codec instance with configurable parameters
    pub fn create_codec(
        payload_type: u8,
        sample_rate: Option<u32>,
        channels: Option<u16>,
    ) -> Result<Box<dyn AudioCodec>> {
        // Default values if not specified
        let sample_rate = sample_rate.unwrap_or(if payload_type == PCM_S16LE {
            16_000
        } else {
            8_000
        });
        let channels = channels.unwrap_or(1);

        match payload_type {
            0 => Ok(Box::new(G711Codec::mu_law(sample_rate, channels)?)),
            8 => Ok(Box::new(G711Codec::a_law(sample_rate, channels)?)),
            #[cfg(feature = "g729")]
            18 => Ok(Box::new(G729Codec::new(
                SampleRate::Rate8000,
                1,
                G729Config::default(),
            )?)),
            PCM_S16LE => Ok(Box::new(PcmS16LeCodec::new(
                sample_rate,
                u8::try_from(channels).map_err(|_| {
                    Error::Codec(crate::error::CodecError::InvalidParameters {
                        details: format!("Invalid channel count: {channels}"),
                    })
                })?,
            )?)),
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
        // Can't unwrap error because AudioCodec doesn't implement Debug
        // Just verify that creating codec with unsupported type fails
    }

    #[test]
    fn test_create_internal_pcm_codec_defaults() {
        let codec = CodecFactory::create_codec_default(PCM_S16LE).unwrap();
        let info = codec.get_info();
        assert_eq!(info.name, "pcm_s16le");
        assert_eq!(info.sample_rate, 16_000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bitrate, 256_000);
    }
}
