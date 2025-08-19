//! G.711 payload format handler (moved from rtp-core)
//!
//! This module implements payload format handling for G.711 µ-law and A-law audio.
//! G.711 is defined in RFC 3551 and is a commonly used codec for VoIP.

use bytes::{Bytes, BytesMut};
use std::any::Any;
use super::traits::PayloadFormat;

/// G.711 µ-law payload format handler
///
/// This implements payload format 0 from RFC 3551
pub struct G711UPayloadFormat {
    /// Clock rate (usually 8000 Hz)
    clock_rate: u32,
    /// Number of channels (usually 1)
    channels: u8,
    /// Preferred packet duration in milliseconds (usually 20ms)
    preferred_duration: u32,
}

impl G711UPayloadFormat {
    /// Create a new G.711 µ-law payload format handler
    pub fn new(clock_rate: u32) -> Self {
        Self {
            clock_rate,
            channels: 1,
            preferred_duration: 20, // 20ms is common for VoIP
        }
    }
}

impl PayloadFormat for G711UPayloadFormat {
    fn payload_type(&self) -> u8 {
        0 // PCMU is payload type 0
    }
    
    fn clock_rate(&self) -> u32 {
        self.clock_rate
    }
    
    fn channels(&self) -> u8 {
        self.channels
    }
    
    fn preferred_packet_duration(&self) -> u32 {
        self.preferred_duration
    }
    
    fn packet_size_from_duration(&self, duration_ms: u32) -> usize {
        // G.711 uses 8 bits per sample
        // So the calculation is: samples × channels × bytes_per_sample
        let samples = self.samples_from_duration(duration_ms);
        samples as usize * self.channels as usize
    }
    
    fn duration_from_packet_size(&self, packet_size: usize) -> u32 {
        // Calculate how many samples are in the packet
        let samples = packet_size / self.channels as usize;
        self.duration_from_samples(samples as u32)
    }
    
    fn pack(&self, media_data: &[u8], _timestamp: u32) -> Bytes {
        // G.711 data is already in the correct format (1 byte per sample)
        // so we just need to copy it
        Bytes::copy_from_slice(media_data)
    }
    
    fn unpack(&self, payload: &[u8], _timestamp: u32) -> Bytes {
        // G.711 data is already in the correct format (1 byte per sample)
        // so we just need to copy it
        Bytes::copy_from_slice(payload)
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// G.711 A-law payload format handler
///
/// This implements payload format 8 from RFC 3551
pub struct G711APayloadFormat {
    /// Clock rate (usually 8000 Hz)
    clock_rate: u32,
    /// Number of channels (usually 1)
    channels: u8,
    /// Preferred packet duration in milliseconds (usually 20ms)
    preferred_duration: u32,
}

impl G711APayloadFormat {
    /// Create a new G.711 A-law payload format handler
    pub fn new(clock_rate: u32) -> Self {
        Self {
            clock_rate,
            channels: 1,
            preferred_duration: 20, // 20ms is common for VoIP
        }
    }
}

impl PayloadFormat for G711APayloadFormat {
    fn payload_type(&self) -> u8 {
        8 // PCMA is payload type 8
    }
    
    fn clock_rate(&self) -> u32 {
        self.clock_rate
    }
    
    fn channels(&self) -> u8 {
        self.channels
    }
    
    fn preferred_packet_duration(&self) -> u32 {
        self.preferred_duration
    }
    
    fn packet_size_from_duration(&self, duration_ms: u32) -> usize {
        // G.711 uses 8 bits per sample
        // So the calculation is: samples × channels × bytes_per_sample
        let samples = self.samples_from_duration(duration_ms);
        samples as usize * self.channels as usize
    }
    
    fn duration_from_packet_size(&self, packet_size: usize) -> u32 {
        // Calculate how many samples are in the packet
        let samples = packet_size / self.channels as usize;
        self.duration_from_samples(samples as u32)
    }
    
    fn pack(&self, media_data: &[u8], _timestamp: u32) -> Bytes {
        // G.711 data is already in the correct format (1 byte per sample)
        // so we just need to copy it
        Bytes::copy_from_slice(media_data)
    }
    
    fn unpack(&self, payload: &[u8], _timestamp: u32) -> Bytes {
        // G.711 data is already in the correct format (1 byte per sample)
        // so we just need to copy it
        Bytes::copy_from_slice(payload)
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_g711u_payload_format() {
        let format = G711UPayloadFormat::new(8000);
        
        assert_eq!(format.payload_type(), 0);
        assert_eq!(format.clock_rate(), 8000);
        assert_eq!(format.channels(), 1);
        assert_eq!(format.preferred_packet_duration(), 20);
        
        // 20ms at 8kHz = 160 samples
        assert_eq!(format.samples_from_duration(20), 160);
        // 160 samples at 8kHz = 20ms
        assert_eq!(format.duration_from_samples(160), 20);
        
        // 20ms of G.711 = 160 bytes
        assert_eq!(format.packet_size_from_duration(20), 160);
        // 160 bytes of G.711 = 20ms
        assert_eq!(format.duration_from_packet_size(160), 20);
    }
    
    #[test]
    fn test_g711a_payload_format() {
        let format = G711APayloadFormat::new(8000);
        
        assert_eq!(format.payload_type(), 8);
        assert_eq!(format.clock_rate(), 8000);
        assert_eq!(format.channels(), 1);
        assert_eq!(format.preferred_packet_duration(), 20);
        
        // 20ms at 8kHz = 160 samples
        assert_eq!(format.samples_from_duration(20), 160);
        // 160 samples at 8kHz = 20ms
        assert_eq!(format.duration_from_samples(160), 20);
        
        // 20ms of G.711 = 160 bytes
        assert_eq!(format.packet_size_from_duration(20), 160);
        // 160 bytes of G.711 = 20ms
        assert_eq!(format.duration_from_packet_size(160), 20);
    }
}