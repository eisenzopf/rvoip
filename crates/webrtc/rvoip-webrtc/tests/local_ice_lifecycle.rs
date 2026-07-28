//! Route-owned local ICE completion and bounded signaling lifecycle.

use std::time::Duration;

use rvoip_webrtc::{LocalIceEvent, PeerRole, RvoipPeerConnection, WebRtcConfig};

#[tokio::test]
async fn trickle_peer_reports_candidates_then_explicit_completion() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = WebRtcConfig::loopback();
    config.trickle_ice = true;
    config.handler_channel_capacity = 32;
    let peer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("peer");
    peer.add_local_audio_track().await.expect("audio track");
    let offer = peer.create_offer_and_gather().await.expect("offer");
    assert!(offer.contains("m=audio"));

    let mut saw_candidate = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), peer.recv_local_ice_event())
            .await
            .expect("ICE event timeout")
            .expect("ICE event channel");
        match event {
            LocalIceEvent::Candidate(_) => saw_candidate = true,
            LocalIceEvent::Complete => break,
            LocalIceEvent::Overflow => panic!("local ICE queue overflowed"),
        }
    }
    assert!(
        saw_candidate,
        "loopback gathering produced no host candidate"
    );
    peer.close().await.expect("close peer");
}
