//! Tolerant, streaming traversal of a compound RTCP packet.
//!
//! [`RtcpCompoundPacket::parse`](super::RtcpCompoundPacket::parse) is strict:
//! every sub-packet must be a type [`RtcpPacket`] knows how to parse, or the
//! whole compound fails with an error. That is correct for a well-formed
//! outgoing compound built by this crate, but a caller reading an inbound
//! compound from the wire may see RTCP types this crate has no variant for
//! yet, most commonly RTPFB (PT 205) or PSFB (PT 206) feedback. Today those
//! abort parsing before any later SR/RR in the same compound is reached.
//!
//! [`RtcpPacketIter`] walks the same wire format sub-packet by sub-packet,
//! but only requires each one's common RTCP header (version + length) to be
//! valid. Known types are parsed and yielded as [`RtcpPacketItem::Known`];
//! anything else is yielded as [`RtcpPacketItem::Unknown`] with its raw
//! bytes, so the caller can skip it and keep walking toward the next SR/RR.
//!
//! This does not change [`RtcpPacket`] or [`RtcpCompoundPacket`]'s existing
//! behavior; it is a separate, additive way to read the same bytes.

use bytes::Bytes;

use super::{RtcpPacket, RtcpPacketType, RTCP_VERSION};
use crate::error::Error;
use crate::Result;

/// One sub-packet read out of a compound RTCP buffer by [`RtcpPacketIter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtcpPacketItem {
    /// A sub-packet of a type [`RtcpPacket::parse`] understands.
    Known(RtcpPacket),

    /// A sub-packet whose RTCP header is well-formed but whose packet type
    /// this crate doesn't model (e.g. RTPFB/PT 205, PSFB/PT 206). Carries
    /// the raw bytes of the sub-packet, header included, so a caller that
    /// does understand the type can parse it itself.
    Unknown { packet_type: u8, data: Bytes },
}

/// Iterates the sub-packets of a compound RTCP buffer, tolerating packet
/// types this crate doesn't model instead of aborting on them.
///
/// Stops (and does not yield anything further) after the first sub-packet
/// with an invalid RTCP version or a length that doesn't fit in the
/// remaining buffer; that byte range is not recoverable RTCP framing.
pub struct RtcpPacketIter<'a> {
    buf: &'a [u8],
    /// Set once `next()` has returned an error, so the iterator stops
    /// instead of re-reading past a position it couldn't make sense of.
    done: bool,
}

impl<'a> RtcpPacketIter<'a> {
    /// Create an iterator over the compound RTCP sub-packets in `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { buf: data, done: false }
    }
}

impl<'a> Iterator for RtcpPacketIter<'a> {
    type Item = Result<RtcpPacketItem>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.buf.is_empty() {
            return None;
        }

        if self.buf.len() < 4 {
            self.done = true;
            return Some(Err(Error::BufferTooSmall {
                required: 4,
                available: self.buf.len(),
            }));
        }

        let first_byte = self.buf[0];
        let version = (first_byte >> 6) & 0x03;
        if version != RTCP_VERSION {
            self.done = true;
            return Some(Err(Error::RtcpError(format!(
                "Invalid RTCP version: {}",
                version
            ))));
        }

        let packet_type_byte = self.buf[1];
        let length = (u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize) * 4;
        let total_packet_size = 4 + length;

        if self.buf.len() < total_packet_size {
            self.done = true;
            return Some(Err(Error::BufferTooSmall {
                required: total_packet_size,
                available: self.buf.len(),
            }));
        }

        let packet_data = &self.buf[..total_packet_size];
        self.buf = &self.buf[total_packet_size..];

        match RtcpPacketType::try_from(packet_type_byte) {
            Ok(_) => match RtcpPacket::parse(packet_data) {
                Ok(packet) => Some(Ok(RtcpPacketItem::Known(packet))),
                Err(e) => {
                    self.done = true;
                    Some(Err(e))
                }
            },
            Err(_) => Some(Ok(RtcpPacketItem::Unknown {
                packet_type: packet_type_byte,
                data: Bytes::copy_from_slice(packet_data),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::rtcp::{NtpTimestamp, RtcpReceiverReport, RtcpSenderReport};

    /// Builds the raw bytes of a minimal RTCP Picture Loss Indication
    /// (PSFB, PT 206, FMT 1, RFC 4585 section 6.3.1): common header, sender
    /// SSRC, media source SSRC, no feedback control information.
    fn build_pli_bytes(sender_ssrc: u32, media_ssrc: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12);
        bytes.push((RTCP_VERSION << 6) | 1); // V=2, P=0, FMT=1
        bytes.push(206); // PT=PSFB
        bytes.extend_from_slice(&2u16.to_be_bytes()); // length = 2 (12 bytes total)
        bytes.extend_from_slice(&sender_ssrc.to_be_bytes());
        bytes.extend_from_slice(&media_ssrc.to_be_bytes());
        bytes
    }

    fn sr_bytes(ssrc: u32) -> Bytes {
        RtcpPacket::SenderReport(RtcpSenderReport {
            ssrc,
            ntp_timestamp: NtpTimestamp { seconds: 1, fraction: 2 },
            rtp_timestamp: 3,
            sender_packet_count: 4,
            sender_octet_count: 5,
            report_blocks: Vec::new(),
        })
        .serialize()
        .expect("serialize SR")
    }

    fn rr_bytes(ssrc: u32) -> Bytes {
        RtcpPacket::ReceiverReport(RtcpReceiverReport {
            ssrc,
            report_blocks: Vec::new(),
        })
        .serialize()
        .expect("serialize RR")
    }

    #[test]
    fn walks_past_unknown_feedback_type_to_reach_the_next_known_packet() {
        // SR -> PLI (PT 206, unmodeled) -> RR. RtcpCompoundPacket::parse
        // would fail on the PLI and never reach the RR; RtcpPacketIter
        // should yield all three.
        let mut compound = Vec::new();
        compound.extend_from_slice(&sr_bytes(0x1111_1111));
        compound.extend_from_slice(&build_pli_bytes(0x1111_1111, 0x2222_2222));
        compound.extend_from_slice(&rr_bytes(0x3333_3333));

        let items: Vec<_> = RtcpPacketIter::new(&compound)
            .collect::<Result<Vec<_>>>()
            .expect("all sub-packets should be readable");

        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], RtcpPacketItem::Known(RtcpPacket::SenderReport(sr)) if sr.ssrc == 0x1111_1111));
        assert!(matches!(
            &items[1],
            RtcpPacketItem::Unknown { packet_type: 206, .. }
        ));
        assert!(matches!(&items[2], RtcpPacketItem::Known(RtcpPacket::ReceiverReport(rr)) if rr.ssrc == 0x3333_3333));
    }

    #[test]
    fn unknown_item_preserves_raw_bytes_for_the_caller_to_reparse() {
        let pli = build_pli_bytes(0xaaaa_aaaa, 0xbbbb_bbbb);

        let mut iter = RtcpPacketIter::new(&pli);
        let item = iter.next().expect("one item").expect("readable");

        match item {
            RtcpPacketItem::Unknown { packet_type, data } => {
                assert_eq!(packet_type, 206);
                assert_eq!(data.as_ref(), pli.as_slice());
            }
            RtcpPacketItem::Known(_) => panic!("PSFB must not be classified as Known"),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn empty_buffer_yields_no_items() {
        assert!(RtcpPacketIter::new(&[]).next().is_none());
    }

    #[test]
    fn invalid_version_stops_iteration_with_an_error() {
        let mut bad = sr_bytes(1).to_vec();
        bad[0] = (1u8 << 6) | (bad[0] & 0x3F); // corrupt version to 1

        let mut iter = RtcpPacketIter::new(&bad);
        assert!(iter.next().expect("one item").is_err());
        assert!(iter.next().is_none(), "iterator must not continue after an error");
    }

    #[test]
    fn truncated_length_stops_iteration_with_an_error() {
        let sr = sr_bytes(1);
        let truncated = &sr[..sr.len() - 1];

        let mut iter = RtcpPacketIter::new(truncated);
        assert!(iter.next().expect("one item").is_err());
        assert!(iter.next().is_none());
    }

    #[test]
    fn matches_rtcp_compound_packet_behavior_for_known_only_compounds() {
        // For a compound with only known types, RtcpPacketIter and
        // RtcpCompoundPacket::parse must agree.
        let mut compound = Vec::new();
        compound.extend_from_slice(&sr_bytes(42));
        compound.extend_from_slice(&rr_bytes(43));

        let via_iter: Vec<_> = RtcpPacketIter::new(&compound)
            .map(|item| match item.expect("readable") {
                RtcpPacketItem::Known(p) => p,
                RtcpPacketItem::Unknown { .. } => panic!("unexpected unknown item"),
            })
            .collect();

        let via_compound = super::super::RtcpCompoundPacket::parse(&compound)
            .expect("strict parse should also succeed");

        assert_eq!(via_iter, via_compound.packets);
    }
}
