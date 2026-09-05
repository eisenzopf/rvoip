//! Provider-neutral checked RTP framing for SIP, WebRTC, and UCTP gateways.

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::rtp_boundary::{
    depacketize_rtp, NegotiatedRtpPayload, RtpCodecKind, RtpPacketizer,
};
use rvoip_core::StreamId;
use rvoip_rtp_core::RtpPacket;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let negotiated = NegotiatedRtpPayload::new(111, RtpCodecKind::Opus, 48_000)?;
    let inbound = RtpPacket::new_with_payload(
        111,
        7,
        960,
        0x1234_5678,
        Bytes::from_static(b"encoded-opus"),
    );
    let depacketized = depacketize_rtp(inbound, StreamId::new(), Utc::now(), negotiated, 1_200)?;

    // A tunnel that did not transform or repacketize can retain every RTP
    // header field exactly. Taking `into_frame()` deliberately gives this up.
    let preserved = depacketized.preserve_packet(negotiated)?;
    assert_eq!(preserved.header.sequence_number, 7);

    // A transcoding/repacketizing gateway owns a fresh deterministic stream.
    let frame = depacketized.into_frame();
    let mut packetizer = RtpPacketizer::new(negotiated, 0x8765_4321, 100, 9_600, 960)?;
    let outbound = packetizer.packetize(&frame, true)?;
    assert_eq!(outbound.header.sequence_number, 100);
    assert_eq!(outbound.header.timestamp, 9_600);
    Ok(())
}
