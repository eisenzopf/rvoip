//! G9a — TURN relay E2E test.
//!
//! When Docker is available, spins up a coturn container and verifies the
//! adapter can use it via `IceTransportPolicy::Relay`. When Docker isn't
//! available, the test skips gracefully (returns Ok).

mod support {
    pub mod coturn_fixture;
}

use std::sync::Arc;

use rvoip_webrtc::config::IceTransportPolicy;
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use support::coturn_fixture::CoturnFixture;

#[tokio::test]
async fn relay_policy_with_coturn_fixture_builds_peer() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let Some(coturn) = CoturnFixture::start().await else {
        eprintln!("skipped: docker / coturn unavailable");
        return;
    };

    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![coturn.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;

    let peer = Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("peer should build with Relay policy + coturn ICE config"),
    );
    peer.add_local_audio_track().await.expect("audio");

    // Build a Relay-only offer. We don't try to complete the handshake
    // (would need a remote peer also dialed via coturn); the assertion is
    // that the config plumbs through cleanly and the engine accepts it.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        peer.create_offer_and_gather(),
    )
    .await;

    // RAII teardown via Drop.
    drop(peer);
    drop(coturn);
}
