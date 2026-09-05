//! Checked RTP boundary used by a WebRTC gateway instead of hand-built headers.

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::rtp_boundary::{
    depacketize_rtp, NegotiatedRtpPayload, RtpCodecKind, RtpPacketizer,
};
use rvoip_core::{MediaFrame, StreamId, StreamKind};
use rvoip_rtp_core::RtpPacket;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let negotiated = NegotiatedRtpPayload::new(120, RtpCodecKind::Opus, 48_000)?;
    let inbound =
        RtpPacket::new_with_payload(120, 7, 960, 0x1234_5678, Bytes::from_static(b"opus"));
    let inbound = depacketize_rtp(inbound, StreamId::new(), Utc::now(), negotiated, 1_200)?;
    let frame: &MediaFrame = inbound.frame();
    assert_eq!(frame.kind, StreamKind::Audio);

    let mut egress = RtpPacketizer::new(negotiated, 0x8765_4321, 100, 9_600, 960)?;
    let outbound = egress.packetize(frame, true)?;
    assert_eq!(outbound.header.payload_type, 120);
    Ok(())
}
