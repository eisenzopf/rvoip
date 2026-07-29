//! Opus Audio Codec Implementation
//!
//! Thin adapter over `rvoip-codec-core`'s Opus implementation (the sole
//! place Opus encode/decode is actually implemented — see that crate for
//! backend details). This module only translates between media-core's
//! `AudioFrame`-based [`AudioCodec`] trait and codec-core's raw-`&[i16]`
//! one; it mirrors the same delegation shape used by
//! [`super::g729::G729Codec`].

use super::common::{AudioCodec, CodecInfo};
use crate::error::{CodecError, Result};
use crate::types::{AudioFrame, SampleRate};
#[cfg(feature = "opus")]
use codec_core::codecs::opus::OpusCodec as CodecCoreOpus;
#[cfg(feature = "opus")]
use codec_core::types::{
    AudioCodec as CodecCoreAudioCodec, CodecConfig as CodecCoreConfig,
    SampleRate as CodecCoreSampleRate,
};
use tracing::debug;
#[cfg(not(feature = "opus"))]
use tracing::warn;

/// Opus codec configuration
#[derive(Debug, Clone)]
pub struct OpusConfig {
    /// Target bitrate (6000-510000 bps)
    pub bitrate: u32,
    /// Encoding complexity (0-10, higher = better quality/more CPU)
    pub complexity: u32,
    /// Use variable bitrate
    pub vbr: bool,
    /// Application type
    pub application: OpusApplication,
    /// Frame size in milliseconds (2.5, 5, 10, 20, 40, 60)
    pub frame_size_ms: f32,
}

/// Opus application types
#[derive(Debug, Clone, Copy)]
pub enum OpusApplication {
    /// Voice over IP
    Voip,
    /// Audio streaming
    Audio,
}

impl Default for OpusConfig {
    fn default() -> Self {
        Self {
            bitrate: 64000, // 64 kbps - good quality for VoIP
            complexity: 5,  // Balanced complexity
            vbr: true,      // Variable bitrate for efficiency
            application: OpusApplication::Voip,
            frame_size_ms: 20.0, // 20ms frames (standard for VoIP)
        }
    }
}

/// Opus audio codec implementation
pub struct OpusCodec {
    /// Codec configuration
    #[allow(dead_code)]
    config: OpusConfig,
    /// Sample rate
    sample_rate: u32,
    /// Number of channels
    channels: u8,
    /// Frame size in samples (interleaved total, i.e. per-channel frame
    /// size * channels). Only read by the `opus`-feature encode path.
    #[cfg_attr(not(feature = "opus"), allow(dead_code))]
    frame_size: usize,
    /// Codec-core Opus adapter.
    #[cfg(feature = "opus")]
    inner: CodecCoreOpus,
}

impl OpusCodec {
    /// Create a new Opus codec
    pub fn new(sample_rate: SampleRate, channels: u8, config: OpusConfig) -> Result<Self> {
        let sample_rate_hz = sample_rate.as_hz();

        // Validate parameters
        if channels == 0 || channels > 2 {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid channel count: {}", channels),
            }
            .into());
        }

        // Validate sample rate (Opus supports 8, 12, 16, 24, 48 kHz)
        if !matches!(sample_rate_hz, 8000 | 12000 | 16000 | 24000 | 48000) {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid sample rate: {}", sample_rate_hz),
            }
            .into());
        }

        // Calculate frame size
        let frame_size =
            ((sample_rate_hz as f32 * config.frame_size_ms / 1000.0) as usize) * channels as usize;

        debug!(
            "Creating Opus codec: {}Hz, {}ch, {}ms frames",
            sample_rate_hz, channels, config.frame_size_ms
        );

        #[cfg(feature = "opus")]
        let inner = CodecCoreOpus::new(codec_core_config(&config, sample_rate_hz, channels)).map_err(
            |e| CodecError::InitializationFailed {
                reason: format!("Opus codec-core initialization failed: {e}"),
            },
        )?;

        Ok(Self {
            config,
            sample_rate: sample_rate_hz,
            channels,
            frame_size,
            #[cfg(feature = "opus")]
            inner,
        })
    }
}

// `audio_frame` / `encoded_data` parameters are consumed by the
// `#[cfg(feature = "opus")]` arms and dropped in the
// `#[cfg(not(feature = "opus"))]` stub arms; allow the unused
// bindings since they're only "unused" with opus disabled.
#[allow(unused_variables)]
impl AudioCodec for OpusCodec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        #[cfg(feature = "opus")]
        {
            if audio_frame.samples.len() != self.frame_size {
                return Err(CodecError::InvalidFrameSize {
                    expected: self.frame_size,
                    actual: audio_frame.samples.len(),
                }
                .into());
            }

            let encoded = self
                .inner
                .encode(&audio_frame.samples)
                .map_err(|e| CodecError::EncodingFailed {
                    reason: format!("Opus encoding failed: {e}"),
                })?;
            Ok(encoded)
        }

        #[cfg(not(feature = "opus"))]
        {
            warn!("Opus codec not available - feature 'opus' not enabled");
            Err(CodecError::NotFound {
                name: "Opus".to_string(),
            }
            .into())
        }
    }

    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        #[cfg(feature = "opus")]
        {
            let decoded_samples =
                self.inner
                    .decode(encoded_data)
                    .map_err(|e| CodecError::DecodingFailed {
                        reason: format!("Opus decoding failed: {e}"),
                    })?;

            Ok(AudioFrame::new(
                decoded_samples,
                self.sample_rate,
                self.channels,
                0, // Timestamp to be set by caller
            ))
        }

        #[cfg(not(feature = "opus"))]
        {
            warn!("Opus codec not available - feature 'opus' not enabled");
            Err(CodecError::NotFound {
                name: "Opus".to_string(),
            }
            .into())
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
        {
            let _ = self.inner.reset();
        }
        debug!("Opus codec reset");
    }
}

#[cfg(feature = "opus")]
fn codec_core_config(config: &OpusConfig, sample_rate_hz: u32, channels: u8) -> CodecCoreConfig {
    let mut core_config = CodecCoreConfig::opus()
        .with_sample_rate(CodecCoreSampleRate::from_hz(sample_rate_hz))
        .with_channels(channels)
        .with_frame_size_ms(config.frame_size_ms);

    core_config.parameters.opus.application = match config.application {
        OpusApplication::Voip => codec_core::types::OpusApplication::Voip,
        OpusApplication::Audio => codec_core::types::OpusApplication::Audio,
    };
    core_config.parameters.opus.bitrate = config.bitrate;
    core_config.parameters.opus.vbr = config.vbr;
    // codec-core's complexity is 0-10 per RFC 6716 §2.1.6, same range this
    // struct has always documented for its own `complexity` field (u32
    // here only because nothing previously enforced the range - the old
    // libopus binding predating this adapter didn't apply it at all).
    core_config.parameters.opus.complexity = config.complexity.min(10) as u8;

    core_config
}
