//! RTCP Packet module
//!
//! This module provides structures for handling RTCP packets as defined in RFC 3550.
//! It includes implementations for different RTCP packet types: SR, RR, SDES, BYE, APP.
//! Extended Reports (XR) are defined in RFC 3611.

use bytes::{BufMut, Bytes, BytesMut};

use crate::error::Error;
use crate::Result;

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
            _ => Err(Error::RtcpError(format!(
                "Unknown RTCP packet type: {}",
                value
            ))),
        }
    }
}

// Import and re-export types from submodules
mod app;
mod bye;
mod compound;
mod ntp;
mod receiver_report;
mod report_block;
mod sdes;
mod sender_report;
mod xr;

// Re-export all public types
pub use app::RtcpApplicationDefined;
pub use bye::RtcpGoodbye;
pub use compound::{RtcpCompoundMember, RtcpCompoundPacket, RtcpTolerantCompoundPacket};
pub use ntp::NtpTimestamp;
pub use receiver_report::RtcpReceiverReport;
pub use report_block::RtcpReportBlock;
pub use sdes::{RtcpSdesChunk, RtcpSdesItem, RtcpSdesItemType, RtcpSourceDescription};
pub use sender_report::RtcpSenderReport;
pub use xr::{
    ReceiverReferenceTimeBlock, RtcpExtendedReport, RtcpXrBlock, RtcpXrBlockType, VoipMetricsBlock,
};

/// A well-formed RTCP packet whose packet type is not implemented.
///
/// Both the body and any RTCP padding are retained so tolerant compound
/// parsing can round-trip packets such as RTPFB/PSFB without interpreting
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpUnknownPacket {
    /// Raw RTCP packet type byte.
    pub packet_type: u8,
    /// The five count/subtype bits from the common header.
    pub count: u8,
    /// Packet body, excluding the common header and RTCP padding.
    pub payload: Bytes,
    /// Exact RTCP padding bytes. Empty when the padding bit was clear.
    pub padding: Bytes,
}

#[derive(Debug)]
struct ParsedRtcpEnvelope {
    packet_type: u8,
    count: u8,
    payload: Bytes,
    padding: Bytes,
}

fn parse_rtcp_envelope(data: &[u8]) -> Result<ParsedRtcpEnvelope> {
    if data.len() < 4 {
        return Err(Error::BufferTooSmall {
            required: 4,
            available: data.len(),
        });
    }

    let first_byte = data[0];
    let version = (first_byte >> 6) & 0x03;
    if version != RTCP_VERSION {
        return Err(Error::RtcpError(format!(
            "Invalid RTCP version: {}",
            version
        )));
    }

    let has_padding = first_byte & 0x20 != 0;
    let count = first_byte & 0x1f;
    let packet_type = data[1];
    let length_words_minus_one = u16::from_be_bytes([data[2], data[3]]) as usize;
    let packet_size = (length_words_minus_one + 1) * 4;
    if data.len() < packet_size {
        return Err(Error::BufferTooSmall {
            required: packet_size,
            available: data.len(),
        });
    }
    if data.len() > packet_size {
        return Err(Error::RtcpError(format!(
            "RTCP packet length declares {} bytes but {} bytes were supplied",
            packet_size,
            data.len()
        )));
    }

    let mut body_end = packet_size;
    let padding = if has_padding {
        if packet_size == 4 {
            return Err(Error::RtcpError(
                "RTCP padding flag is set on a packet with no body".to_string(),
            ));
        }

        let padding_size = data[packet_size - 1] as usize;
        let body_size = packet_size - 4;
        if padding_size == 0 || padding_size > body_size {
            return Err(Error::RtcpError(format!(
                "Invalid RTCP padding length {} for {}-byte body",
                padding_size, body_size
            )));
        }
        body_end -= padding_size;
        Bytes::copy_from_slice(&data[body_end..packet_size])
    } else {
        Bytes::new()
    };

    Ok(ParsedRtcpEnvelope {
        packet_type,
        count,
        payload: Bytes::copy_from_slice(&data[4..body_end]),
        padding,
    })
}

impl RtcpUnknownPacket {
    /// Parse and preserve a well-formed RTCP packet type not implemented by
    /// this crate.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let envelope = parse_rtcp_envelope(data)?;
        if RtcpPacketType::try_from(envelope.packet_type).is_ok() {
            return Err(Error::InvalidParameter(format!(
                "RTCP packet type {} is implemented and is not unknown",
                envelope.packet_type
            )));
        }

        Ok(Self {
            packet_type: envelope.packet_type,
            count: envelope.count,
            payload: envelope.payload,
            padding: envelope.padding,
        })
    }

    /// Serialize the preserved packet without interpreting its payload.
    pub fn serialize(&self) -> Result<Bytes> {
        if RtcpPacketType::try_from(self.packet_type).is_ok() {
            return Err(Error::InvalidParameter(format!(
                "RTCP packet type {} is implemented and cannot use RtcpUnknownPacket",
                self.packet_type
            )));
        }
        if self.count > 31 {
            return Err(Error::InvalidParameter(format!(
                "RTCP count/subtype {} exceeds five bits",
                self.count
            )));
        }
        if !self.padding.is_empty()
            && (self.padding.len() > u8::MAX as usize
                || self.padding[self.padding.len() - 1] as usize != self.padding.len())
        {
            return Err(Error::InvalidParameter(
                "Unknown RTCP packet has malformed padding".to_string(),
            ));
        }

        let packet_size = 4 + self.payload.len() + self.padding.len();
        if packet_size % 4 != 0 {
            return Err(Error::InvalidParameter(format!(
                "Unknown RTCP packet size {} is not 32-bit aligned",
                packet_size
            )));
        }
        let words_minus_one = packet_size / 4 - 1;
        if words_minus_one > u16::MAX as usize {
            return Err(Error::InvalidParameter(
                "Unknown RTCP packet is too large".to_string(),
            ));
        }

        let mut buf = BytesMut::with_capacity(packet_size);
        let padding_bit = u8::from(!self.padding.is_empty()) << 5;
        buf.put_u8((RTCP_VERSION << 6) | padding_bit | self.count);
        buf.put_u8(self.packet_type);
        buf.put_u16(words_minus_one as u16);
        buf.extend_from_slice(&self.payload);
        buf.extend_from_slice(&self.padding);
        Ok(buf.freeze())
    }

    fn has_padding(&self) -> bool {
        !self.padding.is_empty()
    }
}

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
        let envelope = parse_rtcp_envelope(data)?;
        let report_count = envelope.count;
        let packet_type = RtcpPacketType::try_from(envelope.packet_type)?;
        let mut buf = envelope.payload;

        // Parse specific packet type
        match packet_type {
            RtcpPacketType::SenderReport => {
                let report = sender_report::parse_sender_report(&mut buf, report_count)?;
                if !buf.is_empty() {
                    return Err(Error::RtcpError(format!(
                        "Sender Report has {} unexpected trailing bytes",
                        buf.len()
                    )));
                }
                Ok(RtcpPacket::SenderReport(report))
            }
            RtcpPacketType::ReceiverReport => {
                let report = receiver_report::parse_receiver_report(&mut buf, report_count)?;
                if !buf.is_empty() {
                    return Err(Error::RtcpError(format!(
                        "Receiver Report has {} unexpected trailing bytes",
                        buf.len()
                    )));
                }
                Ok(RtcpPacket::ReceiverReport(report))
            }
            RtcpPacketType::SourceDescription => Ok(RtcpPacket::SourceDescription(
                sdes::parse_sdes(&buf, report_count)?,
            )),
            RtcpPacketType::Goodbye => {
                let goodbye = bye::parse_bye(&mut buf, report_count)?;
                if !buf.is_empty() {
                    return Err(Error::RtcpError(format!(
                        "BYE packet has {} unexpected trailing bytes",
                        buf.len()
                    )));
                }
                Ok(RtcpPacket::Goodbye(goodbye))
            }
            RtcpPacketType::ApplicationDefined => {
                Ok(RtcpPacket::ApplicationDefined(app::parse_app(&mut buf)?))
            }
            RtcpPacketType::ExtendedReport => {
                Ok(RtcpPacket::ExtendedReport(xr::parse_xr(&mut buf)?))
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
                let first_byte =
                    (RTCP_VERSION << 6) | (0 << 5) | (sr.report_blocks.len() as u8 & 0x1F);
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
                let first_byte =
                    (RTCP_VERSION << 6) | (0 << 5) | (rr.report_blocks.len() as u8 & 0x1F);
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
        assert_eq!(
            RtcpPacketType::try_from(200).unwrap(),
            RtcpPacketType::SenderReport
        );
        assert_eq!(
            RtcpPacketType::try_from(201).unwrap(),
            RtcpPacketType::ReceiverReport
        );
        assert_eq!(
            RtcpPacketType::try_from(202).unwrap(),
            RtcpPacketType::SourceDescription
        );
        assert_eq!(
            RtcpPacketType::try_from(203).unwrap(),
            RtcpPacketType::Goodbye
        );
        assert_eq!(
            RtcpPacketType::try_from(204).unwrap(),
            RtcpPacketType::ApplicationDefined
        );
        assert_eq!(
            RtcpPacketType::try_from(207).unwrap(),
            RtcpPacketType::ExtendedReport
        );

        assert!(RtcpPacketType::try_from(100).is_err());
    }

    #[test]
    fn unknown_packet_is_preserved() {
        let bytes = [0x85, 205, 0, 2, 1, 2, 3, 4, 5, 6, 7, 8];
        let parsed = RtcpUnknownPacket::parse(&bytes).unwrap();

        assert_eq!(parsed.packet_type, 205);
        assert_eq!(parsed.count, 5);
        assert_eq!(&parsed.payload[..], &bytes[4..]);
        assert!(parsed.padding.is_empty());
        assert_eq!(&parsed.serialize().unwrap()[..], &bytes);

        // The legacy single-packet parser remains strict, so adding tolerant
        // compound parsing does not add a variant to the public RtcpPacket
        // enum and break downstream exhaustive matches.
        assert!(RtcpPacket::parse(&bytes).is_err());
    }

    #[test]
    fn malformed_rtcp_padding_is_rejected() {
        assert!(RtcpUnknownPacket::parse(&[0xa0, 205, 0, 1, 1, 2, 3, 0]).is_err());
        assert!(RtcpUnknownPacket::parse(&[0xa0, 205, 0, 1, 1, 2, 3, 5]).is_err());
        assert!(RtcpUnknownPacket::parse(&[0xa0, 205, 0, 0]).is_err());
    }

    #[test]
    fn declared_packet_length_must_match_input() {
        let rr_with_trailing_packet = [0x80, 201, 0, 1, 0x12, 0x34, 0x56, 0x78, 0x80, 205, 0, 0];
        assert!(RtcpPacket::parse(&rr_with_trailing_packet).is_err());

        // RR declares a second body word that is not a report block.
        let rr_with_internal_trailing_word = [0x80, 201, 0, 2, 0x12, 0x34, 0x56, 0x78, 0, 0, 0, 0];
        assert!(RtcpPacket::parse(&rr_with_internal_trailing_word).is_err());
    }

    #[test]
    fn malformed_known_sdes_member_is_rejected() {
        // The header declares one SDES chunk, but the body contains no SSRC.
        assert!(RtcpPacket::parse(&[0x81, 202, 0, 0]).is_err());

        // The SSRC is present, but the mandatory END item is missing.
        assert!(RtcpPacket::parse(&[0x81, 202, 0, 1, 0x12, 0x34, 0x56, 0x78,]).is_err());
    }
}
