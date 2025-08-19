//! # Media Core library for the RVOIP project
//! 
//! `media-core` provides comprehensive media processing capabilities for SIP servers.
//! It focuses exclusively on media processing, codec management, and media session
//! coordination while integrating cleanly with `session-core` and `rtp-core`.
//!
//! ## Core Components
//! 
//! - **MediaEngine**: Central orchestrator for all media processing
//! - **MediaSession**: Per-dialog media management
//! - **Codec Framework**: Audio codec support (G.711, Opus, etc.)
//! - **Audio Processing**: AEC, AGC, VAD, noise suppression
//! - **Quality Monitoring**: Real-time quality metrics and adaptation
//! 
//! ## Audio Muting
//! 
//! The media-core library implements silence-based muting that maintains continuous
//! RTP packet flow. When a session is muted, audio samples are replaced with silence
//! before encoding, preserving:
//! 
//! - RTP sequence numbers and timestamps
//! - NAT traversal and binding keepalive
//! - Compatibility with all SIP endpoints
//! - Instant mute/unmute without renegotiation
//! 
//! Use `MediaSessionController::set_audio_muted()` for production-ready muting.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use rvoip_media_core::relay::controller::MediaSessionController;
//! use rvoip_media_core::types::{MediaSessionId, DialogId};
//! 
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create media session controller
//!     let controller = MediaSessionController::new();
//!     
//!     // RTP bridge callbacks can be added for external RTP event handling
//!     // This enables integration with session-core for audio frame delivery
//!     
//!     Ok(())
//! }
//! ```

// Core modules
pub mod api;         // NEW - API types and errors for moved functionality
pub mod error;
pub mod types;
pub mod engine;
pub mod session;     // New session module
pub mod processing;  // New processing pipeline module
pub mod quality;     // New quality monitoring module
pub mod integration; // New integration module
pub mod buffer;      // New buffer module
pub mod performance; // New performance optimization module
pub mod rtp_processing; // NEW - RTP media processing (moved from rtp-core)

// Working modules from old implementation (to be refactored)
pub mod codec;
pub mod relay;

// Audio utilities
pub mod audio;

// Re-export core types
pub use error::{Error, Result};
pub use types::*;

// Re-export RTP statistics types from rtp-core
pub use rvoip_rtp_core::session::{RtpSessionStats, RtpStreamStats};

// Re-export engine components
pub use engine::{
    MediaEngine, 
    MediaEngineConfig, 
    EngineCapabilities,
    MediaSessionParams,
    MediaSessionHandle,
    EngineState,
};

// NEW: Enhanced configuration exports from media_engine
pub use engine::media_engine::{
    PerformanceLevel,
    AdvancedProcessorFactory,
};

// Re-export session components
pub use session::{
    MediaSession,
    MediaSessionConfig,
    MediaSessionState,
    MediaSessionEvent as SessionEvent, // Rename to avoid conflict
    MediaSessionEventType,
};

// Re-export integration components
pub use integration::{
    RtpBridge,
    RtpBridgeConfig,
    SessionBridge,
    SessionBridgeConfig,
    IntegrationEvent,
    IntegrationEventType,
    RtpEvent,
    RtpEventCallback,
};

// Legacy exports (will be replaced in Phase 2)
pub use codec::{Codec, CodecRegistry};
pub use relay::{
    MediaSessionController,
    MediaConfig,
    MediaSessionStatus,
    MediaSessionInfo,
    G711PcmuCodec,
    G711PcmaCodec,
};

// NEW: Enhanced configuration re-exports
pub use engine::config::{
    PerformanceConfig,
    AdvancedProcessingConfig,
    AudioConfig,
    CodecConfig,
    QualityConfig,
    BufferConfig,
};

/// Media sample type (raw audio data)
pub type Sample = i16;

/// Audio format (channels, bit depth, sample rate)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    /// Number of channels (1 for mono, 2 for stereo)
    pub channels: u8,
    /// Bits per sample (typically 8, 16, or 32)
    pub bit_depth: u8,
    /// Sample rate in Hz
    pub sample_rate: crate::types::SampleRate,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            channels: 1,         // Default to mono
            bit_depth: 16,       // Default to 16-bit
            sample_rate: crate::types::SampleRate::default(),
        }
    }
}

impl AudioFormat {
    /// Create a new audio format
    pub fn new(channels: u8, bit_depth: u8, sample_rate: crate::types::SampleRate) -> Self {
        Self {
            channels,
            bit_depth,
            sample_rate,
        }
    }
    
    /// Create a new mono 16-bit format with the given sample rate
    pub fn mono_16bit(sample_rate: crate::types::SampleRate) -> Self {
        Self::new(1, 16, sample_rate)
    }
    
    /// Create a new stereo 16-bit format with the given sample rate
    pub fn stereo_16bit(sample_rate: crate::types::SampleRate) -> Self {
        Self::new(2, 16, sample_rate)
    }
    
    /// Standard narrowband telephony format (mono, 16-bit, 8kHz)
    pub fn telephony() -> Self {
        Self::mono_16bit(crate::types::SampleRate::Rate8000)
    }
}

/// A chunk of audio samples (PCM or encoded)
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    /// Raw audio data
    pub data: bytes::Bytes,
    /// Audio format information
    pub format: AudioFormat,
}

impl AudioBuffer {
    /// Create a new audio buffer with the given data and format
    pub fn new(data: bytes::Bytes, format: AudioFormat) -> Self {
        Self { data, format }
    }
    
    /// Get the duration of the audio in milliseconds
    pub fn duration_ms(&self) -> u32 {
        let bytes_per_sample = (self.format.bit_depth / 8) as u32;
        let samples = (self.data.len() as u32) / bytes_per_sample / (self.format.channels as u32);
        (samples * 1000) / self.format.sample_rate.as_hz()
    }
    
    /// Get the number of samples in the buffer
    pub fn samples(&self) -> usize {
        let bytes_per_sample = (self.format.bit_depth / 8) as usize;
        self.data.len() / bytes_per_sample / (self.format.channels as usize)
    }
}

/// Prelude module with commonly used types
pub mod prelude {
    // Re-export basic types for convenience
    pub use crate::types::{
        // Basic types
        DialogId,
        MediaSessionId,
        PayloadType,
        MediaType,
        MediaDirection,
        SampleRate,
        AudioFrame,
        VideoFrame,
        MediaPacket,
        // Payload types
        payload_types::{static_types, dynamic_range},
        // Statistics types
        MediaStatistics,
        MediaProcessingStats,
        QualityMetrics,
    };
    
    // Re-export from codec module
    pub use crate::codec::{
        mapping::CodecMapper,
        audio::AudioCodec,
    };
    
    // Re-export from engine module  
    pub use crate::engine::{
        media_engine::MediaEngine,
        config::{
            MediaEngineConfig,
            AudioConfig,
            CodecConfig,
            QualityConfig,
            BufferConfig,
            PerformanceConfig,
            AdvancedProcessingConfig,
            EngineCapabilities,
            AudioCodecCapability,
            AudioProcessingCapabilities,
        },
    };
    
    // Re-export from RTP core
    pub use rvoip_rtp_core::{
        RtpHeader,
        RtpPacket,
        RtpSession,
        RtpSessionConfig,
    };
    
    // Re-export from error module
    pub use crate::error::{Error, Result};
} 