//! Core audio types and structures
//!
//! This module defines the fundamental types used throughout the audio-core library,
//! including audio formats, device information, codecs, and quality metrics.

use std::fmt;
use std::time::Duration;
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Audio direction (input/output)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioDirection {
    /// Audio input (microphone, line-in)
    Input,
    /// Audio output (speakers, headphones, line-out)
    Output,
}

impl fmt::Display for AudioDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioDirection::Input => write!(f, "Input"),
            AudioDirection::Output => write!(f, "Output"),
        }
    }
}

/// Audio format specification
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    /// Sample rate in Hz (e.g., 8000, 16000, 44100, 48000)
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,
    /// Bits per sample (8, 16, 24, 32)
    pub bits_per_sample: u16,
    /// Frame size in milliseconds (typically 10, 20, or 30)
    pub frame_size_ms: u32,
}

impl AudioFormat {
    /// Create a new audio format
    pub fn new(sample_rate: u32, channels: u16, bits_per_sample: u16, frame_size_ms: u32) -> Self {
        Self {
            sample_rate,
            channels,
            bits_per_sample,
            frame_size_ms,
        }
    }

    /// Create standard VoIP format (8kHz, mono, 16-bit, 20ms)
    pub fn pcm_8khz_mono() -> Self {
        Self::new(8000, 1, 16, 20)
    }

    /// Create wideband VoIP format (16kHz, mono, 16-bit, 20ms)
    pub fn pcm_16khz_mono() -> Self {
        Self::new(16000, 1, 16, 20)
    }

    /// Create CD quality format (44.1kHz, stereo, 16-bit, 20ms)
    pub fn pcm_44khz_stereo() -> Self {
        Self::new(44100, 2, 16, 20)
    }

    /// Create high-quality format (48kHz, stereo, 16-bit, 20ms)
    pub fn pcm_48khz_stereo() -> Self {
        Self::new(48000, 2, 16, 20)
    }

    /// Calculate the number of samples per frame
    pub fn samples_per_frame(&self) -> usize {
        ((self.sample_rate * self.frame_size_ms) / 1000) as usize
    }

    /// Calculate the number of bytes per frame
    pub fn bytes_per_frame(&self) -> usize {
        self.samples_per_frame() * self.channels as usize * (self.bits_per_sample / 8) as usize
    }

    /// Check if this format is compatible with another format
    pub fn is_compatible_with(&self, other: &AudioFormat) -> bool {
        self.sample_rate == other.sample_rate && 
        self.channels == other.channels &&
        self.bits_per_sample == other.bits_per_sample
    }

    /// Get a human-readable description of the format
    pub fn description(&self) -> String {
        format!("{}Hz, {} ch, {}-bit, {}ms frames", 
            self.sample_rate, 
            self.channels, 
            self.bits_per_sample,
            self.frame_size_ms
        )
    }

    /// Check if this is a VoIP-suitable format
    pub fn is_voip_suitable(&self) -> bool {
        // VoIP typically uses 8kHz or 16kHz, mono, 16-bit
        (self.sample_rate == 8000 || self.sample_rate == 16000) &&
        self.channels == 1 &&
        self.bits_per_sample == 16 &&
        (self.frame_size_ms >= 10 && self.frame_size_ms <= 30)
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// Audio device information
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    /// Unique device identifier
    pub id: String,
    /// Human-readable device name
    pub name: String,
    /// Device direction (input/output)
    pub direction: AudioDirection,
    /// Whether this is the system default device
    pub is_default: bool,
    /// Supported sample rates
    pub supported_sample_rates: Vec<u32>,
    /// Supported channel counts
    pub supported_channels: Vec<u16>,
    /// Supported bit depths
    pub supported_bit_depths: Vec<u16>,
}

impl AudioDeviceInfo {
    /// Create a new audio device info
    pub fn new<S: Into<String>>(id: S, name: S, direction: AudioDirection) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            direction,
            is_default: false,
            supported_sample_rates: vec![8000, 16000, 44100, 48000],
            supported_channels: vec![1, 2],
            supported_bit_depths: vec![16],
        }
    }

    /// Check if the device supports a specific format
    pub fn supports_format(&self, format: &AudioFormat) -> bool {
        self.supported_sample_rates.contains(&format.sample_rate) &&
        self.supported_channels.contains(&format.channels) &&
        self.supported_bit_depths.contains(&format.bits_per_sample)
    }

    /// Get the best supported format for VoIP
    pub fn best_voip_format(&self) -> AudioFormat {
        // Prefer 16kHz if available, fallback to 8kHz
        let sample_rate = if self.supported_sample_rates.contains(&16000) {
            16000
        } else if self.supported_sample_rates.contains(&8000) {
            8000
        } else {
            *self.supported_sample_rates.first().unwrap_or(&48000)
        };

        // Prefer mono for VoIP
        let channels = if self.supported_channels.contains(&1) {
            1
        } else {
            *self.supported_channels.first().unwrap_or(&2)
        };

        // Prefer 16-bit
        let bits_per_sample = if self.supported_bit_depths.contains(&16) {
            16
        } else {
            *self.supported_bit_depths.first().unwrap_or(&16)
        };

        AudioFormat::new(sample_rate, channels, bits_per_sample, 20)
    }
}

/// Audio frame containing PCM samples
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// PCM audio samples (signed 16-bit integers)
    pub samples: Vec<i16>,
    /// Audio format of this frame
    pub format: AudioFormat,
    /// Timestamp when this frame was captured/should be played (RTP timestamp)
    pub timestamp: u32,
    /// Sequence number for ordering
    pub sequence: u32,
    /// Optional metadata
    pub metadata: std::collections::HashMap<String, String>,
}

impl AudioFrame {
    /// Create a new audio frame
    pub fn new(samples: Vec<i16>, format: AudioFormat, timestamp: u32) -> Self {
        Self {
            samples,
            format,
            timestamp,
            sequence: 0,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Create a silent audio frame
    pub fn silent(format: AudioFormat, timestamp: u32) -> Self {
        let samples = vec![0; format.samples_per_frame()];
        Self::new(samples, format, timestamp)
    }

    /// Get the duration of this frame
    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.format.frame_size_ms as u64)
    }

    /// Check if this frame contains silence
    pub fn is_silent(&self) -> bool {
        self.samples.iter().all(|&sample| sample.abs() < 100) // Threshold for silence
    }

    /// Get the RMS (Root Mean Square) level of this frame
    pub fn rms_level(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }

        let sum_of_squares: f64 = self.samples
            .iter()
            .map(|&sample| (sample as f64).powi(2))
            .sum();
        
        (sum_of_squares / self.samples.len() as f64).sqrt() as f32
    }

    /// Convert to session-core AudioFrame format
    pub fn to_session_core(&self) -> rvoip_session_core::api::types::AudioFrame {
        rvoip_session_core::api::types::AudioFrame::new(
            self.samples.clone(),
            self.format.sample_rate,
            self.format.channels as u8, // Convert u16 to u8
            self.timestamp,
        )
    }

    /// Create from session-core AudioFrame
    pub fn from_session_core(
        frame: &rvoip_session_core::api::types::AudioFrame,
        frame_size_ms: u32,
    ) -> Self {
        let format = AudioFormat::new(
            frame.sample_rate,
            frame.channels as u16, // Convert u8 to u16
            16, // Assume 16-bit samples
            frame_size_ms,
        );

        Self::new(frame.samples.clone(), format, frame.timestamp)
    }
}

/// Audio codec enumeration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodec {
    /// G.711 μ-law (PCMU)
    G711U,
    /// G.711 A-law (PCMA)
    G711A,
    /// G.722 wideband codec
    G722,
    /// Opus codec
    Opus,
    /// Linear PCM
    PCM,
    /// Custom codec
    Custom(String),
}

impl AudioCodec {
    /// Get the codec name
    pub fn name(&self) -> &str {
        match self {
            AudioCodec::G711U => "PCMU",
            AudioCodec::G711A => "PCMA",
            AudioCodec::G722 => "G722",
            AudioCodec::Opus => "opus",
            AudioCodec::PCM => "L16",
            AudioCodec::Custom(name) => name,
        }
    }

    /// Get the RTP payload type (if standard)
    pub fn payload_type(&self) -> Option<u8> {
        match self {
            AudioCodec::G711U => Some(0),
            AudioCodec::G711A => Some(8),
            AudioCodec::G722 => Some(9),
            AudioCodec::PCM => Some(11),
            AudioCodec::Opus => Some(96), // Dynamic payload type
            AudioCodec::Custom(_) => None,
        }
    }

    /// Check if this codec supports variable bitrate
    pub fn supports_vbr(&self) -> bool {
        matches!(self, AudioCodec::Opus)
    }

    /// Get the typical sample rate for this codec
    pub fn typical_sample_rate(&self) -> u32 {
        match self {
            AudioCodec::G711U | AudioCodec::G711A => 8000,
            AudioCodec::G722 => 16000,
            AudioCodec::Opus => 48000,
            AudioCodec::PCM => 44100,
            AudioCodec::Custom(_) => 8000,
        }
    }
}

impl fmt::Display for AudioCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Audio stream configuration
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStreamConfig {
    /// Input audio format (from device)
    pub input_format: AudioFormat,
    /// Output audio format (to RTP)
    pub output_format: AudioFormat,
    /// Codec to use for encoding
    pub codec: AudioCodec,
    /// Enable echo cancellation
    pub enable_aec: bool,
    /// Enable automatic gain control
    pub enable_agc: bool,
    /// Enable noise suppression
    pub enable_noise_suppression: bool,
    /// Enable voice activity detection
    pub enable_vad: bool,
    /// Buffer size in milliseconds
    pub buffer_size_ms: u32,
    /// Jitter buffer size in packets
    pub jitter_buffer_size: u32,
}

impl AudioStreamConfig {
    /// Create a basic VoIP configuration
    pub fn voip_basic() -> Self {
        Self {
            input_format: AudioFormat::pcm_8khz_mono(),
            output_format: AudioFormat::pcm_8khz_mono(),
            codec: AudioCodec::G711U,
            enable_aec: true,
            enable_agc: true,
            enable_noise_suppression: false,
            enable_vad: false,
            buffer_size_ms: 100,
            jitter_buffer_size: 10,
        }
    }

    /// Create a high-quality configuration
    pub fn voip_high_quality() -> Self {
        Self {
            input_format: AudioFormat::pcm_48khz_stereo(),
            output_format: AudioFormat::pcm_16khz_mono(),
            codec: AudioCodec::Opus,
            enable_aec: true,
            enable_agc: true,
            enable_noise_suppression: true,
            enable_vad: true,
            buffer_size_ms: 50,
            jitter_buffer_size: 15,
        }
    }
}

/// Audio quality metrics
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioQualityMetrics {
    /// Mean Opinion Score (1.0 to 5.0)
    pub mos_score: f32,
    /// Round-trip time in milliseconds
    pub rtt_ms: u32,
    /// Packet loss percentage (0.0 to 100.0)
    pub packet_loss_percent: f32,
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    /// Audio level (RMS) 
    pub audio_level: f32,
    /// Signal-to-noise ratio in dB
    pub snr_db: f32,
    /// Timestamp of measurement
    pub timestamp: DateTime<Utc>,
}

impl AudioQualityMetrics {
    /// Create new quality metrics
    pub fn new() -> Self {
        Self {
            mos_score: 0.0,
            rtt_ms: 0,
            packet_loss_percent: 0.0,
            jitter_ms: 0.0,
            audio_level: 0.0,
            snr_db: 0.0,
            timestamp: Utc::now(),
        }
    }

    /// Check if the quality is acceptable for VoIP
    pub fn is_acceptable(&self) -> bool {
        self.mos_score >= 3.5 &&
        self.packet_loss_percent < 5.0 &&
        self.jitter_ms < 100.0
    }

    /// Get a quality rating string
    pub fn quality_rating(&self) -> &'static str {
        match self.mos_score {
            x if x >= 4.5 => "Excellent",
            x if x >= 4.0 => "Good", 
            x if x >= 3.5 => "Fair",
            x if x >= 2.5 => "Poor",
            _ => "Bad",
        }
    }
}

impl Default for AudioQualityMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_creation() {
        let format = AudioFormat::pcm_8khz_mono();
        assert_eq!(format.sample_rate, 8000);
        assert_eq!(format.channels, 1);
        assert_eq!(format.bits_per_sample, 16);
        assert_eq!(format.frame_size_ms, 20);
    }

    #[test]
    fn test_audio_format_calculations() {
        let format = AudioFormat::pcm_8khz_mono();
        assert_eq!(format.samples_per_frame(), 160); // 8000 * 20 / 1000
        assert_eq!(format.bytes_per_frame(), 320); // 160 * 1 * 2
    }

    #[test]
    fn test_audio_format_compatibility() {
        let format1 = AudioFormat::pcm_8khz_mono();
        let format2 = AudioFormat::pcm_8khz_mono();
        let format3 = AudioFormat::pcm_16khz_mono();

        assert!(format1.is_compatible_with(&format2));
        assert!(!format1.is_compatible_with(&format3));
    }

    #[test]
    fn test_voip_suitable_format() {
        let voip_format = AudioFormat::pcm_8khz_mono();
        assert!(voip_format.is_voip_suitable());

        let cd_format = AudioFormat::pcm_44khz_stereo();
        assert!(!cd_format.is_voip_suitable());
    }

    #[test]
    fn test_audio_frame_silence() {
        let format = AudioFormat::pcm_8khz_mono();
        let silent_frame = AudioFrame::silent(format.clone(), 1000);
        assert!(silent_frame.is_silent());

        let loud_samples = vec![1000; format.samples_per_frame()];
        let loud_frame = AudioFrame::new(loud_samples, format, 1000);
        assert!(!loud_frame.is_silent());
    }

    #[test]
    fn test_codec_properties() {
        let g711 = AudioCodec::G711U;
        assert_eq!(g711.name(), "PCMU");
        assert_eq!(g711.payload_type(), Some(0));
        assert_eq!(g711.typical_sample_rate(), 8000);
        assert!(!g711.supports_vbr());

        let opus = AudioCodec::Opus;
        assert_eq!(opus.name(), "opus");
        assert!(opus.supports_vbr());
    }

    #[test]
    fn test_quality_metrics() {
        let mut metrics = AudioQualityMetrics::new();
        metrics.mos_score = 4.2;
        metrics.packet_loss_percent = 1.0;
        metrics.jitter_ms = 15.0;

        assert!(metrics.is_acceptable());
        assert_eq!(metrics.quality_rating(), "Good");
    }

    #[test]
    fn test_device_info() {
        let device = AudioDeviceInfo::new("test-mic", "Test Microphone", AudioDirection::Input);
        let format = AudioFormat::pcm_8khz_mono();
        assert!(device.supports_format(&format));

        let best_format = device.best_voip_format();
        assert!(best_format.is_voip_suitable());
    }
} 