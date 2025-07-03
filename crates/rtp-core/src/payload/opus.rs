//! Opus payload format handler
//!
//! This module implements payload format handling for Opus audio.
//! Opus is a modern codec defined in RFC 7587 with dynamic payload type.
//! It supports multiple bandwidths, bitrates, and frame sizes.

use bytes::{Bytes, BytesMut};
use std::any::Any;
use super::traits::PayloadFormat;

/// Opus bandwidth modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpusBandwidth {
    /// Narrowband (4kHz)
    Narrowband,
    /// Mediumband (6kHz)
    Mediumband,
    /// Wideband (8kHz)
    Wideband,
    /// Super wideband (12kHz)
    SuperWideband,
    /// Fullband (20kHz)
    Fullband,
}

impl OpusBandwidth {
    /// Get the sampling rate for this bandwidth
    pub fn sampling_rate(&self) -> u32 {
        match self {
            Self::Narrowband => 8000,
            Self::Mediumband => 12000,
            Self::Wideband => 16000,
            Self::SuperWideband => 24000,
            Self::Fullband => 48000,
        }
    }
    
    /// Get a descriptive name for this bandwidth
    pub fn name(&self) -> &'static str {
        match self {
            Self::Narrowband => "Narrowband (4kHz)",
            Self::Mediumband => "Mediumband (6kHz)",
            Self::Wideband => "Wideband (8kHz)",
            Self::SuperWideband => "Super Wideband (12kHz)",
            Self::Fullband => "Fullband (20kHz)",
        }
    }
}

/// Opus payload format handler
///
/// This implements Opus payload format according to RFC 7587.
/// Opus is a versatile codec with support for multiple bandwidth modes,
/// and variable frame durations (2.5, 5, 10, 20, 40, 60, 80, 100, 120 ms).
pub struct OpusPayloadFormat {
    /// Clock rate for RTP timestamps (always 48000 Hz for Opus)
    clock_rate: u32,
    /// Number of channels (1 for mono, 2 for stereo)
    channels: u8,
    /// Preferred packet duration in milliseconds (usually 20ms)
    preferred_duration: u32,
    /// Payload type (dynamic, typically 96-127)
    payload_type: u8,
    /// Maximum bitrate in bits per second
    max_bitrate: u32,
    /// Bandwidth mode
    bandwidth: OpusBandwidth,
}

impl OpusPayloadFormat {
    /// Create a new Opus payload format handler
    pub fn new(payload_type: u8, channels: u8) -> Self {
        Self {
            clock_rate: 48000, // Opus always uses 48kHz for RTP timestamps
            channels,
            preferred_duration: 20, // 20ms is common for VoIP
            payload_type,
            max_bitrate: 64000, // 64 kbit/s by default
            bandwidth: OpusBandwidth::Fullband,
        }
    }
    
    /// Set the maximum bitrate
    pub fn with_max_bitrate(mut self, max_bitrate: u32) -> Self {
        self.max_bitrate = max_bitrate;
        self
    }
    
    /// Set the bandwidth mode
    pub fn with_bandwidth(mut self, bandwidth: OpusBandwidth) -> Self {
        self.bandwidth = bandwidth;
        self
    }
    
    /// Set the preferred frame duration
    pub fn with_duration(mut self, duration_ms: u32) -> Self {
        // Opus supports 2.5, 5, 10, 20, 40, 60, 80, 100, and 120 ms
        // Default to 20ms if an unsupported value is provided
        self.preferred_duration = match duration_ms {
            3 => 2, // Round to nearest (2.5ms)
            5 | 10 | 20 | 40 | 60 | 80 | 100 | 120 => duration_ms,
            _ => 20,
        };
        self
    }
    
    /// Get the maximum bitrate
    pub fn max_bitrate(&self) -> u32 {
        self.max_bitrate
    }
    
    /// Get the bandwidth mode
    pub fn bandwidth(&self) -> OpusBandwidth {
        self.bandwidth
    }
}

impl PayloadFormat for OpusPayloadFormat {
    fn payload_type(&self) -> u8 {
        self.payload_type
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
        // Opus is a variable bitrate codec, so this is just an estimate
        // Maximum bytes = bitrate * duration / 8 bits per byte
        let max_bytes = (self.max_bitrate * duration_ms) / (8 * 1000);
        max_bytes as usize
    }
    
    fn duration_from_packet_size(&self, packet_size: usize) -> u32 {
        // This is a rough estimate based on max bitrate
        // In practice, TOC byte in Opus packets needs to be examined
        // to determine the actual frame duration
        let bits = packet_size as u32 * 8;
        (bits * 1000) / self.max_bitrate
    }
    
    fn pack(&self, media_data: &[u8], _timestamp: u32) -> Bytes {
        // This would typically involve Opus encoding
        // For this implementation, we'll just forward the already-encoded data
        Bytes::copy_from_slice(media_data)
    }
    
    fn unpack(&self, payload: &[u8], _timestamp: u32) -> Bytes {
        // This would typically involve Opus decoding
        // For this implementation, we'll just forward the encoded data
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
    fn test_opus_payload_format() {
        // Create a mono Opus format with dynamic PT 101
        let format = OpusPayloadFormat::new(101, 1)
            .with_max_bitrate(32000) // 32 kbit/s
            .with_bandwidth(OpusBandwidth::Wideband)
            .with_duration(20); // 20ms
        
        assert_eq!(format.payload_type(), 101);
        assert_eq!(format.clock_rate(), 48000);
        assert_eq!(format.channels(), 1);
        assert_eq!(format.preferred_packet_duration(), 20);
        assert_eq!(format.max_bitrate(), 32000);
        assert_eq!(format.bandwidth(), OpusBandwidth::Wideband);
        
        // 20ms at 48kHz = 960 samples
        assert_eq!(format.samples_from_duration(20), 960);
        
        // 20ms of Opus at 32kbit/s = 80 bytes (max)
        assert_eq!(format.packet_size_from_duration(20), 80);
        
        // Test with stereo
        let stereo_format = OpusPayloadFormat::new(102, 2)
            .with_max_bitrate(64000); // 64 kbit/s
            
        assert_eq!(stereo_format.channels(), 2);
        assert_eq!(stereo_format.max_bitrate(), 64000);
        
        // 20ms of stereo Opus at 64kbit/s = 160 bytes (max)
        assert_eq!(stereo_format.packet_size_from_duration(20), 160);
    }
    
    #[test]
    fn test_opus_bandwidth_modes() {
        assert_eq!(OpusBandwidth::Narrowband.sampling_rate(), 8000);
        assert_eq!(OpusBandwidth::Mediumband.sampling_rate(), 12000);
        assert_eq!(OpusBandwidth::Wideband.sampling_rate(), 16000);
        assert_eq!(OpusBandwidth::SuperWideband.sampling_rate(), 24000);
        assert_eq!(OpusBandwidth::Fullband.sampling_rate(), 48000);
    }
} 