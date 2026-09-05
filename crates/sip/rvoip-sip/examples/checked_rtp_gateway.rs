//! Checked RTP boundary used by a SIP gateway instead of hand-built headers.

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::rtp_boundary::{
    depacketize_rtp, NegotiatedRtpPayload, RtpCodecKind, RtpPacketizer,
};
use rvoip_core::{MediaFrame, StreamId, StreamKind};
use rvoip_rtp_core::RtpPacket;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let negotiated = NegotiatedRtpPayload::new(0, RtpCodecKind::Pcmu, 8_000)?;
    let inbound = RtpPacket::new_with_payload(0, 7, 160, 0x1234_5678, Bytes::from_static(b"pcmu"));
    let inbound = depacketize_rtp(inbound, StreamId::new(), Utc::now(), negotiated, 1_200)?;
    let frame: &MediaFrame = inbound.frame();
    assert_eq!(frame.kind, StreamKind::Audio);

    let mut egress = RtpPacketizer::new(negotiated, 0x8765_4321, 100, 3_200, 160)?;
    let outbound = egress.packetize(frame, true)?;
    assert_eq!(outbound.header.payload_type, 0);
    Ok(())
}
