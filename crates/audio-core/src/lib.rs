//! RVOIP Audio Core Library
//!
//! Comprehensive audio handling for VoIP applications, providing device management,
//! format conversion, codec processing, and RTP audio stream integration.
//!
//! # Architecture
//!
//! The audio-core library is organized into several key modules:
//!
//! - **Device Management**: Cross-platform audio device access and control
//! - **Format Bridge**: Audio format conversion and resampling
//! - **Codec Engine**: Audio codec encoding/decoding for VoIP
//! - **Pipeline**: High-level audio streaming pipelines
//! - **RTP Integration**: RTP payload encoding/decoding
//! - **Processing**: Audio signal processing (AEC, AGC, etc.)
//!
//! # Quick Start
//!
//! ## Device Enumeration
//!
//! ```rust,no_run
//! use rvoip_audio_core::device::AudioDeviceManager;
//! use rvoip_audio_core::types::AudioDirection;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let device_manager = AudioDeviceManager::new().await?;
//! 
//! // List available devices
//! let input_devices = device_manager.list_devices(AudioDirection::Input).await?;
//! let output_devices = device_manager.list_devices(AudioDirection::Output).await?;
//! 
//! println!("Found {} input devices", input_devices.len());
//! println!("Found {} output devices", output_devices.len());
//! # Ok(())
//! # }
//! ```
//!
//! ## Audio Pipeline
//!
//! ```rust,no_run
//! use rvoip_audio_core::pipeline::AudioPipeline;
//! use rvoip_audio_core::types::{AudioFormat, AudioDirection};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a basic audio pipeline
//! let mut pipeline = AudioPipeline::builder()
//!     .input_format(AudioFormat::pcm_8khz_mono())
//!     .output_format(AudioFormat::pcm_48khz_stereo())
//!     .build()
//!     .await?;
//!
//! // Start the pipeline
//! pipeline.start().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Features
//!
//! This crate supports multiple optional features:
//!
//! - `device-cpal`: CPAL-based audio device support (default)
//! - `format-conversion`: Audio format conversion and resampling
//! - `codec-g711`, `codec-g722`, `codec-opus`: Various audio codecs
//! - `processing-*`: Audio signal processing features
//!
//! Enable features in your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! rvoip-audio-core = { version = "0.1", features = ["full"] }
//! ```

#![warn(missing_docs)]
#![doc(html_root_url = "https://docs.rs/rvoip-audio-core/0.1.0")]

// Core modules
pub mod error;
pub mod types;

// Device management
pub mod device;

// Format conversion and processing
#[cfg(feature = "format-conversion")]
pub mod format;

// Codec support
pub mod codec;

// Audio pipeline
pub mod pipeline;

// RTP integration
pub mod rtp;

// Audio signal processing
#[cfg(any(
    feature = "processing-aec",
    feature = "processing-agc", 
    feature = "processing-noise",
    feature = "processing-vad"
))]
pub mod processing;

// Re-export commonly used types
pub use error::{AudioError, AudioResult};
pub use types::{
    AudioFormat, AudioFrame, AudioDirection, AudioCodec,
    AudioDeviceInfo, AudioStreamConfig, AudioQualityMetrics
};

// Re-export device management
pub use device::{AudioDeviceManager, AudioDevice};

// Re-export pipeline
pub use pipeline::AudioPipeline;

// Re-export codec engine
pub use codec::{CodecType, CodecConfig, CodecFactory, CodecNegotiator, CodecQualityMetrics};

// Re-export integration types from session-core and rtp-core
pub use rvoip_session_core::api::types::SessionId;
pub use rvoip_session_core::api::MediaControl;
pub use rvoip_rtp_core::packet::RtpPacket;

/// Audio-core library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default audio configuration constants
pub mod defaults {
    use crate::types::AudioFormat;
    
    /// Default sample rate for VoIP (8kHz narrowband)
    pub const SAMPLE_RATE_NARROWBAND: u32 = 8000;
    
    /// Default sample rate for wideband VoIP (16kHz)
    pub const SAMPLE_RATE_WIDEBAND: u32 = 16000;
    
    /// Default sample rate for high-quality audio (48kHz)
    pub const SAMPLE_RATE_HIFI: u32 = 48000;
    
    /// Default frame size in milliseconds
    pub const FRAME_SIZE_MS: u32 = 20;
    
    /// Default number of channels (mono)
    pub const CHANNELS: u16 = 1;
    
    /// Default bit depth
    pub const BIT_DEPTH: u16 = 16;
    
    /// Default VoIP audio format (8kHz, mono, 16-bit, 20ms frames)
    pub fn voip_format() -> AudioFormat {
        AudioFormat::new(
            SAMPLE_RATE_NARROWBAND,
            CHANNELS,
            BIT_DEPTH,
            FRAME_SIZE_MS,
        )
    }
    
    /// Default wideband VoIP audio format (16kHz, mono, 16-bit, 20ms frames)
    pub fn wideband_format() -> AudioFormat {
        AudioFormat::new(
            SAMPLE_RATE_WIDEBAND,
            CHANNELS,
            BIT_DEPTH,
            FRAME_SIZE_MS,
        )
    }
    
    /// Default high-quality audio format (48kHz, stereo, 16-bit, 20ms frames)
    pub fn hifi_format() -> AudioFormat {
        AudioFormat::new(
            SAMPLE_RATE_HIFI,
            2, // stereo
            BIT_DEPTH,
            FRAME_SIZE_MS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_version_defined() {
        assert!(!VERSION.is_empty());
    }
    
    #[test]
    fn test_default_formats() {
        let voip = defaults::voip_format();
        assert_eq!(voip.sample_rate, 8000);
        assert_eq!(voip.channels, 1);
        
        let wideband = defaults::wideband_format();
        assert_eq!(wideband.sample_rate, 16000);
        assert_eq!(wideband.channels, 1);
        
        let hifi = defaults::hifi_format();
        assert_eq!(hifi.sample_rate, 48000);
        assert_eq!(hifi.channels, 2);
    }
} 