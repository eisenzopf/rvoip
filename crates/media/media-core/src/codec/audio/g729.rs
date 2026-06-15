//! G.729 Audio Codec Implementation
//!
//! This module implements the G.729 codec, a low bit-rate audio codec
//! standardized by ITU-T, commonly used in VoIP for its excellent compression.

use super::common::{AudioCodec, CodecInfo};
use crate::error::{CodecError, Result};
use crate::types::{AudioFrame, SampleRate};
#[cfg(feature = "g729")]
use codec_core::codecs::g729::G729Codec as CodecCoreG729;
#[cfg(feature = "g729")]
use codec_core::types::{
    AudioCodec as CodecCoreAudioCodec, CodecConfig as CodecCoreConfig, CodecType as CodecCoreType,
    SampleRate as CodecCoreSampleRate,
};
use tracing::debug;
#[cfg(not(feature = "g729"))]
use tracing::warn;
/// G.729 codec configuration
#[derive(Debug, Clone)]
pub struct G729Config {
    /// Annexes supported (A, B, etc.)
    pub annexes: G729Annexes,
    /// Frame size in milliseconds (10ms standard)
    pub frame_size_ms: f32,
    /// Enable Voice Activity Detection (VAD)
    pub enable_vad: bool,
    /// Enable Comfort Noise Generation (CNG)
    pub enable_cng: bool,
}

/// G.729 annexes configuration
#[derive(Debug, Clone)]
pub struct G729Annexes {
    /// Annex A: Reduced complexity (G.729A)
    pub annex_a: bool,
    /// Annex B: Silence compression with VAD/CNG
    pub annex_b: bool,
}

impl Default for G729Config {
    fn default() -> Self {
        Self {
            annexes: G729Annexes {
                annex_a: true, // Use reduced complexity by default
                annex_b: true, // Enable silence compression
            },
            frame_size_ms: 10.0, // Standard 10ms frames
            enable_vad: true,    // Voice Activity Detection
            enable_cng: true,    // Comfort Noise Generation
        }
    }
}

/// G.729 audio codec implementation. The owned `config` is held so
/// the encoder/decoder can be re-initialised when the `g729` feature
/// is enabled; the stub build keeps the field for ABI parity.
pub struct G729Codec {
    /// Codec configuration
    #[allow(dead_code)]
    config: G729Config,
    /// Sample rate (fixed at 8kHz for G.729)
    sample_rate: u32,
    /// Number of channels (fixed at 1 for G.729)
    channels: u8,
    /// Frame size in samples (80 samples for 10ms at 8kHz)
    frame_size: usize,
    /// Codec-core G.729 adapter.
    #[cfg(feature = "g729")]
    inner: CodecCoreG729,
}

impl G729Codec {
    /// Create a new G.729 codec
    pub fn new(sample_rate: SampleRate, channels: u8, config: G729Config) -> Result<Self> {
        let sample_rate_hz = sample_rate.as_hz();

        // G.729 only supports 8kHz mono
        if sample_rate_hz != 8000 {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "G.729 only supports 8kHz sample rate, got {}",
                    sample_rate_hz
                ),
            }
            .into());
        }

        if channels != 1 {
            return Err(CodecError::InvalidParameters {
                details: format!("G.729 only supports mono audio, got {} channels", channels),
            }
            .into());
        }

        // Calculate frame size (10ms at 8kHz = 80 samples)
        let frame_size = (sample_rate_hz as f32 * config.frame_size_ms / 1000.0) as usize;

        if frame_size != 80 {
            return Err(CodecError::InvalidParameters {
                details: format!("G.729 requires 80 sample frames (10ms), got {}", frame_size),
            }
            .into());
        }

        debug!(
            "Creating G.729 codec: {}Hz, {}ch, {}ms frames, VAD={}, CNG={}",
            sample_rate_hz, channels, config.frame_size_ms, config.enable_vad, config.enable_cng
        );

        #[cfg(feature = "g729")]
        let inner = CodecCoreG729::new(codec_core_config(&config)?).map_err(|e| {
            CodecError::InitializationFailed {
                reason: format!("G.729 codec-core initialization failed: {e}"),
            }
        })?;

        Ok(Self {
            config,
            sample_rate: sample_rate_hz,
            channels,
            frame_size,
            #[cfg(feature = "g729")]
            inner,
        })
    }

    /// Simulate G.729 encoding (when actual codec not available)
    #[cfg(not(feature = "g729"))]
    fn simulate_encode(&self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        // G.729 produces 10 bytes per 10ms frame (8 kbps)
        // This is a simulation for testing purposes
        let mut encoded = Vec::with_capacity(10);

        // Simple simulation based on energy level
        let energy = audio_frame
            .samples
            .iter()
            .map(|&s| (s as f32).abs())
            .sum::<f32>()
            / audio_frame.samples.len() as f32;

        // Generate deterministic "encoded" data based on energy
        let energy_byte = (energy / 32768.0 * 255.0) as u8;
        for i in 0..10 {
            encoded.push(energy_byte.wrapping_add(i as u8));
        }

        debug!(
            "G.729 simulation: encoded {} samples -> {} bytes",
            audio_frame.samples.len(),
            encoded.len()
        );

        Ok(encoded)
    }

    /// Simulate G.729 decoding (when actual codec not available)
    #[cfg(not(feature = "g729"))]
    fn simulate_decode(&self, encoded_data: &[u8]) -> Result<AudioFrame> {
        // G.729 decodes to 80 samples per frame
        if encoded_data.len() != 10 {
            return Err(CodecError::InvalidFrameSize {
                expected: 10,
                actual: encoded_data.len(),
            }
            .into());
        }

        // Simple simulation: generate silence or tone based on first byte
        let mut samples = Vec::with_capacity(80);
        let energy_level = encoded_data[0] as i16 * 128; // Scale to 16-bit range

        for i in 0..80 {
            // Generate a simple pattern based on encoded data
            let sample = if energy_level > 16384 {
                // Generate a simple tone for non-silence
                (((i as f32 * 2.0 * std::f32::consts::PI * 440.0) / 8000.0).sin()
                    * energy_level as f32) as i16
            } else {
                // Generate silence
                0
            };
            samples.push(sample);
        }

        debug!(
            "G.729 simulation: decoded {} bytes -> {} samples",
            encoded_data.len(),
            samples.len()
        );

        Ok(AudioFrame::new(
            samples,
            self.sample_rate,
            self.channels,
            0, // Timestamp to be set by caller
        ))
    }
}

impl AudioCodec for G729Codec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        if audio_frame.samples.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize {
                expected: self.frame_size,
                actual: audio_frame.samples.len(),
            }
            .into());
        }

        #[cfg(feature = "g729")]
        {
            self.inner.encode(&audio_frame.samples).map_err(|e| {
                CodecError::EncodingFailed {
                    reason: format!("G.729 encoding failed: {e}"),
                }
                .into()
            })
        }

        #[cfg(not(feature = "g729"))]
        {
            warn!("G.729 codec not available - using simulation");
            self.simulate_encode(audio_frame)
        }
    }

    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        // G.729 frames are typically 10 bytes. Under Annex B, empty no-data
        // payloads are valid only when the real codec-core backend is enabled.
        if encoded_data.is_empty() && !cfg!(feature = "g729") {
            return Err(CodecError::InvalidFrameSize {
                expected: 10,
                actual: 0,
            }
            .into());
        }

        #[cfg(feature = "g729")]
        {
            let decoded_samples =
                self.inner
                    .decode(encoded_data)
                    .map_err(|e| CodecError::DecodingFailed {
                        reason: format!("G.729 decoding failed: {e}"),
                    })?;

            Ok(AudioFrame::new(
                decoded_samples,
                self.sample_rate,
                self.channels,
                0, // Timestamp to be set by caller
            ))
        }

        #[cfg(not(feature = "g729"))]
        {
            warn!("G.729 codec not available - using simulation");
            self.simulate_decode(encoded_data)
        }
    }

    fn get_info(&self) -> CodecInfo {
        CodecInfo {
            name: "G.729".to_string(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: 8000, // 8 kbps
        }
    }

    fn reset(&mut self) {
        #[cfg(feature = "g729")]
        {
            let _ = self.inner.reset();
        }
        debug!("G.729 codec reset");
    }
}

#[cfg(feature = "g729")]
fn codec_core_config(config: &G729Config) -> Result<CodecCoreConfig> {
    if !config.annexes.annex_a {
        return Err(CodecError::InvalidParameters {
            details: "Full-complexity base G.729 is not implemented; Annex A is required"
                .to_string(),
        }
        .into());
    }

    let annex_b = config.annexes.annex_b && config.enable_vad && config.enable_cng;
    let codec_type = if annex_b {
        CodecCoreType::G729BA
    } else {
        CodecCoreType::G729A
    };

    let mut core_config = CodecCoreConfig::new(codec_type)
        .with_sample_rate(CodecCoreSampleRate::Rate8000)
        .with_channels(1)
        .with_frame_size_ms(config.frame_size_ms)
        .with_g729_annex_a(true)
        .with_g729_annex_b(annex_b);

    core_config.parameters.g729.annex_a = true;
    core_config.parameters.g729.annex_b = annex_b;

    Ok(core_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_g729_creation() {
        let config = G729Config::default();
        let codec = G729Codec::new(SampleRate::Rate8000, 1, config);
        assert!(codec.is_ok());

        let codec = codec.unwrap();
        assert_eq!(codec.sample_rate, 8000);
        assert_eq!(codec.channels, 1);
        assert_eq!(codec.frame_size, 80);
    }

    #[test]
    fn test_g729_invalid_sample_rate() {
        let config = G729Config::default();
        let result = G729Codec::new(SampleRate::Rate16000, 1, config);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(
                e,
                crate::error::Error::Codec(CodecError::InvalidParameters { .. })
            ));
        }
    }

    #[test]
    fn test_g729_invalid_channels() {
        let config = G729Config::default();
        let result = G729Codec::new(SampleRate::Rate8000, 2, config);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(
                e,
                crate::error::Error::Codec(CodecError::InvalidParameters { .. })
            ));
        }
    }

    #[test]
    fn test_g729_encode_decode() {
        let config = G729Config::default();
        let mut codec = G729Codec::new(SampleRate::Rate8000, 1, config).unwrap();

        // Create test frame (80 samples for 10ms at 8kHz)
        let samples: Vec<i16> = (0..80).map(|i| (i * 100) as i16).collect();
        let frame = AudioFrame::new(samples.clone(), 8000, 1, 1000);

        // Test encoding
        let encoded = codec.encode(&frame);
        assert!(encoded.is_ok());

        let encoded_data = encoded.unwrap();
        assert_eq!(encoded_data.len(), 10); // G.729 produces 10 bytes per frame

        // Test decoding
        let decoded = codec.decode(&encoded_data);
        assert!(decoded.is_ok());

        let decoded_frame = decoded.unwrap();
        assert_eq!(decoded_frame.samples.len(), 80);
        assert_eq!(decoded_frame.sample_rate, 8000);
        assert_eq!(decoded_frame.channels, 1);
    }

    #[test]
    fn test_g729_invalid_frame_size() {
        let config = G729Config::default();
        let mut codec = G729Codec::new(SampleRate::Rate8000, 1, config).unwrap();

        // Test with wrong frame size
        let samples: Vec<i16> = vec![0; 160]; // Wrong size (should be 80)
        let frame = AudioFrame::new(samples, 8000, 1, 1000);

        let result = codec.encode(&frame);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(
                e,
                crate::error::Error::Codec(CodecError::InvalidFrameSize { .. })
            ));
        }
    }

    #[test]
    fn test_g729_codec_info() {
        let config = G729Config::default();
        let codec = G729Codec::new(SampleRate::Rate8000, 1, config).unwrap();

        let info = codec.get_info();
        assert_eq!(info.name, "G.729");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bitrate, 8000);
    }

    #[test]
    fn test_g729_config_default() {
        let config = G729Config::default();
        assert!(config.annexes.annex_a);
        assert!(config.annexes.annex_b);
        assert_eq!(config.frame_size_ms, 10.0);
        assert!(config.enable_vad);
        assert!(config.enable_cng);
    }
}
