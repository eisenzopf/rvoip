//! Core types and constants for media-core
//!
//! This module defines the fundamental data structures, identifiers, and constants
//! used throughout the media processing library.

use std::fmt;
use std::time::{Duration, Instant};
use bytes::Bytes;

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
    
    /// G.711 μ-law (PCMU)
    pub const PCMU: PayloadType = 0;
    /// G.711 A-law (PCMA)
    pub const PCMA: PayloadType = 8;
    /// G.722 wideband
    pub const G722: PayloadType = 9;
    /// Telephone event (DTMF)
    pub const TELEPHONE_EVENT: PayloadType = 101;
    /// Opus (dynamic)
    pub const OPUS: PayloadType = 111;
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