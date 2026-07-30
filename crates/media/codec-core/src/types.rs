//! Core types and traits for the codec library
//!
//! This module defines the fundamental types and traits that form the
//! foundation of the codec library's API.

use crate::error::{CodecError, Result};
use std::fmt;

/// Primary trait for audio codecs
///
/// This trait defines the core operations that all audio codecs must implement:
/// encoding, decoding, and configuration management.
pub trait AudioCodec: Send + Sync {
    /// Encode audio samples to compressed data
    ///
    /// # Arguments
    ///
    /// * `samples` - Input audio samples as 16-bit PCM
    ///
    /// # Returns
    ///
    /// Compressed audio data as bytes
    ///
    /// # Errors
    ///
    /// Returns an error if encoding fails or input is invalid
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>>;

    /// Decode compressed data to audio samples
    ///
    /// # Arguments
    ///
    /// * `data` - Compressed audio data
    ///
    /// # Returns
    ///
    /// Decoded audio samples as 16-bit PCM
    ///
    /// # Errors
    ///
    /// Returns an error if decoding fails or data is invalid
    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>>;

    /// Get codec information
    fn info(&self) -> CodecInfo;

    /// Reset codec state
    ///
    /// This clears all internal state and prepares the codec for fresh input.
    /// Useful for handling stream discontinuities.
    ///
    /// # Errors
    ///
    /// Returns an error if the codec cannot reset its internal state.
    fn reset(&mut self) -> Result<()>;

    /// Get the expected frame size in samples
    fn frame_size(&self) -> usize;

    /// Check if the codec supports variable frame sizes
    fn supports_variable_frame_size(&self) -> bool {
        false
    }
}

/// Extended trait for codecs with advanced features
pub trait AudioCodecExt: AudioCodec {
    /// Encode with pre-allocated output buffer (zero-copy)
    ///
    /// # Arguments
    ///
    /// * `samples` - Input audio samples
    /// * `output` - Pre-allocated output buffer
    ///
    /// # Returns
    ///
    /// Number of bytes written to output buffer
    ///
    /// # Errors
    ///
    /// Returns an error when the input is invalid, encoding fails, or the
    /// output buffer is too small.
    fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize>;

    /// Decode with pre-allocated output buffer (zero-copy)
    ///
    /// # Arguments
    ///
    /// * `data` - Compressed audio data
    /// * `output` - Pre-allocated output buffer
    ///
    /// # Returns
    ///
    /// Number of samples written to output buffer
    ///
    /// # Errors
    ///
    /// Returns an error when the packet is invalid, decoding fails, or the
    /// output buffer is too small.
    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize>;

    /// Get maximum encoded size for a given input size
    fn max_encoded_size(&self, input_samples: usize) -> usize;

    /// Get maximum decoded size for a given input size
    fn max_decoded_size(&self, input_bytes: usize) -> usize;
}

/// Audio codec information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecInfo {
    /// Codec name (e.g., "PCMU", "PCMA", "opus")
    pub name: &'static str,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u8,
    /// Bitrate in bits per second
    pub bitrate: u32,
    /// Frame size in samples
    pub frame_size: usize,
    /// RTP payload type (if standard)
    pub payload_type: Option<u8>,
}

/// Audio codec types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecType {
    /// G.711 μ-law (PCMU)
    G711Pcmu,
    /// G.711 A-law (PCMA)
    G711Pcma,
    /// G.722 wideband

    /// G.729 low-bitrate RTP compatibility alias.
    ///
    /// The built-in implementation is Annex A speech coding with optional
    /// Annex B; it does not implement full-complexity base G.729.
    G729,
    /// G.729A reduced complexity
    G729A,
    /// G.729BA reduced complexity with VAD/DTX/CNG
    G729BA,
    /// Opus modern codec
    Opus,
}

impl CodecType {
    /// Get the codec name
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::G711Pcmu => "PCMU",
            Self::G711Pcma => "PCMA",

            Self::G729 => "G729",
            Self::G729A => "G729A",
            Self::G729BA => "G729BA",
            Self::Opus => "opus",
        }
    }

    /// Get the default sample rate
    #[must_use]
    pub const fn default_sample_rate(self) -> u32 {
        match self {
            Self::G711Pcmu | Self::G711Pcma | Self::G729 | Self::G729A | Self::G729BA => 8000,
            Self::Opus => 48000,
        }
    }

    /// Get the default bitrate
    #[must_use]
    pub const fn default_bitrate(self) -> u32 {
        match self {
            Self::G711Pcmu | Self::G711Pcma | Self::Opus => 64000,
            Self::G729 | Self::G729A | Self::G729BA => 8000,
        }
    }

    /// Get the standard RTP payload type
    #[must_use]
    pub const fn payload_type(self) -> Option<u8> {
        match self {
            Self::G711Pcmu => Some(0),
            Self::G711Pcma => Some(8),

            Self::G729 | Self::G729A | Self::G729BA => Some(18),
            Self::Opus => None, // Dynamic payload type
        }
    }

    /// Get supported sample rates
    #[must_use]
    pub const fn supported_sample_rates(self) -> &'static [u32] {
        match self {
            Self::G711Pcmu | Self::G711Pcma | Self::G729 | Self::G729A | Self::G729BA => &[8000],
            Self::Opus => &[8000, 12000, 16000, 24000, 48000],
        }
    }

    /// Get supported channel counts
    #[must_use]
    pub const fn supported_channels(self) -> &'static [u8] {
        match self {
            Self::G711Pcmu | Self::G711Pcma | Self::G729 | Self::G729A | Self::G729BA => &[1],
            Self::Opus => &[1, 2],
        }
    }
}

impl fmt::Display for CodecType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Sample rate enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleRate {
    /// 8 kHz (narrowband)
    Rate8000,
    /// 12 kHz
    Rate12000,
    /// 16 kHz (wideband)
    Rate16000,
    /// 24 kHz
    Rate24000,
    /// 32 kHz
    Rate32000,
    /// 44.1 kHz (CD quality)
    Rate44100,
    /// 48 kHz (professional)
    Rate48000,
    /// Custom sample rate
    Custom(u32),
}

impl SampleRate {
    /// Get the sample rate value in Hz
    #[must_use]
    pub const fn hz(self) -> u32 {
        match self {
            Self::Rate8000 => 8000,
            Self::Rate12000 => 12000,
            Self::Rate16000 => 16000,
            Self::Rate24000 => 24000,
            Self::Rate32000 => 32000,
            Self::Rate44100 => 44100,
            Self::Rate48000 => 48000,
            Self::Custom(rate) => rate,
        }
    }

    /// Create from Hz value
    #[must_use]
    pub const fn from_hz(hz: u32) -> Self {
        match hz {
            8000 => Self::Rate8000,
            12000 => Self::Rate12000,
            16000 => Self::Rate16000,
            24000 => Self::Rate24000,
            32000 => Self::Rate32000,
            44100 => Self::Rate44100,
            48000 => Self::Rate48000,
            rate => Self::Custom(rate),
        }
    }

    /// Check if this is a standard telephony rate
    #[must_use]
    pub const fn is_telephony(self) -> bool {
        matches!(self, Self::Rate8000 | Self::Rate16000)
    }

    /// Check if this is a standard audio rate
    #[must_use]
    pub const fn is_audio(self) -> bool {
        matches!(self, Self::Rate44100 | Self::Rate48000)
    }
}

impl fmt::Display for SampleRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Hz", self.hz())
    }
}

/// Audio frame structure
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFrame {
    /// Audio samples as 16-bit PCM
    pub samples: Vec<i16>,
    /// Sample rate
    pub sample_rate: SampleRate,
    /// Number of channels
    pub channels: u8,
    /// Timestamp (optional)
    pub timestamp: Option<u64>,
}

impl AudioFrame {
    /// Create a new audio frame
    #[must_use]
    pub const fn new(samples: Vec<i16>, sample_rate: SampleRate, channels: u8) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
            timestamp: None,
        }
    }

    /// Create a new audio frame with timestamp
    #[must_use]
    pub const fn new_with_timestamp(
        samples: Vec<i16>,
        sample_rate: SampleRate,
        channels: u8,
        timestamp: u64,
    ) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
            timestamp: Some(timestamp),
        }
    }

    /// Get the frame duration in milliseconds
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn duration_ms(&self) -> f64 {
        let samples_per_channel = self.samples.len() / usize::from(self.channels);
        (samples_per_channel as f64 * 1000.0) / f64::from(self.sample_rate.hz())
    }

    /// Get the frame size in samples per channel
    #[must_use]
    pub const fn frame_size(&self) -> usize {
        self.samples.len() / self.channels as usize
    }

    /// Validate the frame structure
    ///
    /// # Errors
    ///
    /// Returns an error when the channel count is zero, the frame is empty,
    /// or the samples cannot be divided evenly among the channels.
    pub fn validate(&self) -> Result<()> {
        if self.channels == 0 {
            return Err(CodecError::InvalidChannelCount {
                channels: self.channels,
                supported: vec![1, 2],
            });
        }

        if !self
            .samples
            .len()
            .is_multiple_of(usize::from(self.channels))
        {
            return Err(CodecError::invalid_format(
                "Sample count must be divisible by channel count",
            ));
        }

        if self.samples.is_empty() {
            return Err(CodecError::invalid_format("Frame cannot be empty"));
        }

        Ok(())
    }
}

/// Codec configuration
#[derive(Debug, Clone, PartialEq)]
pub struct CodecConfig {
    /// Codec type
    pub codec_type: CodecType,
    /// Sample rate
    pub sample_rate: SampleRate,
    /// Number of channels
    pub channels: u8,
    /// Bitrate (if applicable)
    pub bitrate: Option<u32>,
    /// Frame size in milliseconds
    pub frame_size_ms: Option<f32>,
    /// Codec-specific parameters
    pub parameters: CodecParameters,
}

impl CodecConfig {
    /// Create a new codec configuration
    #[must_use]
    pub fn new(codec_type: CodecType) -> Self {
        Self {
            codec_type,
            sample_rate: SampleRate::from_hz(codec_type.default_sample_rate()),
            channels: 1,
            bitrate: Some(codec_type.default_bitrate()),
            frame_size_ms: None,
            parameters: CodecParameters::default(),
        }
    }

    /// Create G.711 PCMU configuration
    #[must_use]
    pub fn g711_pcmu() -> Self {
        Self::new(CodecType::G711Pcmu)
    }

    /// Create G.711 PCMA configuration
    #[must_use]
    pub fn g711_pcma() -> Self {
        Self::new(CodecType::G711Pcma)
    }

    /// Create G.729 configuration
    #[must_use]
    pub fn g729() -> Self {
        Self::new(CodecType::G729)
    }

    /// Create G.729 Annex A configuration
    #[must_use]
    pub fn g729a() -> Self {
        Self::new(CodecType::G729A).with_g729_annex_b(false)
    }

    /// Create G.729 Annex A plus Annex B configuration
    #[must_use]
    pub fn g729ba() -> Self {
        Self::new(CodecType::G729BA).with_g729_annex_b(true)
    }

    /// Create Opus configuration
    #[must_use]
    pub fn opus() -> Self {
        Self::new(CodecType::Opus)
    }

    /// Set sample rate
    #[must_use]
    pub const fn with_sample_rate(mut self, sample_rate: SampleRate) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    /// Set channel count
    #[must_use]
    pub const fn with_channels(mut self, channels: u8) -> Self {
        self.channels = channels;
        self
    }

    /// Set bitrate
    #[must_use]
    pub fn with_bitrate(mut self, bitrate: u32) -> Self {
        self.bitrate = Some(bitrate);
        // Opus historically treated the codec-specific bitrate as the
        // authoritative value. Keep the generic convenience setter and that
        // public field synchronized so existing callers do not silently get
        // the default 64 kbit/s configuration.
        if self.codec_type == CodecType::Opus {
            self.parameters.opus.bitrate = bitrate;
        }
        self
    }

    /// Set frame size in milliseconds
    #[must_use]
    pub const fn with_frame_size_ms(mut self, frame_size_ms: f32) -> Self {
        self.frame_size_ms = Some(frame_size_ms);
        self
    }

    /// Set codec parameters
    #[must_use]
    pub const fn with_parameters(mut self, parameters: CodecParameters) -> Self {
        self.parameters = parameters;
        self
    }

    /// Set Opus application type
    #[must_use]
    pub const fn with_opus_application(mut self, application: OpusApplication) -> Self {
        self.parameters.opus.application = application;
        self
    }

    /// Set Opus VBR mode
    #[must_use]
    pub const fn with_opus_vbr(mut self, vbr: bool) -> Self {
        self.parameters.opus.vbr = vbr;
        self
    }

    /// Set Opus complexity
    #[must_use]
    pub const fn with_opus_complexity(mut self, complexity: u8) -> Self {
        self.parameters.opus.complexity = complexity;
        self
    }

    /// Set Opus FEC
    #[must_use]
    pub const fn with_opus_fec(mut self, fec: bool) -> Self {
        self.parameters.opus.inband_fec = fec;
        self
    }

    /// Set whether G.729 Annex A reduced-complexity mode is enabled.
    ///
    /// Setting this to `false` requests full-complexity base G.729, which is
    /// not implemented by the built-in G.729 codec and will be rejected.
    #[must_use]
    #[allow(deprecated)] // keep the legacy mirror field in sync
    pub const fn with_g729_annex_a(mut self, annex_a: bool) -> Self {
        self.parameters.g729.annex_a = annex_a;
        self.parameters.g729.reduced_complexity = annex_a;
        self
    }

    /// Set whether G.729 Annex B VAD/DTX/CNG is enabled.
    #[must_use]
    #[allow(deprecated)] // keep legacy VAD/CNG mirror fields in sync
    pub const fn with_g729_annex_b(mut self, annex_b: bool) -> Self {
        self.parameters.g729.annex_b = annex_b;
        self.parameters.g729.vad_enabled = annex_b;
        self.parameters.g729.cng_enabled = annex_b;
        self
    }

    /// Set G.729 VAD
    #[must_use]
    #[allow(deprecated)] // intentionally writes the legacy field for backward compat
    pub const fn with_g729_vad(mut self, vad: bool) -> Self {
        self.parameters.g729.vad_enabled = vad;
        self
    }

    /// Set G.729 CNG
    #[must_use]
    #[allow(deprecated)] // intentionally writes the legacy field for backward compat
    pub const fn with_g729_cng(mut self, cng: bool) -> Self {
        self.parameters.g729.cng_enabled = cng;
        self
    }

    /// Validate the configuration
    ///
    /// # Errors
    ///
    /// Returns an error when the sample rate, channel count, or bitrate is not
    /// supported by the selected codec.
    pub fn validate(&self) -> Result<()> {
        // Check if sample rate is supported
        let supported_rates = self.codec_type.supported_sample_rates();
        if !supported_rates.contains(&self.sample_rate.hz()) {
            return Err(CodecError::InvalidSampleRate {
                rate: self.sample_rate.hz(),
                supported: supported_rates.to_vec(),
            });
        }

        // Check if channel count is supported
        let supported_channels = self.codec_type.supported_channels();
        if !supported_channels.contains(&self.channels) {
            return Err(CodecError::InvalidChannelCount {
                channels: self.channels,
                supported: supported_channels.to_vec(),
            });
        }

        // Validate bitrate if specified
        if let Some(bitrate) = self.bitrate {
            let (min_bitrate, max_bitrate) = self.codec_type.bitrate_range();
            if bitrate < min_bitrate || bitrate > max_bitrate {
                return Err(CodecError::InvalidBitrate {
                    bitrate,
                    min: min_bitrate,
                    max: max_bitrate,
                });
            }
        }

        Ok(())
    }
}

impl CodecType {
    /// Get the bitrate range for this codec type
    #[must_use]
    pub const fn bitrate_range(self) -> (u32, u32) {
        match self {
            Self::G711Pcmu | Self::G711Pcma => (64000, 64000),

            Self::G729 | Self::G729A | Self::G729BA => (8000, 8000),
            Self::Opus => (6_000, 510_000),
        }
    }
}

/// Codec-specific parameters
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodecParameters {
    /// G.711 specific parameters
    pub g711: G711Parameters,

    /// G.729 specific parameters
    pub g729: G729Parameters,
    /// Opus specific parameters
    pub opus: OpusParameters,
}

/// G.711 codec parameters
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct G711Parameters {
    /// Use A-law instead of μ-law (for PCMA)
    pub use_alaw: bool,
}

/// G.729 codec parameters
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct G729Parameters {
    /// Enable G.729 Annex A (reduced complexity - ~40% faster)
    pub annex_a: bool,
    /// Enable G.729 Annex B (VAD/DTX/CNG - ~50% bandwidth savings)
    pub annex_b: bool,

    // Legacy fields for backward compatibility (deprecated)
    /// Use reduced complexity mode (Annex A) - DEPRECATED: use `annex_a` instead
    #[deprecated(since = "0.1.14", note = "use annex_a instead")]
    pub reduced_complexity: bool,
    /// Enable Voice Activity Detection (VAD) - DEPRECATED: use `annex_b` instead
    #[deprecated(since = "0.1.14", note = "use annex_b instead")]
    pub vad_enabled: bool,
    /// Enable Comfort Noise Generation (CNG) - DEPRECATED: use `annex_b` instead
    #[deprecated(since = "0.1.14", note = "use annex_b instead")]
    pub cng_enabled: bool,
}

impl Default for G729Parameters {
    fn default() -> Self {
        Self {
            annex_a: true, // Default to reduced complexity
            annex_b: true, // Default to VAD/DTX/CNG enabled (G.729BA)
            #[allow(deprecated)]
            reduced_complexity: true,
            #[allow(deprecated)]
            vad_enabled: true,
            #[allow(deprecated)]
            cng_enabled: true,
        }
    }
}

/// Opus codec parameters
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpusParameters {
    /// Application type
    pub application: OpusApplication,
    /// Bitrate in bits per second
    pub bitrate: u32,
    /// Complexity (0-10, higher is better quality)
    pub complexity: u8,
    /// Use variable bitrate
    pub vbr: bool,
    /// Use constrained VBR
    pub cvbr: bool,
    /// Enable inband FEC
    pub inband_fec: bool,
    /// Packet loss percentage (0-100)
    pub packet_loss_perc: u8,
    /// Use DTX (discontinuous transmission)
    pub dtx: bool,
    /// Force mono encoding
    pub force_mono: bool,
}

impl Default for OpusParameters {
    fn default() -> Self {
        Self {
            application: OpusApplication::Voip,
            bitrate: 64000,
            complexity: 5,
            vbr: true,
            cvbr: false,
            inband_fec: false,
            packet_loss_perc: 0,
            dtx: false,
            force_mono: false,
        }
    }
}

/// Opus application type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpusApplication {
    /// `VoIP` application
    Voip,
    /// Audio application
    Audio,
    /// Low-delay application
    RestrictedLowDelay,
}

/// Codec capability information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecCapability {
    /// Codec type
    pub codec_type: CodecType,
    /// Supported sample rates
    pub sample_rates: Vec<u32>,
    /// Supported channel counts
    pub channels: Vec<u8>,
    /// Supported bitrate range
    pub bitrate_range: (u32, u32),
    /// Quality score (0-100)
    pub quality_score: u8,
}

impl CodecCapability {
    /// Create capability info for a codec type
    #[must_use]
    pub fn for_codec(codec_type: CodecType) -> Self {
        Self {
            codec_type,
            sample_rates: codec_type.supported_sample_rates().to_vec(),
            channels: codec_type.supported_channels().to_vec(),
            bitrate_range: codec_type.bitrate_range(),
            quality_score: codec_type.quality_score(),
        }
    }
}

impl CodecType {
    /// Get the quality score for this codec type
    #[must_use]
    pub const fn quality_score(self) -> u8 {
        match self {
            Self::G711Pcmu | Self::G711Pcma => 70,

            Self::G729 | Self::G729A | Self::G729BA => 85,
            Self::Opus => 95,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_type_properties() {
        assert_eq!(CodecType::G711Pcmu.name(), "PCMU");
        assert_eq!(CodecType::G711Pcmu.default_sample_rate(), 8000);
        assert_eq!(CodecType::G711Pcmu.payload_type(), Some(0));
    }

    #[test]
    fn test_sample_rate_conversion() {
        assert_eq!(SampleRate::Rate8000.hz(), 8000);
        assert_eq!(SampleRate::from_hz(8000), SampleRate::Rate8000);
        assert_eq!(SampleRate::from_hz(22050), SampleRate::Custom(22050));
    }

    #[test]
    fn test_audio_frame_creation() {
        let samples = vec![0i16; 160];
        let frame = AudioFrame::new(samples.clone(), SampleRate::Rate8000, 1);
        assert_eq!(frame.samples, samples);
        assert_eq!(frame.sample_rate, SampleRate::Rate8000);
        assert_eq!(frame.channels, 1);
        assert!((frame.duration_ms() - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_codec_config_validation() {
        let config = CodecConfig::g711_pcmu();
        assert!(config.validate().is_ok());

        let invalid_config =
            CodecConfig::new(CodecType::G711Pcmu).with_sample_rate(SampleRate::Rate48000); // Invalid for G.711
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_codec_capability() {
        let cap = CodecCapability::for_codec(CodecType::Opus);
        assert_eq!(cap.codec_type, CodecType::Opus);
        assert!(cap.sample_rates.contains(&48000));
        assert!(cap.channels.contains(&1));
        assert!(cap.channels.contains(&2));
    }

    #[test]
    fn test_frame_validation() {
        let frame = AudioFrame::new(vec![0i16; 160], SampleRate::Rate8000, 1);
        assert!(frame.validate().is_ok());

        let invalid_frame = AudioFrame::new(vec![0i16; 161], SampleRate::Rate8000, 2);
        assert!(invalid_frame.validate().is_err());

        let empty_frame = AudioFrame::new(vec![], SampleRate::Rate8000, 1);
        assert!(empty_frame.validate().is_err());
    }
}
