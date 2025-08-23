//! VP8 payload format handler
//!
//! This module implements payload format handling for VP8 video according to RFC 7741.

use bytes::{Bytes, BytesMut, Buf, BufMut};
use std::any::Any;
use super::traits::PayloadFormat;

/// VP8 payload header descriptor types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Vp8DescriptorType {
    /// Basic descriptor with minimal fields
    Basic,
    /// Extended descriptor with additional fields
    Extended,
}

/// VP8 payload format handler
///
/// This implements VP8 payload format according to RFC 7741.
/// VP8 is a video codec developed by Google and acquired through the purchase
/// of On2 Technologies.
pub struct Vp8PayloadFormat {
    /// Clock rate for RTP timestamps (90kHz default for video)
    clock_rate: u32,
    /// Payload type (dynamic, typically 96-127)
    payload_type: u8,
    /// Whether to use extended headers
    use_extended_header: bool,
    /// Picture ID present flag
    picture_id_present: bool,
    /// Maximum frame rate (used for packet size estimation)
    max_frame_rate: u32,
    /// Average bitrate for packet size estimation
    average_bitrate: u32,
}

impl Vp8PayloadFormat {
    /// Create a new VP8 payload format handler
    pub fn new(payload_type: u8) -> Self {
        Self {
            // Default clock rate for video is 90kHz
            clock_rate: 90000,
            payload_type,
            use_extended_header: true,
            picture_id_present: true,
            max_frame_rate: 30,
            average_bitrate: 1_000_000, // 1 Mbps default
        }
    }
    
    /// Set whether to use extended headers
    pub fn with_extended_header(mut self, use_extended: bool) -> Self {
        self.use_extended_header = use_extended;
        self
    }
    
    /// Set whether picture IDs are present
    pub fn with_picture_id(mut self, picture_id_present: bool) -> Self {
        self.picture_id_present = picture_id_present;
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
        let mut size = 1; // X byte
        
        if self.use_extended_header {
            size += 1; // I, L, T, K byte
            
            if self.picture_id_present {
                size += 2; // Up to 2 bytes for picture ID (extended)
            }
        }
        
        size
    }
    
    /// Parse a VP8 RTP payload descriptor from a buffer
    fn parse_descriptor(buffer: &[u8]) -> Option<(usize, bool)> {
        if buffer.is_empty() {
            return None;
        }
        
        let x = (buffer[0] & 0x80) != 0; // Extended control bit
        let start_of_frame = (buffer[0] & 0x10) != 0; // S bit (start of VP8 partition)
        
        let mut offset = 1;
        
        if x && buffer.len() > 1 {
            // Extended control bits present
            let i = (buffer[1] & 0x80) != 0; // PictureID present
            let l = (buffer[1] & 0x40) != 0; // TL0PICIDX present
            let t = (buffer[1] & 0x20) != 0; // TID present
            let k = (buffer[1] & 0x10) != 0; // KEYIDX present
            
            offset += 1;
            
            if i {
                // Picture ID present
                if buffer.len() <= offset {
                    return None;
                }
                
                if (buffer[offset] & 0x80) != 0 {
                    // Picture ID is 15 bits
                    offset += 2;
                } else {
                    // Picture ID is 7 bits
                    offset += 1;
                }
            }
            
            if l && buffer.len() > offset {
                offset += 1; // TL0PICIDX
            }
            
            if (t || k) && buffer.len() > offset {
                offset += 1; // TID and KEYIDX
            }
        }
        
        Some((offset, start_of_frame))
    }
}

impl PayloadFormat for Vp8PayloadFormat {
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
        
        // Create VP8 RTP payload descriptor
        let mut first_byte = 0u8;
        
        // This is a start of frame packet
        first_byte |= 0x10;  // S bit
        
        if self.use_extended_header {
            first_byte |= 0x80;  // X bit (extended control bits)
            buffer.put_u8(first_byte);
            
            let mut second_byte = 0u8;
            if self.picture_id_present {
                second_byte |= 0x80;  // I bit (PictureID present)
            }
            buffer.put_u8(second_byte);
            
            if self.picture_id_present {
                // Use 15-bit picture ID derived from timestamp
                // In a real implementation, this would be managed with a proper counter
                let picture_id = (timestamp % 32768) as u16;
                buffer.put_u8((picture_id >> 8) as u8 | 0x80); // High bit set for 15-bit ID
                buffer.put_u8(picture_id as u8);
            }
        } else {
            buffer.put_u8(first_byte);
        }
        
        // Add the VP8 payload
        buffer.extend_from_slice(media_data);
        
        buffer.freeze()
    }
    
    fn unpack(&self, payload: &[u8], _timestamp: u32) -> Bytes {
        if payload.is_empty() {
            return Bytes::new();
        }
        
        // Parse the VP8 payload descriptor
        if let Some((offset, _start_of_frame)) = Self::parse_descriptor(payload) {
            // Extract the actual VP8 payload data (skipping the descriptor)
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
    fn test_vp8_payload_format() {
        // Create a VP8 format with dynamic PT 96
        let format = Vp8PayloadFormat::new(96)
            .with_extended_header(true)
            .with_picture_id(true)
            .with_bitrate(2_000_000) // 2 Mbps
            .with_frame_rate(30);
        
        assert_eq!(format.payload_type(), 96);
        assert_eq!(format.clock_rate(), 90000);
        assert_eq!(format.channels(), 1);
        assert_eq!(format.preferred_packet_duration(), 33); // ~33ms for 30fps
        
        // Clock rate = 90kHz, so 33ms = 2970 samples
        assert_eq!(format.samples_from_duration(33), 2970);
        
        // Test packet size calculations
        // 33ms of VP8 at 2Mbps = ~8.25KB + descriptor size
        let expected_size = format.packet_size_from_duration(33);
        assert_eq!(expected_size, 8254);
        
        // Test packing
        let test_data = vec![0u8; 100];
        let packed = format.pack(&test_data, 1234);
        
        // Expected size with descriptor
        assert_eq!(packed.len(), 100 + format.descriptor_size());
        
        // Test unpacking
        let unpacked = format.unpack(&packed, 1234);
        assert_eq!(unpacked.len(), 100);
    }
    
    #[test]
    fn test_vp8_descriptor_parsing() {
        // Test basic descriptor
        let basic_desc = vec![0x10, 0x01, 0x02, 0x03];
        let (offset, is_start) = Vp8PayloadFormat::parse_descriptor(&basic_desc).unwrap();
        assert_eq!(offset, 1);
        assert_eq!(is_start, true);
        
        // Test extended descriptor with picture ID
        let ext_desc = vec![0x90, 0x80, 0x81, 0x23, 0x01, 0x02, 0x03];
        let (offset, is_start) = Vp8PayloadFormat::parse_descriptor(&ext_desc).unwrap();
        assert_eq!(offset, 4);
        assert_eq!(is_start, true);
    }
} 