//! RTCP Packet module
//!
//! This module provides structures for handling RTCP packets as defined in RFC 3550.
//! It includes implementations for different RTCP packet types: SR, RR, SDES, BYE, APP.
//! Extended Reports (XR) are defined in RFC 3611.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::Error;
use crate::{Result, RtpSsrc};

/// RTCP version (same as RTP, always 2)
pub const RTCP_VERSION: u8 = 2;

/// RTCP packet types as defined in RFC 3550
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RtcpPacketType {
    /// Sender Report (SR)
    SenderReport = 200,
    
    /// Receiver Report (RR)
    ReceiverReport = 201,
    
    /// Source Description (SDES)
    SourceDescription = 202,
    
    /// Goodbye (BYE)
    Goodbye = 203,
    
    /// Application-Defined (APP)
    ApplicationDefined = 204,
    
    /// Extended Reports (XR) as defined in RFC 3611
    ExtendedReport = 207,
}

impl TryFrom<u8> for RtcpPacketType {
    type Error = Error;
    
    fn try_from(value: u8) -> Result<Self> {
        match value {
            200 => Ok(RtcpPacketType::SenderReport),
            201 => Ok(RtcpPacketType::ReceiverReport),
            202 => Ok(RtcpPacketType::SourceDescription),
            203 => Ok(RtcpPacketType::Goodbye),
            204 => Ok(RtcpPacketType::ApplicationDefined),
            207 => Ok(RtcpPacketType::ExtendedReport),
            _ => Err(Error::RtcpError(format!("Unknown RTCP packet type: {}", value))),
        }
    }
}

// Import and re-export types from submodules
mod sender_report;
mod receiver_report;
mod sdes;
mod bye;
mod app;
mod report_block;
mod ntp;
mod xr;
mod compound;

// Re-export all public types
pub use report_block::RtcpReportBlock;
pub use ntp::NtpTimestamp;
pub use sender_report::RtcpSenderReport;
pub use receiver_report::RtcpReceiverReport;
pub use sdes::{RtcpSourceDescription, RtcpSdesChunk, RtcpSdesItem, RtcpSdesItemType};
pub use bye::RtcpGoodbye;
pub use app::RtcpApplicationDefined;
pub use xr::{
    RtcpExtendedReport, RtcpXrBlock, RtcpXrBlockType,
    ReceiverReferenceTimeBlock, VoipMetricsBlock
};
pub use compound::RtcpCompoundPacket;

/// RTCP packet variants
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtcpPacket {
    /// Sender Report (SR)
    SenderReport(RtcpSenderReport),
    
    /// Receiver Report (RR)
    ReceiverReport(RtcpReceiverReport),
    
    /// Source Description (SDES)
    SourceDescription(RtcpSourceDescription),
    
    /// Goodbye (BYE)
    Goodbye(RtcpGoodbye),
    
    /// Application-Defined (APP)
    ApplicationDefined(RtcpApplicationDefined),
    
    /// Extended Reports (XR)
    ExtendedReport(RtcpExtendedReport),
}

impl RtcpPacket {
    /// Parse an RTCP packet from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut buf = Bytes::copy_from_slice(data);
        
        // Parse common header (first 4 bytes)
        if buf.remaining() < 4 {
            return Err(Error::BufferTooSmall {
                required: 4,
                available: buf.remaining(),
            });
        }
        
        let first_byte = buf.get_u8();
        // Check version (2 bits)
        let version = (first_byte >> 6) & 0x03;
        if version != RTCP_VERSION {
            return Err(Error::RtcpError(format!("Invalid RTCP version: {}", version)));
        }
        
        // Check padding flag (1 bit)
        let _padding = ((first_byte >> 5) & 0x01) != 0;
        
        // Get reception report count (5 bits)
        let report_count = first_byte & 0x1F;
        
        // Get packet type
        let packet_type = RtcpPacketType::try_from(buf.get_u8())?;
        
        // Get length in 32-bit words minus one (convert to bytes)
        let length = buf.get_u16() as usize * 4;
        
        if buf.remaining() < length {
            return Err(Error::BufferTooSmall {
                required: length,
                available: buf.remaining(),
            });
        }
        
        // Parse specific packet type
        match packet_type {
            RtcpPacketType::SenderReport => {
                Ok(RtcpPacket::SenderReport(
                    sender_report::parse_sender_report(&mut buf, report_count)?
                ))
            }
            RtcpPacketType::ReceiverReport => {
                Ok(RtcpPacket::ReceiverReport(
                    receiver_report::parse_receiver_report(&mut buf, report_count)?
                ))
            }
            RtcpPacketType::SourceDescription => {
                Ok(RtcpPacket::SourceDescription(
                    RtcpSourceDescription { chunks: Vec::new() }
                ))
            }
            RtcpPacketType::Goodbye => {
                Ok(RtcpPacket::Goodbye(
                    bye::parse_bye(&mut buf, report_count)?
                ))
            }
            RtcpPacketType::ApplicationDefined => {
                Ok(RtcpPacket::ApplicationDefined(
                    app::parse_app(&mut buf)?
                ))
            }
            RtcpPacketType::ExtendedReport => {
                Ok(RtcpPacket::ExtendedReport(
                    xr::parse_xr(&mut buf)?
                ))
            }
        }
    }
    
    /// Serialize an RTCP packet to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::new();
        
        match self {
            RtcpPacket::SenderReport(sr) => {
                // Create a buffer for the SR content
                let sr_content = sender_report::serialize_sender_report(sr)?;
                let content_size = sr_content.len();
                
                // Calculate length in 32-bit words minus one
                let words = (content_size + 4) / 4; // content plus header, in 32-bit words
                let length = words - 1; // minus one as per RFC
                
                // Write header
                // First byte: version (2 bits) | padding (1 bit) | report count (5 bits)
                let first_byte = (RTCP_VERSION << 6) | (0 << 5) | (sr.report_blocks.len() as u8 & 0x1F);
                buf.put_u8(first_byte);
                
                // Write packet type
                buf.put_u8(RtcpPacketType::SenderReport as u8);
                
                // Write length
                buf.put_u16(length as u16);
                
                // Write SR content
                buf.extend_from_slice(&sr_content);
                
                // Pad to 32-bit boundary if needed
                let padding_bytes = (4 - (buf.len() % 4)) % 4;
                for _ in 0..padding_bytes {
                    buf.put_u8(0);
                }
            }
            RtcpPacket::ReceiverReport(rr) => {
                // Create a buffer for the RR content
                let rr_content = receiver_report::serialize_receiver_report(rr)?;
                let content_size = rr_content.len();
                
                // Calculate length in 32-bit words minus one
                let words = (content_size + 4) / 4; // content plus header, in 32-bit words
                let length = words - 1; // minus one as per RFC
                
                // Write header
                // First byte: version (2 bits) | padding (1 bit) | report count (5 bits)
                let first_byte = (RTCP_VERSION << 6) | (0 << 5) | (rr.report_blocks.len() as u8 & 0x1F);
                buf.put_u8(first_byte);
                
                // Write packet type
                buf.put_u8(RtcpPacketType::ReceiverReport as u8);
                
                // Write length
                buf.put_u16(length as u16);
                
                // Write RR content
                buf.extend_from_slice(&rr_content);
                
                // Pad to 32-bit boundary if needed
                let padding_bytes = (4 - (buf.len() % 4)) % 4;
                for _ in 0..padding_bytes {
                    buf.put_u8(0);
                }
            }
            RtcpPacket::SourceDescription(sdes) => {
                // Create a buffer for the SDES content
                let sdes_content = sdes::serialize_sdes(sdes)?;
                let content_size = sdes_content.len();
                
                // Calculate length in 32-bit words minus one
                let words = (content_size + 4) / 4; // content plus header, in 32-bit words
                let length = words - 1; // minus one as per RFC
                
                // Write header
                // First byte: version (2 bits) | padding (1 bit) | chunk count (5 bits)
                let first_byte = (RTCP_VERSION << 6) | (0 << 5) | (sdes.chunks.len() as u8 & 0x1F);
                buf.put_u8(first_byte);
                
                // Write packet type
                buf.put_u8(RtcpPacketType::SourceDescription as u8);
                
                // Write length
                buf.put_u16(length as u16);
                
                // Write SDES content
                buf.extend_from_slice(&sdes_content);
                
                // Pad to 32-bit boundary if needed
                let padding_bytes = (4 - (buf.len() % 4)) % 4;
                for _ in 0..padding_bytes {
                    buf.put_u8(0);
                }
            }
            RtcpPacket::Goodbye(bye) => {
                // Create a buffer for the BYE packet content
                let bye_content = bye.serialize()?;
                let content_size = bye_content.len();
                
                // Calculate length in 32-bit words minus one
                let words = (content_size + 4) / 4; // content plus header, in 32-bit words
                let length = words - 1; // minus one as per RFC
                
                // Write header
                // First byte: version (2 bits) | padding (1 bit) | source count (5 bits)
                let first_byte = (RTCP_VERSION << 6) | (0 << 5) | (bye.sources.len() as u8 & 0x1F);
                buf.put_u8(first_byte);
                
                // Write packet type
                buf.put_u8(RtcpPacketType::Goodbye as u8);
                
                // Write length
                buf.put_u16(length as u16);
                
                // Write BYE content
                buf.extend_from_slice(&bye_content);
                
                // Pad to 32-bit boundary if needed
                let padding_bytes = (4 - (buf.len() % 4)) % 4;
                for _ in 0..padding_bytes {
                    buf.put_u8(0);
                }
            }
            RtcpPacket::ApplicationDefined(app) => {
                // Create a buffer for the APP packet content
                let app_content = app.serialize()?;
                let content_size = app_content.len();
                
                // Calculate length in 32-bit words minus one
                let words = (content_size + 4) / 4; // content plus header, in 32-bit words
                let length = words - 1; // minus one as per RFC
                
                // Write header
                // First byte: version (2 bits) | padding (1 bit) | subtype (5 bits)
                // For APP packets, subtype is always 0 in this implementation
                let first_byte = (RTCP_VERSION << 6) | (0 << 5) | 0;
                buf.put_u8(first_byte);
                
                // Write packet type
                buf.put_u8(RtcpPacketType::ApplicationDefined as u8);
                
                // Write length
                buf.put_u16(length as u16);
                
                // Write APP content
                buf.extend_from_slice(&app_content);
                
                // Pad to 32-bit boundary if needed
                let padding_bytes = (4 - (buf.len() % 4)) % 4;
                for _ in 0..padding_bytes {
                    buf.put_u8(0);
                }
            }
            RtcpPacket::ExtendedReport(xr) => {
                // Create a buffer for the XR packet content
                let xr_content = xr.serialize()?;
                let content_size = xr_content.len();
                
                // Calculate length in 32-bit words minus one
                let words = (content_size + 4) / 4; // content plus header, in 32-bit words
                let length = words - 1; // minus one as per RFC
                
                // Write header
                // First byte: version (2 bits) | padding (1 bit) | reserved (5 bits)
                let first_byte = (RTCP_VERSION << 6) | (0 << 5) | 0;
                buf.put_u8(first_byte);
                
                // Write packet type
                buf.put_u8(RtcpPacketType::ExtendedReport as u8);
                
                // Write length
                buf.put_u16(length as u16);
                
                // Write XR content
                buf.extend_from_slice(&xr_content);
                
                // Pad to 32-bit boundary if needed
                let padding_bytes = (4 - (buf.len() % 4)) % 4;
                for _ in 0..padding_bytes {
                    buf.put_u8(0);
                }
            }
        }
        
        Ok(buf.freeze())
    }
    
    /// Get the RTCP packet type
    pub fn packet_type(&self) -> RtcpPacketType {
        match self {
            RtcpPacket::SenderReport(_) => RtcpPacketType::SenderReport,
            RtcpPacket::ReceiverReport(_) => RtcpPacketType::ReceiverReport,
            RtcpPacket::SourceDescription(_) => RtcpPacketType::SourceDescription,
            RtcpPacket::Goodbye(_) => RtcpPacketType::Goodbye,
            RtcpPacket::ApplicationDefined(_) => RtcpPacketType::ApplicationDefined,
            RtcpPacket::ExtendedReport(_) => RtcpPacketType::ExtendedReport,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_rtcp_packet_type_conversion() {
        assert_eq!(RtcpPacketType::try_from(200).unwrap(), RtcpPacketType::SenderReport);
        assert_eq!(RtcpPacketType::try_from(201).unwrap(), RtcpPacketType::ReceiverReport);
        assert_eq!(RtcpPacketType::try_from(202).unwrap(), RtcpPacketType::SourceDescription);
        assert_eq!(RtcpPacketType::try_from(203).unwrap(), RtcpPacketType::Goodbye);
        assert_eq!(RtcpPacketType::try_from(204).unwrap(), RtcpPacketType::ApplicationDefined);
        assert_eq!(RtcpPacketType::try_from(207).unwrap(), RtcpPacketType::ExtendedReport);
        
        assert!(RtcpPacketType::try_from(100).is_err());
    }
} 