#![no_main]
//! Fuzz the checked RTP-to-core-MediaFrame boundary with bounded allocation.

use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use rvoip_core::rtp_boundary::{depacketize_rtp, NegotiatedRtpPayload, RtpCodecKind};
use rvoip_core::StreamId;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let owned = Bytes::copy_from_slice(data);
    let Ok(packet) = rvoip_rtp_core::RtpPacket::parse_from_bytes(owned) else {
        return;
    };
    let payload_type = packet.header.payload_type;
    let codec = match payload_type {
        0 => RtpCodecKind::Pcmu,
        8 => RtpCodecKind::Pcma,
        101 => RtpCodecKind::TelephoneEvent,
        _ => RtpCodecKind::Opus,
    };
    let clock_rate = if matches!(codec, RtpCodecKind::Opus) {
        48_000
    } else {
        8_000
    };
    if let Ok(mapping) = NegotiatedRtpPayload::new(payload_type, codec, clock_rate) {
        let _ = depacketize_rtp(packet, StreamId::new(), chrono::Utc::now(), mapping, 1_200);
    }
});
