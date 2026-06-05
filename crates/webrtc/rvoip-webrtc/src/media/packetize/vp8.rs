//! D3b — RFC 7741 VP8 RTP packetizer.
//!
//! Splits an encoded VP8 frame into one or more RTP payloads, each
//! prefixed by the mandatory VP8 payload descriptor. Supports the
//! minimal-but-valid shape: no Picture-ID, no Layer indices, no Keyidx,
//! single partition.
//!
//! The marker bit on the **last** packet is signaled via the
//! [`Vp8Packet::marker`] field; the caller (the outbound RTP pump) sets
//! the actual RTP header marker.
//!
//! Reference: <https://datatracker.ietf.org/doc/html/rfc7741#section-4.2>.

use bytes::Bytes;

/// One packetized VP8 RTP payload — codec payload only (no RTP header).
/// `marker` is `true` on the *last* packet of a frame (RFC 7741 §4.2).
#[derive(Clone, Debug)]
pub struct Vp8Packet {
    pub payload: Bytes,
    pub marker: bool,
}

/// Default packetization MTU. webrtc-rs uses 1200; matches Chrome's
/// path-MTU floor for IPv4 + DTLS + SRTP overhead headroom.
pub const DEFAULT_MTU: usize = 1200;

/// Packetize a single VP8 encoded frame.
///
/// `frame` is the raw VP8 bitstream (start of partition 0 / "first frame
/// header" onwards). `mtu` is the maximum *total* RTP payload size,
/// including the VP8 payload descriptor.
///
/// Returns at least one packet. The first packet's descriptor has S=1
/// (start of partition); the last packet has `marker=true` so the
/// outbound RTP pump can stamp the RTP marker bit.
pub fn packetize_vp8(frame: &[u8], mtu: usize) -> Vec<Vp8Packet> {
    let desc_len = 1; // minimal descriptor — 1 byte
    if mtu <= desc_len {
        // Pathological tiny MTU — produce one packet anyway so the caller
        // doesn't have to handle the empty-vec case. Webrtc-rs defaults
        // to 1200; nobody passes < 16.
        return vec![Vp8Packet {
            payload: Bytes::copy_from_slice(&[descriptor(true)]),
            marker: true,
        }];
    }
    let payload_room = mtu - desc_len;
    if frame.is_empty() {
        return vec![Vp8Packet {
            payload: Bytes::copy_from_slice(&[descriptor(true)]),
            marker: true,
        }];
    }

    let mut packets = Vec::new();
    let mut offset = 0usize;
    while offset < frame.len() {
        let chunk_end = (offset + payload_room).min(frame.len());
        let chunk = &frame[offset..chunk_end];
        let is_first = offset == 0;
        let is_last = chunk_end == frame.len();
        let mut buf = Vec::with_capacity(desc_len + chunk.len());
        buf.push(descriptor(is_first));
        buf.extend_from_slice(chunk);
        packets.push(Vp8Packet {
            payload: Bytes::from(buf),
            marker: is_last,
        });
        offset = chunk_end;
    }
    packets
}

/// Build the mandatory 1-byte VP8 payload descriptor (RFC 7741 §4.2).
///
/// Bit layout:
/// ```text
/// 0 1 2 3 4 5 6 7
/// |X|R|N|S|R|PID|
/// ```
/// We emit X=0 (no extended descriptor), R=0, N=0 (non-reference frames
/// are signaled per-partition; default 0 is safe), S=1 only on the
/// first packet of the frame, R=0, PID=0 (we ship one partition).
fn descriptor(start_of_partition: bool) -> u8 {
    if start_of_partition {
        0b0001_0000
    } else {
        0b0000_0000
    }
}

/// Decode the descriptor's S bit — convenience for tests / receivers
/// that want a sanity check.
pub fn payload_is_start_of_partition(payload: &[u8]) -> bool {
    payload
        .first()
        .map(|b| (b & 0b0001_0000) != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_frame_single_packet_with_marker() {
        let frame = vec![0xAB; 100];
        let pkts = packetize_vp8(&frame, 1200);
        assert_eq!(pkts.len(), 1);
        assert!(pkts[0].marker, "single-packet frame must carry marker bit");
        assert!(
            payload_is_start_of_partition(&pkts[0].payload),
            "first packet must have S=1"
        );
        assert_eq!(
            pkts[0].payload.len(),
            101,
            "1 descriptor byte + 100 payload"
        );
    }

    #[test]
    fn frame_larger_than_mtu_fragments_with_marker_on_last() {
        // 3500 bytes split at MTU=1200 → 1 desc + 1199 payload per chunk → 3 packets:
        //   1199 + 1199 + (3500 - 2398) = 1199 + 1199 + 1102
        let frame = vec![0xCD; 3500];
        let pkts = packetize_vp8(&frame, 1200);
        assert_eq!(
            pkts.len(),
            3,
            "3500 bytes should split into 3 MTU-1200 packets"
        );
        assert!(payload_is_start_of_partition(&pkts[0].payload));
        assert!(!payload_is_start_of_partition(&pkts[1].payload));
        assert!(!payload_is_start_of_partition(&pkts[2].payload));
        assert!(!pkts[0].marker);
        assert!(!pkts[1].marker);
        assert!(pkts[2].marker, "last packet must carry marker bit");
        // Round-trip: concat payloads (after stripping 1-byte descriptors)
        // and confirm we recover the original frame.
        let mut recovered = Vec::new();
        for p in &pkts {
            recovered.extend_from_slice(&p.payload[1..]);
        }
        assert_eq!(recovered, frame);
    }

    #[test]
    fn empty_frame_yields_one_empty_packet() {
        let pkts = packetize_vp8(&[], 1200);
        assert_eq!(pkts.len(), 1);
        assert!(pkts[0].marker);
        assert_eq!(pkts[0].payload.len(), 1);
    }

    #[test]
    fn pathological_tiny_mtu_does_not_panic() {
        let frame = vec![0u8; 16];
        let pkts = packetize_vp8(&frame, 1);
        assert!(!pkts.is_empty());
    }
}
