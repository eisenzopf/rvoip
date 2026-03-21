//! Opus Audio Codec Implementation
//!
//! This module implements the Opus codec, a modern audio codec standardized
//! by the Internet Engineering Task Force (IETF) in RFC 6716. Opus combines
//! the best features of both speech and music codecs with very low latency.

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecInfo, SampleRate};
use crate::utils::{validate_opus_frame};
use tracing::{debug, trace, warn};

// Re-export OpusApplication from types to avoid duplication
pub use crate::types::OpusApplication;

// SAFETY: opus::Encoder and opus::Decoder contain raw pointers that are not
// Sync, but they are only accessed via &mut self (exclusive access), so sharing
// across threads is safe as long as we hold &mut. We use a wrapper to satisfy
// the Sync bound required by AudioCodec.
#[cfg(feature = "opus")]
struct SyncEncoder(opus::Encoder);
#[cfg(feature = "opus")]
// SAFETY: Encoder is only used behind &mut self, never shared concurrently
unsafe impl Sync for SyncEncoder {}

#[cfg(feature = "opus")]
struct SyncDecoder(opus::Decoder);
#[cfg(feature = "opus")]
// SAFETY: Decoder is only used behind &mut self, never shared concurrently
unsafe impl Sync for SyncDecoder {}

/// Opus codec implementation
pub struct OpusCodec {
    /// Sample rate (8, 12, 16, 24, or 48 kHz)
    sample_rate: u32,
    /// Number of channels (1 or 2)
    channels: u8,
    /// Frame size in samples
    frame_size: usize,
    /// Codec configuration
    config: OpusConfig,
    /// Opus encoder (lazily initialized)
    #[cfg(feature = "opus")]
    encoder: Option<SyncEncoder>,
    /// Opus decoder (lazily initialized)
    #[cfg(feature = "opus")]
    decoder: Option<SyncDecoder>,
}

/// Opus codec configuration
#[derive(Debug, Clone)]
pub struct OpusConfig {
    /// Application type (VoIP, Audio, or Low Delay)
    pub application: OpusApplication,
    /// Bitrate in bits per second
    pub bitrate: u32,
    /// Enable variable bitrate
    pub vbr: bool,
    /// Enable constrained VBR
    pub cvbr: bool,
    /// Complexity (0-10)
    pub complexity: u8,
    /// Enable inband FEC
    pub inband_fec: bool,
    /// DTX (Discontinuous Transmission)
    pub dtx: bool,
    /// Packet loss percentage (0-100)
    pub packet_loss_perc: u8,
    /// Force mono encoding
    pub force_mono: bool,
}

impl Default for OpusConfig {
    fn default() -> Self {
        Self {
            application: OpusApplication::Voip,
            bitrate: 64000,
            vbr: true,
            cvbr: false,
            complexity: 5,
            inband_fec: false,
            dtx: false,
            packet_loss_perc: 0,
            force_mono: false,
        }
    }
}

impl OpusCodec {
    /// Create a new Opus codec
    pub fn new(config: CodecConfig) -> Result<Self> {
        // Validate configuration
        let sample_rate = config.sample_rate.hz();

        // Opus supports 8, 12, 16, 24, 48 kHz
        if ![8000, 12000, 16000, 24000, 48000].contains(&sample_rate) {
            return Err(CodecError::InvalidSampleRate {
                rate: sample_rate,
                supported: vec![8000, 12000, 16000, 24000, 48000],
            });
        }

        // Opus supports mono and stereo
        if config.channels == 0 || config.channels > 2 {
            return Err(CodecError::InvalidChannelCount {
                channels: config.channels,
                supported: vec![1, 2],
            });
        }

        // Calculate frame size based on frame_size_ms or use default
        let frame_size = if let Some(frame_ms) = config.frame_size_ms {
            let samples_per_ms = sample_rate as f32 / 1000.0;
            (samples_per_ms * frame_ms) as usize
        } else {
            // Default to 20ms
            (sample_rate * 20 / 1000) as usize
        };

        // Create Opus configuration
        let opus_config = OpusConfig {
            application: config.parameters.opus.application,
            bitrate: config.parameters.opus.bitrate,
            vbr: config.parameters.opus.vbr,
            cvbr: config.parameters.opus.cvbr,
            complexity: config.parameters.opus.complexity,
            inband_fec: config.parameters.opus.inband_fec,
            dtx: config.parameters.opus.dtx,
            packet_loss_perc: config.parameters.opus.packet_loss_perc,
            force_mono: config.parameters.opus.force_mono,
        };

        debug!("Creating Opus codec: {}Hz, {}ch, {}bps, {:?} mode",
               sample_rate, config.channels, opus_config.bitrate, opus_config.application);

        Ok(Self {
            sample_rate,
            channels: config.channels,
            frame_size,
            config: opus_config,
            #[cfg(feature = "opus")]
            encoder: None,
            #[cfg(feature = "opus")]
            decoder: None,
        })
    }

    /// Get the compression ratio (variable for Opus)
    pub fn compression_ratio(&self) -> f32 {
        let uncompressed_bits = self.frame_size as f32 * 16.0 * self.channels as f32;
        let compressed_bits = self.config.bitrate as f32 * (self.frame_size as f32 / self.sample_rate as f32);
        compressed_bits / uncompressed_bits
    }

    /// Set the bitrate
    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        if bitrate < 6000 || bitrate > 510000 {
            return Err(CodecError::InvalidBitrate {
                bitrate,
                min: 6000,
                max: 510000,
            });
        }

        self.config.bitrate = bitrate;
        // Update live encoder if already initialized
        #[cfg(feature = "opus")]
        if let Some(ref mut enc) = self.encoder {
            enc.0.set_bitrate(opus::Bitrate::Bits(bitrate as i32))
                .map_err(|e| CodecError::EncodingFailed {
                    reason: format!("Failed to set bitrate: {e:?}"),
                })?;
        }
        debug!("Opus bitrate set to {} bps", bitrate);
        Ok(())
    }

    /// Set complexity level (0-10)
    pub fn set_complexity(&mut self, complexity: u8) -> Result<()> {
        if complexity > 10 {
            return Err(CodecError::invalid_config("Complexity must be 0-10"));
        }

        self.config.complexity = complexity;
        debug!("Opus complexity set to {}", complexity);
        Ok(())
    }

    /// Ensure the encoder is initialized
    #[cfg(feature = "opus")]
    fn ensure_encoder(&mut self) -> Result<()> {
        if self.encoder.is_some() {
            return Ok(());
        }

        let app = match self.config.application {
            OpusApplication::Voip => opus::Application::Voip,
            OpusApplication::Audio => opus::Application::Audio,
            OpusApplication::RestrictedLowDelay => opus::Application::LowDelay,
        };

        let channels = match self.channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            _ => return Err(CodecError::InitializationFailed {
                reason: format!("Unsupported channel count: {}", self.channels),
            }),
        };

        let mut encoder = opus::Encoder::new(self.sample_rate, channels, app)
            .map_err(|e| CodecError::InitializationFailed {
                reason: format!("Opus encoder creation failed: {e:?}"),
            })?;

        encoder.set_bitrate(opus::Bitrate::Bits(self.config.bitrate as i32))
            .map_err(|e| CodecError::InitializationFailed {
                reason: format!("Failed to set bitrate: {e:?}"),
            })?;

        encoder.set_vbr(self.config.vbr)
            .map_err(|e| CodecError::InitializationFailed {
                reason: format!("Failed to set VBR: {e:?}"),
            })?;

        if self.config.inband_fec {
            encoder.set_inband_fec(true)
                .map_err(|e| CodecError::InitializationFailed {
                    reason: format!("Failed to set inband FEC: {e:?}"),
                })?;
        }

        if self.config.packet_loss_perc > 0 {
            encoder.set_packet_loss_perc(self.config.packet_loss_perc as i32)
                .map_err(|e| CodecError::InitializationFailed {
                    reason: format!("Failed to set packet loss percentage: {e:?}"),
                })?;
        }

        debug!("Opus encoder initialized: {}Hz, {}ch, {}bps",
               self.sample_rate, self.channels, self.config.bitrate);
        self.encoder = Some(SyncEncoder(encoder));
        Ok(())
    }

    /// Ensure the decoder is initialized
    #[cfg(feature = "opus")]
    fn ensure_decoder(&mut self) -> Result<()> {
        if self.decoder.is_some() {
            return Ok(());
        }

        let channels = match self.channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            _ => return Err(CodecError::InitializationFailed {
                reason: format!("Unsupported channel count: {}", self.channels),
            }),
        };

        let decoder = opus::Decoder::new(self.sample_rate, channels)
            .map_err(|e| CodecError::InitializationFailed {
                reason: format!("Opus decoder creation failed: {e:?}"),
            })?;

        debug!("Opus decoder initialized: {}Hz, {}ch",
               self.sample_rate, self.channels);
        self.decoder = Some(SyncDecoder(decoder));
        Ok(())
    }
}

impl AudioCodec for OpusCodec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // Validate input
        validate_opus_frame(samples, SampleRate::from_hz(self.sample_rate))?;

        #[cfg(feature = "opus")]
        {
            self.ensure_encoder()?;
            let encoder = &mut self.encoder.as_mut()
                .ok_or_else(|| CodecError::InitializationFailed {
                    reason: "Encoder not initialized".to_string(),
                })?.0;
            let mut output = vec![0u8; 4000]; // max opus frame size
            let len = encoder.encode(samples, &mut output)
                .map_err(|e| CodecError::EncodingFailed {
                    reason: format!("Opus encoding failed: {e:?}"),
                })?;
            output.truncate(len);

            trace!("Opus encoded {} samples to {} bytes", samples.len(), len);
            Ok(output)
        }

        #[cfg(not(feature = "opus"))]
        {
            Err(CodecError::FeatureNotEnabled {
                feature: "opus".to_string(),
            })
        }
    }

    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }

        #[cfg(feature = "opus")]
        {
            self.ensure_decoder()?;
            let channels = self.channels;
            let decoder = &mut self.decoder.as_mut()
                .ok_or_else(|| CodecError::InitializationFailed {
                    reason: "Decoder not initialized".to_string(),
                })?.0;
            // 5760 = max frame size at 48kHz (120ms)
            let max_samples = 5760 * channels as usize;
            let mut output = vec![0i16; max_samples];
            let len = decoder.decode(data, &mut output, false)
                .map_err(|e| CodecError::DecodingFailed {
                    reason: format!("Opus decoding failed: {e:?}"),
                })?;
            output.truncate(len * channels as usize);

            trace!("Opus decoded {} bytes to {} samples", data.len(), output.len());
            Ok(output)
        }

        #[cfg(not(feature = "opus"))]
        {
            Err(CodecError::FeatureNotEnabled {
                feature: "opus".to_string(),
            })
        }
    }

    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: "Opus",
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: self.config.bitrate,
            frame_size: self.frame_size,
            payload_type: Some(111), // Dynamic payload type
        }
    }

    fn reset(&mut self) -> Result<()> {
        #[cfg(feature = "opus")]
        {
            if let Some(ref mut enc) = self.encoder {
                enc.0.reset_state()
                    .map_err(|e| CodecError::ResetFailed {
                        reason: format!("Opus encoder reset failed: {e:?}"),
                    })?;
            }
            if let Some(ref mut dec) = self.decoder {
                dec.0.reset_state()
                    .map_err(|e| CodecError::ResetFailed {
                        reason: format!("Opus decoder reset failed: {e:?}"),
                    })?;
            }
        }
        debug!("Opus codec reset");
        Ok(())
    }

    fn frame_size(&self) -> usize {
        self.frame_size
    }

    fn supports_variable_frame_size(&self) -> bool {
        true // Opus supports multiple frame sizes
    }
}

impl AudioCodecExt for OpusCodec {
    fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        // Validate input
        validate_opus_frame(samples, SampleRate::from_hz(self.sample_rate))?;

        #[cfg(feature = "opus")]
        {
            self.ensure_encoder()?;
            let encoder = &mut self.encoder.as_mut()
                .ok_or_else(|| CodecError::InitializationFailed {
                    reason: "Encoder not initialized".to_string(),
                })?.0;
            let len = encoder.encode(samples, output)
                .map_err(|e| CodecError::EncodingFailed {
                    reason: format!("Opus encoding failed: {e:?}"),
                })?;

            trace!("Opus encoded {} samples to {} bytes (buffer)", samples.len(), len);
            Ok(len)
        }

        #[cfg(not(feature = "opus"))]
        {
            Err(CodecError::FeatureNotEnabled {
                feature: "opus".to_string(),
            })
        }
    }

    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }

        #[cfg(feature = "opus")]
        {
            self.ensure_decoder()?;
            let channels = self.channels;
            let decoder = &mut self.decoder.as_mut()
                .ok_or_else(|| CodecError::InitializationFailed {
                    reason: "Decoder not initialized".to_string(),
                })?.0;
            let len = decoder.decode(data, output, false)
                .map_err(|e| CodecError::DecodingFailed {
                    reason: format!("Opus decoding failed: {e:?}"),
                })?;
            let total_samples = len * channels as usize;

            trace!("Opus decoded {} bytes to {} samples (buffer)", data.len(), total_samples);
            Ok(total_samples)
        }

        #[cfg(not(feature = "opus"))]
        {
            Err(CodecError::FeatureNotEnabled {
                feature: "opus".to_string(),
            })
        }
    }

    fn max_encoded_size(&self, input_samples: usize) -> usize {
        // Opus maximum frame size is 1275 bytes
        let bits_per_sample = self.config.bitrate as f32 / self.sample_rate as f32;
        let max_bytes = (input_samples as f32 * bits_per_sample / 8.0) as usize;
        max_bytes.min(1275)
    }

    fn max_decoded_size(&self, _input_bytes: usize) -> usize {
        // Opus can decode to various frame sizes
        let max_frame_ms = 60.0; // 60ms is the maximum
        ((self.sample_rate as f32 * max_frame_ms / 1000.0) as usize) * self.channels as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CodecConfig, CodecType, SampleRate};

    fn create_test_config() -> CodecConfig {
        CodecConfig::new(CodecType::Opus)
            .with_sample_rate(SampleRate::Rate48000)
            .with_channels(1)
            .with_frame_size_ms(20.0)
    }

    #[test]
    fn test_opus_creation() {
        let config = create_test_config();
        let codec = OpusCodec::new(config);
        assert!(codec.is_ok());

        let codec = codec.unwrap();
        assert_eq!(codec.frame_size(), 960); // 20ms at 48kHz

        let info = codec.info();
        assert_eq!(info.name, "Opus");
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(info.payload_type, Some(111));
    }

    #[test]
    #[cfg(feature = "opus")]
    fn test_encoding_decoding_roundtrip() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();

        // Create test signal (1kHz sine wave)
        let mut samples = Vec::new();
        for i in 0..960 {
            let t = i as f32 / 48000.0;
            let sample = ((2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 16000.0) as i16;
            samples.push(sample);
        }

        // Encode
        let encoded = codec.encode(&samples).unwrap();
        assert!(!encoded.is_empty());
        // Real opus should produce a compact representation
        assert!(encoded.len() < samples.len() * 2);

        // Decode
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), samples.len());
    }

    #[test]
    #[cfg(not(feature = "opus"))]
    fn test_encoding_without_feature_returns_error() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();

        let samples = vec![0i16; 960];
        let result = codec.encode(&samples);
        assert!(result.is_err());
    }

    #[test]
    fn test_bitrate_control() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();

        // Test valid bitrates
        assert!(codec.set_bitrate(32000).is_ok());
        assert!(codec.set_bitrate(128000).is_ok());

        // Test invalid bitrates
        assert!(codec.set_bitrate(1000).is_err());
        assert!(codec.set_bitrate(1000000).is_err());
    }

    #[test]
    fn test_complexity_control() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();

        // Test valid complexity levels
        for complexity in 0..=10 {
            assert!(codec.set_complexity(complexity).is_ok());
        }

        // Test invalid complexity
        assert!(codec.set_complexity(11).is_err());
    }
}
