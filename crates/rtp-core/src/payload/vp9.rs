//! VP9 payload format handler
//!
//! This module implements payload format handling for VP9 video according to RFC 8741.

use bytes::{Bytes, BytesMut, Buf, BufMut};
use std::any::Any;
use super::traits::PayloadFormat;

/// VP9 payload header descriptor flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Vp9DescriptorFlags {
    /// I (picture ID) present
    i: bool,
    /// P (inter-picture predicted frame) present
    p: bool,
    /// L (layer indices) present
    l: bool,
    /// F (flexible mode) present
    f: bool,
    /// B (start of a frame) present
    b: bool,
    /// E (end of a frame) present
    e: bool,
    /// V (scalability structure) present
    v: bool,
}

/// VP9 payload format handler
///
/// This implements VP9 payload format according to RFC 8741.
/// VP9 is a video codec developed by Google.
pub struct Vp9PayloadFormat {
    /// Clock rate for RTP timestamps (90kHz default for video)
    clock_rate: u32,
    /// Payload type (dynamic, typically 96-127)
    payload_type: u8,
    /// Descriptor flags
    descriptor_flags: Vp9DescriptorFlags,
    /// Maximum frame rate (used for packet size estimation)
    max_frame_rate: u32,
    /// Average bitrate for packet size estimation
    average_bitrate: u32,
}

impl Vp9PayloadFormat {
    /// Create a new VP9 payload format handler
    pub fn new(payload_type: u8) -> Self {
        Self {
            // Default clock rate for video is 90kHz
            clock_rate: 90000,
            payload_type,
            descriptor_flags: Vp9DescriptorFlags {
                i: true,  // Picture ID present
                p: false, // Inter-picture predicted frame flag
                l: true,  // Layer indices present
                f: false, // Flexible mode
                b: true,  // Start of frame
                e: true,  // End of frame
                v: false, // Scalability structure
            },
            max_frame_rate: 30,
            average_bitrate: 1_500_000, // 1.5 Mbps default
        }
    }
    
    /// Set the picture ID present flag
    pub fn with_picture_id(mut self, present: bool) -> Self {
        self.descriptor_flags.i = present;
        self
    }
    
    /// Set the layer indices present flag
    pub fn with_layer_indices(mut self, present: bool) -> Self {
        self.descriptor_flags.l = present;
        self
    }
    
    /// Set the flexible mode flag
    pub fn with_flexible_mode(mut self, flexible: bool) -> Self {
        self.descriptor_flags.f = flexible;
        self
    }
    
    /// Set the average bitrate for packet size estimation
    pub fn with_bitrate(mut self, bitrate: u32) -> Self {
        self.average_bitrate = bitrate;
        self
    }
    
    /// Set the maximum frame rate
    pub fn with_frame_rate(mut self, frame_rate: u32) -> Self {
        self.max_frame_rate = frame_rate;
        self
    }
    
    /// Calculate the descriptor size based on configuration
    pub fn descriptor_size(&self) -> usize {
        let mut size = 1; // First octet (mandatory)
        
        if self.descriptor_flags.i {
            size += 1; // Picture ID (7 bits)
            
            // For simplicity, we'll assume extended picture ID (15 bits)
            // is used for all picture IDs
            size += 1;
        }
        
        if self.descriptor_flags.l {
            size += 1; // L header
        }
        
        if self.descriptor_flags.v {
            // Scalability structure
            size += 1; // V header
            
            // For simplicity, we'll assume a minimal SS with
            // no additional bytes for pattern, layers, etc.
        }
        
        size
    }
    
    /// Parse a VP9 RTP payload descriptor from a buffer
    fn parse_descriptor(buffer: &[u8]) -> Option<(usize, bool, bool)> {
        if buffer.is_empty() {
            return None;
        }
        
        // Parse the first byte (required)
        let i = (buffer[0] & 0x80) != 0; // Picture ID present
        let p = (buffer[0] & 0x40) != 0; // Reference frame
        let l = (buffer[0] & 0x20) != 0; // Layer indices present
        let f = (buffer[0] & 0x10) != 0; // Flexible mode
        let b = (buffer[0] & 0x08) != 0; // Start of a frame
        let e = (buffer[0] & 0x04) != 0; // End of a frame
        let v = (buffer[0] & 0x02) != 0; // Scalability structure
        
        let mut offset = 1;
        
        // Picture ID (PID)
        if i && buffer.len() > offset {
            if (buffer[offset] & 0x80) != 0 {
                // 15-bit picture ID
                offset += 2;
            } else {
                // 7-bit picture ID
                offset += 1;
            }
        }
        
        // Layer indices (TID, SID)
        if l && buffer.len() > offset {
            offset += 1;
        }
        
        // Reference indices
        if p && buffer.len() > offset {
            // Skip reference indices
            let refs_p = buffer[offset] & 0x0F;
            offset += 1;
            
            // Skip additional bytes for reference indices
            offset += refs_p as usize;
        }
        
        // Scalability structure
        if v && buffer.len() > offset {
            // Parse SS header
            let pattern_count = (buffer[offset] >> 5) & 0x07;
            offset += 1;
            
            // Skip pattern values
            offset += pattern_count as usize;
            
            // Check for additional bytes
            if buffer.len() > offset {
                let layers = buffer[offset] & 0x3F;
                offset += 1;
                
                // Skip layer information
                offset += 3 * layers as usize;
            }
        }
        
        Some((offset, b, e))
    }
}

impl PayloadFormat for Vp9PayloadFormat {
    fn payload_type(&self) -> u8 {
        self.payload_type
    }
    
    fn clock_rate(&self) -> u32 {
        self.clock_rate
    }
    
    fn channels(&self) -> u8 {
        1 // Video has only one "channel"
    }
    
    fn preferred_packet_duration(&self) -> u32 {
        // For video, this is the frame duration in milliseconds
        let frame_duration = 1000 / self.max_frame_rate;
        frame_duration
    }
    
    fn packet_size_from_duration(&self, duration_ms: u32) -> usize {
        // Estimate based on average bitrate and duration
        // For video, packetization often happens based on MTU rather than duration
        // but we'll use a simple estimate here
        let descriptor_size = self.descriptor_size();
        let max_bytes = (self.average_bitrate * duration_ms) / (8 * 1000);
        
        // Add descriptor size to the payload size
        (max_bytes as usize) + descriptor_size
    }
    
    fn duration_from_packet_size(&self, packet_size: usize) -> u32 {
        // This is a rough estimate for video
        let descriptor_size = self.descriptor_size();
        let payload_size = packet_size.saturating_sub(descriptor_size);
        let bits = payload_size as u32 * 8;
        (bits * 1000) / self.average_bitrate
    }
    
    fn pack(&self, media_data: &[u8], timestamp: u32) -> Bytes {
        let descriptor_size = self.descriptor_size();
        let mut buffer = BytesMut::with_capacity(descriptor_size + media_data.len());
        
        // Create VP9 RTP payload descriptor (first byte)
        let mut first_byte = 0u8;
        
        if self.descriptor_flags.i {
            first_byte |= 0x80; // I bit
        }
        
        if self.descriptor_flags.p {
            first_byte |= 0x40; // P bit
        }
        
        if self.descriptor_flags.l {
            first_byte |= 0x20; // L bit
        }
        
        if self.descriptor_flags.f {
            first_byte |= 0x10; // F bit
        }
        
        if self.descriptor_flags.b {
            first_byte |= 0x08; // B bit
        }
        
        if self.descriptor_flags.e {
            first_byte |= 0x04; // E bit
        }
        
        if self.descriptor_flags.v {
            first_byte |= 0x02; // V bit
        }
        
        buffer.put_u8(first_byte);
        
        // Picture ID (if present)
        if self.descriptor_flags.i {
            // Use 15-bit picture ID derived from timestamp
            // In a real implementation, this would be managed with a proper counter
            let picture_id = (timestamp % 32768) as u16;
            buffer.put_u8((picture_id >> 8) as u8 | 0x80); // High bit set for 15-bit ID
            buffer.put_u8(picture_id as u8);
        }
        
        // Layer indices (if present)
        if self.descriptor_flags.l {
            // For simplicity, use zero indices
            buffer.put_u8(0);
        }
        
        // Scalability structure (if present)
        if self.descriptor_flags.v {
            // For simplicity, add a minimal SS
            buffer.put_u8(0);
        }
        
        // Add the VP9 payload
        buffer.extend_from_slice(media_data);
        
        buffer.freeze()
    }
    
    fn unpack(&self, payload: &[u8], _timestamp: u32) -> Bytes {
        if payload.is_empty() {
            return Bytes::new();
        }
        
        // Parse the VP9 payload descriptor
        if let Some((offset, _start_of_frame, _end_of_frame)) = Self::parse_descriptor(payload) {
            // Extract the actual VP9 payload data (skipping the descriptor)
            if offset < payload.len() {
                return Bytes::copy_from_slice(&payload[offset..]);
            }
        }
        
        // Fallback to returning the whole payload if parsing fails
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
    fn test_vp9_payload_format() {
        // Create a VP9 format with dynamic PT 97
        let format = Vp9PayloadFormat::new(97)
            .with_picture_id(true)
            .with_layer_indices(true)
            .with_flexible_mode(false)
            .with_bitrate(3_000_000) // 3 Mbps
            .with_frame_rate(60);
        
        assert_eq!(format.payload_type(), 97);
        assert_eq!(format.clock_rate(), 90000);
        assert_eq!(format.channels(), 1);
        assert_eq!(format.preferred_packet_duration(), 16); // ~16ms for 60fps
        
        // Clock rate = 90kHz, so 16ms = 1440 samples
        assert_eq!(format.samples_from_duration(16), 1440);
        
        // Test packet size calculations
        // 16ms of VP9 at 3Mbps = ~6KB + descriptor size
        let expected_size = format.packet_size_from_duration(16);
        assert_eq!(expected_size, 6004);
        
        // Test packing
        let test_data = vec![0u8; 100];
        let packed = format.pack(&test_data, 5678);
        
        // Expected size with descriptor
        assert_eq!(packed.len(), 100 + format.descriptor_size());
        
        // Test unpacking
        let unpacked = format.unpack(&packed, 5678);
        assert_eq!(unpacked.len(), 100);
    }
    
    #[test]
    fn test_vp9_descriptor_parsing() {
        // Test basic descriptor
        let basic_desc = vec![0x08, 0x01, 0x02, 0x03];
        let (offset, is_start, is_end) = Vp9PayloadFormat::parse_descriptor(&basic_desc).unwrap();
        assert_eq!(offset, 1);
        assert_eq!(is_start, true);
        assert_eq!(is_end, false);
        
        // Test descriptor with picture ID and layer indices
        let ext_desc = vec![0xA8, 0x81, 0x23, 0x00, 0x01, 0x02, 0x03];
        let (offset, is_start, is_end) = Vp9PayloadFormat::parse_descriptor(&ext_desc).unwrap();
        assert_eq!(offset, 4);
        assert_eq!(is_start, true);
        assert_eq!(is_end, false);
    }
} 