//! Opus audio codec adapter.
//!
//! The actual encoder and decoder live in `rvoip-codec-core`. This module only
//! converts between media-core frames and codec-core's PCM-oriented API.

use super::common::{AudioCodec, CodecInfo};
use crate::error::{CodecError, Result};
use crate::types::{AudioFrame, SampleRate};
#[cfg(feature = "opus")]
use codec_core::codecs::opus::OpusCodec as CodecCoreOpus;
#[cfg(feature = "opus")]
use codec_core::types::{
    AudioCodec as CodecCoreAudioCodec, CodecConfig as CodecCoreConfig, CodecType as CodecCoreType,
    OpusApplication as CodecCoreApplication, SampleRate as CodecCoreSampleRate,
};
#[cfg(feature = "opus")]
use tracing::debug;

/// Opus codec configuration.
#[derive(Debug, Clone)]
pub struct OpusConfig {
    /// Target bitrate (6000-510000 bps).
    pub bitrate: u32,
    /// Encoding complexity (0-10, higher is better quality/more CPU).
    pub complexity: u32,
    /// Use variable bitrate.
    pub vbr: bool,
    /// Application type.
    pub application: OpusApplication,
    /// Frame size in milliseconds (2.5, 5, 10, 20, 40, or 60).
    pub frame_size_ms: f32,
}

/// Opus application types.
#[derive(Debug, Clone, Copy)]
pub enum OpusApplication {
    /// Voice over IP.
    Voip,
    /// General audio streaming.
    Audio,
}

impl Default for OpusConfig {
    fn default() -> Self {
        Self {
            bitrate: 64_000,
            complexity: 5,
            vbr: true,
            application: OpusApplication::Voip,
            frame_size_ms: 20.0,
        }
    }
}

/// Media-core adapter for codec-core's real libopus implementation.
pub struct OpusCodec {
    config: OpusConfig,
    sample_rate: u32,
    channels: u8,
    #[cfg(feature = "opus")]
    frame_size: usize,
    #[cfg(feature = "opus")]
    inner: CodecCoreOpus,
}

impl OpusCodec {
    /// Create a new Opus codec.
    ///
    /// Construction fails when the `opus` feature is disabled. This prevents
    /// callers from mistaking a placeholder object for a working codec.
    pub fn new(sample_rate: SampleRate, channels: u8, config: OpusConfig) -> Result<Self> {
        let sample_rate_hz = sample_rate.as_hz();
        if channels == 0 || channels > 2 {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid Opus channel count: {channels}"),
            }
            .into());
        }
        if !matches!(sample_rate_hz, 8_000 | 12_000 | 16_000 | 24_000 | 48_000) {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid Opus sample rate: {sample_rate_hz}"),
            }
            .into());
        }
        if !(6_000..=510_000).contains(&config.bitrate) {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid Opus bitrate: {}", config.bitrate),
            }
            .into());
        }
        if config.complexity > 10 {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid Opus complexity: {}", config.complexity),
            }
            .into());
        }
        if !matches!(config.frame_size_ms, 2.5 | 5.0 | 10.0 | 20.0 | 40.0 | 60.0) {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid Opus frame duration: {}ms", config.frame_size_ms),
            }
            .into());
        }

        #[cfg(not(feature = "opus"))]
        {
            let _ = (sample_rate_hz, channels, config);
            return Err(CodecError::NotFound {
                name: "Opus (enable the `opus` feature)".to_string(),
            }
            .into());
        }

        #[cfg(feature = "opus")]
        {
            let core_sample_rate = CodecCoreSampleRate::from_hz(sample_rate_hz);
            let core_application = match config.application {
                OpusApplication::Voip => CodecCoreApplication::Voip,
                OpusApplication::Audio => CodecCoreApplication::Audio,
            };
            let mut core_config = CodecCoreConfig::new(CodecCoreType::Opus)
                .with_sample_rate(core_sample_rate)
                .with_channels(channels)
                .with_bitrate(config.bitrate)
                .with_frame_size_ms(config.frame_size_ms)
                .with_opus_application(core_application)
                .with_opus_vbr(config.vbr)
                .with_opus_complexity(config.complexity as u8);
            core_config.parameters.opus.bitrate = config.bitrate;

            let inner = CodecCoreOpus::new(core_config).map_err(|error| {
                CodecError::InitializationFailed {
                    reason: format!("codec-core Opus initialization failed: {error}"),
                }
            })?;
            let frame_size = (sample_rate_hz as f32 * config.frame_size_ms / 1_000.0) as usize
                * usize::from(channels);

            debug!(
                "Creating codec-core Opus adapter: {}Hz, {}ch, {}ms frames",
                sample_rate_hz, channels, config.frame_size_ms
            );
            Ok(Self {
                config,
                sample_rate: sample_rate_hz,
                channels,
                frame_size,
                inner,
            })
        }
    }
}

impl AudioCodec for OpusCodec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        #[cfg(not(feature = "opus"))]
        {
            let _ = audio_frame;
            Err(CodecError::NotFound {
                name: "Opus (enable the `opus` feature)".to_string(),
            }
            .into())
        }

        #[cfg(feature = "opus")]
        {
            if audio_frame.sample_rate != self.sample_rate || audio_frame.channels != self.channels
            {
                return Err(CodecError::InvalidParameters {
                    details: format!(
                        "Opus frame format is {}Hz/{}ch, expected {}Hz/{}ch",
                        audio_frame.sample_rate,
                        audio_frame.channels,
                        self.sample_rate,
                        self.channels
                    ),
                }
                .into());
            }
            if audio_frame.samples.len() != self.frame_size {
                return Err(CodecError::InvalidFrameSize {
                    expected: self.frame_size,
                    actual: audio_frame.samples.len(),
                }
                .into());
            }
            self.inner.encode(&audio_frame.samples).map_err(|error| {
                CodecError::EncodingFailed {
                    reason: format!("codec-core Opus encoding failed: {error}"),
                }
                .into()
            })
        }
    }

    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        #[cfg(not(feature = "opus"))]
        {
            let _ = encoded_data;
            Err(CodecError::NotFound {
                name: "Opus (enable the `opus` feature)".to_string(),
            }
            .into())
        }

        #[cfg(feature = "opus")]
        {
            if encoded_data.is_empty() {
                return Err(CodecError::DecodingFailed {
                    reason: "empty Opus packet".to_string(),
                }
                .into());
            }
            let samples =
                self.inner
                    .decode(encoded_data)
                    .map_err(|error| CodecError::DecodingFailed {
                        reason: format!("codec-core Opus decoding failed: {error}"),
                    })?;
            Ok(AudioFrame::new(samples, self.sample_rate, self.channels, 0))
        }
    }

    fn get_info(&self) -> CodecInfo {
        CodecInfo {
            name: "Opus".to_string(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: self.config.bitrate,
        }
    }

    fn reset(&mut self) {
        #[cfg(feature = "opus")]
        if let Err(error) = self.inner.reset() {
            tracing::warn!("codec-core Opus reset failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "opus")]
    #[test]
    fn real_opus_round_trip_uses_codec_core() {
        let mut codec = OpusCodec::new(SampleRate::Rate48000, 1, OpusConfig::default()).unwrap();
        let samples: Vec<i16> = (0..960)
            .map(|index| {
                let t = index as f32 / 48_000.0;
                (12_000.0 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()) as i16
            })
            .collect();
        let encoded = codec
            .encode(&AudioFrame::new(samples, 48_000, 1, 0))
            .unwrap();
        assert!(!encoded.is_empty());
        assert_ne!(
            encoded,
            (0..encoded.len()).map(|i| i as u8).collect::<Vec<_>>()
        );

        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.samples.len(), 960);
        assert!(decoded.samples.iter().any(|sample| *sample != 0));
    }

    #[cfg(not(feature = "opus"))]
    #[test]
    fn opus_construction_fails_without_feature() {
        assert!(matches!(
            OpusCodec::new(SampleRate::Rate48000, 1, OpusConfig::default()),
            Err(crate::error::Error::Codec(CodecError::NotFound { .. }))
        ));
    }

    #[cfg(feature = "opus-sim")]
    #[test]
    fn deprecated_opus_sim_alias_constructs_the_real_backend() {
        let mut codec = OpusCodec::new(SampleRate::Rate48000, 1, OpusConfig::default()).unwrap();
        let samples = vec![0; 960];
        let packet = codec
            .encode(&AudioFrame::new(samples, 48_000, 1, 0))
            .unwrap();

        assert!(!packet.is_empty());
        assert_eq!(codec.decode(&packet).unwrap().samples.len(), 960);
    }
}
