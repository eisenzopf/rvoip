//! Adapter layer between our RTP/RTCP types and the production `rtp`/`rtcp` crates
//! (webrtc-rs ecosystem, 3.8M+ downloads each).
//!
//! This module provides bidirectional conversion between:
//! - Our `RtpHeader` <-> `rtp::header::Header`
//! - Our `RtpPacket` <-> `rtp::packet::Packet`
//! - Our RTCP types <-> `rtcp::` types
//!
//! The adapter enables gradual migration: internal code can continue using our
//! types while gaining access to the battle-tested webrtc-rs implementations
//! at network boundaries.

use bytes::Bytes;
use tracing::warn;
use webrtc_util::marshal::{Marshal, MarshalSize, Unmarshal};

use crate::error::Error;
use crate::Result;
use super::header::RtpHeader;
use super::rtp::RtpPacket;
use super::extension::{ExtensionFormat, RtpHeaderExtensions, ExtensionElement};

// Re-export webrtc-rs types under a clear namespace
pub use rtp::header::Header as WebrtcRtpHeader;
pub use rtp::header::Extension as WebrtcRtpExtension;
pub use rtp::packet::Packet as WebrtcRtpPacket;

// ─── RTP Header Conversion ──────────────────────────────────────────────────

impl RtpHeader {
    /// Convert our `RtpHeader` into a webrtc-rs `rtp::header::Header`.
    pub fn to_webrtc(&self) -> WebrtcRtpHeader {
        let (extension_profile, extensions) = match &self.extensions {
            Some(ext) => {
                let profile = ext.profile_id;
                let exts: Vec<WebrtcRtpExtension> = ext.elements.iter().map(|e| {
                    WebrtcRtpExtension {
                        id: e.id,
                        payload: e.data.clone(),
                    }
                }).collect();
                (profile, exts)
            }
            None => (0, Vec::new()),
        };

        WebrtcRtpHeader {
            version: self.version,
            padding: self.padding,
            extension: self.extension,
            marker: self.marker,
            payload_type: self.payload_type,
            sequence_number: self.sequence_number,
            timestamp: self.timestamp,
            ssrc: self.ssrc,
            csrc: self.csrc.clone(),
            extension_profile,
            extensions,
            extensions_padding: 0,
        }
    }

    /// Create our `RtpHeader` from a webrtc-rs `rtp::header::Header`.
    pub fn from_webrtc(h: &WebrtcRtpHeader) -> Self {
        let extensions = if h.extension && !h.extensions.is_empty() {
            let format = super::extension::ExtensionFormat::from_extension_id(h.extension_profile);
            let elements: Vec<ExtensionElement> = h.extensions.iter().map(|e| {
                ExtensionElement {
                    id: e.id,
                    data: e.payload.clone(),
                }
            }).collect();
            Some(RtpHeaderExtensions {
                format,
                profile_id: h.extension_profile,
                elements,
            })
        } else if h.extension {
            // Extension flag set but no parsed extensions
            Some(RtpHeaderExtensions {
                format: ExtensionFormat::from_extension_id(h.extension_profile),
                profile_id: h.extension_profile,
                elements: Vec::new(),
            })
        } else {
            None
        };

        RtpHeader {
            version: h.version,
            padding: h.padding,
            extension: h.extension,
            cc: h.csrc.len() as u8,
            marker: h.marker,
            payload_type: h.payload_type,
            sequence_number: h.sequence_number,
            timestamp: h.timestamp,
            ssrc: h.ssrc,
            csrc: h.csrc.clone(),
            extensions,
        }
    }
}

// ─── RTP Packet Conversion ──────────────────────────────────────────────────

impl RtpPacket {
    /// Convert our `RtpPacket` into a webrtc-rs `rtp::packet::Packet`.
    pub fn to_webrtc(&self) -> WebrtcRtpPacket {
        WebrtcRtpPacket {
            header: self.header.to_webrtc(),
            payload: self.payload.clone(),
        }
    }

    /// Create our `RtpPacket` from a webrtc-rs `rtp::packet::Packet`.
    pub fn from_webrtc(p: &WebrtcRtpPacket) -> Self {
        RtpPacket {
            header: RtpHeader::from_webrtc(&p.header),
            payload: p.payload.clone(),
        }
    }

    /// Parse raw bytes using the webrtc-rs `rtp` crate's `Unmarshal` implementation.
    ///
    /// This delegates parsing to the production crate and converts the result
    /// to our internal type. Useful for validating our parser against a reference
    /// or as an alternative code path.
    pub fn parse_via_webrtc(data: &[u8]) -> Result<Self> {
        let mut buf = Bytes::copy_from_slice(data);
        let webrtc_packet = WebrtcRtpPacket::unmarshal(&mut buf)
            .map_err(|e| Error::InvalidPacket(format!("webrtc-rs rtp unmarshal failed: {}", e)))?;
        Ok(Self::from_webrtc(&webrtc_packet))
    }

    /// Serialize to bytes using the webrtc-rs `rtp` crate's `Marshal` implementation.
    ///
    /// Returns the raw bytes produced by the production crate. Useful for
    /// cross-validating serialization or as an alternative code path.
    pub fn serialize_via_webrtc(&self) -> Result<Bytes> {
        let webrtc_packet = self.to_webrtc();
        let size = webrtc_packet.marshal_size();
        let mut buf = vec![0u8; size];
        let n = webrtc_packet.marshal_to(&mut buf)
            .map_err(|e| Error::InvalidPacket(format!("webrtc-rs rtp marshal failed: {}", e)))?;
        buf.truncate(n);
        Ok(Bytes::from(buf))
    }
}

// ─── RTCP Adapter ───────────────────────────────────────────────────────────

/// Adapter for converting between our RTCP types and webrtc-rs `rtcp` crate types.
///
/// The webrtc-rs `rtcp` crate uses trait objects (`Box<dyn rtcp::packet::Packet>`)
/// while we use an enum `RtcpPacket`. This module provides conversion at
/// serialization boundaries.
pub mod rtcp_adapter {
    use bytes::Bytes;
    use crate::error::Error;
    use crate::Result;
    use crate::packet::rtcp::{
        RtcpPacket, RtcpSenderReport, RtcpReceiverReport,
        RtcpReportBlock, RtcpGoodbye, NtpTimestamp,
    };

    /// Parse raw RTCP bytes using the webrtc-rs `rtcp` crate.
    ///
    /// Returns the parsed packets as webrtc-rs trait objects. This is primarily
    /// useful for validation and interoperability testing rather than replacing
    /// our internal RTCP parsing, since the webrtc-rs types use a trait-object
    /// approach that differs from our enum-based design.
    pub fn parse_rtcp_via_webrtc(data: &[u8]) -> Result<Vec<Box<dyn rtcp::packet::Packet + Send + Sync>>> {
        let mut buf = Bytes::copy_from_slice(data);
        let packets = rtcp::packet::unmarshal(&mut buf)
            .map_err(|e| Error::RtcpError(format!("webrtc-rs rtcp unmarshal failed: {}", e)))?;
        Ok(packets)
    }

    /// Serialize a webrtc-rs RTCP packet to bytes.
    pub fn serialize_rtcp_webrtc(packets: &[Box<dyn rtcp::packet::Packet + Send + Sync>]) -> Result<Bytes> {
        let data = rtcp::packet::marshal(packets)
            .map_err(|e| Error::RtcpError(format!("webrtc-rs rtcp marshal failed: {}", e)))?;
        Ok(data)
    }

    /// Convert our `RtcpSenderReport` to a webrtc-rs `SenderReport`.
    pub fn sender_report_to_webrtc(sr: &RtcpSenderReport) -> rtcp::sender_report::SenderReport {
        let reports: Vec<rtcp::reception_report::ReceptionReport> = sr.report_blocks.iter().map(|rb| {
            rtcp::reception_report::ReceptionReport {
                ssrc: rb.ssrc,
                fraction_lost: rb.fraction_lost,
                total_lost: rb.cumulative_lost,
                last_sequence_number: rb.highest_seq,
                jitter: rb.jitter,
                last_sender_report: rb.last_sr,
                delay: rb.delay_since_last_sr,
            }
        }).collect();

        rtcp::sender_report::SenderReport {
            ssrc: sr.ssrc,
            ntp_time: sr.ntp_timestamp.to_u64(),
            rtp_time: sr.rtp_timestamp,
            packet_count: sr.sender_packet_count,
            octet_count: sr.sender_octet_count,
            reports,
            ..Default::default()
        }
    }

    /// Convert our `RtcpReceiverReport` to a webrtc-rs `ReceiverReport`.
    pub fn receiver_report_to_webrtc(rr: &RtcpReceiverReport) -> rtcp::receiver_report::ReceiverReport {
        let reports: Vec<rtcp::reception_report::ReceptionReport> = rr.report_blocks.iter().map(|rb| {
            rtcp::reception_report::ReceptionReport {
                ssrc: rb.ssrc,
                fraction_lost: rb.fraction_lost,
                total_lost: rb.cumulative_lost,
                last_sequence_number: rb.highest_seq,
                jitter: rb.jitter,
                last_sender_report: rb.last_sr,
                delay: rb.delay_since_last_sr,
            }
        }).collect();

        rtcp::receiver_report::ReceiverReport {
            ssrc: rr.ssrc,
            reports,
            ..Default::default()
        }
    }

    /// Convert our `RtcpGoodbye` to a webrtc-rs `Goodbye`.
    pub fn goodbye_to_webrtc(bye: &RtcpGoodbye) -> rtcp::goodbye::Goodbye {
        rtcp::goodbye::Goodbye {
            sources: bye.sources.clone(),
            reason: if let Some(ref r) = bye.reason {
                Bytes::copy_from_slice(r.as_bytes())
            } else {
                Bytes::new()
            },
        }
    }

    /// Convert a webrtc-rs `SenderReport` to our `RtcpSenderReport`.
    pub fn sender_report_from_webrtc(sr: &rtcp::sender_report::SenderReport) -> RtcpSenderReport {
        let report_blocks: Vec<RtcpReportBlock> = sr.reports.iter().map(|rb| {
            RtcpReportBlock {
                ssrc: rb.ssrc,
                fraction_lost: rb.fraction_lost,
                cumulative_lost: rb.total_lost,
                highest_seq: rb.last_sequence_number,
                jitter: rb.jitter,
                last_sr: rb.last_sender_report,
                delay_since_last_sr: rb.delay,
            }
        }).collect();

        RtcpSenderReport {
            ssrc: sr.ssrc,
            ntp_timestamp: NtpTimestamp::from_u64(sr.ntp_time),
            rtp_timestamp: sr.rtp_time,
            sender_packet_count: sr.packet_count,
            sender_octet_count: sr.octet_count,
            report_blocks,
        }
    }

    /// Convert a webrtc-rs `ReceiverReport` to our `RtcpReceiverReport`.
    pub fn receiver_report_from_webrtc(rr: &rtcp::receiver_report::ReceiverReport) -> RtcpReceiverReport {
        let report_blocks: Vec<RtcpReportBlock> = rr.reports.iter().map(|rb| {
            RtcpReportBlock {
                ssrc: rb.ssrc,
                fraction_lost: rb.fraction_lost,
                cumulative_lost: rb.total_lost,
                highest_seq: rb.last_sequence_number,
                jitter: rb.jitter,
                last_sr: rb.last_sender_report,
                delay_since_last_sr: rb.delay,
            }
        }).collect();

        RtcpReceiverReport {
            ssrc: rr.ssrc,
            report_blocks,
        }
    }

    /// Convert a webrtc-rs `Goodbye` to our `RtcpGoodbye`.
    pub fn goodbye_from_webrtc(bye: &rtcp::goodbye::Goodbye) -> RtcpGoodbye {
        RtcpGoodbye {
            sources: bye.sources.clone(),
            reason: if bye.reason.is_empty() {
                None
            } else {
                String::from_utf8(bye.reason.to_vec()).ok()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_rtp_header_roundtrip_via_webrtc() {
        let original = RtpHeader::new(96, 1000, 12345, 0xabcdef01);

        // Convert to webrtc-rs and back
        let webrtc_header = original.to_webrtc();
        let roundtripped = RtpHeader::from_webrtc(&webrtc_header);

        assert_eq!(roundtripped.version, original.version);
        assert_eq!(roundtripped.padding, original.padding);
        assert_eq!(roundtripped.extension, original.extension);
        assert_eq!(roundtripped.marker, original.marker);
        assert_eq!(roundtripped.payload_type, original.payload_type);
        assert_eq!(roundtripped.sequence_number, original.sequence_number);
        assert_eq!(roundtripped.timestamp, original.timestamp);
        assert_eq!(roundtripped.ssrc, original.ssrc);
        assert_eq!(roundtripped.csrc, original.csrc);
    }

    #[test]
    fn test_rtp_header_with_csrc_roundtrip() {
        let mut original = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        original.add_csrc(0x11111111);
        original.add_csrc(0x22222222);

        let webrtc_header = original.to_webrtc();
        let roundtripped = RtpHeader::from_webrtc(&webrtc_header);

        assert_eq!(roundtripped.cc, 2);
        assert_eq!(roundtripped.csrc, vec![0x11111111, 0x22222222]);
    }

    #[test]
    fn test_rtp_header_with_extension_roundtrip() {
        let mut original = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        // Use one-byte extension format
        let mut ext = RtpHeaderExtensions::new_one_byte();
        ext.add_extension(1, vec![1u8, 2, 3, 4]).unwrap_or_default();
        ext.add_extension(2, vec![5u8, 6]).unwrap_or_default();
        original.extensions = Some(ext);
        original.extension = true;

        let webrtc_header = original.to_webrtc();

        // Verify webrtc header has correct extension profile
        assert_eq!(webrtc_header.extension_profile, 0xBEDE);
        assert_eq!(webrtc_header.extensions.len(), 2);
        assert_eq!(webrtc_header.extensions[0].id, 1);
        assert_eq!(webrtc_header.extensions[0].payload, Bytes::from_static(&[1, 2, 3, 4]));

        let roundtripped = RtpHeader::from_webrtc(&webrtc_header);
        assert!(roundtripped.extension);
        let rt_ext = roundtripped.extensions.as_ref().unwrap_or_else(|| panic!("expected extensions"));
        assert_eq!(rt_ext.elements.len(), 2);
        assert_eq!(rt_ext.elements[0].id, 1);
        assert_eq!(rt_ext.elements[0].data, Bytes::from_static(&[1, 2, 3, 4]));
    }

    #[test]
    fn test_rtp_packet_roundtrip_via_webrtc() {
        let payload = Bytes::from_static(b"test payload data");
        let original = RtpPacket::new_with_payload(96, 1000, 12345, 0xabcdef01, payload.clone());

        let webrtc_pkt = original.to_webrtc();
        let roundtripped = RtpPacket::from_webrtc(&webrtc_pkt);

        assert_eq!(roundtripped.header.payload_type, 96);
        assert_eq!(roundtripped.header.sequence_number, 1000);
        assert_eq!(roundtripped.header.timestamp, 12345);
        assert_eq!(roundtripped.header.ssrc, 0xabcdef01);
        assert_eq!(roundtripped.payload, payload);
    }

    #[test]
    fn test_parse_via_webrtc() {
        // Create a packet with our code, serialize it, then parse with webrtc-rs
        let payload = Bytes::from_static(b"test payload data");
        let original = RtpPacket::new_with_payload(96, 1000, 12345, 0xabcdef01, payload.clone());
        let serialized = original.serialize().unwrap_or_else(|e| panic!("serialize failed: {}", e));

        let parsed = RtpPacket::parse_via_webrtc(&serialized)
            .unwrap_or_else(|e| panic!("parse_via_webrtc failed: {}", e));

        assert_eq!(parsed.header.payload_type, 96);
        assert_eq!(parsed.header.sequence_number, 1000);
        assert_eq!(parsed.header.timestamp, 12345);
        assert_eq!(parsed.header.ssrc, 0xabcdef01);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn test_serialize_via_webrtc() {
        // Create a packet, serialize with webrtc-rs, then parse with our code
        let payload = Bytes::from_static(b"test payload data");
        let original = RtpPacket::new_with_payload(96, 1000, 12345, 0xabcdef01, payload.clone());

        let serialized = original.serialize_via_webrtc()
            .unwrap_or_else(|e| panic!("serialize_via_webrtc failed: {}", e));

        let parsed = RtpPacket::parse(&serialized)
            .unwrap_or_else(|e| panic!("parse failed: {}", e));

        assert_eq!(parsed.header.payload_type, 96);
        assert_eq!(parsed.header.sequence_number, 1000);
        assert_eq!(parsed.header.timestamp, 12345);
        assert_eq!(parsed.header.ssrc, 0xabcdef01);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn test_cross_serialization_compatibility() {
        // Verify that our serialization and webrtc-rs serialization produce
        // equivalent results (both parseable by both implementations)
        let payload = Bytes::from_static(b"cross-compat test");
        let original = RtpPacket::new_with_payload(0, 64000, 160000, 0x12345678, payload);

        let our_bytes = original.serialize()
            .unwrap_or_else(|e| panic!("our serialize failed: {}", e));
        let webrtc_bytes = original.serialize_via_webrtc()
            .unwrap_or_else(|e| panic!("webrtc serialize failed: {}", e));

        // Both should parse successfully with both parsers
        let _ = RtpPacket::parse(&our_bytes)
            .unwrap_or_else(|e| panic!("parse our_bytes failed: {}", e));
        let _ = RtpPacket::parse(&webrtc_bytes)
            .unwrap_or_else(|e| panic!("parse webrtc_bytes failed: {}", e));
        let _ = RtpPacket::parse_via_webrtc(&our_bytes)
            .unwrap_or_else(|e| panic!("parse_via_webrtc our_bytes failed: {}", e));
        let _ = RtpPacket::parse_via_webrtc(&webrtc_bytes)
            .unwrap_or_else(|e| panic!("parse_via_webrtc webrtc_bytes failed: {}", e));

        // For a basic packet without extensions, the output should be identical
        assert_eq!(our_bytes, webrtc_bytes,
            "Basic packet serialization differs between our impl and webrtc-rs");
    }

    #[test]
    fn test_rtp_packet_with_marker_roundtrip() {
        let mut header = RtpHeader::new(111, 50000, 3200, 0xDEADBEEF);
        header.marker = true;
        let packet = RtpPacket::new(header, Bytes::from_static(b"marker test"));

        let webrtc_pkt = packet.to_webrtc();
        assert!(webrtc_pkt.header.marker);

        let roundtripped = RtpPacket::from_webrtc(&webrtc_pkt);
        assert!(roundtripped.header.marker);
        assert_eq!(roundtripped.header.payload_type, 111);
    }

    // ── RTCP adapter roundtrip tests ────────────────────────────────────

    #[test]
    fn test_rtcp_sender_report_roundtrip_via_adapter() {
        use crate::packet::adapter::rtcp_adapter::*;
        use crate::packet::rtcp::{RtcpSenderReport, RtcpReportBlock, NtpTimestamp};

        // Build our SenderReport with non-trivial fields
        let mut sr = RtcpSenderReport::new(0xAABBCCDD);
        sr.ntp_timestamp = NtpTimestamp { seconds: 0x11223344, fraction: 0x55667788 };
        sr.rtp_timestamp = 0xDEAD_BEEF;
        sr.sender_packet_count = 4200;
        sr.sender_octet_count = 672000;
        sr.add_report_block(RtcpReportBlock {
            ssrc: 0x12345678,
            fraction_lost: 25,
            cumulative_lost: 300,
            highest_seq: 65000,
            jitter: 42,
            last_sr: 0xABCD_1234,
            delay_since_last_sr: 5000,
        });

        // Convert ours -> webrtc
        let webrtc_sr = sender_report_to_webrtc(&sr);

        // Convert webrtc -> ours
        let roundtripped = sender_report_from_webrtc(&webrtc_sr);

        // Verify every field survives the roundtrip
        assert_eq!(roundtripped.ssrc, sr.ssrc);
        assert_eq!(roundtripped.ntp_timestamp, sr.ntp_timestamp);
        assert_eq!(roundtripped.rtp_timestamp, sr.rtp_timestamp);
        assert_eq!(roundtripped.sender_packet_count, sr.sender_packet_count);
        assert_eq!(roundtripped.sender_octet_count, sr.sender_octet_count);
        assert_eq!(roundtripped.report_blocks.len(), 1);
        let rb = &roundtripped.report_blocks[0];
        let orig_rb = &sr.report_blocks[0];
        assert_eq!(rb.ssrc, orig_rb.ssrc);
        assert_eq!(rb.fraction_lost, orig_rb.fraction_lost);
        assert_eq!(rb.cumulative_lost, orig_rb.cumulative_lost);
        assert_eq!(rb.highest_seq, orig_rb.highest_seq);
        assert_eq!(rb.jitter, orig_rb.jitter);
        assert_eq!(rb.last_sr, orig_rb.last_sr);
        assert_eq!(rb.delay_since_last_sr, orig_rb.delay_since_last_sr);
    }

    #[test]
    fn test_rtcp_receiver_report_roundtrip_via_adapter() {
        use crate::packet::adapter::rtcp_adapter::*;
        use crate::packet::rtcp::{RtcpReceiverReport, RtcpReportBlock};

        let mut rr = RtcpReceiverReport::new(0x99887766);
        rr.add_report_block(RtcpReportBlock {
            ssrc: 0xFEDCBA98,
            fraction_lost: 128,
            cumulative_lost: 10_000,
            highest_seq: 200_000,
            jitter: 500,
            last_sr: 0x1111_2222,
            delay_since_last_sr: 7500,
        });
        rr.add_report_block(RtcpReportBlock {
            ssrc: 0x55443322,
            fraction_lost: 0,
            cumulative_lost: 0,
            highest_seq: 100,
            jitter: 1,
            last_sr: 0,
            delay_since_last_sr: 0,
        });

        let webrtc_rr = receiver_report_to_webrtc(&rr);
        let roundtripped = receiver_report_from_webrtc(&webrtc_rr);

        assert_eq!(roundtripped.ssrc, rr.ssrc);
        assert_eq!(roundtripped.report_blocks.len(), 2);
        for (rt, orig) in roundtripped.report_blocks.iter().zip(rr.report_blocks.iter()) {
            assert_eq!(rt.ssrc, orig.ssrc);
            assert_eq!(rt.fraction_lost, orig.fraction_lost);
            assert_eq!(rt.cumulative_lost, orig.cumulative_lost);
            assert_eq!(rt.highest_seq, orig.highest_seq);
            assert_eq!(rt.jitter, orig.jitter);
            assert_eq!(rt.last_sr, orig.last_sr);
            assert_eq!(rt.delay_since_last_sr, orig.delay_since_last_sr);
        }
    }

    #[test]
    fn test_rtcp_goodbye_roundtrip_via_adapter() {
        use crate::packet::adapter::rtcp_adapter::*;
        use crate::packet::rtcp::RtcpGoodbye;

        // Goodbye with multiple sources and a reason
        let bye = RtcpGoodbye {
            sources: vec![0x11111111, 0x22222222, 0x33333333],
            reason: Some("session ending".to_string()),
        };

        let webrtc_bye = goodbye_to_webrtc(&bye);
        let roundtripped = goodbye_from_webrtc(&webrtc_bye);

        assert_eq!(roundtripped.sources, bye.sources);
        assert_eq!(roundtripped.reason, bye.reason);

        // Also test the no-reason variant
        let bye_no_reason = RtcpGoodbye {
            sources: vec![0xAAAA_BBBB],
            reason: None,
        };
        let webrtc_bye2 = goodbye_to_webrtc(&bye_no_reason);
        let roundtripped2 = goodbye_from_webrtc(&webrtc_bye2);

        assert_eq!(roundtripped2.sources, bye_no_reason.sources);
        assert_eq!(roundtripped2.reason, None);
    }
}
