//! Media frame definitions
//!
//! This module defines the common media frame types used by both client and server APIs.

/// Media frame types that can be transported
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFrameType {
    /// Audio frame
    Audio,
    /// Video frame
    Video,
    /// Data channel frame
    Data,
}

/// A media frame containing encoded media data
#[derive(Debug, Clone)]
pub struct MediaFrame {
    /// The type of media frame
    pub frame_type: MediaFrameType,
    /// The payload data
    pub data: Vec<u8>,
    /// Timestamp in media clock units
    pub timestamp: u32,
    /// Sequence identifier for ordering
    pub sequence: u16,
    /// Marker bit (e.g., end of frame for video)
    pub marker: bool,
    /// Payload type identifier
    pub payload_type: u8,
    /// Synchronization source identifier
    pub ssrc: u32,
} 