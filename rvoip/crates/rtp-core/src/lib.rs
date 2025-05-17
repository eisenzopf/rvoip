//! RTP Core library for the RVOIP project
//! 
//! This crate provides RTP packet encoding/decoding, RTCP support,
//! and other utilities for handling real-time media transport.
//!
//! The library is organized into several modules:
//!
//! - `packet`: RTP and RTCP packet definitions and processing
//! - `session`: RTP session management
//! - `transport`: Network transport for RTP/RTCP
//! - `srtp`: Secure RTP implementation
//! - `stats`: RTP statistics collection
//! - `time`: Timing and clock utilities
//! - `traits`: Public traits for integration with other crates

mod error;

// Main modules
pub mod packet;
pub mod session;
pub mod transport;
pub mod srtp;
pub mod stats;
pub mod time;
pub mod traits;

// Re-export core types
pub use error::Error;

// Re-export common types from packet module
pub use packet::{RtpPacket, RtpHeader};
pub use packet::rtcp::{
    RtcpPacket, RtcpSenderReport, RtcpReceiverReport, 
    RtcpReportBlock, NtpTimestamp, RtcpSourceDescription,
    RtcpGoodbye, RtcpApplicationDefined
};

// Re-export session types
pub use session::{RtpSession, RtpSessionConfig, RtpSessionEvent, RtpSessionStats};

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

/// Prelude module with commonly used types
pub mod prelude {
    pub use crate::{
        RtpPacket, RtpHeader, RtpSession, RtpSessionConfig,
        RtpTimestamp, RtpSequenceNumber, RtpSsrc, RtpCsrc,
        Error, Result,
    };
    
    pub use crate::packet::rtcp::{
        RtcpPacket, RtcpSenderReport, RtcpReceiverReport, 
        RtcpReportBlock, NtpTimestamp
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use packet::{RtpHeader, hex_dump};
    use tracing::{debug, info};

    // Set up a simple test logger
    fn init_test_logging() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
    }

    #[test]
    fn test_rtp_header_serialize_parse() {
        init_test_logging();
        
        // Create a simple RTP header
        let original_header = RtpHeader::new(96, 1000, 0x12345678, 0xabcdef01);
        debug!("Original header: PT={}", original_header.payload_type);
        
        // Serialize the header
        let mut buf = bytes::BytesMut::with_capacity(12);
        original_header.serialize(&mut buf).unwrap();
        
        // Debug serialized buffer
        debug!("Serialized header bytes: [{}]", hex_dump(&buf));
        
        // Convert to bytes
        let buf = buf.freeze();
        
        // Parse the header back
        let mut reader = buf.clone();
        let parsed_header = RtpHeader::parse(&mut reader).unwrap();
        debug!("Parsed header: PT={}", parsed_header.payload_type);
        
        // Verify fields
        assert_eq!(parsed_header.version, 2);
        assert_eq!(parsed_header.payload_type, 96, "Payload type mismatch: expected 96, got {}", parsed_header.payload_type);
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
        init_test_logging();
        
        // Create header with extension
        let mut header = RtpHeader::new(96, 1000, 0x12345678, 0xabcdef01);
        header.extension = true;
        header.extension_id = Some(0x1234);
        header.extension_data = Some(Bytes::from_static(b"extension data"));
        debug!("Original header with extension: ext={}, ext_id={:?}, ext_data_len={:?}", 
              header.extension, header.extension_id, 
              header.extension_data.as_ref().map(|d| d.len()));
        
        // Serialize the header
        let mut buf = bytes::BytesMut::with_capacity(40);
        header.serialize(&mut buf).unwrap();
        debug!("Serialized extension header (size={}): [{}]", buf.len(), hex_dump(&buf));
        
        // Directly check if extension bit is correctly set in serialized data
        let first_byte = buf[0];
        debug!("First byte: 0x{:02x}, extension bit set: {}", 
               first_byte, ((first_byte >> 4) & 0x01) == 1);
        
        // Manually parse first byte to make sure our bit positions are correct
        let version = (first_byte >> 6) & 0x03;
        let padding = ((first_byte >> 5) & 0x01) == 1;
        let extension = ((first_byte >> 4) & 0x01) == 1;
        let cc = first_byte & 0x0F;
        debug!("Manual parse of first byte 0x{:02x}: V={}, P={}, X={}, CC={}",
               first_byte, version, padding, extension, cc);
        
        // Parse it back with our parser
        let mut reader = buf.freeze();
        debug!("Buffer size for parsing: {}", reader.len());
        
        let parse_result = RtpHeader::parse(&mut reader);
        if let Err(ref e) = parse_result {
            debug!("Parse error: {:?}", e);
        }
        
        let parsed_header = parse_result.unwrap();
        debug!("Remaining bytes after parse: {}", reader.len());
        
        // Verify fields
        assert_eq!(parsed_header.extension, true);
        assert_eq!(parsed_header.extension_id, Some(0x1234));
        assert!(parsed_header.extension_data.is_some());
        
        // Get the parsed extension data and the original data
        let parsed_data = parsed_header.extension_data.unwrap();
        let original_data = b"extension data";
        
        // Verify that the parsed data starts with the original data
        // (may contain padding bytes at the end)
        assert!(parsed_data.starts_with(original_data), 
                "Extension data doesn't match. Expected to start with: {:?}, got: {:?}", 
                original_data, parsed_data);
    }
    
    #[test]
    fn test_parse_real_world_packet() {
        init_test_logging();
        
        // This is the hex data from a typical RTP packet:
        // First byte: 0x80 = Version 2, no padding, no extension, 0 CSRCs
        // Second byte: 0x00 = No marker, PT 0 (PCMU/G.711)
        let packet_data = [
            0x80, 0x00, 0xfd, 0x70, 0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 0x54, 0x65, 0x73, 0x74
        ];
        
        debug!("Test packet data: [{}]", hex_dump(&packet_data));
        
        // Try to parse the RTP header directly first
        let mut buf = Bytes::copy_from_slice(&packet_data);
        let header_result = packet::RtpHeader::parse(&mut buf);
        
        if let Err(ref e) = header_result {
            debug!("RTP header parse failed: {:?}", e);
        } else {
            debug!("RTP header parse succeeded, remaining bytes: {}", buf.len());
        }
        
        assert!(header_result.is_ok(), "Failed to parse RTP header: {:?}", header_result.err());
        
        // Now try to parse the full packet
        let packet_result = RtpPacket::parse(&packet_data);
        
        if let Err(ref e) = packet_result {
            debug!("RTP packet parse failed: {:?}", e);
        } else {
            debug!("RTP packet parse succeeded");
        }
        
        assert!(packet_result.is_ok(), "Failed to parse RTP packet: {:?}", packet_result.err());
        
        let parsed = packet_result.unwrap();
        
        // Verify header fields based on the hex data
        assert_eq!(parsed.header.version, 2); // 0x80 -> version 2
        assert_eq!(parsed.header.payload_type, 0); // 0x00 -> PT 0
        assert_eq!(parsed.header.cc, 0); // 0x80 -> 0 CSRCs
        assert_eq!(parsed.header.sequence_number, 0xfd70); // Sequence from bytes 2-3
        assert_eq!(parsed.header.timestamp, 0); // Timestamp from bytes 4-7
        assert_eq!(parsed.header.ssrc, 0); // SSRC from bytes 8-11
        
        // The payload should be "Test"
        assert_eq!(parsed.payload.len(), 4);
        assert_eq!(parsed.payload.as_ref(), &b"Test"[..]);
    }
} 