//! RTCP Compound Packet handling
//!
//! This module implements RTCP compound packet handling as specified in RFC 3550 Section 6.1.
//! RTCP packets are usually sent as compound packets containing:
//! 1. A mandatory SR or RR packet first
//! 2. Optional additional packets (SDES, BYE, APP, XR)

use bytes::{Buf, Bytes, BytesMut};

use super::{
    RtcpApplicationDefined, RtcpExtendedReport, RtcpGoodbye, RtcpPacket, RtcpReceiverReport,
    RtcpSenderReport, RtcpSourceDescription, RtcpUnknownPacket,
};
use crate::error::Error;
use crate::Result;

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

/// A member of a tolerantly parsed compound RTCP packet.
///
/// Keeping unknown members in this new wrapper avoids adding a variant to
/// [`RtcpPacket`], whose existing public variants can therefore still be
/// exhaustively matched by downstream 0.3.x users.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtcpCompoundMember {
    /// A packet type implemented by this crate.
    Known(RtcpPacket),
    /// A well-formed packet type not implemented by this crate.
    Unknown(RtcpUnknownPacket),
}

impl RtcpCompoundMember {
    /// Return the numeric RTCP packet type carried on the wire.
    pub fn packet_type_byte(&self) -> u8 {
        match self {
            Self::Known(packet) => packet.packet_type() as u8,
            Self::Unknown(packet) => packet.packet_type,
        }
    }

    fn has_padding(&self) -> bool {
        matches!(self, Self::Unknown(packet) if packet.has_padding())
    }

    fn serialize(&self) -> Result<Bytes> {
        match self {
            Self::Known(packet) => packet.serialize(),
            Self::Unknown(packet) => packet.serialize(),
        }
    }
}

/// A compound RTCP packet that preserves unimplemented but well-formed
/// packet types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpTolerantCompoundPacket {
    /// Known and unknown members in their original wire order.
    pub packets: Vec<RtcpCompoundMember>,
}

impl RtcpTolerantCompoundPacket {
    /// Validate the compound-packet ordering and padding rules.
    pub fn validate(&self) -> Result<()> {
        match self.packets.first() {
            Some(RtcpCompoundMember::Known(
                RtcpPacket::SenderReport(_) | RtcpPacket::ReceiverReport(_),
            )) => {}
            Some(_) => {
                return Err(Error::RtcpError(
                    "First packet in compound packet must be SR or RR".to_string(),
                ));
            }
            None => {
                return Err(Error::RtcpError(
                    "Compound packet must contain at least one packet".to_string(),
                ));
            }
        }

        if self
            .packets
            .iter()
            .take(self.packets.len().saturating_sub(1))
            .any(RtcpCompoundMember::has_padding)
        {
            return Err(Error::RtcpError(
                "Only the final packet in a compound RTCP packet may be padded".to_string(),
            ));
        }

        Ok(())
    }

    /// Serialize all known and preserved unknown members.
    pub fn serialize(&self) -> Result<Bytes> {
        self.validate()?;

        let mut buf = BytesMut::new();
        for packet in &self.packets {
            buf.extend_from_slice(&packet.serialize()?);
        }
        Ok(buf.freeze())
    }

    /// Parse an entire compound RTCP packet while preserving unimplemented
    /// but well-formed packet types.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut buf = Bytes::copy_from_slice(data);
        let mut packets = Vec::new();

        while buf.has_remaining() {
            if buf.remaining() < 4 {
                return Err(Error::BufferTooSmall {
                    required: 4,
                    available: buf.remaining(),
                });
            }

            let mut peek_buf = buf.clone();
            let first_byte = peek_buf.get_u8();
            let packet_type = peek_buf.get_u8();
            let length = peek_buf.get_u16() as usize * 4;
            let total_packet_size = 4 + length;

            if buf.remaining() < total_packet_size {
                return Err(Error::BufferTooSmall {
                    required: total_packet_size,
                    available: buf.remaining(),
                });
            }
            if first_byte & 0x20 != 0 && total_packet_size != buf.remaining() {
                return Err(Error::RtcpError(
                    "Only the final packet in a compound RTCP packet may be padded".to_string(),
                ));
            }

            let packet_data = &buf[..total_packet_size];
            let packet = if super::RtcpPacketType::try_from(packet_type).is_ok() {
                RtcpCompoundMember::Known(RtcpPacket::parse(packet_data)?)
            } else {
                RtcpCompoundMember::Unknown(RtcpUnknownPacket::parse(packet_data)?)
            };
            packets.push(packet);
            buf.advance(total_packet_size);
        }

        let compound = Self { packets };
        compound.validate()?;
        Ok(compound)
    }
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

    /// Get the Sender Report if the first packet is an SR
    pub fn get_sr(&self) -> Option<&RtcpSenderReport> {
        if let Some(RtcpPacket::SenderReport(sr)) = self.packets.first() {
            Some(sr)
        } else {
            None
        }
    }

    /// Get the Receiver Report if the first packet is an RR
    pub fn get_rr(&self) -> Option<&RtcpReceiverReport> {
        if let Some(RtcpPacket::ReceiverReport(rr)) = self.packets.first() {
            Some(rr)
        } else {
            None
        }
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
                "Compound packet must contain at least one packet".to_string(),
            ));
        }

        // Check if the first packet is SR or RR
        match self.packets[0] {
            RtcpPacket::SenderReport(_) | RtcpPacket::ReceiverReport(_) => {}
            _ => Err(Error::RtcpError(
                "First packet in compound packet must be SR or RR".to_string(),
            ))?,
        }

        Ok(())
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

        // Parse packets until we run out of data.
        while buf.has_remaining() {
            if buf.remaining() < 4 {
                return Err(Error::BufferTooSmall {
                    required: 4,
                    available: buf.remaining(),
                });
            }

            // Look ahead to see packet length without consuming
            let mut peek_buf = buf.clone();
            let first_byte = peek_buf.get_u8();
            let _packet_type_byte = peek_buf.get_u8();
            let length = peek_buf.get_u16() as usize * 4;

            // Total packet size = header (4 bytes) + length field * 4
            let total_packet_size = 4 + length;

            if buf.remaining() < total_packet_size {
                return Err(Error::BufferTooSmall {
                    required: total_packet_size,
                    available: buf.remaining(),
                });
            }

            let has_padding = first_byte & 0x20 != 0;
            if has_padding && total_packet_size != buf.remaining() {
                return Err(Error::RtcpError(
                    "Only the final packet in a compound RTCP packet may be padded".to_string(),
                ));
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

    /// Parse a compound packet while preserving well-formed packet types
    /// this crate does not implement.
    pub fn parse_tolerant(data: &[u8]) -> Result<RtcpTolerantCompoundPacket> {
        RtcpTolerantCompoundPacket::parse(data)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{NtpTimestamp, RtcpSdesItem, RtcpSdesItemType};
    use super::*;

    #[test]
    fn test_compound_packet_validation() {
        // Valid compound packet with SR first
        let sr = RtcpSenderReport {
            ssrc: 0x12345678,
            ntp_timestamp: NtpTimestamp {
                seconds: 0,
                fraction: 0,
            },
            rtp_timestamp: 0,
            sender_packet_count: 0,
            sender_octet_count: 0,
            report_blocks: Vec::new(),
        };

        let compound = RtcpCompoundPacket::new_with_sr(sr);
        assert!(compound.validate().is_ok());

        // Valid compound packet with RR first
        let rr = RtcpReceiverReport {
            ssrc: 0x12345678,
            report_blocks: Vec::new(),
        };

        let compound = RtcpCompoundPacket::new_with_rr(rr);
        assert!(compound.validate().is_ok());

        // Invalid compound packet - no packets
        let invalid = RtcpCompoundPacket {
            packets: Vec::new(),
        };
        assert!(invalid.validate().is_err());

        // Invalid compound packet - doesn't start with SR/RR
        let bye = RtcpGoodbye {
            sources: vec![0x12345678],
            reason: None,
        };

        let mut invalid = RtcpCompoundPacket {
            packets: Vec::new(),
        };
        invalid.add_bye(bye);
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_compound_packet_serialize_parse() {
        // Create a compound packet with SR + SDES + BYE
        let sr = RtcpSenderReport {
            ssrc: 0x12345678,
            ntp_timestamp: NtpTimestamp {
                seconds: 1234,
                fraction: 5678,
            },
            rtp_timestamp: 0x87654321,
            sender_packet_count: 100,
            sender_octet_count: 12345,
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
            }
            _ => panic!("Expected SR packet"),
        }

        match &parsed.packets[1] {
            RtcpPacket::Goodbye(bye) => {
                assert_eq!(bye.sources.len(), 1);
                assert_eq!(bye.sources[0], 0x12345678);
                assert_eq!(bye.reason.as_deref(), Some("Goodbye"));
            }
            _ => panic!("Expected BYE packet"),
        }
    }

    #[test]
    fn tolerant_parser_preserves_unknown_mixed_packets() {
        let rr = RtcpReceiverReport {
            ssrc: 0x12345678,
            report_blocks: Vec::new(),
        };
        let unknown = RtcpUnknownPacket {
            packet_type: 205,
            count: 1,
            payload: Bytes::from_static(&[0xaa, 0xbb, 0xcc, 0xdd]),
            padding: Bytes::new(),
        };
        let bye = RtcpGoodbye::new_for_source(0x12345678);
        let compound = RtcpTolerantCompoundPacket {
            packets: vec![
                RtcpCompoundMember::Known(RtcpPacket::ReceiverReport(rr)),
                RtcpCompoundMember::Unknown(unknown.clone()),
                RtcpCompoundMember::Known(RtcpPacket::Goodbye(bye)),
            ],
        };

        let wire = compound.serialize().unwrap();
        let parsed = RtcpCompoundPacket::parse_tolerant(&wire).unwrap();
        assert_eq!(parsed.packets.len(), 3);
        assert_eq!(parsed.packets[1], RtcpCompoundMember::Unknown(unknown));
        assert!(matches!(
            parsed.packets[2],
            RtcpCompoundMember::Known(RtcpPacket::Goodbye(_))
        ));
        assert_eq!(parsed.serialize().unwrap(), wire);

        // The compatibility-preserving strict parser still reports the
        // unimplemented member rather than changing RtcpPacket's variants.
        assert!(RtcpCompoundPacket::parse(&wire).is_err());
    }

    #[test]
    fn tolerant_mixed_parser_decodes_sdes_and_preserves_unknown_member() {
        let rr = RtcpReceiverReport {
            ssrc: 0x12345678,
            report_blocks: Vec::new(),
        };
        let mut sdes = RtcpSourceDescription::new();
        let mut chunk = super::super::RtcpSdesChunk::new(0x12345678);
        chunk.add_item(RtcpSdesItem::new(
            RtcpSdesItemType::CName,
            "alice@example.test".to_string(),
        ));
        sdes.add_chunk(chunk);
        let unknown = RtcpUnknownPacket {
            packet_type: 205,
            count: 1,
            payload: Bytes::from_static(&[0xaa, 0xbb, 0xcc, 0xdd]),
            padding: Bytes::new(),
        };
        let compound = RtcpTolerantCompoundPacket {
            packets: vec![
                RtcpCompoundMember::Known(RtcpPacket::ReceiverReport(rr)),
                RtcpCompoundMember::Known(RtcpPacket::SourceDescription(sdes)),
                RtcpCompoundMember::Unknown(unknown),
            ],
        };

        let parsed = RtcpTolerantCompoundPacket::parse(&compound.serialize().unwrap()).unwrap();
        assert!(matches!(
            &parsed.packets[1],
            RtcpCompoundMember::Known(RtcpPacket::SourceDescription(description))
                if description.find_cname(0x12345678) == Some("alice@example.test")
        ));
        assert!(matches!(
            parsed.packets[2],
            RtcpCompoundMember::Unknown(RtcpUnknownPacket {
                packet_type: 205,
                ..
            })
        ));
    }

    #[test]
    fn malformed_compound_packets_remain_errors() {
        let rr = [0x80, 201, 0, 1, 0x12, 0x34, 0x56, 0x78];

        let mut bad_version = rr.to_vec();
        bad_version.extend_from_slice(&[0x40, 205, 0, 0]);
        assert!(RtcpTolerantCompoundPacket::parse(&bad_version).is_err());

        let mut trailing_bytes = rr.to_vec();
        trailing_bytes.extend_from_slice(&[0x80, 205, 0]);
        assert!(RtcpTolerantCompoundPacket::parse(&trailing_bytes).is_err());

        // A padded unknown packet followed by another packet is invalid even
        // though both individual lengths are well formed.
        let mut non_final_padding = rr.to_vec();
        non_final_padding.extend_from_slice(&[0xa0, 205, 0, 1, 0, 0, 0, 4]);
        non_final_padding.extend_from_slice(&[0x80, 206, 0, 0]);
        assert!(RtcpTolerantCompoundPacket::parse(&non_final_padding).is_err());
    }

    #[test]
    fn final_padded_unknown_packet_round_trips_exactly() {
        let rr = RtcpReceiverReport {
            ssrc: 0x12345678,
            report_blocks: Vec::new(),
        };
        let unknown = RtcpUnknownPacket {
            packet_type: 205,
            count: 7,
            payload: Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
            padding: Bytes::from_static(&[0, 0, 0, 4]),
        };
        let compound = RtcpTolerantCompoundPacket {
            packets: vec![
                RtcpCompoundMember::Known(RtcpPacket::ReceiverReport(rr)),
                RtcpCompoundMember::Unknown(unknown.clone()),
            ],
        };

        let wire = compound.serialize().unwrap();
        let parsed = RtcpTolerantCompoundPacket::parse(&wire).unwrap();
        assert_eq!(parsed.packets[1], RtcpCompoundMember::Unknown(unknown));
        assert_eq!(parsed.serialize().unwrap(), wire);
    }
}
