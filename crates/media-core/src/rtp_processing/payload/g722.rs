//! G.722 payload format handler (moved from rtp-core)
//!
//! This module implements payload format handling for G.722 audio.
//! G.722 is defined in RFC 3551 with payload type 9.
//! It is a wideband codec operating at 64, 56 or 48 kbit/s.

use bytes::{Bytes, BytesMut};
use std::any::Any;
use super::traits::PayloadFormat;

/// G.722 payload format handler
///
/// This implements payload format 9 from RFC 3551.
/// Note that G.722 has a special timestamp handling quirk:
/// While the codec operates at 16kHz, the RTP timestamp increases
/// at 8kHz rate for backward compatibility.
pub struct G722PayloadFormat {
    /// Clock rate for RTP timestamps (always 8000 Hz for G.722)
    clock_rate: u32,
    /// Actual sampling rate (16000 Hz for G.722)
    sampling_rate: u32,
    /// Number of channels (usually 1)
    channels: u8,
    /// Preferred packet duration in milliseconds (usually 20ms)
    preferred_duration: u32,
}

impl G722PayloadFormat {
    /// Create a new G.722 payload format handler
    pub fn new(clock_rate: u32) -> Self {
        Self {
            clock_rate,
            sampling_rate: 16000, // G.722 always operates at 16kHz
            channels: 1,
            preferred_duration: 20, // 20ms is common for VoIP
        }
    }
    
    /// Get the actual sampling rate (different from clock rate for G.722)
    pub fn sampling_rate(&self) -> u32 {
        self.sampling_rate
    }
    
    /// Calculate actual samples from duration based on sampling rate
    pub fn actual_samples_from_duration(&self, duration_ms: u32) -> u32 {
        (self.sampling_rate * duration_ms) / 1000
    }
}

impl PayloadFormat for G722PayloadFormat {
    fn payload_type(&self) -> u8 {
        9 // G.722 is payload type 9
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
        // G.722 encodes 16kHz samples at 4 bits per sample (0.5 bytes)
        // So the calculation is: (samples at 16kHz × 4 bits) / 8 bits per byte
        let samples_16k = self.actual_samples_from_duration(duration_ms);
        ((samples_16k * 4) / 8) as usize * self.channels as usize
    }
    
    fn duration_from_packet_size(&self, packet_size: usize) -> u32 {
        // Calculate how many 16kHz samples are in the packet
        // Each byte contains 2 samples (4 bits per sample)
        let samples_16k = (packet_size * 2) as u32 / self.channels as u32;
        (samples_16k * 1000) / self.sampling_rate
    }
    
    fn pack(&self, media_data: &[u8], _timestamp: u32) -> Bytes {
        // G.722 data is already encoded, so we just need to copy it
        // In a real implementation, we might need to convert from raw PCM to G.722
        Bytes::copy_from_slice(media_data)
    }
    
    fn unpack(&self, payload: &[u8], _timestamp: u32) -> Bytes {
        // G.722 data needs to be decoded to get PCM
        // In a real implementation, we would run a G.722 decoder here
        // For now, we just return the encoded data
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
    fn test_g722_payload_format() {
        let format = G722PayloadFormat::new(8000);
        
        assert_eq!(format.payload_type(), 9);
        assert_eq!(format.clock_rate(), 8000);
        assert_eq!(format.sampling_rate(), 16000);
        assert_eq!(format.channels(), 1);
        assert_eq!(format.preferred_packet_duration(), 20);
        
        // 20ms at 8kHz = 160 samples (for RTP timestamp)
        assert_eq!(format.samples_from_duration(20), 160);
        
        // 20ms at 16kHz = 320 samples (actual audio samples)
        assert_eq!(format.actual_samples_from_duration(20), 320);
        
        // 20ms of G.722 = 320 samples at 16kHz, encoded at 4 bits per sample = 160 bytes
        assert_eq!(format.packet_size_from_duration(20), 160);
        
        // 160 bytes of G.722 = 320 samples at 16kHz = 20ms
        assert_eq!(format.duration_from_packet_size(160), 20);
    }
}