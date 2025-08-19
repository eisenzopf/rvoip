//! Opus payload format handler (moved from rtp-core)

use bytes::Bytes;
use std::any::Any;
use super::traits::PayloadFormat;

#[derive(Debug, Clone, Copy)]
pub enum OpusBandwidth {
    Narrowband,   // 8 kHz
    Mediumband,   // 12 kHz
    Wideband,     // 16 kHz
    SuperWideband, // 24 kHz
    Fullband,     // 48 kHz
}

/// Opus payload format handler
pub struct OpusPayloadFormat {
    payload_type: u8,
    channels: u8,
}

impl OpusPayloadFormat {
    pub fn new(payload_type: u8, channels: u8) -> Self {
        Self { payload_type, channels }
    }
}

impl PayloadFormat for OpusPayloadFormat {
    fn payload_type(&self) -> u8 { self.payload_type }
    fn clock_rate(&self) -> u32 { 48000 } // Opus always uses 48kHz
    fn channels(&self) -> u8 { self.channels }
    fn preferred_packet_duration(&self) -> u32 { 20 }
    fn packet_size_from_duration(&self, duration_ms: u32) -> usize {
        // Opus is variable bitrate, this is an estimate
        match duration_ms {
            10 => 80,   // ~64 kbps
            20 => 160,  // ~64 kbps  
            40 => 320,  // ~64 kbps
            60 => 480,  // ~64 kbps
            _ => (duration_ms * 8) as usize, // Fallback estimate
        }
    }
    fn duration_from_packet_size(&self, _packet_size: usize) -> u32 {
        20 // Opus packets are typically 20ms
    }
    fn pack(&self, media_data: &[u8], _timestamp: u32) -> Bytes {
        Bytes::copy_from_slice(media_data) // Simplified - real implementation would encode
    }
    fn unpack(&self, payload: &[u8], _timestamp: u32) -> Bytes {
        Bytes::copy_from_slice(payload) // Simplified - real implementation would decode
    }
    fn as_any(&self) -> &dyn Any { self }
}