//! Opus Audio Codec Implementation
//!
//! This module implements the Opus codec, a modern audio codec standardized
//! by the Internet Engineering Task Force (IETF) in RFC 6716. Opus combines
//! the best features of both speech and music codecs with very low latency.

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecInfo, SampleRate};
use crate::utils::validate_opus_frame;
use std::sync::Mutex;
use tracing::{debug, trace};

const MAX_OPUS_PACKET_BYTES: usize = 1275;

// Re-export OpusApplication from types to avoid duplication
pub use crate::types::OpusApplication;

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
    /// The real libopus encoder. A mutex is used only to satisfy the codec
    /// trait's `Sync` bound; codec operations already require `&mut self`.
    encoder: Mutex<opus::Encoder>,
    /// The matching libopus decoder.
    decoder: Mutex<opus::Decoder>,
}

/// `Opus` codec configuration.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct OpusConfig {
    /// Application type (`VoIP`, audio, or low delay).
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
    /// Create a new `Opus` codec.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported parameters or if libopus cannot create
    /// and configure the encoder and decoder.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(config: CodecConfig) -> Result<Self> {
        if config.codec_type != crate::types::CodecType::Opus {
            return Err(CodecError::unsupported_codec(format!(
                "{} configuration passed to OpusCodec",
                config.codec_type
            )));
        }
        config.validate()?;
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
        let frame_duration_ms = config.frame_size_ms.unwrap_or(20.0);
        let frame_size = opus_frame_size(sample_rate, frame_duration_ms).ok_or_else(|| {
            CodecError::invalid_config(format!(
                "Unsupported Opus frame duration: {frame_duration_ms}ms"
            ))
        })?;

        // The codec-specific field was the public Opus configuration surface
        // before the real backend landed, so it remains authoritative. The
        // generic `with_bitrate` convenience setter keeps both values in sync.
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

        if opus_config.complexity > 10 {
            return Err(CodecError::invalid_config(
                "Opus complexity must be in the range 0-10",
            ));
        }
        if opus_config.packet_loss_perc > 100 {
            return Err(CodecError::invalid_config(
                "Opus packet loss percentage must be in the range 0-100",
            ));
        }
        if !(6_000..=510_000).contains(&opus_config.bitrate) {
            return Err(CodecError::InvalidBitrate {
                bitrate: opus_config.bitrate,
                min: 6_000,
                max: 510_000,
            });
        }

        let opus_channels = match config.channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            _ => unreachable!("channel count was validated above"),
        };
        let application = match opus_config.application {
            OpusApplication::Voip => opus::Application::Voip,
            OpusApplication::Audio => opus::Application::Audio,
            OpusApplication::RestrictedLowDelay => opus::Application::LowDelay,
        };

        let mut encoder =
            opus::Encoder::new(sample_rate, opus_channels, application).map_err(|error| {
                CodecError::initialization_failed(format!(
                    "libopus encoder creation failed: {error}"
                ))
            })?;
        let decoder = opus::Decoder::new(sample_rate, opus_channels).map_err(|error| {
            CodecError::initialization_failed(format!("libopus decoder creation failed: {error}"))
        })?;

        configure_encoder(&mut encoder, &opus_config)?;

        debug!(
            "Creating Opus codec: {}Hz, {}ch, {}bps, {:?} mode",
            sample_rate, config.channels, opus_config.bitrate, opus_config.application
        );

        Ok(Self {
            sample_rate,
            channels: config.channels,
            frame_size,
            config: opus_config,
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
        })
    }

    /// Get the compression ratio (variable for `Opus`).
    #[allow(clippy::cast_precision_loss)]
    pub fn compression_ratio(&self) -> f32 {
        let uncompressed_bits = self.frame_size as f32 * 16.0 * f32::from(self.channels);
        let compressed_bits =
            self.config.bitrate as f32 * (self.frame_size as f32 / self.sample_rate as f32);
        compressed_bits / uncompressed_bits
    }

    /// Set the bitrate.
    ///
    /// # Errors
    ///
    /// Returns an error if the bitrate is outside libopus's supported range
    /// or if libopus rejects the update.
    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        if !(6_000..=510_000).contains(&bitrate) {
            return Err(CodecError::InvalidBitrate {
                bitrate,
                min: 6_000,
                max: 510_000,
            });
        }

        let backend_bitrate = i32::try_from(bitrate)
            .map_err(|_| CodecError::internal_error("validated Opus bitrate did not fit i32"))?;
        self.encoder
            .get_mut()
            .map_err(|_| CodecError::internal_error("Opus encoder lock was poisoned"))?
            .set_bitrate(opus::Bitrate::Bits(backend_bitrate))
            .map_err(|error| {
                CodecError::encoding_failed(format!("libopus bitrate update failed: {error}"))
            })?;
        self.config.bitrate = bitrate;
        debug!("Opus bitrate set to {} bps", bitrate);
        Ok(())
    }

    /// Set complexity level (0-10).
    ///
    /// # Errors
    ///
    /// Returns an error for an out-of-range value or if libopus rejects the
    /// update.
    pub fn set_complexity(&mut self, complexity: u8) -> Result<()> {
        if complexity > 10 {
            return Err(CodecError::invalid_config("Complexity must be 0-10"));
        }

        self.encoder
            .get_mut()
            .map_err(|_| CodecError::internal_error("Opus encoder lock was poisoned"))?
            .set_complexity(i32::from(complexity))
            .map_err(|error| {
                CodecError::encoding_failed(format!("libopus complexity update failed: {error}"))
            })?;
        self.config.complexity = complexity;
        debug!("Opus complexity set to {}", complexity);
        Ok(())
    }

    fn validate_input(&self, samples: &[i16]) -> Result<()> {
        let channels = usize::from(self.channels);
        if !samples.len().is_multiple_of(channels) {
            return Err(CodecError::invalid_format(format!(
                "Opus input sample count {} is not divisible by {} channels",
                samples.len(),
                self.channels
            )));
        }
        validate_opus_frame(
            &samples[..samples.len() / channels],
            SampleRate::from_hz(self.sample_rate),
        )
    }
}

fn opus_frame_size(sample_rate: u32, frame_duration_ms: f32) -> Option<usize> {
    let divisor = match frame_duration_ms {
        2.5 => 400,
        5.0 => 200,
        10.0 => 100,
        20.0 => 50,
        40.0 => 25,
        60.0 => return usize::try_from(sample_rate.checked_mul(3)? / 50).ok(),
        _ => return None,
    };
    usize::try_from(sample_rate / divisor).ok()
}

fn configure_encoder(encoder: &mut opus::Encoder, config: &OpusConfig) -> Result<()> {
    let configure = |operation: &'static str, result: opus::Result<()>| {
        result.map_err(|error| {
            CodecError::initialization_failed(format!("libopus {operation} failed: {error}"))
        })
    };

    let bitrate = i32::try_from(config.bitrate)
        .map_err(|_| CodecError::invalid_config("Opus bitrate does not fit i32"))?;
    configure(
        "bitrate configuration",
        encoder.set_bitrate(opus::Bitrate::Bits(bitrate)),
    )?;
    configure("VBR configuration", encoder.set_vbr(config.vbr))?;
    configure(
        "constrained VBR configuration",
        encoder.set_vbr_constraint(config.cvbr),
    )?;
    configure(
        "complexity configuration",
        encoder.set_complexity(i32::from(config.complexity)),
    )?;
    configure(
        "in-band FEC configuration",
        encoder.set_inband_fec(config.inband_fec),
    )?;
    configure(
        "packet-loss configuration",
        encoder.set_packet_loss_perc(i32::from(config.packet_loss_perc)),
    )?;
    configure("DTX configuration", encoder.set_dtx(config.dtx))?;
    configure(
        "channel configuration",
        encoder.set_force_channels(if config.force_mono {
            Some(opus::Channels::Mono)
        } else {
            None
        }),
    )?;
    Ok(())
}

impl AudioCodec for OpusCodec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        self.validate_input(samples)?;
        let mut encoded = vec![0; MAX_OPUS_PACKET_BYTES];
        let encoded_len = self
            .encoder
            .get_mut()
            .map_err(|_| CodecError::internal_error("Opus encoder lock was poisoned"))?
            .encode(samples, &mut encoded)
            .map_err(|error| {
                CodecError::encoding_failed(format!("libopus encoding failed: {error}"))
            })?;
        encoded.truncate(encoded_len);

        trace!(
            "Opus encoded {} samples to {} bytes",
            samples.len(),
            encoded.len()
        );

        Ok(encoded)
    }

    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }

        // A decoder output buffer must accommodate the maximum Opus packet
        // duration (120ms), regardless of the configured encoder frame size.
        let max_samples_per_channel = self.sample_rate as usize * 120 / 1000;
        let mut decoded = vec![0; max_samples_per_channel * usize::from(self.channels)];
        let decoded_samples_per_channel = self
            .decoder
            .get_mut()
            .map_err(|_| CodecError::internal_error("Opus decoder lock was poisoned"))?
            .decode(data, &mut decoded, false)
            .map_err(|error| {
                CodecError::decoding_failed(format!("libopus decoding failed: {error}"))
            })?;
        decoded.truncate(decoded_samples_per_channel * usize::from(self.channels));

        trace!(
            "Opus decoded {} bytes to {} samples",
            data.len(),
            decoded.len()
        );

        Ok(decoded)
    }

    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: "Opus",
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: self.config.bitrate,
            frame_size: self.frame_size,
            payload_type: None, // Opus payload types are negotiated dynamically
        }
    }

    fn reset(&mut self) -> Result<()> {
        self.encoder
            .get_mut()
            .map_err(|_| CodecError::internal_error("Opus encoder lock was poisoned"))?
            .reset_state()
            .map_err(|error| CodecError::ResetFailed {
                reason: format!("libopus encoder reset failed: {error}"),
            })?;
        self.decoder
            .get_mut()
            .map_err(|_| CodecError::internal_error("Opus decoder lock was poisoned"))?
            .reset_state()
            .map_err(|error| CodecError::ResetFailed {
                reason: format!("libopus decoder reset failed: {error}"),
            })?;
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
        self.validate_input(samples)?;
        // Opus is variable-rate, so the exact encoded size is not knowable
        // without advancing the stateful encoder. Require the documented
        // worst-case packet capacity and fail before touching encoder state.
        if output.len() < MAX_OPUS_PACKET_BYTES {
            return Err(CodecError::BufferTooSmall {
                needed: MAX_OPUS_PACKET_BYTES,
                actual: output.len(),
            });
        }
        let encoded_len = self
            .encoder
            .get_mut()
            .map_err(|_| CodecError::internal_error("Opus encoder lock was poisoned"))?
            .encode(samples, output)
            .map_err(|error| match error.code() {
                opus::ErrorCode::BufferTooSmall => CodecError::BufferTooSmall {
                    needed: MAX_OPUS_PACKET_BYTES,
                    actual: output.len(),
                },
                _ => CodecError::encoding_failed(format!("libopus encoding failed: {error}")),
            })?;

        trace!(
            "Opus encoded {} samples to {} bytes (zero-alloc)",
            samples.len(),
            encoded_len
        );

        Ok(encoded_len)
    }

    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }

        let channels = usize::from(self.channels);
        let decoder = self
            .decoder
            .get_mut()
            .map_err(|_| CodecError::internal_error("Opus decoder lock was poisoned"))?;
        let decoded_samples_per_channel = decoder.get_nb_samples(data).map_err(|error| {
            CodecError::decoding_failed(format!("invalid Opus packet: {error}"))
        })?;
        let needed = decoded_samples_per_channel * channels;
        if output.len() < needed {
            return Err(CodecError::BufferTooSmall {
                needed,
                actual: output.len(),
            });
        }
        let decoded_samples_per_channel =
            decoder
                .decode(data, output, false)
                .map_err(|error| match error.code() {
                    opus::ErrorCode::BufferTooSmall => CodecError::BufferTooSmall {
                        needed,
                        actual: output.len(),
                    },
                    _ => CodecError::decoding_failed(format!("libopus decoding failed: {error}")),
                })?;
        let decoded_len = decoded_samples_per_channel * channels;

        trace!(
            "Opus decoded {} bytes to {} samples (zero-alloc)",
            data.len(),
            decoded_len
        );

        Ok(decoded_len)
    }

    fn max_encoded_size(&self, _input_samples: usize) -> usize {
        MAX_OPUS_PACKET_BYTES
    }

    fn max_decoded_size(&self, _input_bytes: usize) -> usize {
        self.sample_rate as usize * 120 / 1000 * usize::from(self.channels)
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
        assert_eq!(info.payload_type, None);
    }

    #[test]
    fn test_encoding_decoding_roundtrip() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();

        // Create a deterministic square-wave test signal without lossy casts.
        let samples: Vec<i16> = (0..960)
            .map(|index| {
                if (index / 24) % 2 == 0 {
                    16_000
                } else {
                    -16_000
                }
            })
            .collect();

        // Encode
        let encoded = codec.encode(&samples).unwrap();
        assert!(!encoded.is_empty());

        // Decode
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), samples.len());
    }

    #[test]
    fn test_real_backend_output_depends_on_pcm_input() {
        let silence = vec![0; 960];
        let tone: Vec<i16> = (0..960)
            .map(|index| {
                if (index / 55) % 2 == 0 {
                    12_000
                } else {
                    -12_000
                }
            })
            .collect();

        // Use independent encoders so codec history cannot explain a packet
        // difference. The retired simulator ignored PCM input and emitted the
        // same counter bytes for both frames.
        let silence_packet = OpusCodec::new(create_test_config())
            .unwrap()
            .encode(&silence)
            .unwrap();
        let tone_packet = OpusCodec::new(create_test_config())
            .unwrap()
            .encode(&tone)
            .unwrap();

        assert_ne!(silence_packet, tone_packet);
        assert_eq!(
            opus::packet::get_nb_samples(&tone_packet, 48_000).unwrap(),
            960
        );
    }

    #[cfg(feature = "opus-sim")]
    #[test]
    fn test_deprecated_opus_sim_alias_uses_real_backend() {
        let mut codec = OpusCodec::new(create_test_config()).unwrap();
        let packet = codec.encode(&[0; 960]).unwrap();

        assert_eq!(opus::packet::get_nb_samples(&packet, 48_000).unwrap(), 960);
        assert_eq!(codec.decode(&packet).unwrap().len(), 960);
    }

    #[test]
    fn test_opus_bitrate_configuration_remains_backward_compatible() {
        let mut specific_config = create_test_config();
        assert_eq!(specific_config.bitrate, Some(64_000));
        specific_config.parameters.opus.bitrate = 32_000;
        let specific_codec = OpusCodec::new(specific_config).unwrap();
        assert_eq!(specific_codec.info().bitrate, 32_000);

        let generic_config = create_test_config().with_bitrate(48_000);
        assert_eq!(generic_config.bitrate, Some(48_000));
        assert_eq!(generic_config.parameters.opus.bitrate, 48_000);
        let generic_codec = OpusCodec::new(generic_config).unwrap();
        assert_eq!(generic_codec.info().bitrate, 48_000);
    }

    #[test]
    fn test_non_opus_configuration_is_rejected() {
        assert!(matches!(
            OpusCodec::new(CodecConfig::g711_pcmu()),
            Err(CodecError::UnsupportedCodec { .. })
        ));
    }

    #[test]
    fn test_invalid_frames_are_rejected() {
        let mut codec = OpusCodec::new(create_test_config()).unwrap();
        assert!(matches!(
            codec.encode(&[0; 123]),
            Err(CodecError::InvalidFrameSize { .. })
        ));
        assert!(matches!(
            codec.decode(&[]),
            Err(CodecError::InvalidPayload { .. })
        ));
        assert!(matches!(
            codec.decode(&[0x03]),
            Err(CodecError::DecodingFailed { .. })
        ));

        let invalid_duration = create_test_config().with_frame_size_ms(7.0);
        assert!(matches!(
            OpusCodec::new(invalid_duration),
            Err(CodecError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn test_stereo_and_variable_duration_buffer_contracts() {
        let stereo_20ms_config = create_test_config().with_channels(2);
        let mut stereo_encoder = OpusCodec::new(stereo_20ms_config.clone()).unwrap();
        assert!(matches!(
            stereo_encoder.encode(&[0; 1_919]),
            Err(CodecError::InvalidFormat { .. })
        ));

        let stereo_frame = vec![1_000; 1_920];
        let mut tiny_encoded = [0; 1];
        let tiny_result = stereo_encoder.encode_to_buffer(&stereo_frame, &mut tiny_encoded);
        assert!(
            matches!(
                tiny_result,
                Err(CodecError::BufferTooSmall {
                    needed: MAX_OPUS_PACKET_BYTES,
                    actual: 1
                })
            ),
            "unexpected tiny-buffer result: {tiny_result:?}"
        );

        let mut long_encoder = OpusCodec::new(
            create_test_config()
                .with_channels(2)
                .with_frame_size_ms(60.0),
        )
        .unwrap();
        let long_stereo_frame: Vec<i16> = (0..2_880)
            .flat_map(|index| {
                let sample = if (index / 27) % 2 == 0 {
                    10_000
                } else {
                    -10_000
                };
                [sample, -sample]
            })
            .collect();
        let long_packet = long_encoder.encode(&long_stereo_frame).unwrap();

        // A decoder configured for 20 ms must still size the output from the
        // packet on the wire, which may legally contain a longer duration.
        let mut decoder = OpusCodec::new(stereo_20ms_config).unwrap();
        let mut configured_size_only = vec![0; 1_920];
        assert!(matches!(
            decoder.decode_to_buffer(&long_packet, &mut configured_size_only),
            Err(CodecError::BufferTooSmall {
                needed: 5_760,
                actual: 1_920
            })
        ));

        let mut full_output = vec![0; decoder.max_decoded_size(long_packet.len())];
        assert_eq!(
            decoder
                .decode_to_buffer(&long_packet, &mut full_output)
                .unwrap(),
            5_760
        );
    }

    #[test]
    fn test_bitrate_control() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();

        // Test valid bitrates
        assert!(codec.set_bitrate(32_000).is_ok());
        assert!(codec.set_bitrate(128_000).is_ok());

        // Test invalid bitrates
        assert!(codec.set_bitrate(1_000).is_err());
        assert!(codec.set_bitrate(1_000_000).is_err());
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
