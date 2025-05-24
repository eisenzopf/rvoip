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
//! ## Quick Start
//!
//! ```rust
//! use rvoip_media_core::prelude::*;
//! 
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Create and start media engine
//!     let config = MediaEngineConfig::default();
//!     let engine = MediaEngine::new(config).await?;
//!     engine.start().await?;
//!     
//!     // Create media session for SIP dialog
//!     let dialog_id = DialogId::new("call-123");
//!     let params = MediaSessionParams::audio_only()
//!         .with_preferred_codec(payload_types::PCMU);
//!     let session = engine.create_media_session(dialog_id, params).await?;
//!     
//!     // Get codec capabilities for SDP negotiation
//!     let capabilities = engine.get_supported_codecs();
//!     
//!     // Clean shutdown
//!     engine.stop().await?;
//!     Ok(())
//! }
//! ```

// Core modules
pub mod error;
pub mod types;
pub mod engine;
pub mod session;     // New session module
pub mod processing;  // New processing pipeline module
pub mod quality;     // New quality monitoring module
pub mod integration; // New integration module
pub mod buffer;      // New buffer module

// Working modules from old implementation (to be refactored)
pub mod codec;
pub mod relay;

// Re-export core types
pub use error::{Error, Result};
pub use types::*;

// Re-export engine components
pub use engine::{
    MediaEngine, 
    MediaEngineConfig, 
    EngineCapabilities,
    MediaSessionParams,
    MediaSessionHandle,
    EngineState,
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
    pub sample_rate: SampleRate,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            channels: 1,         // Default to mono
            bit_depth: 16,       // Default to 16-bit
            sample_rate: SampleRate::default(),
        }
    }
}

impl AudioFormat {
    /// Create a new audio format
    pub fn new(channels: u8, bit_depth: u8, sample_rate: SampleRate) -> Self {
        Self {
            channels,
            bit_depth,
            sample_rate,
        }
    }
    
    /// Create a new mono 16-bit format with the given sample rate
    pub fn mono_16bit(sample_rate: SampleRate) -> Self {
        Self::new(1, 16, sample_rate)
    }
    
    /// Create a new stereo 16-bit format with the given sample rate
    pub fn stereo_16bit(sample_rate: SampleRate) -> Self {
        Self::new(2, 16, sample_rate)
    }
    
    /// Standard narrowband telephony format (mono, 16-bit, 8kHz)
    pub fn telephony() -> Self {
        Self::mono_16bit(SampleRate::Rate8000)
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
    // Core types
    pub use crate::{
        Error, 
        Result,
        DialogId,
        MediaSessionId,
        PayloadType,
        AudioFrame,
        MediaPacket,
        MediaType,
        MediaDirection,
        SampleRate,
    };
    
    // Engine components
    pub use crate::engine::{
        MediaEngine,
        MediaEngineConfig,
        EngineCapabilities,
        MediaSessionParams,
        MediaSessionHandle,
        EngineState,
    };
    
    // Session components
    pub use crate::session::{
        MediaSession,
        MediaSessionConfig,
        MediaSessionState,
        MediaSessionEvent as SessionEvent, // Rename to avoid conflict
        MediaSessionEventType,
    };
    
    // Integration components
    pub use crate::integration::{
        RtpBridge,
        RtpBridgeConfig,
        SessionBridge,
        SessionBridgeConfig,
        IntegrationEvent,
        IntegrationEventType,
    };
    
    // Processing pipeline components
    pub use crate::processing::{
        ProcessingPipeline,
        ProcessingConfig,
        AudioProcessor,
        AudioProcessingConfig,
        VoiceActivityDetector,
        AutomaticGainControl,
        AcousticEchoCanceller,
        FormatConverter,
        ConversionParams,
    };
    
    // Quality monitoring components
    pub use crate::quality::{
        QualityMonitor,
        QualityMonitorConfig,
        QualityMetrics,
        SessionMetrics,
        OverallMetrics,
        QualityAdjustment,
        AdaptationEngine,
        AdaptationStrategy,
    };
    
    // Buffer components
    pub use crate::buffer::{
        JitterBuffer,
        JitterBufferConfig,
        JitterBufferStats,
        AdaptiveBuffer,
        AdaptiveConfig,
        FrameBuffer,
        FrameBufferConfig,
        RingBuffer,
        RingBufferError,
    };
    
    // Audio codec components
    pub use crate::codec::audio::{
        G711Codec,
        G711Config,
        G711Variant,
        OpusCodec,
        OpusConfig,
        OpusApplication,
    };
    
    // Payload type constants for convenience
    pub use crate::types::payload_types;
    
    // Legacy types (temporary)
    pub use crate::codec::{Codec, CodecRegistry};
} 