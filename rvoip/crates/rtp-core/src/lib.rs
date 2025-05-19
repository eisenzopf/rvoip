//! RTP Core library for the RVOIP project
//! 
//! This crate provides RTP packet encoding/decoding, RTCP support,
//! and other utilities for handling real-time media transport.
//!
//! The library is organized into several modules:
//!
//! - `packet`: RTP and RTCP packet definitions and processing
//! - `session`: RTP session management including SSRC demultiplexing
//! - `transport`: Network transport for RTP/RTCP
//! - `srtp`: Secure RTP implementation
//! - `stats`: RTP statistics collection
//! - `time`: Timing and clock utilities
//! - `sync`: Media synchronization for multiple streams
//! - `traits`: Public traits for integration with other crates
//! - `payload`: RTP payload format handlers
//! - `buffer`: High-performance buffer management for receiving and transmitting packets
//! - `csrc`: CSRC management
//! - `error`: Error handling
//! - `rtcp`: RTCP packet definitions and processing
//! - `dtls`: DTLS support
//! - `api`: New API module with client/server separation
//!
//! ## New API Structure
//!
//! The `api` module provides a higher-level interface with clear client/server separation:
//!
//! - `api::client`: Client-side media transport for sending/receiving media frames
//! - `api::server`: Server-side media transport for handling multiple clients
//! - `api::common`: Shared types and utilities used by both client and server
//!
//! This structure makes the library easier to use for higher-level components like media-core.
//!
//! ## Buffer Management
//!
//! The `buffer` module provides optimized memory and packet management
//! for high-scale deployments:
//!
//! - Memory pooling to reduce allocations
//! - Adaptive jitter buffer for handling network variation
//! - Priority-based transmit buffering
//! - Congestion control for network adaptation
//! - Global memory limits to prevent OOM conditions
//! - Efficient packet ordering and scheduling
//!
//! This is ideal for deployments handling tens of thousands of concurrent streams.

mod error;

// Main modules
pub mod packet;
pub mod session;
pub mod transport;
pub mod srtp;
pub mod stats;
pub mod time;
pub mod traits;
pub mod payload;
pub mod buffer;
pub mod csrc;
pub mod rtcp;
pub mod dtls;
pub mod sync;
pub mod api;

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

// Re-export core types
pub use error::Error;

// Re-export common types from packet module
pub use packet::rtp::RtpPacket;
pub use packet::header::RtpHeader;
pub use packet::rtcp::{
    RtcpPacket, RtcpSenderReport, RtcpReceiverReport, 
    RtcpReportBlock, NtpTimestamp, RtcpSourceDescription,
    RtcpGoodbye, RtcpApplicationDefined, RtcpExtendedReport,
    RtcpCompoundPacket, RtcpXrBlock, VoipMetricsBlock
};
pub use packet::extension::{
    ExtensionElement, RtpHeaderExtensions,
    ids::AUDIO_LEVEL, ids::VIDEO_ORIENTATION, 
    ids::TRANSPORT_CC,
    ids::FRAME_MARKING, ids::SDES, 
    uris::ABS_SEND_TIME, uris::MID, uris::STREAM_ID, uris::REPAIR_RTP_STREAM_ID,
    uris::VIDEO_CONTENT_TYPE
};

// Re-export session types
pub use session::{RtpSession, RtpSessionConfig, RtpSessionEvent, RtpSessionStats, RtpStream, RtpStreamStats};

// Re-export transport types
pub use transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};

// Re-export traits for media-core integration
pub use traits::{MediaTransport, RtpEvent};
pub use traits::media_transport::RtpMediaTransport;

// Re-export payload format types
pub use payload::{
    PayloadType, PayloadFormat, PayloadFormatFactory, create_payload_format,
    G711UPayloadFormat, G711APayloadFormat, G722PayloadFormat, 
    OpusPayloadFormat, OpusBandwidth, Vp8PayloadFormat, Vp9PayloadFormat
};

pub use csrc::{CsrcMapping, CsrcManager, MAX_CSRC_COUNT};

// Re-export sync utilities
pub use sync::{MediaSync};
pub use sync::mapping::TimestampMapper;
pub use sync::clock::MediaClock;

// Re-export the new API components for easier access
pub use api::client::{MediaTransportClient, ClientFactory, ClientConfig, ClientConfigBuilder};
pub use api::server::{MediaTransportServer, ServerFactory, ServerConfig, ServerConfigBuilder, ClientInfo};
pub use api::common::frame::{MediaFrame, MediaFrameType};
pub use api::common::events::{MediaTransportEvent, MediaEventCallback};
pub use api::common::stats::{MediaStats, QualityLevel};
pub use api::common::config::{SecurityMode, SrtpProfile, SecurityInfo, NetworkPreset, BaseTransportConfig};
pub use api::common::error::{MediaTransportError, SecurityError, BufferError, StatsError};
pub use api::common::buffer::{MediaBuffer, MediaBufferConfig, BufferStats};

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
    
    pub use crate::traits::{MediaTransport, RtpMediaTransport};
    
    // Add new API types to prelude for easy access
    pub use crate::api::client::{MediaTransportClient, ClientFactory, ClientConfig};
    pub use crate::api::server::{MediaTransportServer, ServerFactory, ServerConfig, ClientInfo};
    pub use crate::api::common::frame::{MediaFrame, MediaFrameType};
    pub use crate::api::common::events::{MediaTransportEvent, MediaEventCallback};
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
        
        // Create extensions with legacy format (0x1234 profile ID)
        let mut ext = RtpHeaderExtensions::new_legacy(0x1234);
        // Add a single extension element with the extension data
        ext.elements.push(ExtensionElement {
            id: 1, // Any ID for legacy format
            data: Bytes::from_static(b"extension data"),
        });
        header.extensions = Some(ext);
        
        debug!("Original header with extension: ext={}, format={:?}, data_len={:?}", 
              header.extension, header.extensions.as_ref().map(|e| e.format), 
              header.extensions.as_ref().map(|e| e.elements.iter().map(|el| el.data.len()).sum::<usize>()));
        
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
        assert!(parsed_header.extensions.is_some());
        
        let parsed_extensions = parsed_header.extensions.unwrap();
        assert_eq!(parsed_extensions.profile_id, 0x1234);
        assert!(!parsed_extensions.elements.is_empty());
        
        // Get the parsed extension data
        let parsed_data = &parsed_extensions.elements[0].data;
        let original_data = b"extension data";
        
        // Verify that the parsed data contains the original data
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

    #[test]
    fn test_serialize_rtp_packet_with_extension() {
        // Create a header with extension
        let mut header = RtpHeader::new(96, 1000, 12345, 0xABCDEF01);
        header.extension = true;
        
        // Create extensions with legacy format (0x1234 profile ID)
        let mut ext = RtpHeaderExtensions::new_legacy(0x1234);
        // Add a single extension element with the extension data
        ext.elements.push(ExtensionElement {
            id: 1, // Any ID for legacy format
            data: Bytes::from_static(b"extension data"),
        });
        header.extensions = Some(ext);
        
        println!("Extension: {}, profile ID: {}, Data length: {}",
              header.extension, 
              header.extensions.as_ref().map(|e| e.profile_id).unwrap_or(0),
              header.extensions.as_ref().map(|e| e.elements.iter().map(|el| el.data.len()).sum::<usize>()).unwrap_or(0));
        
        // Create packet
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new(header, payload);
        
        // Serialize
        let bytes = packet.serialize().unwrap();
        
        // Should have extension flag set in header
        assert_eq!(bytes[0] & 0x10, 0x10);
        
        // Extension header offset: 12 (fixed header) + 0 (no CSRCs)
        let ext_header_offset = 12;
        
        // Extension header: defined ID (16 bits) + length in 32-bit words (16 bits)
        let ext_id = ((bytes[ext_header_offset] as u16) << 8) | (bytes[ext_header_offset + 1] as u16);
        let ext_len_words = ((bytes[ext_header_offset + 2] as u16) << 8) | (bytes[ext_header_offset + 3] as u16);
        
        assert_eq!(ext_id, 0x1234);
        
        // Length in 32-bit words, so multiply by 4 to get bytes
        assert_eq!(ext_len_words * 4, 16); // 16 bytes (rounded up to multiple of 4)
        
        // Extension data starts after extension header
        let ext_data_offset = ext_header_offset + 4;
        let ext_data = &bytes[ext_data_offset..ext_data_offset + 14];
        
        // Check that the first bytes match our extension
        let expected_data = b"extension data";
        for i in 0..expected_data.len() {
            assert_eq!(ext_data[i], expected_data[i]);
        }
    }
    
    #[test]
    fn test_parse_rtp_packet_with_extension() {
        // Create a header with extension
        let mut header = RtpHeader::new(96, 1000, 12345, 0xABCDEF01);
        header.extension = true;
        
        // Create extensions with legacy format (0x1234 profile ID)
        let mut ext = RtpHeaderExtensions::new_legacy(0x1234);
        // Add a single extension element with the extension data
        ext.elements.push(ExtensionElement {
            id: 1, // Any ID for legacy format
            data: Bytes::from_static(b"extension data"),
        });
        header.extensions = Some(ext);
        
        // Create packet
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new(header, payload);
        
        // Serialize
        let bytes = packet.serialize().unwrap();
        
        // Parse back
        let parsed_packet = RtpPacket::parse(&bytes).unwrap();
        let parsed_header = parsed_packet.header;
        
        // Check extension fields
        assert_eq!(parsed_header.extension, true);
        assert!(parsed_header.extensions.is_some());
        
        let parsed_extensions = parsed_header.extensions.unwrap();
        assert_eq!(parsed_extensions.profile_id, 0x1234);
        assert!(!parsed_extensions.elements.is_empty());
        
        // Get the parsed extension data
        let parsed_data = &parsed_extensions.elements[0].data;
        
        // Compare the content, accounting for possible padding bytes
        let original_data = b"extension data";
        assert!(parsed_data.starts_with(original_data), 
               "Extension data doesn't match original. Expected to start with: {:?}, got: {:?}", 
               original_data, parsed_data);
        
        // Check that the payload is correctly parsed
        assert_eq!(parsed_packet.payload.as_ref(), b"test payload" as &[u8]);
    }
} 