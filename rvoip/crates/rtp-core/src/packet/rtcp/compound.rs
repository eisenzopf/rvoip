//! RTCP Compound Packet handling
//!
//! This module implements RTCP compound packet handling as specified in RFC 3550 Section 6.1.
//! RTCP packets are usually sent as compound packets containing:
//! 1. A mandatory SR or RR packet first
//! 2. Optional additional packets (SDES, BYE, APP, XR)

use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io::Cursor;

use crate::error::Error;
use crate::Result;
use super::{
    RtcpPacket, RtcpPacketType, 
    RtcpSenderReport, RtcpReceiverReport,
    RtcpSourceDescription, RtcpGoodbye, 
    RtcpApplicationDefined, RtcpExtendedReport
};

/// RTCP Compound Packet
///
/// A compound packet is a concatenation of multiple RTCP packets
/// with special formatting requirements:
/// - The first packet must be SR or RR
/// - SDES packet with CNAME item should be included
/// - Other packets may be included as needed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpCompoundPacket {
    /// Packets in the compound packet
    pub packets: Vec<RtcpPacket>,
}

impl RtcpCompoundPacket {
    /// Create a new compound packet starting with a Sender Report
    pub fn new_with_sr(sr: RtcpSenderReport) -> Self {
        let mut packets = Vec::new();
        packets.push(RtcpPacket::SenderReport(sr));
        
        Self { packets }
    }
    
    /// Create a new compound packet starting with a Receiver Report
    pub fn new_with_rr(rr: RtcpReceiverReport) -> Self {
        let mut packets = Vec::new();
        packets.push(RtcpPacket::ReceiverReport(rr));
        
        Self { packets }
    }
    
    /// Add an SDES packet
    pub fn add_sdes(&mut self, sdes: RtcpSourceDescription) {
        self.packets.push(RtcpPacket::SourceDescription(sdes));
    }
    
    /// Add a BYE packet
    pub fn add_bye(&mut self, bye: RtcpGoodbye) {
        self.packets.push(RtcpPacket::Goodbye(bye));
    }
    
    /// Add an APP packet
    pub fn add_app(&mut self, app: RtcpApplicationDefined) {
        self.packets.push(RtcpPacket::ApplicationDefined(app));
    }
    
    /// Add an XR packet
    pub fn add_xr(&mut self, xr: RtcpExtendedReport) {
        self.packets.push(RtcpPacket::ExtendedReport(xr));
    }
    
    /// Validate that the compound packet meets RFC requirements
    pub fn validate(&self) -> Result<()> {
        // Check if the compound packet has at least one packet
        if self.packets.is_empty() {
            return Err(Error::RtcpError(
                "Compound packet must contain at least one packet".to_string()
            ));
        }
        
        // Check if the first packet is SR or RR
        match self.packets[0] {
            RtcpPacket::SenderReport(_) | RtcpPacket::ReceiverReport(_) => Ok(()),
            _ => Err(Error::RtcpError(
                "First packet in compound packet must be SR or RR".to_string()
            )),
        }
    }
    
    /// Serialize the compound packet
    pub fn serialize(&self) -> Result<Bytes> {
        // Validate the compound packet
        self.validate()?;
        
        // Serialize each packet
        let mut buf = BytesMut::new();
        
        for packet in &self.packets {
            let packet_bytes = packet.serialize()?;
            buf.extend_from_slice(&packet_bytes);
        }
        
        Ok(buf.freeze())
    }
    
    /// Parse a compound packet from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut buf = Bytes::copy_from_slice(data);
        let mut packets = Vec::new();
        
        // Parse packets until we run out of data
        while buf.remaining() >= 4 {
            // Check if we have enough data for a header
            if buf.remaining() < 4 {
                break;
            }
            
            // Look ahead to see packet length without consuming
            let mut peek_buf = buf.clone();
            let first_byte = peek_buf.get_u8();
            let packet_type_byte = peek_buf.get_u8();
            let length = peek_buf.get_u16() as usize * 4;
            
            // Total packet size = header (4 bytes) + length field * 4
            let total_packet_size = 4 + length;
            
            if buf.remaining() < total_packet_size {
                return Err(Error::BufferTooSmall {
                    required: total_packet_size,
                    available: buf.remaining(),
                });
            }
            
            // Slice the current packet
            let packet_data = &buf[0..total_packet_size];
            
            // Parse the individual RTCP packet
            let packet = RtcpPacket::parse(packet_data)?;
            packets.push(packet);
            
            // Advance buffer past this packet
            buf.advance(total_packet_size);
        }
        
        // Create and validate the compound packet
        let compound = Self { packets };
        compound.validate()?;
        
        Ok(compound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RtpSsrc;
    use super::super::{RtcpReportBlock, NtpTimestamp};
    
    #[test]
    fn test_compound_packet_validation() {
        // Valid compound packet with SR first
        let sr = RtcpSenderReport {
            ssrc: 0x12345678,
            ntp_timestamp: NtpTimestamp { seconds: 0, fraction: 0 },
            rtp_timestamp: 0,
            packet_count: 0,
            octet_count: 0,
            report_blocks: Vec::new(),
        };
        
        let mut compound = RtcpCompoundPacket::new_with_sr(sr);
        assert!(compound.validate().is_ok());
        
        // Valid compound packet with RR first
        let rr = RtcpReceiverReport {
            ssrc: 0x12345678,
            report_blocks: Vec::new(),
        };
        
        let compound = RtcpCompoundPacket::new_with_rr(rr);
        assert!(compound.validate().is_ok());
        
        // Invalid compound packet - no packets
        let invalid = RtcpCompoundPacket { packets: Vec::new() };
        assert!(invalid.validate().is_err());
        
        // Invalid compound packet - doesn't start with SR/RR
        let bye = RtcpGoodbye {
            sources: vec![0x12345678],
            reason: None,
        };
        
        let mut invalid = RtcpCompoundPacket { packets: Vec::new() };
        invalid.add_bye(bye);
        assert!(invalid.validate().is_err());
    }
    
    #[test]
    fn test_compound_packet_serialize_parse() {
        // Create a compound packet with SR + SDES + BYE
        let sr = RtcpSenderReport {
            ssrc: 0x12345678,
            ntp_timestamp: NtpTimestamp { seconds: 1234, fraction: 5678 },
            rtp_timestamp: 0x87654321,
            packet_count: 100,
            octet_count: 12345,
            report_blocks: Vec::new(),
        };
        
        let bye = RtcpGoodbye {
            sources: vec![0x12345678],
            reason: Some("Goodbye".to_string()),
        };
        
        let mut compound = RtcpCompoundPacket::new_with_sr(sr);
        compound.add_bye(bye);
        
        // Serialize
        let bytes = compound.serialize().unwrap();
        
        // Parse back
        let parsed = RtcpCompoundPacket::parse(&bytes).unwrap();
        
        // Compare
        assert_eq!(parsed.packets.len(), 2);
        
        match &parsed.packets[0] {
            RtcpPacket::SenderReport(sr) => {
                assert_eq!(sr.ssrc, 0x12345678);
                assert_eq!(sr.ntp_timestamp.seconds, 1234);
                assert_eq!(sr.rtp_timestamp, 0x87654321);
            },
            _ => panic!("Expected SR packet"),
        }
        
        match &parsed.packets[1] {
            RtcpPacket::Goodbye(bye) => {
                assert_eq!(bye.sources.len(), 1);
                assert_eq!(bye.sources[0], 0x12345678);
                assert_eq!(bye.reason.as_deref(), Some("Goodbye"));
            },
            _ => panic!("Expected BYE packet"),
        }
    }
} 