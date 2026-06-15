//! G.729 Annex A/B codec adapter.
//!
//! This module adapts the vendored `rvoip-g729` implementation to
//! codec-core's [`crate::types::AudioCodec`] traits. The underlying codec
//! implements G.729 Annex A as its only speech path, with optional Annex B
//! VAD/DTX/CNG support. It does not implement full-complexity base G.729.

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecInfo, CodecType};
use ::g729 as g729_impl;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum G729Profile {
    AnnexA,
    AnnexAB,
}

impl G729Profile {
    const fn name(self) -> &'static str {
        match self {
            Self::AnnexA => "G729A",
            Self::AnnexAB => "G729BA",
        }
    }

    const fn annex_b(self) -> bool {
        matches!(self, Self::AnnexAB)
    }
}

/// G.729A/G.729AB codec implementation.
///
/// `G729Codec` accepts exactly one 10 ms, 80-sample, 8 kHz mono PCM frame per
/// encode call. In Annex A mode it always emits a 10-byte speech frame. In
/// Annex B mode it can emit 10-byte speech, 2-byte SID, or 0-byte no-data
/// payloads according to the underlying VAD/DTX/CNG state.
pub struct G729Codec {
    encoder: g729_impl::G729Encoder,
    decoder: g729_impl::G729Decoder,
    profile: G729Profile,
}

impl G729Codec {
    /// Number of PCM samples in one G.729 frame.
    pub const FRAME_SAMPLES: usize = g729_impl::FRAME_SAMPLES;

    /// Maximum size of an encoded speech payload in bytes.
    pub const MAX_ENCODED_BYTES: usize = g729_impl::SPEECH_FRAME_BYTES;

    /// Create a new G.729 codec from a codec-core configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration requests unsupported audio
    /// format settings or the unimplemented full-complexity base G.729 path.
    pub fn new(config: CodecConfig) -> Result<Self> {
        config.validate()?;
        validate_g729_format(&config)?;

        let profile = profile_from_config(&config)?;
        let g729_config = g729_impl::G729Config {
            annex_b: profile.annex_b(),
        };

        Ok(Self {
            encoder: g729_impl::G729Encoder::new(g729_config),
            decoder: g729_impl::G729Decoder::new(g729_config),
            profile,
        })
    }

    /// Return whether Annex B VAD/DTX/CNG behavior is enabled.
    pub fn annex_b_enabled(&self) -> bool {
        self.profile.annex_b()
    }
}

impl AudioCodec for G729Codec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        let mut output = [0u8; Self::MAX_ENCODED_BYTES];
        let written = self.encode_to_buffer(samples, &mut output)?;
        Ok(output[..written].to_vec())
    }

    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        let mut output = [0i16; Self::FRAME_SAMPLES];
        let written = self.decode_to_buffer(data, &mut output)?;
        Ok(output[..written].to_vec())
    }

    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: self.profile.name(),
            sample_rate: 8000,
            channels: 1,
            bitrate: 8000,
            frame_size: Self::FRAME_SAMPLES,
            payload_type: Some(18),
        }
    }

    fn reset(&mut self) -> Result<()> {
        let g729_config = g729_impl::G729Config {
            annex_b: self.profile.annex_b(),
        };
        self.encoder = g729_impl::G729Encoder::new(g729_config);
        self.decoder = g729_impl::G729Decoder::new(g729_config);
        Ok(())
    }

    fn frame_size(&self) -> usize {
        Self::FRAME_SAMPLES
    }
}

impl AudioCodecExt for G729Codec {
    fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        if samples.len() != Self::FRAME_SAMPLES {
            return Err(CodecError::InvalidFrameSize {
                expected: Self::FRAME_SAMPLES,
                actual: samples.len(),
            });
        }

        if output.len() < Self::MAX_ENCODED_BYTES {
            return Err(CodecError::BufferTooSmall {
                needed: Self::MAX_ENCODED_BYTES,
                actual: output.len(),
            });
        }

        let mut frame = [0i16; Self::FRAME_SAMPLES];
        frame.copy_from_slice(samples);

        let mut encoded = [0u8; Self::MAX_ENCODED_BYTES];
        let frame_type = self.encoder.encode(&frame, &mut encoded);
        let byte_len = frame_type.byte_len();
        output[..byte_len].copy_from_slice(&encoded[..byte_len]);
        Ok(byte_len)
    }

    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        if output.len() < Self::FRAME_SAMPLES {
            return Err(CodecError::BufferTooSmall {
                needed: Self::FRAME_SAMPLES,
                actual: output.len(),
            });
        }

        let decoded = self.decoder.decode_frame(data).map_err(map_g729_error)?;
        output[..Self::FRAME_SAMPLES].copy_from_slice(&decoded);
        Ok(Self::FRAME_SAMPLES)
    }

    fn max_encoded_size(&self, input_samples: usize) -> usize {
        if input_samples == 0 {
            0
        } else {
            input_samples.div_ceil(Self::FRAME_SAMPLES) * Self::MAX_ENCODED_BYTES
        }
    }

    fn max_decoded_size(&self, input_bytes: usize) -> usize {
        if matches!(
            input_bytes,
            0 | g729_impl::SID_FRAME_BYTES | g729_impl::SPEECH_FRAME_BYTES
        ) {
            Self::FRAME_SAMPLES
        } else {
            0
        }
    }
}

fn validate_g729_format(config: &CodecConfig) -> Result<()> {
    if config.sample_rate.hz() != 8000 {
        return Err(CodecError::InvalidSampleRate {
            rate: config.sample_rate.hz(),
            supported: vec![8000],
        });
    }

    if config.channels != 1 {
        return Err(CodecError::InvalidChannelCount {
            channels: config.channels,
            supported: vec![1],
        });
    }

    if let Some(frame_size_ms) = config.frame_size_ms {
        if !frame_size_ms.is_finite() || frame_size_ms <= 0.0 {
            return Err(CodecError::invalid_config(
                "G.729 frame size must be a positive finite duration",
            ));
        }

        let frame_samples = (8.0 * frame_size_ms).round() as usize;
        if frame_samples != G729Codec::FRAME_SAMPLES {
            return Err(CodecError::InvalidFrameSize {
                expected: G729Codec::FRAME_SAMPLES,
                actual: frame_samples,
            });
        }
    }

    Ok(())
}

fn profile_from_config(config: &CodecConfig) -> Result<G729Profile> {
    let params = &config.parameters.g729;

    #[allow(deprecated)]
    let annex_a_enabled = params.annex_a && params.reduced_complexity;
    if !annex_a_enabled {
        return Err(CodecError::invalid_config(
            "Full-complexity base G.729 is not implemented; use Annex A",
        ));
    }

    #[allow(deprecated)]
    let annex_b_enabled = params.annex_b && params.vad_enabled && params.cng_enabled;

    match config.codec_type {
        CodecType::G729A => Ok(G729Profile::AnnexA),
        CodecType::G729BA => {
            if annex_b_enabled {
                Ok(G729Profile::AnnexAB)
            } else {
                Err(CodecError::invalid_config(
                    "G729BA requires Annex B; use G729A for Annex A-only operation",
                ))
            }
        }
        CodecType::G729 => Ok(if annex_b_enabled {
            G729Profile::AnnexAB
        } else {
            G729Profile::AnnexA
        }),
        codec_type => Err(CodecError::unsupported_codec(codec_type.name())),
    }
}

fn map_g729_error(error: g729_impl::CodecError) -> CodecError {
    match error {
        g729_impl::CodecError::InvalidPcmLength { expected, got } => CodecError::InvalidFrameSize {
            expected,
            actual: got,
        },
        g729_impl::CodecError::InvalidBitstreamLength { expected, got } => {
            CodecError::InvalidPayload {
                details: format!(
                    "Invalid G.729 bitstream length: expected {expected:?}, got {got}"
                ),
            }
        }
        g729_impl::CodecError::InvalidParameterLength { expected, got } => {
            CodecError::InvalidPayload {
                details: format!("Invalid G.729 parameter length: expected {expected}, got {got}"),
            }
        }
        g729_impl::CodecError::InvalidFrameType { got } => CodecError::InvalidPayload {
            details: format!("Invalid G.729 frame type code: {got}"),
        },
        g729_impl::CodecError::IoUnavailable => {
            CodecError::decoding_failed("G.729 I/O helpers unavailable")
        }
        g729_impl::CodecError::BackendFailure => {
            CodecError::decoding_failed("G.729 backend failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::CodecFactory;
    use crate::types::{AudioCodec, SampleRate};

    const SAMPLE_RATE_HZ: f64 = 8_000.0;
    const TONE_HZ: f64 = 300.0;

    fn speech_samples() -> Vec<i16> {
        (0..G729Codec::FRAME_SAMPLES)
            .map(|sample| (sample * 128) as i16)
            .collect()
    }

    fn tone_samples(frame_count: usize) -> Vec<i16> {
        (0..G729Codec::FRAME_SAMPLES * frame_count)
            .map(|sample| {
                let t = sample as f64 / SAMPLE_RATE_HZ;
                (10_000.0 * (2.0 * std::f64::consts::PI * TONE_HZ * t).sin()).round() as i16
            })
            .collect()
    }

    fn round_trip_frames(codec: &mut G729Codec, samples: &[i16]) -> (Vec<i16>, Vec<usize>) {
        let mut decoded = Vec::with_capacity(samples.len());
        let mut encoded_lengths = Vec::with_capacity(samples.len() / G729Codec::FRAME_SAMPLES);

        for frame in samples.chunks_exact(G729Codec::FRAME_SAMPLES) {
            let encoded = codec.encode(frame).unwrap();
            encoded_lengths.push(encoded.len());
            decoded.extend(codec.decode(&encoded).unwrap());
        }

        (decoded, encoded_lengths)
    }

    fn correlation(a: &[i16], b: &[i16]) -> f64 {
        assert_eq!(a.len(), b.len());

        let mean_a = a.iter().map(|&sample| f64::from(sample)).sum::<f64>() / a.len() as f64;
        let mean_b = b.iter().map(|&sample| f64::from(sample)).sum::<f64>() / b.len() as f64;

        let mut covariance = 0.0;
        let mut variance_a = 0.0;
        let mut variance_b = 0.0;
        for (&sample_a, &sample_b) in a.iter().zip(b) {
            let centered_a = f64::from(sample_a) - mean_a;
            let centered_b = f64::from(sample_b) - mean_b;
            covariance += centered_a * centered_b;
            variance_a += centered_a * centered_a;
            variance_b += centered_b * centered_b;
        }

        covariance / (variance_a.sqrt() * variance_b.sqrt())
    }

    fn rms(samples: &[i16]) -> f64 {
        let power = samples
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum::<f64>()
            / samples.len() as f64;
        power.sqrt()
    }

    #[test]
    fn g729a_encodes_fixed_speech_frame() {
        let config = CodecConfig::new(CodecType::G729A)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);
        let mut codec = G729Codec::new(config).unwrap();

        let encoded = codec.encode(&speech_samples()).unwrap();
        assert_eq!(encoded.len(), g729_impl::SPEECH_FRAME_BYTES);
        assert!(!codec.annex_b_enabled());

        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), G729Codec::FRAME_SAMPLES);
    }

    #[test]
    fn g729a_tone_round_trip_preserves_audible_signal() {
        let config = CodecConfig::new(CodecType::G729A)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);
        let mut codec = G729Codec::new(config).unwrap();
        let input = tone_samples(50);

        let (decoded, encoded_lengths) = round_trip_frames(&mut codec, &input);
        assert_eq!(decoded.len(), input.len());
        assert!(encoded_lengths.iter().all(|&len| len == 10));

        let warmup = G729Codec::FRAME_SAMPLES * 5;
        let corr = correlation(&input[warmup..], &decoded[warmup..]);
        let decoded_rms = rms(&decoded[warmup..]);

        assert!(
            corr.abs() > 0.80,
            "G.729A 300 Hz tone correlation too low after round trip: {corr:.4}"
        );
        assert!(
            decoded_rms > 500.0,
            "G.729A 300 Hz tone degraded to near-silence: RMS {decoded_rms:.2}"
        );
    }

    #[test]
    fn g729ba_tone_round_trip_preserves_audible_signal() {
        let config = CodecConfig::new(CodecType::G729BA)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);
        let mut codec = G729Codec::new(config).unwrap();
        let input = tone_samples(50);

        let (decoded, encoded_lengths) = round_trip_frames(&mut codec, &input);
        assert_eq!(decoded.len(), input.len());
        assert!(encoded_lengths.iter().any(|&len| len == 10));

        let warmup = G729Codec::FRAME_SAMPLES * 5;
        let corr = correlation(&input[warmup..], &decoded[warmup..]);
        let decoded_rms = rms(&decoded[warmup..]);

        assert!(
            corr.abs() > 0.70,
            "G.729BA 300 Hz tone correlation too low after round trip: {corr:.4}"
        );
        assert!(
            decoded_rms > 500.0,
            "G.729BA 300 Hz tone degraded to near-silence: RMS {decoded_rms:.2}"
        );
    }

    #[test]
    fn g729ba_accepts_sid_and_nodata_payloads() {
        let config = CodecConfig::new(CodecType::G729BA)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);
        let mut codec = G729Codec::new(config).unwrap();
        assert!(codec.annex_b_enabled());

        let decoded_nodata = codec.decode(&[]).unwrap();
        assert_eq!(decoded_nodata.len(), G729Codec::FRAME_SAMPLES);

        let sid = [0u8; g729_impl::SID_FRAME_BYTES];
        let decoded_sid = codec.decode(&sid).unwrap();
        assert_eq!(decoded_sid.len(), G729Codec::FRAME_SAMPLES);
    }

    #[test]
    fn g729_rejects_full_complexity_request() {
        let mut config = CodecConfig::new(CodecType::G729);
        config.parameters.g729.annex_a = false;

        let error = match G729Codec::new(config) {
            Ok(_) => panic!("full-complexity G.729 config should be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, CodecError::InvalidConfig { .. }));
    }

    #[test]
    #[allow(deprecated)]
    fn g729_rejects_legacy_full_complexity_request() {
        let mut config = CodecConfig::new(CodecType::G729);
        config.parameters.g729.reduced_complexity = false;

        let error = match G729Codec::new(config) {
            Ok(_) => panic!("legacy full-complexity G.729 config should be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, CodecError::InvalidConfig { .. }));
    }

    #[test]
    fn g729_validates_frame_size() {
        let config = CodecConfig::new(CodecType::G729A).with_frame_size_ms(20.0);
        let error = match G729Codec::new(config) {
            Ok(_) => panic!("20 ms G.729 frames should be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, CodecError::InvalidFrameSize { .. }));
    }

    #[test]
    fn factory_supports_g729_names_and_payload_type() {
        assert!(CodecFactory::is_supported("G729"));
        assert!(CodecFactory::is_supported("G.729"));
        assert!(CodecFactory::is_supported("G729A"));
        assert!(CodecFactory::is_supported("G729AB"));
        assert!(CodecFactory::is_supported("G729BA"));

        let codec =
            CodecFactory::create_by_name("G729A", CodecConfig::new(CodecType::G729)).unwrap();
        assert_eq!(codec.info().name, "G729A");

        let codec =
            CodecFactory::create_by_name("G729AB", CodecConfig::new(CodecType::G729)).unwrap();
        assert_eq!(codec.info().name, "G729BA");

        let codec =
            CodecFactory::create_by_payload_type(18, CodecConfig::new(CodecType::G729)).unwrap();
        assert_eq!(codec.info().payload_type, Some(18));
    }

    #[test]
    fn reset_preserves_selected_profile() {
        let config = CodecConfig::new(CodecType::G729A);
        let mut codec = G729Codec::new(config).unwrap();
        codec.reset().unwrap();

        let encoded = codec.encode(&speech_samples()).unwrap();
        assert_eq!(encoded.len(), g729_impl::SPEECH_FRAME_BYTES);
        assert!(!codec.annex_b_enabled());
    }
}
