use bytes::{Buf, BufMut, BytesMut};

use crate::error::Error;
use crate::{Result, RtpSsrc};

/// RTCP Goodbye (BYE) packet
/// Defined in RFC 3550 Section 6.6
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpGoodbye {
    /// SSRC/CSRC identifiers
    pub sources: Vec<RtpSsrc>,
    
    /// Reason for leaving (optional)
    pub reason: Option<String>,
}

impl RtcpGoodbye {
    /// Create a new BYE packet
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            reason: None,
        }
    }
    
    /// Create a new BYE packet for a single source
    pub fn new_for_source(ssrc: RtpSsrc) -> Self {
        let mut bye = Self::new();
        bye.add_source(ssrc);
        bye
    }
    
    /// Create a new BYE packet for a single source with a reason
    pub fn new_with_reason(ssrc: RtpSsrc, reason: String) -> Self {
        let mut bye = Self::new_for_source(ssrc);
        bye.reason = Some(reason);
        bye
    }
    
    /// Add a source to the BYE packet
    pub fn add_source(&mut self, ssrc: RtpSsrc) {
        self.sources.push(ssrc);
    }
    
    /// Set the reason for leaving
    pub fn set_reason(&mut self, reason: String) {
        self.reason = Some(reason);
    }
    
    /// Calculate the size of the BYE packet in bytes
    pub fn size(&self) -> usize {
        let mut size = self.sources.len() * 4; // Sources (4 bytes each)
        
        // Add reason text if present (length byte + text + padding to 4-byte boundary)
        if let Some(reason) = &self.reason {
            size += 1; // Length byte
            size += reason.len(); // Reason text
            // Padding to 4-byte boundary if needed
            let padding = (4 - ((1 + reason.len()) % 4)) % 4;
            size += padding;
        }
        
        size
    }
    
    /// Serialize the BYE packet to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        let mut buf = BytesMut::with_capacity(self.size());
        
        // Add SSRC/CSRC identifiers
        for ssrc in &self.sources {
            buf.put_u32(*ssrc);
        }
        
        // Add reason if present
        if let Some(reason) = &self.reason {
            // Length of the reason text (max 255 bytes)
            let reason_len = reason.len().min(255);
            buf.put_u8(reason_len as u8);
            
            // Add reason text
            buf.put_slice(&reason.as_bytes()[0..reason_len]);
            
            // Add padding to align to 4-byte boundary
            let padding_bytes = (4 - ((1 + reason_len) % 4)) % 4;
            for _ in 0..padding_bytes {
                buf.put_u8(0);
            }
        }
        
        Ok(buf)
    }
}

/// Parse BYE packet from bytes
pub fn parse_bye(buf: &mut impl Buf, source_count: u8) -> Result<RtcpGoodbye> {
    // Extract SSRC/CSRC identifiers
    let mut sources = Vec::with_capacity(source_count as usize);
    for _ in 0..source_count {
        if buf.remaining() < 4 {
            return Err(Error::BufferTooSmall {
                required: 4,
                available: buf.remaining(),
            });
        }
        sources.push(buf.get_u32());
    }
    
    // Extract reason if present
    let reason = if buf.has_remaining() {
        if buf.remaining() < 1 {
            return Err(Error::BufferTooSmall {
                required: 1,
                available: buf.remaining(),
            });
        }
        
        let reason_len = buf.get_u8() as usize;
        if buf.remaining() < reason_len {
            return Err(Error::BufferTooSmall {
                required: reason_len,
                available: buf.remaining(),
            });
        }
        
        let mut reason_bytes = vec![0u8; reason_len];
        buf.copy_to_slice(&mut reason_bytes);
        
        // Skip padding bytes
        let padding_bytes = (4 - ((1 + reason_len) % 4)) % 4;
        for _ in 0..padding_bytes {
            if buf.has_remaining() {
                buf.advance(1);
            }
        }
        
        Some(String::from_utf8_lossy(&reason_bytes).to_string())
    } else {
        None
    };
    
    Ok(RtcpGoodbye { sources, reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bye_creation() {
        let bye = RtcpGoodbye::new();
        assert!(bye.sources.is_empty());
        assert!(bye.reason.is_none());
        
        let bye = RtcpGoodbye::new_for_source(0x12345678);
        assert_eq!(bye.sources.len(), 1);
        assert_eq!(bye.sources[0], 0x12345678);
        assert!(bye.reason.is_none());
        
        let bye = RtcpGoodbye::new_with_reason(0x12345678, "Leaving session".to_string());
        assert_eq!(bye.sources.len(), 1);
        assert_eq!(bye.sources[0], 0x12345678);
        assert_eq!(bye.reason, Some("Leaving session".to_string()));
    }
    
    #[test]
    fn test_add_source() {
        let mut bye = RtcpGoodbye::new();
        bye.add_source(0x12345678);
        bye.add_source(0xabcdef01);
        
        assert_eq!(bye.sources.len(), 2);
        assert_eq!(bye.sources[0], 0x12345678);
        assert_eq!(bye.sources[1], 0xabcdef01);
    }
    
    #[test]
    fn test_set_reason() {
        let mut bye = RtcpGoodbye::new_for_source(0x12345678);
        bye.set_reason("Leaving session".to_string());
        
        assert_eq!(bye.reason, Some("Leaving session".to_string()));
    }
    
    #[test]
    fn test_size_calculation() {
        // BYE with no sources, no reason
        let bye = RtcpGoodbye::new();
        assert_eq!(bye.size(), 0);
        
        // BYE with one source, no reason
        let bye = RtcpGoodbye::new_for_source(0x12345678);
        assert_eq!(bye.size(), 4); // One SSRC (4 bytes)
        
        // BYE with two sources, no reason
        let mut bye = RtcpGoodbye::new();
        bye.add_source(0x12345678);
        bye.add_source(0xabcdef01);
        assert_eq!(bye.size(), 8); // Two SSRCs (8 bytes)
        
        // BYE with one source and reason (14 bytes)
        let bye = RtcpGoodbye::new_with_reason(0x12345678, "Bye".to_string());
        assert_eq!(bye.size(), 8); // SSRC (4) + length (1) + "Bye" (3) + padding (0)
        
        // BYE with one source and reason that needs padding
        let bye = RtcpGoodbye::new_with_reason(0x12345678, "Goodbye".to_string());
        // SSRC (4) + length (1) + "Goodbye" (7) + padding (0)
        assert_eq!(bye.size(), 12);
        
        // BYE with one source and reason that needs padding
        let bye = RtcpGoodbye::new_with_reason(0x12345678, "A".to_string());
        // SSRC (4) + length (1) + "A" (1) + padding (2)
        assert_eq!(bye.size(), 8);
    }
    
    #[test]
    fn test_serialize_parse() {
        // Test with one source, no reason
        let original = RtcpGoodbye::new_for_source(0x12345678);
        let serialized = original.serialize().unwrap();
        let parsed = parse_bye(&mut serialized.freeze(), 1).unwrap();
        
        assert_eq!(parsed.sources.len(), 1);
        assert_eq!(parsed.sources[0], 0x12345678);
        assert!(parsed.reason.is_none());
        
        // Test with two sources and a reason
        let mut original = RtcpGoodbye::new();
        original.add_source(0x12345678);
        original.add_source(0xabcdef01);
        original.set_reason("Leaving session".to_string());
        
        let serialized = original.serialize().unwrap();
        let parsed = parse_bye(&mut serialized.freeze(), 2).unwrap();
        
        assert_eq!(parsed.sources.len(), 2);
        assert_eq!(parsed.sources[0], 0x12345678);
        assert_eq!(parsed.sources[1], 0xabcdef01);
        assert_eq!(parsed.reason, Some("Leaving session".to_string()));
    }
} 