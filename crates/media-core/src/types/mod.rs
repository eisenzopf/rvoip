//! Core types and constants for media-core
//!
//! This module defines the fundamental data structures, identifiers, and constants
//! used throughout the media processing library.

use std::fmt;
use std::time::{Duration, Instant};
use bytes::Bytes;

// Include specialized type modules
pub mod conference;
pub mod stats;

// Re-export conference types
pub use conference::*;

// Re-export statistics types
pub use stats::{MediaStatistics, MediaProcessingStats, QualityMetrics};

/// Unique identifier for a SIP dialog (from session-core)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DialogId(String);

impl DialogId {
    /// Create a new dialog ID
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    /// Get the inner string
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DialogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a media session
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MediaSessionId(String);

impl MediaSessionId {
    /// Create a new media session ID
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    /// Create from dialog ID
    pub fn from_dialog(dialog_id: &DialogId) -> Self {
        Self(dialog_id.0.clone())
    }
    
    /// Get the inner string
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MediaSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// RTP payload type
pub type PayloadType = u8;

/// Standard payload type constants
pub mod payload_types {
    use super::PayloadType;
    
    /// Static payload types (RFC 3551)
    pub mod static_types {
        use super::PayloadType;
        
        /// G.711 μ-law (PCMU)
        pub const PCMU: PayloadType = 0;
        /// G.711 A-law (PCMA)
        pub const PCMA: PayloadType = 8;
        /// G.722 wideband (16kHz sampling, 8kHz RTP clock)
        pub const G722: PayloadType = 9;
        /// G.729 low-bitrate codec
        pub const G729: PayloadType = 18;
        /// Telephone event (DTMF)
        pub const TELEPHONE_EVENT: PayloadType = 101;
    }
    
    /// Dynamic payload type range (RFC 3551)
    pub mod dynamic_range {
        /// Dynamic payload type range start (RFC 3551)
        pub const DYNAMIC_START: u8 = 96;
        
        /// Dynamic payload type range end (RFC 3551)
        pub const DYNAMIC_END: u8 = 127;
        
        /// Check if a payload type is in the dynamic range
        pub fn is_dynamic(payload_type: u8) -> bool {
            payload_type >= DYNAMIC_START && payload_type <= DYNAMIC_END
        }
        
        /// Get all dynamic payload types
        pub fn all_dynamic() -> Vec<u8> {
            (DYNAMIC_START..=DYNAMIC_END).collect()
        }
    }
    
    // Re-export static types for convenience
    pub use static_types::*;
    
    /// NOTE: Dynamic codecs like Opus do NOT have fixed payload types!
    /// They are assigned during SDP negotiation and can be any value in the range 96-127.
    /// The actual payload type for Opus depends on what was negotiated in the SDP offer/answer.
    /// 
    /// Examples of dynamic payload types (these are NOT fixed):
    /// - Opus: Often 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114
    /// - G.726: Often 112, but varies
    /// - iLBC: Often 113, but varies
    /// - Speex: Often 114, but varies
    pub const DYNAMIC_PAYLOAD_TYPES_NOTE: &str = "Dynamic payload types are assigned during SDP negotiation";
}

/// Media packet containing RTP payload and metadata
#[derive(Debug, Clone)]
pub struct MediaPacket {
    /// RTP payload data
    pub payload: Bytes,
    /// Payload type
    pub payload_type: PayloadType,
    /// RTP timestamp
    pub timestamp: u32,
    /// RTP sequence number
    pub sequence_number: u16,
    /// RTP SSRC
    pub ssrc: u32,
    /// Reception time
    pub received_at: Instant,
}

/// Audio frame with PCM data and format information
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// PCM audio data (interleaved samples)
    pub samples: Vec<i16>,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u8,
    /// Frame duration
    pub duration: Duration,
    /// Timestamp
    pub timestamp: u32,
}

impl AudioFrame {
    /// Create a new audio frame
    pub fn new(samples: Vec<i16>, sample_rate: u32, channels: u8, timestamp: u32) -> Self {
        let sample_count = samples.len() / channels as usize;
        let duration = Duration::from_secs_f64(sample_count as f64 / sample_rate as f64);
        
        Self {
            samples,
            sample_rate,
            channels,
            duration,
            timestamp,
        }
    }
    
    /// Get the number of samples per channel
    pub fn samples_per_channel(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
    
    /// Check if frame is mono
    pub fn is_mono(&self) -> bool {
        self.channels == 1
    }
    
    /// Check if frame is stereo
    pub fn is_stereo(&self) -> bool {
        self.channels == 2
    }
}

/// Video frame (placeholder for future implementation)
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// Encoded video data
    pub data: Bytes,
    /// Frame width
    pub width: u32,
    /// Frame height
    pub height: u32,
    /// Timestamp
    pub timestamp: u32,
    /// Is keyframe
    pub is_keyframe: bool,
}

/// Media type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Audio media
    Audio,
    /// Video media (future)
    Video,
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaType::Audio => write!(f, "audio"),
            MediaType::Video => write!(f, "video"),
        }
    }
}

/// Media direction for a session
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDirection {
    /// Send only
    SendOnly,
    /// Receive only
    RecvOnly,
    /// Send and receive
    SendRecv,
    /// Inactive
    Inactive,
}

impl fmt::Display for MediaDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaDirection::SendOnly => write!(f, "sendonly"),
            MediaDirection::RecvOnly => write!(f, "recvonly"),
            MediaDirection::SendRecv => write!(f, "sendrecv"),
            MediaDirection::Inactive => write!(f, "inactive"),
        }
    }
}

/// Common sample rates for audio processing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRate {
    /// 8 kHz (narrowband)
    Rate8000 = 8000,
    /// 16 kHz (wideband)
    Rate16000 = 16000,
    /// 32 kHz (super-wideband)
    Rate32000 = 32000,
    /// 48 kHz (fullband)
    Rate48000 = 48000,
}

impl SampleRate {
    /// Get the sample rate as Hz
    pub fn as_hz(&self) -> u32 {
        *self as u32
    }
    
    /// Create from Hz value
    pub fn from_hz(hz: u32) -> Option<Self> {
        match hz {
            8000 => Some(Self::Rate8000),
            16000 => Some(Self::Rate16000),
            32000 => Some(Self::Rate32000),
            48000 => Some(Self::Rate48000),
            _ => None,
        }
    }
}

impl Default for SampleRate {
    fn default() -> Self {
        Self::Rate8000 // Default to 8kHz (standard telephony)
    }
} 