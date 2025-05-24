//! MediaEngine configuration and capabilities
//!
//! This module defines configuration structures and capability definitions
//! for the MediaEngine.

use std::time::Duration;
use crate::types::{PayloadType, SampleRate};

/// Configuration for the MediaEngine
#[derive(Debug, Clone)]
pub struct MediaEngineConfig {
    /// Audio processing configuration
    pub audio: AudioConfig,
    /// Codec configuration
    pub codecs: CodecConfig,
    /// Quality monitoring configuration
    pub quality: QualityConfig,
    /// Buffer configuration
    pub buffers: BufferConfig,
    /// Performance configuration
    pub performance: PerformanceConfig,
}

impl Default for MediaEngineConfig {
    fn default() -> Self {
        Self {
            audio: AudioConfig::default(),
            codecs: CodecConfig::default(),
            quality: QualityConfig::default(),
            buffers: BufferConfig::default(),
            performance: PerformanceConfig::default(),
        }
    }
}

/// Audio processing configuration
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Enable acoustic echo cancellation
    pub enable_aec: bool,
    /// Enable automatic gain control
    pub enable_agc: bool,
    /// Enable voice activity detection
    pub enable_vad: bool,
    /// Enable noise suppression
    pub enable_noise_suppression: bool,
    /// Default sample rate
    pub default_sample_rate: SampleRate,
    /// Frame size in milliseconds
    pub frame_size_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enable_aec: false,        // Disabled by default (CPU intensive)
            enable_agc: true,         // Enabled by default
            enable_vad: true,         // Enabled by default
            enable_noise_suppression: false, // Disabled by default (CPU intensive)
            default_sample_rate: SampleRate::Rate8000, // Standard telephony
            frame_size_ms: 20,        // Standard 20ms frames
        }
    }
}

/// Codec configuration
#[derive(Debug, Clone)]
pub struct CodecConfig {
    /// Enabled payload types
    pub enabled_payload_types: Vec<PayloadType>,
    /// Preferred codec for new sessions
    pub preferred_codec: PayloadType,
    /// Enable transcoding between codecs
    pub enable_transcoding: bool,
    /// Maximum codec complexity (0-10)
    pub max_complexity: u8,
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            enabled_payload_types: vec![
                0,   // PCMU
                8,   // PCMA
                111, // Opus (dynamic)
            ],
            preferred_codec: 0, // PCMU by default
            enable_transcoding: false, // Disabled by default
            max_complexity: 5, // Medium complexity
        }
    }
}

/// Quality monitoring configuration
#[derive(Debug, Clone)]
pub struct QualityConfig {
    /// Enable real-time quality monitoring
    pub enable_monitoring: bool,
    /// Quality metrics collection interval
    pub metrics_interval: Duration,
    /// Enable adaptive quality
    pub enable_adaptation: bool,
    /// Quality thresholds for adaptation
    pub thresholds: QualityThresholds,
}

impl Default for QualityConfig {
    fn default() -> Self {
        Self {
            enable_monitoring: true,
            metrics_interval: Duration::from_secs(5),
            enable_adaptation: false, // Disabled by default
            thresholds: QualityThresholds::default(),
        }
    }
}

/// Quality threshold configuration
#[derive(Debug, Clone)]
pub struct QualityThresholds {
    /// Maximum acceptable packet loss (0.0-1.0)
    pub max_packet_loss: f32,
    /// Maximum acceptable jitter in milliseconds
    pub max_jitter_ms: f32,
    /// Minimum acceptable audio level (dB)
    pub min_audio_level_db: f32,
}

impl Default for QualityThresholds {
    fn default() -> Self {
        Self {
            max_packet_loss: 0.05,    // 5% max packet loss
            max_jitter_ms: 100.0,     // 100ms max jitter
            min_audio_level_db: -60.0, // -60dB minimum level
        }
    }
}

/// Buffer configuration
#[derive(Debug, Clone)]
pub struct BufferConfig {
    /// Jitter buffer target delay in milliseconds
    pub jitter_buffer_target_ms: u32,
    /// Jitter buffer maximum delay in milliseconds
    pub jitter_buffer_max_ms: u32,
    /// Enable adaptive buffering
    pub enable_adaptive_buffering: bool,
    /// Initial buffer size
    pub initial_buffer_size: usize,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            jitter_buffer_target_ms: 60,  // 60ms target
            jitter_buffer_max_ms: 200,    // 200ms maximum
            enable_adaptive_buffering: true,
            initial_buffer_size: 1024,    // 1KB initial buffer
        }
    }
}

/// Performance configuration
#[derive(Debug, Clone)]
pub struct PerformanceConfig {
    /// Number of worker threads for processing
    pub worker_threads: usize,
    /// Maximum sessions per engine
    pub max_sessions: usize,
    /// Enable performance profiling
    pub enable_profiling: bool,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            worker_threads: num_cpus::get().max(2), // Use available CPUs, min 2
            max_sessions: 1000,                     // Support up to 1000 sessions
            enable_profiling: false,                // Disabled by default
        }
    }
}

/// MediaEngine capabilities for SDP negotiation
#[derive(Debug, Clone)]
pub struct EngineCapabilities {
    /// Supported audio codecs with their parameters
    pub audio_codecs: Vec<AudioCodecCapability>,
    /// Supported audio processing features
    pub audio_processing: AudioProcessingCapabilities,
    /// Supported sample rates
    pub sample_rates: Vec<SampleRate>,
    /// Maximum supported sessions
    pub max_sessions: usize,
}

/// Audio codec capability information
#[derive(Debug, Clone)]
pub struct AudioCodecCapability {
    /// Payload type
    pub payload_type: PayloadType,
    /// Codec name
    pub name: String,
    /// Supported sample rates
    pub sample_rates: Vec<SampleRate>,
    /// Number of channels
    pub channels: u8,
    /// Clock rate
    pub clock_rate: u32,
}

/// Audio processing capabilities
#[derive(Debug, Clone)]
pub struct AudioProcessingCapabilities {
    /// Echo cancellation available
    pub aec_available: bool,
    /// Automatic gain control available
    pub agc_available: bool,
    /// Voice activity detection available
    pub vad_available: bool,
    /// Noise suppression available
    pub noise_suppression_available: bool,
}

impl Default for AudioProcessingCapabilities {
    fn default() -> Self {
        Self {
            aec_available: true,
            agc_available: true,
            vad_available: true,
            noise_suppression_available: true,
        }
    }
} 