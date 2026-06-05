//! H3: SDP offer must advertise RTCP feedback and the H.264/VP9 codec set.

use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;

#[tokio::test]
async fn offer_sdp_advertises_video_rtcp_feedback() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let peer = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("offerer");
    peer.add_local_audio_track().await.expect("audio");
    peer.add_local_video_track().await.expect("video");
    let sdp = peer.create_offer_and_gather().await.expect("offer");

    // VP8 video rtcp-fb attributes.
    assert!(
        sdp.contains("a=rtcp-fb") && sdp.contains("nack"),
        "expected a=rtcp-fb:.. nack in video m-section"
    );
    assert!(sdp.contains("nack pli"), "expected NACK PLI");
    assert!(sdp.contains("ccm fir"), "expected CCM FIR");
    assert!(sdp.contains("goog-remb"), "expected goog-remb");
    assert!(sdp.contains("transport-cc"), "expected transport-cc");

    peer.close().await.ok();
}

#[tokio::test]
async fn offer_sdp_advertises_h264_and_vp9() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let peer = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("offerer");
    peer.add_local_video_track().await.expect("video");
    let sdp = peer.create_offer_and_gather().await.expect("offer");

    let codecs: Vec<_> = sdp.lines().filter(|l| l.starts_with("a=rtpmap:")).collect();
    let codec_str = codecs.join("\n");

    assert!(codec_str.contains("VP8/"), "VP8 must be advertised");
    assert!(
        codec_str.contains("VP9/"),
        "VP9 must be advertised (H3 codec expansion)"
    );
    assert!(
        codec_str.contains("H264/"),
        "H.264 must be advertised — required for Safari and SIP gateways"
    );

    peer.close().await.ok();
}

#[tokio::test]
async fn offer_sdp_includes_opus_transport_cc_feedback() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let peer = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("offerer");
    peer.add_local_audio_track().await.expect("audio");
    let sdp = peer.create_offer_and_gather().await.expect("offer");

    // The Opus payload-type (111) must carry a transport-cc rtcp-fb line.
    let has_opus_twcc = sdp
        .lines()
        .any(|l| l.contains("a=rtcp-fb:111") && l.contains("transport-cc"));
    assert!(
        has_opus_twcc,
        "Opus (PT 111) must advertise transport-cc rtcp-fb for congestion control. SDP:\n{sdp}"
    );

    peer.close().await.ok();
}
