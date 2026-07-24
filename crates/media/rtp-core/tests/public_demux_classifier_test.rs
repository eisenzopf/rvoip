//! Proves `classify_rtp_mux_packet`/`RtpMuxPacketClass` are genuinely
//! reachable from outside the crate — an external caller bridging its own
//! socket/reactor (e.g. via `transport::dtls_datagram_bridge`) needs this
//! to demux RFC 7983 traffic the same way `UdpRtpTransport`'s own receive
//! loop does. Being importable here at all (this is a separate crate as
//! far as `cargo test` is concerned) is the actual assertion; the
//! per-range checks just confirm nothing was renamed in the process of
//! making it `pub`.

use rvoip_rtp_core::transport::{classify_rtp_mux_packet, RtpMuxPacketClass};

#[test]
fn classifies_stun_range() {
    // RFC 7983 §7: first byte 0-3.
    assert_eq!(
        classify_rtp_mux_packet(&[0x00, 0x01, 0x00, 0x00]),
        RtpMuxPacketClass::Stun
    );
}

#[test]
fn classifies_dtls_range() {
    // RFC 7983 §7: first byte 20-63 (22 = DTLS handshake content type).
    assert_eq!(
        classify_rtp_mux_packet(&[22, 0xfe, 0xfd, 0, 0]),
        RtpMuxPacketClass::Dtls
    );
}

#[test]
fn classifies_turn_channel_data_range() {
    // RFC 7983 §7: first byte 64-79.
    assert_eq!(
        classify_rtp_mux_packet(&[0x40, 0, 0, 0]),
        RtpMuxPacketClass::TurnChannelData
    );
}

#[test]
fn classifies_rtcp_within_the_media_range() {
    // RFC 5761 §4: RTCP sender report, PT 200, within the 128-191 range.
    let rtcp_sr = [0x80, 200, 0, 6, 0, 0, 0, 1];
    assert_eq!(classify_rtp_mux_packet(&rtcp_sr), RtpMuxPacketClass::Rtcp);
    assert!(RtpMuxPacketClass::Rtcp.is_media());
    assert_eq!(RtpMuxPacketClass::Rtcp.as_str(), "rtcp");
}

#[test]
fn classifies_rtp_within_the_media_range() {
    let rtp = [
        0x80, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, // 12-byte minimal RTP header
    ];
    assert_eq!(classify_rtp_mux_packet(&rtp), RtpMuxPacketClass::Rtp);
    assert!(RtpMuxPacketClass::Rtp.is_media());
    assert_eq!(RtpMuxPacketClass::Rtp.as_str(), "rtp");
}

#[test]
fn too_short_datagrams_are_flagged_rather_than_misclassified() {
    assert_eq!(
        classify_rtp_mux_packet(&[0x80]),
        RtpMuxPacketClass::TooSmall
    );
}
