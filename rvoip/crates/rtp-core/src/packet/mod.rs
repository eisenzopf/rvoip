//! RTP Packet module
//!
//! This module provides structures for handling RTP packets as defined in RFC 3550.
//! It includes implementations for RTP headers, packet parsing and serialization.

pub mod rtp;
pub mod header;
pub mod rtcp;

pub use rtp::*;
pub use header::*;

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::RtpSsrc;
    
    #[test]
    fn test_rtp_packet_creation() {
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
    fn test_packet_serialize_parse_roundtrip() {
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
} 