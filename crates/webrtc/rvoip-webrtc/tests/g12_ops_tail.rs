//! G12 — operational tail tests: SDP redaction, Opus settings round-trip,
//! adapter metrics reset.

use std::sync::Arc;

use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::sdp::redact_for_log;
use rvoip_webrtc::{OpusSettings, WebRtcAdapter, WebRtcConfig};

#[test]
fn redact_for_log_strips_ips_and_credentials() {
    let sdp = "v=0\r\n\
o=- 1 1 IN IP4 198.51.100.7\r\n\
c=IN IP4 198.51.100.7\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:0123456789abcdef\r\n\
a=candidate:1 1 udp 2122260223 192.168.1.5 50001 typ host\r\n";
    let red = redact_for_log(sdp);
    assert!(!red.contains("198.51.100.7"));
    assert!(!red.contains("192.168.1.5"));
    assert!(!red.contains("abcd"));
    assert!(!red.contains("0123456789abcdef"));
}

#[test]
fn opus_settings_render_fmtp_with_dtx_and_stereo() {
    let s = OpusSettings {
        use_in_band_fec: true,
        use_dtx: true,
        min_ptime_ms: 20,
        max_average_bitrate_bps: 96_000,
        stereo: true,
    };
    let fmtp = s.to_fmtp_line();
    assert!(fmtp.contains("minptime=20"));
    assert!(fmtp.contains("useinbandfec=1"));
    assert!(fmtp.contains("usedtx=1"));
    assert!(fmtp.contains("maxaveragebitrate=96000"));
    assert!(fmtp.contains("stereo=1"));
}

#[tokio::test]
async fn opus_fmtp_appears_in_offer_when_settings_changed() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = WebRtcConfig::loopback();
    config.opus_settings = OpusSettings {
        use_in_band_fec: true,
        use_dtx: true,
        min_ptime_ms: 20,
        max_average_bitrate_bps: 64_000,
        stereo: false,
    };
    let peer = Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("offerer"),
    );
    peer.add_local_audio_track().await.expect("audio");
    let offer = peer.create_offer_and_gather().await.expect("offer");
    assert!(
        offer.contains("usedtx=1"),
        "Opus fmtp should include usedtx=1: {offer}"
    );
    assert!(
        offer.contains("maxaveragebitrate=64000"),
        "Opus fmtp should include maxaveragebitrate: {offer}"
    );
}

#[tokio::test]
async fn reset_metrics_zeros_counters() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    // Bump counters via note helpers.
    for _ in 0..3 {
        adapter.note_signaling_error();
    }
    let m_before = adapter.metrics();
    assert_eq!(m_before.signaling_errors_total, 3);
    adapter.reset_metrics();
    let m_after = adapter.metrics();
    assert_eq!(m_after.signaling_errors_total, 0);
}
