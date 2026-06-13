//! Media frame definitions
//!
//! This module defines the common media frame types used by both client and server APIs.

use bytes::Bytes;

use crate::RtpCsrc;

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
    pub data: Bytes,
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
    /// Contributing source identifiers
    pub csrcs: Vec<RtpCsrc>,
}

impl MediaFrame {
    /// Create a new media frame from any payload that can become
    /// refcounted `Bytes`.
    pub fn new(
        frame_type: MediaFrameType,
        data: impl Into<Bytes>,
        timestamp: u32,
        sequence: u16,
        marker: bool,
        payload_type: u8,
        ssrc: u32,
    ) -> Self {
        Self {
            frame_type,
            data: data.into(),
            timestamp,
            sequence,
            marker,
            payload_type,
            ssrc,
            csrcs: Vec::new(),
        }
    }

    /// Create a new media frame with explicit CSRCs.
    pub fn with_csrcs(
        frame_type: MediaFrameType,
        data: impl Into<Bytes>,
        timestamp: u32,
        sequence: u16,
        marker: bool,
        payload_type: u8,
        ssrc: u32,
        csrcs: Vec<RtpCsrc>,
    ) -> Self {
        Self {
            frame_type,
            data: data.into(),
            timestamp,
            sequence,
            marker,
            payload_type,
            ssrc,
            csrcs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_preserves_bytes_payload_allocation() {
        let payload = Bytes::from_static(b"payload");
        let ptr = payload.as_ptr();

        let frame = MediaFrame::new(MediaFrameType::Audio, payload.clone(), 10, 20, false, 0, 30);

        assert_eq!(frame.data.as_ptr(), ptr);
        assert_eq!(frame.data, payload);
    }
}
