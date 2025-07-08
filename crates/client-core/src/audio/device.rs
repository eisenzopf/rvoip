//! Audio Device Abstraction
//!
//! This module defines the core traits and types for audio device abstraction.
//! It provides a platform-agnostic interface for audio input/output operations.

use std::fmt;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Audio device direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDirection {
    /// Audio input (microphone)
    Input,
    /// Audio output (speaker)
    Output,
}

/// Audio format specification
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFormat {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u16,
    /// Bits per sample (typically 16)
    pub bits_per_sample: u16,
    /// Frame size in milliseconds
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
    
    /// Create default VoIP format (8kHz, mono, 16-bit, 20ms frames)
    pub fn default_voip() -> Self {
        Self::new(
            crate::audio::DEFAULT_SAMPLE_RATE,
            crate::audio::DEFAULT_CHANNELS,
            16,
            crate::audio::DEFAULT_FRAME_SIZE_MS,
        )
    }
    
    /// Create wideband VoIP format (16kHz, mono, 16-bit, 20ms frames)
    pub fn wideband_voip() -> Self {
        Self::new(16000, 1, 16, 20)
    }
    
    /// Calculate samples per frame
    pub fn samples_per_frame(&self) -> usize {
        crate::audio::samples_per_frame(self.sample_rate, self.frame_size_ms)
    }
    
    /// Calculate bytes per frame
    pub fn bytes_per_frame(&self) -> usize {
        self.samples_per_frame() * self.channels as usize * (self.bits_per_sample / 8) as usize
    }
}

/// Audio device information
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    /// Device identifier
    pub id: String,
    /// Human-readable device name
    pub name: String,
    /// Device direction
    pub direction: AudioDirection,
    /// Whether this is the default device
    pub is_default: bool,
    /// Supported sample rates
    pub supported_sample_rates: Vec<u32>,
    /// Supported channel counts
    pub supported_channels: Vec<u16>,
}

impl AudioDeviceInfo {
    /// Create a new audio device info
    pub fn new(id: String, name: String, direction: AudioDirection) -> Self {
        Self {
            id,
            name,
            direction,
            is_default: false,
            supported_sample_rates: vec![8000, 16000, 44100, 48000],
            supported_channels: vec![1, 2],
        }
    }
    
    /// Check if the device supports the given format
    pub fn supports_format(&self, format: &AudioFormat) -> bool {
        self.supported_sample_rates.contains(&format.sample_rate) &&
        self.supported_channels.contains(&format.channels)
    }
}

/// Audio frame data
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// Audio samples as i16 PCM
    pub samples: Vec<i16>,
    /// Audio format
    pub format: AudioFormat,
    /// Timestamp in milliseconds
    pub timestamp_ms: u64,
}

impl AudioFrame {
    /// Create a new audio frame
    pub fn new(samples: Vec<i16>, format: AudioFormat, timestamp_ms: u64) -> Self {
        Self {
            samples,
            format,
            timestamp_ms,
        }
    }
    
    /// Create a silent frame of the specified duration
    pub fn silent(format: AudioFormat, timestamp_ms: u64) -> Self {
        let samples = vec![0; format.samples_per_frame()];
        Self::new(samples, format, timestamp_ms)
    }
    
    /// Convert to session-core AudioFrame
    pub fn to_session_core(&self) -> rvoip_session_core::api::types::AudioFrame {
        rvoip_session_core::api::types::AudioFrame {
            samples: self.samples.clone(),
            sample_rate: self.format.sample_rate,
            channels: self.format.channels as u8,
            timestamp: (self.timestamp_ms / 1000) as u32,  // Convert ms to seconds
        }
    }
    
    /// Convert from session-core AudioFrame
    pub fn from_session_core(frame: &rvoip_session_core::api::types::AudioFrame, frame_size_ms: u32) -> Self {
        let format = AudioFormat::new(
            frame.sample_rate,
            frame.channels as u16,
            16, // Assume 16-bit samples
            frame_size_ms,
        );
        
        Self::new(
            frame.samples.clone(),
            format,
            (frame.timestamp as u64) * 1000,  // Convert seconds to ms
        )
    }
}

/// Audio device error types
#[derive(Debug, Clone)]
pub enum AudioError {
    /// Device not found
    DeviceNotFound { device_id: String },
    /// Format not supported
    FormatNotSupported { format: AudioFormat, device_id: String },
    /// Device is already in use
    DeviceInUse { device_id: String },
    /// Platform-specific error
    PlatformError { message: String },
    /// IO error
    IoError { message: String },
    /// Configuration error
    ConfigurationError { message: String },
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::DeviceNotFound { device_id } => {
                write!(f, "Audio device not found: {}", device_id)
            }
            AudioError::FormatNotSupported { format, device_id } => {
                write!(f, "Audio format {:?} not supported by device: {}", format, device_id)
            }
            AudioError::DeviceInUse { device_id } => {
                write!(f, "Audio device is already in use: {}", device_id)
            }
            AudioError::PlatformError { message } => {
                write!(f, "Platform audio error: {}", message)
            }
            AudioError::IoError { message } => {
                write!(f, "Audio I/O error: {}", message)
            }
            AudioError::ConfigurationError { message } => {
                write!(f, "Audio configuration error: {}", message)
            }
        }
    }
}

impl std::error::Error for AudioError {}

/// Result type for audio operations
pub type AudioResult<T> = std::result::Result<T, AudioError>;

/// Audio device trait
/// 
/// This trait defines the interface that all audio devices must implement.
/// Platform-specific implementations provide the actual audio I/O functionality.
#[async_trait::async_trait]
pub trait AudioDevice: Send + Sync + std::fmt::Debug {
    /// Get device information
    fn info(&self) -> &AudioDeviceInfo;
    
    /// Check if the device supports the given format
    fn supports_format(&self, format: &AudioFormat) -> bool {
        self.info().supports_format(format)
    }
    
    /// Start audio capture (for input devices)
    /// 
    /// Returns a receiver for audio frames captured from the device.
    /// The device will capture audio in the specified format and send
    /// frames through the returned channel.
    async fn start_capture(&self, format: AudioFormat) -> AudioResult<mpsc::Receiver<AudioFrame>>;
    
    /// Stop audio capture
    async fn stop_capture(&self) -> AudioResult<()>;
    
    /// Start audio playback (for output devices)
    /// 
    /// Returns a sender for audio frames to be played through the device.
    /// Audio frames sent through the returned channel will be played
    /// through the device.
    async fn start_playback(&self, format: AudioFormat) -> AudioResult<mpsc::Sender<AudioFrame>>;
    
    /// Stop audio playback
    async fn stop_playback(&self) -> AudioResult<()>;
    
    /// Check if the device is currently active
    fn is_active(&self) -> bool;
    
    /// Get the current format being used (if active)
    fn current_format(&self) -> Option<AudioFormat>;
} 