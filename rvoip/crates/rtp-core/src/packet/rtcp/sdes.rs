use bytes::{Buf, Bytes, BufMut, BytesMut};
use crate::error::Error;
use crate::{Result, RtpSsrc};

/// RTCP Source Description (SDES) item types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RtcpSdesItemType {
    /// End of SDES item list
    End = 0,
    
    /// Canonical name (CNAME)
    CName = 1,
    
    /// User name (NAME)
    Name = 2,
    
    /// E-mail address (EMAIL)
    Email = 3,
    
    /// Phone number (PHONE)
    Phone = 4,
    
    /// Geographic location (LOC)
    Location = 5,
    
    /// Application or tool name (TOOL)
    Tool = 6,
    
    /// Notice/status (NOTE)
    Note = 7,
    
    /// Private extensions (PRIV)
    Private = 8,
}

impl TryFrom<u8> for RtcpSdesItemType {
    type Error = Error;
    
    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(RtcpSdesItemType::End),
            1 => Ok(RtcpSdesItemType::CName),
            2 => Ok(RtcpSdesItemType::Name),
            3 => Ok(RtcpSdesItemType::Email),
            4 => Ok(RtcpSdesItemType::Phone),
            5 => Ok(RtcpSdesItemType::Location),
            6 => Ok(RtcpSdesItemType::Tool),
            7 => Ok(RtcpSdesItemType::Note),
            8 => Ok(RtcpSdesItemType::Private),
            _ => Err(Error::RtcpError(format!("Unknown SDES item type: {}", value))),
        }
    }
}

/// RTCP Source Description (SDES) item
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSdesItem {
    /// Item type
    pub item_type: RtcpSdesItemType,
    
    /// Item value
    pub value: String,
}

impl RtcpSdesItem {
    /// Create a new SDES item
    pub fn new(item_type: RtcpSdesItemType, value: String) -> Self {
        Self { item_type, value }
    }
    
    /// Create a new CNAME item
    pub fn cname(value: String) -> Self {
        Self::new(RtcpSdesItemType::CName, value)
    }
    
    /// Create a new NAME item
    pub fn name(value: String) -> Self {
        Self::new(RtcpSdesItemType::Name, value)
    }
    
    /// Create a new TOOL item
    pub fn tool(value: String) -> Self {
        Self::new(RtcpSdesItemType::Tool, value)
    }
}

/// RTCP Source Description (SDES) chunk
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSdesChunk {
    /// SSRC/CSRC identifier
    pub ssrc: RtpSsrc,
    
    /// SDES items
    pub items: Vec<RtcpSdesItem>,
}

impl RtcpSdesChunk {
    /// Create a new SDES chunk
    pub fn new(ssrc: RtpSsrc) -> Self {
        Self {
            ssrc,
            items: Vec::new(),
        }
    }
    
    /// Add an SDES item
    pub fn add_item(&mut self, item: RtcpSdesItem) {
        self.items.push(item);
    }
}

/// RTCP Source Description (SDES) packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSourceDescription {
    /// SDES chunks
    pub chunks: Vec<RtcpSdesChunk>,
}

impl RtcpSourceDescription {
    /// Create a new SDES packet
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
        }
    }
    
    /// Add an SDES chunk
    pub fn add_chunk(&mut self, chunk: RtcpSdesChunk) {
        self.chunks.push(chunk);
    }
    
    /// Add a source with optional CNAME
    pub fn add_source(&mut self, ssrc: RtpSsrc, cname: Option<String>) {
        let mut chunk = RtcpSdesChunk::new(ssrc);
        if let Some(cname_value) = cname {
            chunk.add_item(RtcpSdesItem::cname(cname_value));
        }
        self.add_chunk(chunk);
    }
    
    /// Find a chunk by SSRC
    pub fn find_chunk(&self, ssrc: RtpSsrc) -> Option<&RtcpSdesChunk> {
        self.chunks.iter().find(|chunk| chunk.ssrc == ssrc)
    }
    
    /// Find a CNAME for a source
    pub fn find_cname(&self, ssrc: RtpSsrc) -> Option<&str> {
        if let Some(chunk) = self.find_chunk(ssrc) {
            for item in &chunk.items {
                if item.item_type == RtcpSdesItemType::CName {
                    return Some(&item.value);
                }
            }
        }
        None
    }
    
    /// Serialize the SDES packet to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        // Calculate total size
        let mut total_size = 0;
        
        // Calculate size for each chunk
        for chunk in &self.chunks {
            // SSRC (4 bytes)
            total_size += 4;
            
            // Calculate size for each item
            for item in &chunk.items {
                // Type (1 byte) + Length (1 byte) + Value
                total_size += 2 + item.value.len();
            }
            
            // END item (1 byte) + padding to 32-bit boundary
            total_size += 1;
            if total_size % 4 != 0 {
                total_size += 4 - (total_size % 4);
            }
        }
        
        let mut buf = BytesMut::with_capacity(total_size);
        
        // Serialize each chunk
        for chunk in &self.chunks {
            // SSRC
            buf.put_u32(chunk.ssrc);
            
            // Serialize items
            for item in &chunk.items {
                // Item type
                buf.put_u8(item.item_type as u8);
                
                // Item length
                buf.put_u8(item.value.len() as u8);
                
                // Item value
                buf.put_slice(item.value.as_bytes());
            }
            
            // End marker
            buf.put_u8(RtcpSdesItemType::End as u8);
            
            // Pad to 32-bit boundary if needed
            let padding_bytes = (4 - (buf.len() % 4)) % 4;
            for _ in 0..padding_bytes {
                buf.put_u8(0);
            }
        }
        
        Ok(buf)
    }
}

/// Serialize an SDES packet
pub fn serialize_sdes(sdes: &RtcpSourceDescription) -> Result<BytesMut> {
    sdes.serialize()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sdes_item_creation() {
        let item = RtcpSdesItem::cname("user@example.com".to_string());
        assert_eq!(item.item_type, RtcpSdesItemType::CName);
        assert_eq!(item.value, "user@example.com");
        
        let item = RtcpSdesItem::name("Test User".to_string());
        assert_eq!(item.item_type, RtcpSdesItemType::Name);
        assert_eq!(item.value, "Test User");
        
        let item = RtcpSdesItem::tool("rVOIP RTP".to_string());
        assert_eq!(item.item_type, RtcpSdesItemType::Tool);
        assert_eq!(item.value, "rVOIP RTP");
    }
    
    #[test]
    fn test_sdes_chunk() {
        let mut chunk = RtcpSdesChunk::new(0x12345678);
        chunk.add_item(RtcpSdesItem::cname("user@example.com".to_string()));
        chunk.add_item(RtcpSdesItem::tool("rVOIP RTP".to_string()));
        
        assert_eq!(chunk.ssrc, 0x12345678);
        assert_eq!(chunk.items.len(), 2);
        assert_eq!(chunk.items[0].item_type, RtcpSdesItemType::CName);
        assert_eq!(chunk.items[0].value, "user@example.com");
        assert_eq!(chunk.items[1].item_type, RtcpSdesItemType::Tool);
        assert_eq!(chunk.items[1].value, "rVOIP RTP");
    }
    
    #[test]
    fn test_sdes_packet() {
        let mut sdes = RtcpSourceDescription::new();
        
        // Add a chunk with CNAME and TOOL
        let mut chunk1 = RtcpSdesChunk::new(0x12345678);
        chunk1.add_item(RtcpSdesItem::cname("user1@example.com".to_string()));
        chunk1.add_item(RtcpSdesItem::tool("rVOIP RTP".to_string()));
        sdes.add_chunk(chunk1);
        
        // Add a source with just CNAME
        sdes.add_source(0xabcdef01, Some("user2@example.com".to_string()));
        
        // Verify chunks were added
        assert_eq!(sdes.chunks.len(), 2);
        
        // Verify first chunk
        assert_eq!(sdes.chunks[0].ssrc, 0x12345678);
        assert_eq!(sdes.chunks[0].items.len(), 2);
        
        // Verify second chunk
        assert_eq!(sdes.chunks[1].ssrc, 0xabcdef01);
        assert_eq!(sdes.chunks[1].items.len(), 1);
        assert_eq!(sdes.chunks[1].items[0].item_type, RtcpSdesItemType::CName);
        assert_eq!(sdes.chunks[1].items[0].value, "user2@example.com");
        
        // Test finding chunks and CNAMEs
        let chunk = sdes.find_chunk(0x12345678);
        assert!(chunk.is_some());
        assert_eq!(chunk.unwrap().ssrc, 0x12345678);
        
        let cname = sdes.find_cname(0xabcdef01);
        assert!(cname.is_some());
        assert_eq!(cname.unwrap(), "user2@example.com");
        
        // Test for non-existent SSRC
        let chunk = sdes.find_chunk(0x99999999);
        assert!(chunk.is_none());
        
        let cname = sdes.find_cname(0x99999999);
        assert!(cname.is_none());
    }
} 