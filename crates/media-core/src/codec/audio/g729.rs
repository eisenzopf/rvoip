//! G.729 Audio Codec Implementation
//!
//! This module implements the G.729A codec using a pure Rust engine.
//! G.729A (Annex A) is the reduced-complexity variant of ITU-T G.729,
//! commonly used in VoIP for its excellent compression (8kbps).

use tracing::{debug, warn};
use crate::error::{Result, CodecError};
use crate::types::{AudioFrame, SampleRate};
use super::common::{AudioCodec, CodecInfo};
use super::g729_engine::{G729AEncoder, G729ADecoder};

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
                annex_a: true,  // Use reduced complexity by default
                annex_b: true,  // Enable silence compression
            },
            frame_size_ms: 10.0, // Standard 10ms frames
            enable_vad: true,    // Voice Activity Detection
            enable_cng: true,    // Comfort Noise Generation
        }
    }
}

/// G.729 audio codec implementation backed by pure Rust G.729A engine
pub struct G729Codec {
    /// Codec configuration
    config: G729Config,
    /// Sample rate (fixed at 8kHz for G.729)
    sample_rate: u32,
    /// Number of channels (fixed at 1 for G.729)
    channels: u8,
    /// Frame size in samples (80 samples for 10ms at 8kHz)
    frame_size: usize,
    /// Pure Rust G.729A encoder
    encoder: Option<G729AEncoder>,
    /// Pure Rust G.729A decoder
    decoder: Option<G729ADecoder>,
}

impl G729Codec {
    /// Create a new G.729 codec
    pub fn new(sample_rate: SampleRate, channels: u8, config: G729Config) -> Result<Self> {
        let sample_rate_hz = sample_rate.as_hz();

        // G.729 only supports 8kHz mono
        if sample_rate_hz != 8000 {
            return Err(CodecError::InvalidParameters {
                details: format!("G.729 only supports 8kHz sample rate, got {}", sample_rate_hz),
            }.into());
        }

        if channels != 1 {
            return Err(CodecError::InvalidParameters {
                details: format!("G.729 only supports mono audio, got {} channels", channels),
            }.into());
        }

        // Calculate frame size (10ms at 8kHz = 80 samples)
        let frame_size = (sample_rate_hz as f32 * config.frame_size_ms / 1000.0) as usize;

        if frame_size != 80 {
            return Err(CodecError::InvalidParameters {
                details: format!("G.729 requires 80 sample frames (10ms), got {}", frame_size),
            }.into());
        }

        debug!("Creating G.729A codec (pure Rust): {}Hz, {}ch, {}ms frames, VAD={}, CNG={}",
               sample_rate_hz, channels, config.frame_size_ms,
               config.enable_vad, config.enable_cng);

        Ok(Self {
            config,
            sample_rate: sample_rate_hz,
            channels,
            frame_size,
            encoder: None,
            decoder: None,
        })
    }

    /// Initialize encoder lazily
    fn ensure_encoder(&mut self) {
        if self.encoder.is_none() {
            self.encoder = Some(G729AEncoder::new());
            debug!("G.729A encoder initialized (pure Rust)");
        }
    }

    /// Initialize decoder lazily
    fn ensure_decoder(&mut self) {
        if self.decoder.is_none() {
            self.decoder = Some(G729ADecoder::new());
            debug!("G.729A decoder initialized (pure Rust)");
        }
    }
}

impl AudioCodec for G729Codec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        if audio_frame.samples.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize {
                expected: self.frame_size,
                actual: audio_frame.samples.len(),
            }.into());
        }

        self.ensure_encoder();

        let encoder = self.encoder.as_mut().ok_or_else(|| CodecError::InitializationFailed {
            reason: "G.729A encoder not initialized after ensure_encoder".to_string(),
        })?;

        encoder.encode(&audio_frame.samples)
    }

    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        if encoded_data.is_empty() {
            return Err(CodecError::InvalidFrameSize {
                expected: 10,
                actual: 0,
            }.into());
        }

        self.ensure_decoder();

        let decoder = self.decoder.as_mut().ok_or_else(|| CodecError::InitializationFailed {
            reason: "G.729A decoder not initialized after ensure_decoder".to_string(),
        })?;

        let samples = decoder.decode(encoded_data)?;

        Ok(AudioFrame::new(
            samples,
            self.sample_rate,
            self.channels,
            0, // Timestamp to be set by caller
        ))
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
        if let Some(encoder) = &mut self.encoder {
            encoder.reset();
        }
        if let Some(decoder) = &mut self.decoder {
            decoder.reset();
        }
        debug!("G.729A codec reset");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_g729_creation() {
        let config = G729Config::default();
        let codec = G729Codec::new(SampleRate::Rate8000, 1, config);
        assert!(codec.is_ok());

        let codec = codec.unwrap_or_else(|_| panic!("test setup"));
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
            assert!(matches!(e, crate::error::Error::Codec(CodecError::InvalidParameters { .. })));
        }
    }

    #[test]
    fn test_g729_invalid_channels() {
        let config = G729Config::default();
        let result = G729Codec::new(SampleRate::Rate8000, 2, config);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(e, crate::error::Error::Codec(CodecError::InvalidParameters { .. })));
        }
    }

    #[test]
    fn test_g729_encode_decode() {
        let config = G729Config::default();
        let mut codec = G729Codec::new(SampleRate::Rate8000, 1, config)
            .unwrap_or_else(|_| panic!("test setup"));

        // Create test frame (80 samples for 10ms at 8kHz)
        let samples: Vec<i16> = (0..80).map(|i| (i * 100) as i16).collect();
        let frame = AudioFrame::new(samples.clone(), 8000, 1, 1000);

        // Test encoding
        let encoded = codec.encode(&frame);
        assert!(encoded.is_ok());

        let encoded_data = encoded.unwrap_or_default();
        assert_eq!(encoded_data.len(), 10); // G.729 produces 10 bytes per frame

        // Test decoding
        let decoded = codec.decode(&encoded_data);
        assert!(decoded.is_ok());

        let decoded_frame = decoded.unwrap_or_else(|_| AudioFrame::new(vec![], 8000, 1, 0));
        assert_eq!(decoded_frame.samples.len(), 80);
        assert_eq!(decoded_frame.sample_rate, 8000);
        assert_eq!(decoded_frame.channels, 1);
    }

    #[test]
    fn test_g729_invalid_frame_size() {
        let config = G729Config::default();
        let mut codec = G729Codec::new(SampleRate::Rate8000, 1, config)
            .unwrap_or_else(|_| panic!("test setup"));

        // Test with wrong frame size
        let samples: Vec<i16> = vec![0; 160]; // Wrong size (should be 80)
        let frame = AudioFrame::new(samples, 8000, 1, 1000);

        let result = codec.encode(&frame);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(e, crate::error::Error::Codec(CodecError::InvalidFrameSize { .. })));
        }
    }

    #[test]
    fn test_g729_codec_info() {
        let config = G729Config::default();
        let codec = G729Codec::new(SampleRate::Rate8000, 1, config)
            .unwrap_or_else(|_| panic!("test setup"));

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
