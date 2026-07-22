//! Opus Audio Codec Implementation
//!
//! This module implements the Opus codec, a modern audio codec standardized
//! by the Internet Engineering Task Force (IETF) in RFC 6716. Opus combines
//! the best features of both speech and music codecs with very low latency.
//!
//! Two mutually-exclusive-in-intent backends live behind separate features:
//!
//! - `opus`: real encode/decode via [`opus-rs`](https://docs.rs/opus-rs), a
//!   pure-Rust Opus implementation (no C toolchain needed at build time).
//!   This is what you want for actual audio.
//! - `opus-sim`: a deterministic stub that produces plausible-shaped output
//!   (right size, varies with bitrate) without doing any real DSP. Useful
//!   for fast, deterministic pipeline tests that don't care about audio
//!   fidelity. If both features are enabled, `opus` (real) takes priority.

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecInfo, SampleRate};
use crate::utils::validate_opus_frame;
use tracing::{debug, trace};

// Re-export OpusApplication from types to avoid duplication
pub use crate::types::OpusApplication;

/// Opus codec implementation
pub struct OpusCodec {
    /// Sample rate (8, 12, 16, 24, or 48 kHz)
    sample_rate: u32,
    /// Number of channels (1 or 2)
    channels: u8,
    /// Frame size in samples *per channel* (matches opus-rs's own
    /// `frame_size` convention). `AudioCodec::encode`/`decode`'s `samples`
    /// buffers are the interleaved total, `frame_size * channels` long.
    frame_size: usize,
    /// Codec configuration
    config: OpusConfig,
    #[cfg(feature = "opus")]
    real: RealBackend,
}

/// The actual opus-rs encoder/decoder pair, kept behind the `opus` feature
/// so building without it doesn't need opus-rs at all.
#[cfg(feature = "opus")]
struct RealBackend {
    encoder: opus_rs::OpusEncoder,
    decoder: opus_rs::OpusDecoder,
}

#[cfg(feature = "opus")]
fn to_opus_rs_application(application: OpusApplication) -> opus_rs::Application {
    match application {
        OpusApplication::Voip => opus_rs::Application::Voip,
        OpusApplication::Audio => opus_rs::Application::Audio,
        OpusApplication::RestrictedLowDelay => opus_rs::Application::RestrictedLowDelay,
    }
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

        debug!(
            "Creating Opus codec: {}Hz, {}ch, {}bps, {:?} mode",
            sample_rate, config.channels, opus_config.bitrate, opus_config.application
        );

        #[cfg(feature = "opus")]
        let real = {
            let mut encoder = opus_rs::OpusEncoder::new(
                sample_rate as i32,
                config.channels as usize,
                to_opus_rs_application(opus_config.application),
            )
            .map_err(|e| CodecError::ExternalLibraryError {
                library: "opus-rs".to_string(),
                error: format!("encoder init: {e}"),
            })?;
            encoder.bitrate_bps = opus_config.bitrate as i32;
            encoder.use_cbr = !opus_config.vbr;
            encoder.complexity = i32::from(opus_config.complexity);
            encoder.use_inband_fec = opus_config.inband_fec;
            encoder.packet_loss_perc = i32::from(opus_config.packet_loss_perc);

            let decoder = opus_rs::OpusDecoder::new(sample_rate as i32, config.channels as usize)
                .map_err(|e| CodecError::ExternalLibraryError {
                library: "opus-rs".to_string(),
                error: format!("decoder init: {e}"),
            })?;

            RealBackend { encoder, decoder }
        };

        Ok(Self {
            sample_rate,
            channels: config.channels,
            frame_size,
            config: opus_config,
            #[cfg(feature = "opus")]
            real,
        })
    }

    /// Get the compression ratio (variable for Opus)
    pub fn compression_ratio(&self) -> f32 {
        let uncompressed_bits = self.frame_size as f32 * 16.0 * self.channels as f32;
        let compressed_bits =
            self.config.bitrate as f32 * (self.frame_size as f32 / self.sample_rate as f32);
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
        #[cfg(feature = "opus")]
        {
            self.real.encoder.bitrate_bps = bitrate as i32;
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
        #[cfg(feature = "opus")]
        {
            self.real.encoder.complexity = i32::from(complexity);
        }
        debug!("Opus complexity set to {}", complexity);
        Ok(())
    }

    /// Real Opus encoding via opus-rs.
    #[cfg(feature = "opus")]
    fn real_encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // i16 -> f32 in [-1.0, 1.0), the standard PCM float convention
        // opus-rs (and libopus) expect.
        let input: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();

        // 1275 bytes is the largest a single Opus packet can ever be
        // (RFC 6716 §3.2.1).
        let mut output = vec![0u8; 1275];
        let written = self
            .real
            .encoder
            .encode(&input, self.frame_size, &mut output)
            .map_err(|e| CodecError::EncodingFailed {
                reason: e.to_string(),
            })?;
        output.truncate(written);
        Ok(output)
    }

    /// Real Opus decoding via opus-rs.
    #[cfg(feature = "opus")]
    fn real_decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        let mut output = vec![0.0f32; self.frame_size * self.channels as usize];
        let per_channel_written = self
            .real
            .decoder
            .decode(data, self.frame_size, &mut output)
            .map_err(|e| CodecError::DecodingFailed {
                reason: e.to_string(),
            })?;
        output.truncate(per_channel_written * self.channels as usize);

        // f32 [-1.0, 1.0) -> i16, clamped so an over-range decode (e.g. a
        // slightly hot input) saturates instead of wrapping.
        Ok(output
            .iter()
            .map(|&f| (f * 32768.0).clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16)
            .collect())
    }

    /// Deterministic Opus stub (see the module docs for when this runs
    /// instead of [`Self::real_encode`]). Ignores the actual audio content;
    /// output size just tracks the configured bitrate.
    #[cfg(not(feature = "opus"))]
    fn simulate_encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // Calculate target size based on bitrate
        let frame_duration_ms =
            (samples.len() as f32 * 1000.0) / (self.sample_rate as f32 * self.channels as f32);
        let target_bits = (self.config.bitrate as f32 * frame_duration_ms / 1000.0) as usize;
        let target_bytes = target_bits / 8;

        let mut encoded = Vec::with_capacity(target_bytes.max(10));

        // Simple simulation - just create dummy data
        for i in 0..target_bytes {
            encoded.push((i % 256) as u8);
        }

        Ok(encoded)
    }

    /// Deterministic Opus stub decode; see [`Self::simulate_encode`].
    #[cfg(not(feature = "opus"))]
    fn simulate_decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        let mut samples = vec![0i16; self.frame_size * self.channels as usize];

        // Simple simulation - generate noise based on input
        for (i, sample) in samples.iter_mut().enumerate() {
            let data_idx = i % data.len();
            *sample = ((data[data_idx] as i16) << 8) | (i as i16 & 0xFF);
        }

        Ok(samples)
    }

    /// Encode via whichever backend is compiled in: real opus-rs when the
    /// `opus` feature is enabled, the deterministic stub otherwise.
    fn backend_encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        #[cfg(feature = "opus")]
        {
            self.real_encode(samples)
        }
        #[cfg(not(feature = "opus"))]
        {
            self.simulate_encode(samples)
        }
    }

    /// Decode via whichever backend is compiled in; see
    /// [`Self::backend_encode`].
    fn backend_decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        #[cfg(feature = "opus")]
        {
            self.real_decode(data)
        }
        #[cfg(not(feature = "opus"))]
        {
            self.simulate_decode(data)
        }
    }
}

impl AudioCodec for OpusCodec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // Validate input
        validate_opus_frame(samples, SampleRate::from_hz(self.sample_rate))?;

        let encoded = self.backend_encode(samples)?;

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

        let decoded = self.backend_decode(data)?;

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
            payload_type: Some(111), // Dynamic payload type
        }
    }

    fn reset(&mut self) -> Result<()> {
        // The encoder/decoder carry ADPCM-like prediction state (SILK/CELT
        // history, VBR smoothing) across frames; a fresh instance is the
        // only way to actually clear it. Re-applying the tunable fields
        // mirrors what `new()` does from `self.config`.
        #[cfg(feature = "opus")]
        {
            let mut encoder = opus_rs::OpusEncoder::new(
                self.sample_rate as i32,
                self.channels as usize,
                to_opus_rs_application(self.config.application),
            )
            .map_err(|e| CodecError::ResetFailed {
                reason: format!("encoder re-init: {e}"),
            })?;
            encoder.bitrate_bps = self.config.bitrate as i32;
            encoder.use_cbr = !self.config.vbr;
            encoder.complexity = i32::from(self.config.complexity);
            encoder.use_inband_fec = self.config.inband_fec;
            encoder.packet_loss_perc = i32::from(self.config.packet_loss_perc);

            let decoder =
                opus_rs::OpusDecoder::new(self.sample_rate as i32, self.channels as usize)
                    .map_err(|e| CodecError::ResetFailed {
                        reason: format!("decoder re-init: {e}"),
                    })?;

            self.real = RealBackend { encoder, decoder };
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

        let encoded = self.backend_encode(samples)?;

        if output.len() < encoded.len() {
            return Err(CodecError::BufferTooSmall {
                needed: encoded.len(),
                actual: output.len(),
            });
        }

        output[..encoded.len()].copy_from_slice(&encoded);

        trace!(
            "Opus encoded {} samples to {} bytes (zero-alloc)",
            samples.len(),
            encoded.len()
        );

        Ok(encoded.len())
    }

    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }

        let decoded = self.backend_decode(data)?;

        if output.len() < decoded.len() {
            return Err(CodecError::BufferTooSmall {
                needed: decoded.len(),
                actual: output.len(),
            });
        }

        output[..decoded.len()].copy_from_slice(&decoded);

        trace!(
            "Opus decoded {} bytes to {} samples (zero-alloc)",
            data.len(),
            decoded.len()
        );

        Ok(decoded.len())
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
    fn test_encoding_decoding_roundtrip() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();

        // Create test signal
        let mut samples = Vec::new();
        for i in 0..960 {
            let t = i as f32 / 48000.0;
            let sample = ((2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 16000.0) as i16;
            samples.push(sample);
        }

        // Encode
        let encoded = codec.encode(&samples).unwrap();
        assert!(encoded.len() > 0);

        // Decode
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), samples.len());
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

    // These specifically exercise the real opus-rs backend. Opus's SILK
    // layer is adaptive/predictive, not a fixed-delay linear filter like
    // G.722's QMF, so a naive "find the best constant sample lag, then
    // measure SNR" approach (which worked well for G.722) doesn't converge
    // to a stable answer here. Energy/RMS-based checks are lag-independent
    // and, as a bonus, are exactly the kind of check that would have
    // caught the old simulate_encode/simulate_decode stub: its output
    // depended only on configured bitrate, not actual sample content, so
    // loud and near-silent input produced equally noisy "decoded" output.
    #[cfg(feature = "opus")]
    mod real_backend {
        use super::*;

        fn tone_at(
            len: usize,
            sample_rate: u32,
            frequency: f32,
            amplitude: f32,
            start: usize,
        ) -> Vec<i16> {
            (0..len)
                .map(|i| {
                    let t = (start + i) as f32 / sample_rate as f32;
                    (amplitude * (2.0 * std::f32::consts::PI * frequency * t).sin()) as i16
                })
                .collect()
        }

        fn tone(len: usize, sample_rate: u32, frequency: f32, amplitude: f32) -> Vec<i16> {
            tone_at(len, sample_rate, frequency, amplitude, 0)
        }

        fn rms(samples: &[i16]) -> f64 {
            let sum_sq: f64 = samples.iter().map(|&s| f64::from(s).powi(2)).sum();
            (sum_sq / samples.len() as f64).sqrt()
        }

        #[test]
        fn silence_roundtrips_to_near_silence() {
            let mut codec = OpusCodec::new(create_test_config()).unwrap();
            let silence = vec![0i16; 960];

            let encoded = codec.encode(&silence).unwrap();
            let decoded = codec.decode(&encoded).unwrap();

            assert_eq!(decoded.len(), silence.len());
            let level = rms(&decoded);
            assert!(
                level < 500.0,
                "silence should decode to near-silence, got RMS {level:.1} (i16 range is ±32768)"
            );
        }

        #[test]
        fn decoded_energy_tracks_input_energy() {
            let loud = tone(960, 48000, 440.0, 12000.0);
            let quiet = tone(960, 48000, 440.0, 300.0);

            let mut loud_codec = OpusCodec::new(create_test_config()).unwrap();
            let loud_encoded = loud_codec.encode(&loud).unwrap();
            let loud_decoded = loud_codec.decode(&loud_encoded).unwrap();

            let mut quiet_codec = OpusCodec::new(create_test_config()).unwrap();
            let quiet_encoded = quiet_codec.encode(&quiet).unwrap();
            let quiet_decoded = quiet_codec.decode(&quiet_encoded).unwrap();

            let (loud_rms, quiet_rms) = (rms(&loud_decoded), rms(&quiet_decoded));
            assert!(
                loud_rms > quiet_rms * 3.0,
                "a ~40x louder input should decode noticeably louder: loud_rms={loud_rms:.1} quiet_rms={quiet_rms:.1}"
            );
        }

        #[test]
        fn encode_produces_a_valid_opus_packet_size() {
            let mut codec = OpusCodec::new(create_test_config()).unwrap();
            let samples = tone(960, 48000, 440.0, 8000.0);

            let encoded = codec.encode(&samples).unwrap();

            assert!(!encoded.is_empty());
            assert!(
                encoded.len() <= 1275,
                "RFC 6716 §3.2.1: an Opus packet is never larger than 1275 bytes, got {}",
                encoded.len()
            );
        }

        #[test]
        fn streaming_many_frames_round_trips_without_error() {
            let mut encoder = OpusCodec::new(create_test_config()).unwrap();
            let mut decoder = OpusCodec::new(create_test_config()).unwrap();

            for frame_idx in 0..50usize {
                let samples = tone_at(960, 48000, 440.0, 8000.0, frame_idx * 960);
                let encoded = encoder.encode(&samples).unwrap();
                let decoded = decoder.decode(&encoded).unwrap();
                assert_eq!(decoded.len(), samples.len());
            }
        }

        #[test]
        fn reset_succeeds_and_codec_keeps_working_afterward() {
            let mut codec = OpusCodec::new(create_test_config()).unwrap();
            let samples = tone(960, 48000, 440.0, 8000.0);
            let _ = codec.encode(&samples).unwrap();

            codec.reset().unwrap();

            let encoded = codec.encode(&samples).unwrap();
            let decoded = codec.decode(&encoded).unwrap();
            assert_eq!(decoded.len(), samples.len());
        }

        #[test]
        fn set_bitrate_and_complexity_propagate_to_the_real_encoder() {
            let mut codec = OpusCodec::new(create_test_config()).unwrap();
            codec.set_bitrate(96_000).unwrap();
            codec.set_complexity(3).unwrap();

            assert_eq!(codec.real.encoder.bitrate_bps, 96_000);
            assert_eq!(codec.real.encoder.complexity, 3);
        }
    }
}
