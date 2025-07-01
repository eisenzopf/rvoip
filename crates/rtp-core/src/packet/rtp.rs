use bytes::{Bytes, BytesMut};
use std::fmt;
use tracing::debug;

use crate::error::Error;
use crate::{Result, RtpSequenceNumber, RtpSsrc, RtpTimestamp};
use super::header::{RtpHeader, hex_dump, RTP_MIN_HEADER_SIZE};

/// An RTP packet with header and payload
#[derive(Clone, PartialEq, Eq)]
pub struct RtpPacket {
    /// RTP header
    pub header: RtpHeader,
    
    /// Payload data
    pub payload: Bytes,
}

impl RtpPacket {
    /// Create a new RTP packet with the given header and payload
    pub fn new(header: RtpHeader, payload: Bytes) -> Self {
        Self { header, payload }
    }
    
    /// Create a new RTP packet with the standard header fields and payload
    pub fn new_with_payload(
        payload_type: u8,
        sequence_number: RtpSequenceNumber,
        timestamp: RtpTimestamp,
        ssrc: RtpSsrc,
        payload: Bytes,
    ) -> Self {
        let header = RtpHeader::new(payload_type, sequence_number, timestamp, ssrc);
        Self { header, payload }
    }
    
    /// Get the total size of the packet in bytes
    pub fn size(&self) -> usize {
        self.header.size() + self.payload.len()
    }
    
    /// Parse an RTP packet from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        debug!("Parsing RTP packet from {} bytes", data.len());
        
        // Parse the header without consuming the buffer
        let (header, header_size) = RtpHeader::parse_without_consuming(data)?;
        debug!("Parsed header of size {}", header_size);
        
        // Extract the payload
        let payload = if data.len() > header_size {
            Bytes::copy_from_slice(&data[header_size..])
        } else {
            Bytes::new()
        };
        debug!("Extracted payload of size {}", payload.len());
        
        Ok(Self { header, payload })
    }
    
    /// Serialize the packet to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        // Create a buffer with enough size for the entire packet
        let total_size = self.size();
        let mut buf = BytesMut::with_capacity(total_size);
        
        // Serialize the header
        self.header.serialize(&mut buf)?;
        
        // Add the payload
        buf.extend_from_slice(&self.payload);
        
        Ok(buf.freeze())
    }
}

impl fmt::Debug for RtpPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RtpPacket {{ header: {:?}, payload_len: {} }}", 
               self.header, self.payload.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    
    #[test]
    fn test_new_with_payload() {
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new_with_payload(
            96,          // Payload type
            1000,        // Sequence number
            12345,       // Timestamp
            0xabcdef01,  // SSRC
            payload.clone()
        );
        
        assert_eq!(packet.header.payload_type, 96);
        assert_eq!(packet.header.sequence_number, 1000);
        assert_eq!(packet.header.timestamp, 12345);
        assert_eq!(packet.header.ssrc, 0xabcdef01);
        assert_eq!(packet.payload, payload);
    }
    
    #[test]
    fn test_size() {
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new_with_payload(
            96, 1000, 12345, 0xabcdef01, payload
        );
        
        assert_eq!(packet.size(), RTP_MIN_HEADER_SIZE + 12); // 12 bytes payload
    }
    
    #[test]
    fn test_serialize_parse_roundtrip() {
        let payload = Bytes::from_static(b"test payload data");
        let original = RtpPacket::new_with_payload(
            96, 1000, 12345, 0xabcdef01, payload
        );
        
        // Serialize
        let serialized = original.serialize().unwrap();
        
        // Parse
        let parsed = RtpPacket::parse(&serialized).unwrap();
        
        // Verify
        assert_eq!(parsed.header.payload_type, original.header.payload_type);
        assert_eq!(parsed.header.sequence_number, original.header.sequence_number);
        assert_eq!(parsed.header.timestamp, original.header.timestamp);
        assert_eq!(parsed.header.ssrc, original.header.ssrc);
        assert_eq!(parsed.payload, original.payload);
    }
    
    #[test]
    fn test_debug_format() {
        let packet = RtpPacket::new_with_payload(
            96, 1000, 12345, 0xabcdef01, 
            Bytes::from_static(b"test payload")
        );
        
        let debug_str = format!("{:?}", packet);
        assert!(debug_str.contains("payload_len: 12"));
        assert!(debug_str.contains("header:"));
    }
} 