//! VP8 video payload format handler (moved from rtp-core)

use bytes::Bytes;
use std::any::Any;
use super::traits::PayloadFormat;

/// VP8 payload format handler
pub struct Vp8PayloadFormat {
    payload_type: u8,
}

impl Vp8PayloadFormat {
    pub fn new(payload_type: u8) -> Self {
        Self { payload_type }
    }
}

impl PayloadFormat for Vp8PayloadFormat {
    fn payload_type(&self) -> u8 { self.payload_type }
    fn clock_rate(&self) -> u32 { 90000 } // Video clock rate
    fn channels(&self) -> u8 { 1 }
    fn preferred_packet_duration(&self) -> u32 { 33 } // ~30 FPS
    fn packet_size_from_duration(&self, _duration_ms: u32) -> usize {
        1400 // Typical MTU-safe packet size for video
    }
    fn duration_from_packet_size(&self, _packet_size: usize) -> u32 {
        33 // ~30 FPS
    }
    fn pack(&self, media_data: &[u8], _timestamp: u32) -> Bytes {
        Bytes::copy_from_slice(media_data) // Simplified
    }
    fn unpack(&self, payload: &[u8], _timestamp: u32) -> Bytes {
        Bytes::copy_from_slice(payload) // Simplified
    }
    fn as_any(&self) -> &dyn Any { self }
}