//! G.722 Audio Codec Implementation
//!
//! Wraps the [`ezk-g722`](https://docs.rs/ezk-g722) crate's `libg722` module
//! (an ITU-T G.722 reference implementation translated from
//! [sippy/libg722](https://github.com/sippy/libg722)) behind this crate's
//! [`AudioCodec`] trait.
//!
//! ## Notes
//!
//! - G.722 encodes 16 kHz, 16-bit mono PCM at a fixed 64 kbit/s in this
//!   implementation (ITU-T G.722 "Mode 1"; the 56 and 48 kbit/s modes exist
//!   in the underlying library but aren't exposed here since RTP payload
//!   type 9 always means 64 kbit/s).
//! - The encoder operates on sample *pairs*: input must have an even
//!   sample count, and produces exactly `samples.len() / 2` encoded bytes.
//! - RFC 3551 fixes the RTP clock rate for PT 9 at 8 kHz even though the
//!   actual PCM sample rate is 16 kHz, a historical quirk of how G.722 was
//!   registered. [`CodecInfo::sample_rate`] here reports the true 16 kHz
//!   PCM rate; callers building RTP timestamps still need to use an 8 kHz
//!   clock for this payload type, same as `media-worker`'s
//!   `SessionCodec::rtp_clock_rate` does today.
//!
//! ## Usage
//!
//! ```rust
//! use codec_core::codecs::CodecFactory;
//! use codec_core::types::CodecConfig;
//!
//! let mut encoder = CodecFactory::create(CodecConfig::g722())?;
//! let mut decoder = CodecFactory::create(CodecConfig::g722())?;
//!
//! let samples = vec![0i16; 320]; // 20ms at 16kHz
//! let encoded = encoder.encode(&samples)?;
//! let decoded = decoder.decode(&encoded)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecInfo};
use ezk_g722::libg722::{
    decoder::Decoder as LibG722Decoder, encoder::Encoder as LibG722Encoder, Bitrate,
};

const SAMPLE_RATE: u32 = 16_000;
const BITRATE: u32 = 64_000;
const DEFAULT_FRAME_SIZE: usize = 320; // 20ms at 16kHz

fn new_encoder() -> LibG722Encoder {
    LibG722Encoder::new(Bitrate::Mode1_64000, false, false)
}

fn new_decoder() -> LibG722Decoder {
    LibG722Decoder::new(Bitrate::Mode1_64000, false, false)
}

/// G.722 codec implementation (ITU-T G.722, 64 kbit/s mode).
pub struct G722Codec {
    encoder: LibG722Encoder,
    decoder: LibG722Decoder,
    frame_size: usize,
}

impl G722Codec {
    /// Create a new G.722 codec from configuration.
    pub fn new(config: CodecConfig) -> Result<Self> {
        if config.sample_rate.hz() != SAMPLE_RATE {
            return Err(CodecError::InvalidSampleRate {
                rate: config.sample_rate.hz(),
                supported: vec![SAMPLE_RATE],
            });
        }

        if config.channels != 1 {
            return Err(CodecError::InvalidChannelCount {
                channels: config.channels,
                supported: vec![1],
            });
        }

        let frame_size = if let Some(frame_ms) = config.frame_size_ms {
            let samples_per_ms = SAMPLE_RATE as f32 / 1000.0;
            (samples_per_ms * frame_ms) as usize
        } else {
            DEFAULT_FRAME_SIZE
        };

        if frame_size == 0 || frame_size % 2 != 0 {
            return Err(CodecError::InvalidFrameSize {
                expected: DEFAULT_FRAME_SIZE,
                actual: frame_size,
            });
        }

        Ok(Self {
            encoder: new_encoder(),
            decoder: new_decoder(),
            frame_size,
        })
    }
}

impl AudioCodec for G722Codec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        if samples.is_empty() {
            return Err(CodecError::invalid_format("Input samples cannot be empty"));
        }
        if !samples.len().is_multiple_of(2) {
            return Err(CodecError::invalid_format(
                "G.722 encoder requires an even number of samples (it encodes sample pairs)",
            ));
        }

        Ok(self.encoder.encode(samples))
    }

    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }

        Ok(self.decoder.decode(data))
    }

    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: "G722",
            sample_rate: SAMPLE_RATE,
            channels: 1,
            bitrate: BITRATE,
            frame_size: self.frame_size,
            payload_type: Some(9),
        }
    }

    fn reset(&mut self) -> Result<()> {
        // The ADPCM predictor/quantizer state lives entirely inside the
        // encoder/decoder structs; replacing them with fresh instances is
        // the same as clearing that state.
        self.encoder = new_encoder();
        self.decoder = new_decoder();
        Ok(())
    }

    fn frame_size(&self) -> usize {
        self.frame_size
    }

    fn supports_variable_frame_size(&self) -> bool {
        true
    }
}

impl AudioCodecExt for G722Codec {
    fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        let encoded = self.encode(samples)?;
        if output.len() < encoded.len() {
            return Err(CodecError::BufferTooSmall {
                needed: encoded.len(),
                actual: output.len(),
            });
        }
        output[..encoded.len()].copy_from_slice(&encoded);
        Ok(encoded.len())
    }

    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        let decoded = self.decode(data)?;
        if output.len() < decoded.len() {
            return Err(CodecError::BufferTooSmall {
                needed: decoded.len(),
                actual: output.len(),
            });
        }
        output[..decoded.len()].copy_from_slice(&decoded);
        Ok(decoded.len())
    }

    fn max_encoded_size(&self, input_samples: usize) -> usize {
        input_samples.div_ceil(2) // 1 encoded byte per sample pair
    }

    fn max_decoded_size(&self, input_bytes: usize) -> usize {
        input_bytes * 2 // 2 PCM samples per encoded byte
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codec() -> G722Codec {
        G722Codec::new(CodecConfig::g722()).unwrap()
    }

    /// Sine wave test signal, `len` samples at `SAMPLE_RATE`.
    fn sine_wave(len: usize, frequency: f32, amplitude: f32) -> Vec<i16> {
        (0..len)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                (amplitude * (2.0 * std::f32::consts::PI * frequency * t).sin()) as i16
            })
            .collect()
    }

    /// Signal-to-noise ratio in dB between two equal-length sample sets,
    /// comparing `original[i]` against `decoded[i]` directly (no delay
    /// compensation).
    fn snr_db(original: &[i16], decoded: &[i16]) -> f64 {
        assert_eq!(original.len(), decoded.len());
        let signal_power: f64 = original.iter().map(|&s| (s as f64).powi(2)).sum();
        let noise_power: f64 = original
            .iter()
            .zip(decoded)
            .map(|(&o, &d)| ((o as f64) - (d as f64)).powi(2))
            .sum();
        if noise_power == 0.0 {
            return f64::INFINITY;
        }
        10.0 * (signal_power / noise_power).log10()
    }

    /// Best SNR (in dB) over a small window of sample-alignment offsets.
    ///
    /// The encode+decode QMF analysis/synthesis filter pair introduces a
    /// fixed algorithmic group delay (an inherent property of G.722, not
    /// an rvoip- or ezk-g722-specific artifact: any G.722 implementation
    /// has it, and callers building real-time audio pipelines already
    /// budget for codec algorithmic delay same as with any other codec).
    /// A direct sample-by-sample comparison at zero delay massively
    /// understates quality, so this searches nearby offsets for the one
    /// that actually aligns encoder input with decoder output before
    /// scoring, the same way a delay-tolerant quality check would in a
    /// real test rig (e.g. comparing against a reference decoder).
    fn best_aligned_snr_db(original: &[i16], decoded: &[i16], max_lag: usize) -> f64 {
        let margin = max_lag;
        let start = margin;
        let end = original.len().saturating_sub(margin);
        assert!(start < end, "signal too short for the requested lag search");

        (0..=(2 * max_lag))
            .map(|offset| offset as isize - max_lag as isize)
            .map(|lag| {
                let signal_power: f64 = original[start..end]
                    .iter()
                    .map(|&s| (s as f64).powi(2))
                    .sum();
                let noise_power: f64 = (start..end)
                    .map(|i| {
                        let o = original[i] as f64;
                        let d = decoded[(i as isize + lag) as usize] as f64;
                        (o - d).powi(2)
                    })
                    .sum();
                if noise_power == 0.0 {
                    f64::INFINITY
                } else {
                    10.0 * (signal_power / noise_power).log10()
                }
            })
            .fold(f64::NEG_INFINITY, f64::max)
    }

    #[test]
    fn info_reports_g722_wideband_parameters() {
        let info = codec().info();
        assert_eq!(info.name, "G722");
        assert_eq!(info.sample_rate, 16_000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bitrate, 64_000);
        assert_eq!(info.payload_type, Some(9));
        assert_eq!(info.frame_size, 320);
    }

    #[test]
    fn encode_produces_one_byte_per_sample_pair() {
        let mut codec = codec();
        let samples = sine_wave(320, 440.0, 8000.0);

        let encoded = codec.encode(&samples).unwrap();

        assert_eq!(encoded.len(), samples.len() / 2);
    }

    #[test]
    fn encode_rejects_odd_sample_count() {
        let mut codec = codec();
        let samples = vec![0i16; 321];

        assert!(codec.encode(&samples).is_err());
    }

    #[test]
    fn encode_rejects_empty_input() {
        let mut codec = codec();
        assert!(codec.encode(&[]).is_err());
    }

    #[test]
    fn decode_rejects_empty_input() {
        let mut codec = codec();
        assert!(codec.decode(&[]).is_err());
    }

    #[test]
    fn decode_produces_two_samples_per_encoded_byte() {
        let mut codec = codec();
        let samples = sine_wave(320, 440.0, 8000.0);
        let encoded = codec.encode(&samples).unwrap();

        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded.len(), encoded.len() * 2);
    }

    #[test]
    fn roundtrip_preserves_reasonable_signal_quality() {
        let mut encoder = codec();
        let mut decoder = codec();
        let samples = sine_wave(1600, 440.0, 8000.0); // 100ms @ 440Hz

        let encoded = encoder.encode(&samples).unwrap();
        let decoded = decoder.decode(&encoded).unwrap();

        assert_eq!(decoded.len(), samples.len());
        // Delay-tolerant: see best_aligned_snr_db's docs for why a direct,
        // zero-lag comparison isn't the right way to score a subband
        // codec's roundtrip fidelity.
        let snr = best_aligned_snr_db(&samples, &decoded, 32);
        assert!(
            snr > 30.0,
            "G.722 roundtrip SNR too low even after delay alignment: {snr:.1} dB \
             (expected > 30 dB for a 440Hz tone at this amplitude)"
        );
    }

    #[test]
    fn zero_lag_snr_understates_quality_because_of_qmf_group_delay() {
        // Documents why roundtrip_preserves_reasonable_signal_quality
        // can't just use snr_db directly: the encode+decode QMF pair's
        // algorithmic delay means naive zero-lag comparison looks like
        // near-total noise even though the codec is working correctly,
        // as the large gap between these two numbers demonstrates.
        let mut encoder = codec();
        let mut decoder = codec();
        let samples = sine_wave(1600, 440.0, 8000.0);
        let decoded = decoder.decode(&encoder.encode(&samples).unwrap()).unwrap();

        let zero_lag = snr_db(&samples, &decoded);
        let aligned = best_aligned_snr_db(&samples, &decoded, 32);

        assert!(
            aligned > zero_lag + 20.0,
            "expected alignment to reveal much higher fidelity than the \
             naive zero-lag comparison: zero_lag={zero_lag:.1} dB aligned={aligned:.1} dB"
        );
    }

    #[test]
    fn reset_clears_adpcm_state_so_output_matches_a_fresh_codec() {
        let samples = sine_wave(320, 440.0, 8000.0);

        // Encoder A: encode once, reset, encode again.
        let mut encoder_a = codec();
        let _ = encoder_a.encode(&samples).unwrap();
        encoder_a.reset().unwrap();
        let after_reset = encoder_a.encode(&samples).unwrap();

        // Encoder B: fresh instance, encode once.
        let mut encoder_b = codec();
        let fresh = encoder_b.encode(&samples).unwrap();

        assert_eq!(
            after_reset, fresh,
            "reset() must clear ADPCM predictor/quantizer state, not just \
             let the caller re-encode with stale state carried over"
        );
    }

    #[test]
    fn state_carries_across_encode_calls_without_reset() {
        // ADPCM is stateful: encoding the same samples twice in a row
        // without a reset should generally NOT reproduce the exact same
        // bytes the very first call produced, because the predictor state
        // evolved. This is the mirror image of the reset test above.
        let samples = sine_wave(320, 440.0, 8000.0);
        let mut encoder = codec();

        let first = encoder.encode(&samples).unwrap();
        let second = encoder.encode(&samples).unwrap();

        assert_ne!(
            first, second,
            "encoding identical input twice without reset should reflect \
             evolved predictor state, not reset implicitly between calls"
        );
    }

    #[test]
    fn encode_to_buffer_matches_encode() {
        let mut codec_a = codec();
        let mut codec_b = codec();
        let samples = sine_wave(320, 440.0, 8000.0);

        let via_vec = codec_a.encode(&samples).unwrap();
        let mut buf = vec![0u8; via_vec.len()];
        let written = codec_b.encode_to_buffer(&samples, &mut buf).unwrap();

        assert_eq!(written, via_vec.len());
        assert_eq!(&buf[..written], via_vec.as_slice());
    }

    #[test]
    fn encode_to_buffer_rejects_undersized_output() {
        let mut codec = codec();
        let samples = sine_wave(320, 440.0, 8000.0);
        let mut tiny = vec![0u8; 4];

        assert!(codec.encode_to_buffer(&samples, &mut tiny).is_err());
    }

    #[test]
    fn decode_to_buffer_matches_decode() {
        let mut codec_a = codec();
        let mut codec_b = codec();
        let samples = sine_wave(320, 440.0, 8000.0);
        let encoded = codec_a.encode(&samples).unwrap();

        let via_vec = codec_a.decode(&encoded).unwrap();
        let mut buf = vec![0i16; via_vec.len()];
        let written = codec_b.decode_to_buffer(&encoded, &mut buf).unwrap();

        assert_eq!(written, via_vec.len());
        assert_eq!(&buf[..written], via_vec.as_slice());
    }

    #[test]
    fn rejects_non_16khz_sample_rate() {
        let config = CodecConfig::g722().with_sample_rate(crate::types::SampleRate::Rate8000);
        assert!(G722Codec::new(config).is_err());
    }

    #[test]
    fn rejects_stereo() {
        let config = CodecConfig::g722().with_channels(2);
        assert!(G722Codec::new(config).is_err());
    }

    #[test]
    fn codec_type_and_factory_integration() {
        use crate::codecs::CodecFactory;
        use crate::types::CodecType;

        assert_eq!(CodecType::G722.name(), "G722");
        assert_eq!(CodecType::G722.default_sample_rate(), 16_000);
        assert_eq!(CodecType::G722.payload_type(), Some(9));

        let mut codec = CodecFactory::create(CodecConfig::g722()).unwrap();
        let samples = sine_wave(320, 440.0, 8000.0);
        let encoded = codec.encode(&samples).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), samples.len());

        let by_name = CodecFactory::create_by_name("G722", CodecConfig::g722()).unwrap();
        assert_eq!(by_name.info().name, "G722");

        let by_pt = CodecFactory::create_by_payload_type(9, CodecConfig::g722()).unwrap();
        assert_eq!(by_pt.info().payload_type, Some(9));
    }
}
