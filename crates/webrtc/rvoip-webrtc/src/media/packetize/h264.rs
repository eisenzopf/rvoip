//! D3c — RFC 6184 H.264 RTP packetizer.
//!
//! Splits one H.264 access unit (a sequence of NAL units from a single
//! encoded video frame) into one or more RTP payloads using:
//!
//! - **Single NAL Unit packet** when the NAL fits in the MTU
//!   (RFC 6184 §5.6).
//! - **FU-A fragmentation unit** when a NAL is larger than the MTU
//!   (RFC 6184 §5.8).
//!
//! STAP-A aggregation (RFC 6184 §5.7) is omitted from this minimal
//! shipping shape — most encoders emit one NAL per access unit for
//! video frames, so single-NAL + FU-A covers the common case. STAP-A is
//! a follow-on optimization for parameter-set bundling.
//!
//! Input format: NAL units in **Annex-B** byte stream form (start codes
//! `0x00 0x00 0x00 0x01` or `0x00 0x00 0x01`). The packetizer splits on
//! start codes and strips them before encapsulation.

use bytes::Bytes;
use std::fmt;

/// One packetized H.264 RTP payload — codec payload only (no RTP header).
#[derive(Clone)]
pub struct H264Packet {
    pub payload: Bytes,
    /// `true` on the *last* packet of the access unit so the outbound RTP
    /// pump stamps the marker bit (RFC 6184 §5.3: M=1 on the last RTP
    /// packet of an access unit, signalling end-of-frame to the decoder).
    pub marker: bool,
}

impl fmt::Debug for H264Packet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("H264Packet")
            .field("payload_bytes", &self.payload.len())
            .field("marker", &self.marker)
            .finish()
    }
}

/// Packetize one H.264 access unit (Annex-B byte stream).
///
/// `access_unit` is the encoder output for a single frame: one or more
/// NAL units separated by Annex-B start codes. Returns at least one
/// packet; the last packet carries `marker=true`.
pub fn packetize_h264(access_unit: &[u8], mtu: usize) -> Vec<H264Packet> {
    let nals = split_annex_b(access_unit);
    if nals.is_empty() {
        return vec![H264Packet {
            payload: Bytes::new(),
            marker: true,
        }];
    }

    let mut out: Vec<H264Packet> = Vec::new();
    let last_idx = nals.len() - 1;
    for (i, nal) in nals.into_iter().enumerate() {
        let is_last_nal = i == last_idx;
        let mut pkts = encapsulate_nal(nal, mtu);
        let pkt_count = pkts.len();
        for (j, pkt) in pkts.iter_mut().enumerate() {
            // Marker bit only on the very last packet of the AU:
            // last packet (j == pkt_count - 1) of the last NAL.
            pkt.marker = is_last_nal && j + 1 == pkt_count;
        }
        out.extend(pkts);
    }
    out
}

/// Encapsulate one NAL unit (sans start code) as one or more RTP payloads.
fn encapsulate_nal(nal: &[u8], mtu: usize) -> Vec<H264Packet> {
    if nal.is_empty() {
        return vec![H264Packet {
            payload: Bytes::new(),
            marker: true,
        }];
    }
    if nal.len() <= mtu {
        // Single NAL Unit packet — wire the NAL verbatim.
        return vec![H264Packet {
            payload: Bytes::copy_from_slice(nal),
            marker: true,
        }];
    }

    // FU-A fragmentation. RFC 6184 §5.8:
    //   FU indicator: F (1 bit) | NRI (2 bits) | Type=28 (5 bits)
    //   FU header:    S | E | R | Type (5 bits)  where Type is the
    //                 original NAL type and R is reserved (0).
    let nal_header = nal[0];
    let f_nri = nal_header & 0b1110_0000;
    let original_type = nal_header & 0b0001_1111;
    let fu_indicator = f_nri | 28;

    let payload = &nal[1..]; // skip the original NAL header
    if mtu <= 2 {
        // Pathological tiny MTU — fall back to a single packet.
        return vec![H264Packet {
            payload: Bytes::copy_from_slice(nal),
            marker: true,
        }];
    }
    let fragment_room = mtu - 2; // FU indicator + FU header
    let mut packets = Vec::new();
    let mut offset = 0usize;
    while offset < payload.len() {
        let end = (offset + fragment_room).min(payload.len());
        let chunk = &payload[offset..end];
        let is_start = offset == 0;
        let is_end = end == payload.len();
        let mut fu_header = original_type;
        if is_start {
            fu_header |= 0b1000_0000;
        }
        if is_end {
            fu_header |= 0b0100_0000;
        }
        let mut buf = Vec::with_capacity(2 + chunk.len());
        buf.push(fu_indicator);
        buf.push(fu_header);
        buf.extend_from_slice(chunk);
        packets.push(H264Packet {
            payload: Bytes::from(buf),
            marker: is_end,
        });
        offset = end;
    }
    packets
}

/// Split an Annex-B byte stream into NAL units, stripping start codes.
///
/// Handles both 3-byte (`00 00 01`) and 4-byte (`00 00 00 01`) start
/// codes. If the input has no start codes (e.g. AVCC-style length-prefixed
/// or a single bare NAL), the entire slice is returned as one unit.
fn split_annex_b(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                starts.push((i, i + 3));
                i += 3;
                continue;
            }
            if i + 3 < data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                starts.push((i, i + 4));
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    if starts.is_empty() {
        if data.is_empty() {
            return Vec::new();
        }
        return vec![data];
    }
    let mut nals = Vec::with_capacity(starts.len());
    for (idx, (_, payload_start)) in starts.iter().enumerate() {
        let end = if idx + 1 < starts.len() {
            starts[idx + 1].0
        } else {
            data.len()
        };
        nals.push(&data[*payload_start..end]);
    }
    nals
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build an Annex-B access unit with one NAL of the given body.
    fn annex_b(nal_type: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![0x00, 0x00, 0x00, 0x01];
        out.push(nal_type); // NAL header: F=0, NRI=0, Type
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn single_small_nal_yields_one_packet_with_marker() {
        let au = annex_b(5, &[0u8; 100]); // IDR slice (type=5)
        let pkts = packetize_h264(&au, 1200);
        assert_eq!(pkts.len(), 1);
        assert!(pkts[0].marker);
        assert_eq!(pkts[0].payload.len(), 101); // 1 header + 100 body
        assert_eq!(pkts[0].payload[0] & 0x1f, 5, "NAL type 5 (IDR) preserved");
    }

    #[test]
    fn nal_larger_than_mtu_fragments_with_fu_a() {
        // 3000-byte NAL at MTU=1200 → FU-A fragments.
        // Each fragment: 1 FU-ind + 1 FU-hdr + ≤1198 body. Body bytes total = 2999.
        // ceil(2999 / 1198) = 3 fragments.
        let au = annex_b(5, &[0xCC; 2999]);
        let pkts = packetize_h264(&au, 1200);
        assert_eq!(pkts.len(), 3);

        // First fragment: S=1, E=0
        let h0 = pkts[0].payload[1];
        assert_eq!(h0 & 0b1000_0000, 0b1000_0000, "first FU-A must have S=1");
        assert_eq!(h0 & 0b0100_0000, 0, "first FU-A must have E=0");
        assert!(!pkts[0].marker);

        // Middle fragment: S=0, E=0
        let h1 = pkts[1].payload[1];
        assert_eq!(h1 & 0b1000_0000, 0);
        assert_eq!(h1 & 0b0100_0000, 0);
        assert!(!pkts[1].marker);

        // Last fragment: S=0, E=1, marker=true
        let h2 = pkts[2].payload[1];
        assert_eq!(h2 & 0b1000_0000, 0);
        assert_eq!(h2 & 0b0100_0000, 0b0100_0000, "last FU-A must have E=1");
        assert!(pkts[2].marker, "last fragment must carry RTP marker");

        // All FU indicators carry type=28.
        for p in &pkts {
            assert_eq!(p.payload[0] & 0x1f, 28);
        }
        // FU header carries original NAL type 5 in the low 5 bits.
        for p in &pkts {
            assert_eq!(p.payload[1] & 0x1f, 5);
        }
    }

    #[test]
    fn multi_nal_access_unit_only_last_packet_carries_marker() {
        // SPS (type=7) + PPS (type=8) + IDR (type=5) — typical keyframe prefix.
        let mut au = Vec::new();
        au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67]); // SPS NAL header
        au.extend_from_slice(&[0xAA; 8]);
        au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x68]); // PPS
        au.extend_from_slice(&[0xBB; 4]);
        au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x65]); // IDR
        au.extend_from_slice(&[0xCC; 80]);
        let pkts = packetize_h264(&au, 1200);
        assert_eq!(pkts.len(), 3, "three NALs → three packets at this MTU");
        assert!(!pkts[0].marker, "SPS not last");
        assert!(!pkts[1].marker, "PPS not last");
        assert!(pkts[2].marker, "IDR is the last NAL of the AU");
    }

    #[test]
    fn empty_access_unit_yields_one_empty_packet() {
        let pkts = packetize_h264(&[], 1200);
        assert_eq!(pkts.len(), 1);
        assert!(pkts[0].marker);
        assert!(pkts[0].payload.is_empty());
    }

    #[test]
    fn three_byte_start_codes_also_recognised() {
        let mut au = vec![0x00, 0x00, 0x01, 0x65];
        au.extend_from_slice(&[0xDD; 50]);
        let pkts = packetize_h264(&au, 1200);
        assert_eq!(pkts.len(), 1);
        assert_eq!(pkts[0].payload[0] & 0x1f, 5, "NAL type 5 (IDR) preserved");
    }
}
