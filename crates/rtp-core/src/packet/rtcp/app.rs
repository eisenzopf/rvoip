use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::str;

use crate::error::Error;
use crate::{Result, RtpSsrc};

/// RTCP Application-Defined (APP) packet
/// Defined in RFC 3550 Section 6.7
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpApplicationDefined {
    /// SSRC/CSRC identifier
    pub ssrc: RtpSsrc,
    
    /// Name (4 ASCII characters)
    pub name: [u8; 4],
    
    /// Application-dependent data
    pub data: Bytes,
}

impl RtcpApplicationDefined {
    /// Create a new APP packet
    pub fn new(ssrc: RtpSsrc, name: [u8; 4]) -> Self {
        Self {
            ssrc,
            name,
            data: Bytes::new(),
        }
    }
    
    /// Create a new APP packet with a string name (must be exactly 4 characters)
    pub fn new_with_name(ssrc: RtpSsrc, name_str: &str) -> Result<Self> {
        if name_str.len() != 4 {
            return Err(Error::InvalidParameter(
                format!("APP name must be exactly 4 characters, got {}", name_str.len())
            ));
        }
        
        let mut name = [0; 4];
        name.copy_from_slice(name_str.as_bytes());
        
        Ok(Self::new(ssrc, name))
    }
    
    /// Create a new APP packet with data
    pub fn new_with_data(ssrc: RtpSsrc, name: [u8; 4], data: Bytes) -> Self {
        Self { ssrc, name, data }
    }
    
    /// Get the name as a string (if valid ASCII)
    pub fn name_str(&self) -> String {
        String::from_utf8_lossy(&self.name).to_string()
    }
    
    /// Set the application data
    pub fn set_data(&mut self, data: Bytes) {
        self.data = data;
    }
    
    /// Calculate the total size in bytes
    pub fn size(&self) -> usize {
        let mut size = 8; // SSRC (4) + name (4)
        
        // Add data with padding to 4-byte boundary
        if !self.data.is_empty() {
            size += self.data.len();
            // Add padding to align to 4-byte boundary
            let padding = (4 - (self.data.len() % 4)) % 4;
            size += padding;
        }
        
        size
    }
    
    /// Serialize the APP packet to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        let mut buf = BytesMut::with_capacity(self.size());
        
        // SSRC
        buf.put_u32(self.ssrc);
        
        // Name (4 bytes)
        buf.put_slice(&self.name);
        
        // Application-dependent data
        if !self.data.is_empty() {
            buf.put_slice(&self.data);
            
            // Add padding to align to 4-byte boundary
            let padding_bytes = (4 - (self.data.len() % 4)) % 4;
            for _ in 0..padding_bytes {
                buf.put_u8(0);
            }
        }
        
        Ok(buf)
    }
}

/// Parse APP packet from bytes
pub fn parse_app(buf: &mut impl Buf) -> Result<RtcpApplicationDefined> {
    // Check if we have enough data for SSRC and name (8 bytes)
    if buf.remaining() < 8 {
        return Err(Error::BufferTooSmall {
            required: 8,
            available: buf.remaining(),
        });
    }
    
    // Extract SSRC
    let ssrc = buf.get_u32();
    
    // Extract name
    let mut name = [0u8; 4];
    buf.copy_to_slice(&mut name);
    
    // Extract application-dependent data
    let data = if buf.has_remaining() {
        let data_bytes = buf.copy_to_bytes(buf.remaining());
        data_bytes
    } else {
        Bytes::new()
    };
    
    Ok(RtcpApplicationDefined {
        ssrc,
        name,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_app_creation() {
        let app = RtcpApplicationDefined::new(0x12345678, *b"TEST");
        assert_eq!(app.ssrc, 0x12345678);
        assert_eq!(app.name, *b"TEST");
        assert!(app.data.is_empty());
        
        let app = RtcpApplicationDefined::new_with_name(0x12345678, "TEST").unwrap();
        assert_eq!(app.ssrc, 0x12345678);
        assert_eq!(app.name, *b"TEST");
        assert!(app.data.is_empty());
        
        // Test with invalid name length
        let result = RtcpApplicationDefined::new_with_name(0x12345678, "TOOLONG");
        assert!(result.is_err());
        let result = RtcpApplicationDefined::new_with_name(0x12345678, "ABC");
        assert!(result.is_err());
    }
    
    #[test]
    fn test_app_with_data() {
        let data = Bytes::from_static(b"application data");
        let app = RtcpApplicationDefined::new_with_data(0x12345678, *b"TEST", data.clone());
        
        assert_eq!(app.ssrc, 0x12345678);
        assert_eq!(app.name, *b"TEST");
        assert_eq!(app.data, data);
    }
    
    #[test]
    fn test_name_string() {
        let app = RtcpApplicationDefined::new(0x12345678, *b"TEST");
        assert_eq!(app.name_str(), "TEST");
    }
    
    #[test]
    fn test_set_data() {
        let mut app = RtcpApplicationDefined::new(0x12345678, *b"TEST");
        let data = Bytes::from_static(b"application data");
        app.set_data(data.clone());
        
        assert_eq!(app.data, data);
    }
    
    #[test]
    fn test_size_calculation() {
        // APP with no data
        let app = RtcpApplicationDefined::new(0x12345678, *b"TEST");
        assert_eq!(app.size(), 8); // SSRC (4) + name (4)
        
        // APP with data
        let data = Bytes::from_static(b"test");
        let app = RtcpApplicationDefined::new_with_data(0x12345678, *b"TEST", data);
        assert_eq!(app.size(), 12); // SSRC (4) + name (4) + data (4)
        
        // APP with data needing padding
        let data = Bytes::from_static(b"test data");
        let app = RtcpApplicationDefined::new_with_data(0x12345678, *b"TEST", data);
        assert_eq!(app.size(), 20); // SSRC (4) + name (4) + data (9) + padding (3)
    }
    
    #[test]
    fn test_serialize_parse() {
        // Test APP with no data
        let original = RtcpApplicationDefined::new(0x12345678, *b"TEST");
        let serialized = original.serialize().unwrap();
        let parsed = parse_app(&mut serialized.freeze()).unwrap();
        
        assert_eq!(parsed.ssrc, original.ssrc);
        assert_eq!(parsed.name, original.name);
        assert!(parsed.data.is_empty());
        
        // Test APP with data
        let data = Bytes::from_static(b"test data that needs padding");
        let original = RtcpApplicationDefined::new_with_data(0x12345678, *b"TEST", data);
        let serialized = original.serialize().unwrap();
        let parsed = parse_app(&mut serialized.freeze()).unwrap();
        
        assert_eq!(parsed.ssrc, original.ssrc);
        assert_eq!(parsed.name, original.name);
        // Note: The parsed data may include padding bytes, so we check it starts with our data
        assert!(parsed.data.starts_with(b"test data that needs padding"));
    }
} 