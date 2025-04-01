//! RTP Core library for the RVOIP project
//! 
//! This crate provides RTP packet encoding/decoding, RTCP support,
//! and other utilities for handling real-time media transport.

mod error;
mod packet;
pub mod session;
pub mod rtcp;

pub use error::Error;
pub use packet::{RtpPacket, RtpHeader};
pub use session::RtpSession;
pub use session::RtpSessionConfig;
pub use rtcp::{RtcpPacket, RtcpSenderReport, RtcpReceiverReport, RtcpReportBlock, NtpTimestamp};

/// The default maximum size for RTP packets in bytes
pub const DEFAULT_MAX_PACKET_SIZE: usize = 1500;

/// Typedef for RTP timestamp values
pub type RtpTimestamp = u32;

/// Typedef for RTP sequence numbers
pub type RtpSequenceNumber = u16;

/// Typedef for RTP synchronization source identifier
pub type RtpSsrc = u32;

/// Typedef for RTP contributing source identifier
pub type RtpCsrc = u32;

/// Result type for RTP operations
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use packet::RtpHeader;

    #[test]
    fn test_rtp_header_serialize_parse() {
        // Create a simple RTP header
        let original_header = RtpHeader::new(96, 1000, 0x12345678, 0xabcdef01);
        
        // Serialize the header
        let mut buf = bytes::BytesMut::with_capacity(12);
        original_header.serialize(&mut buf).unwrap();
        
        // Convert to bytes
        let buf = buf.freeze();
        
        // Parse the header back
        let mut reader = buf.clone();
        let parsed_header = RtpHeader::parse(&mut reader).unwrap();
        
        // Verify fields
        assert_eq!(parsed_header.version, 2);
        assert_eq!(parsed_header.payload_type, 96);
        assert_eq!(parsed_header.sequence_number, 1000);
        assert_eq!(parsed_header.timestamp, 0x12345678);
        assert_eq!(parsed_header.ssrc, 0xabcdef01);
        assert_eq!(parsed_header.padding, false);
        assert_eq!(parsed_header.extension, false);
        assert_eq!(parsed_header.cc, 0);
        assert_eq!(parsed_header.marker, false);
    }
    
    #[test]
    fn test_rtp_packet_serialize_parse() {
        // Create payload
        let payload_data = b"test payload data";
        let payload = Bytes::copy_from_slice(payload_data);
        
        // Create a packet
        let original_packet = RtpPacket::new_with_payload(
            96,                  // Payload type
            1000,                // Sequence number
            0x12345678,          // Timestamp
            0xabcdef01,          // SSRC
            payload.clone(),
        );
        
        // Serialize the packet
        let serialized = original_packet.serialize().unwrap();
        
        // Parse it back
        let parsed_packet = RtpPacket::parse(&serialized).unwrap();
        
        // Verify fields
        assert_eq!(parsed_packet.header.version, 2);
        assert_eq!(parsed_packet.header.payload_type, 96);
        assert_eq!(parsed_packet.header.sequence_number, 1000);
        assert_eq!(parsed_packet.header.timestamp, 0x12345678);
        assert_eq!(parsed_packet.header.ssrc, 0xabcdef01);
        assert_eq!(parsed_packet.payload, payload);
    }
    
    #[test]
    fn test_rtp_header_with_csrc() {
        // Create header with CSRC list
        let mut header = RtpHeader::new(96, 1000, 0x12345678, 0xabcdef01);
        header.csrc = vec![0x11111111, 0x22222222];
        header.cc = 2;
        
        // Serialize the header
        let mut buf = bytes::BytesMut::with_capacity(20);
        header.serialize(&mut buf).unwrap();
        
        // Parse it back
        let mut reader = buf.freeze();
        let parsed_header = RtpHeader::parse(&mut reader).unwrap();
        
        // Verify fields
        assert_eq!(parsed_header.cc, 2);
        assert_eq!(parsed_header.csrc.len(), 2);
        assert_eq!(parsed_header.csrc[0], 0x11111111);
        assert_eq!(parsed_header.csrc[1], 0x22222222);
    }
    
    #[test]
    fn test_rtp_header_with_extension() {
        // Create header with extension
        let mut header = RtpHeader::new(96, 1000, 0x12345678, 0xabcdef01);
        header.extension = true;
        header.extension_id = Some(0x1234);
        header.extension_data = Some(Bytes::from_static(b"extension data"));
        
        // Serialize the header
        let mut buf = bytes::BytesMut::with_capacity(32);
        header.serialize(&mut buf).unwrap();
        
        // Parse it back
        let mut reader = buf.freeze();
        let parsed_header = RtpHeader::parse(&mut reader).unwrap();
        
        // Verify fields
        assert_eq!(parsed_header.extension, true);
        assert_eq!(parsed_header.extension_id, Some(0x1234));
        assert!(parsed_header.extension_data.is_some());
        assert_eq!(parsed_header.extension_data.unwrap(), Bytes::from_static(b"extension data"));
    }
} 