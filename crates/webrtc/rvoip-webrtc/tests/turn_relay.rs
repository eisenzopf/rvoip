//! TURN configuration behavior with a real, hermetic relay.
//!
//! Relay-only construction requires a usable allocation; an advertised URL
//! alone is insufficient. The media/cleanup contract is exercised further by
//! `turn_relay_e2e`.

mod support {
    pub mod coturn_fixture;
}

use std::time::Duration;

use rvoip_webrtc::config::IceTransportPolicy;
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{IceServerConfig, WebRtcConfig};

#[tokio::test]
async fn relay_policy_requires_allocation_and_suppresses_direct_candidates() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // A relay-only policy with nothing to relay through is rejected when the
    // peer connection is built, which also shows the knob reached the core.
    let mut without_turn = WebRtcConfig::loopback();
    without_turn.ice_transport_policy = IceTransportPolicy::Relay;
    assert!(
        RvoipPeerConnection::new(&without_turn, PeerRole::Offerer)
            .await
            .is_err(),
        "Relay policy without a TURN server must be rejected"
    );

    let relay = support::coturn_fixture::CoturnFixture::start()
        .await
        .expect("start TURN fixture");
    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![relay.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;

    let peer = std::sync::Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("offerer with Relay policy"),
    );
    peer.add_local_audio_track().await.expect("audio");

    // Offer creation should succeed.
    let sdp = tokio::time::timeout(Duration::from_secs(10), peer.create_offer_and_gather())
        .await
        .expect("gather timeout")
        .expect("offer");
    assert!(sdp.contains("m=audio"));
    let candidates: Vec<_> = sdp
        .lines()
        .filter(|line| line.starts_with("a=candidate:"))
        .collect();
    assert!(
        !candidates.is_empty(),
        "relay offer must contain a candidate"
    );
    assert!(candidates.iter().all(|line| line.contains(" typ relay")));
    assert_eq!(relay.allocation_count().await, 1);
    peer.close().await.expect("close peer");
    relay.wait_for_no_allocations().await;
    relay.close().await.expect("close TURN fixture");
}

#[tokio::test]
async fn all_policy_default_still_produces_host_candidates() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    assert_eq!(config.ice_transport_policy, IceTransportPolicy::All);

    let peer = std::sync::Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("offerer"),
    );
    peer.add_local_audio_track().await.expect("audio");
    let sdp = peer.create_offer_and_gather().await.expect("offer");

    let host = sdp
        .lines()
        .any(|l| l.contains("a=candidate:") && l.contains("typ host"));
    assert!(
        host,
        "default All policy must yield host candidates on loopback. SDP:\n{sdp}"
    );
    peer.close().await.expect("close direct peer");
}

#[tokio::test]
async fn turn_url_and_credentials_are_preserved_in_configuration() {
    // Configuration retains customer-provided TURN URLs and credentials.
    let cfg = WebRtcConfig::loopback().with_turn("turn:turn.example.com:3478", "alice", "secret");
    assert_eq!(cfg.ice_servers.len(), 1);
    let turn = &cfg.ice_servers[0];
    assert_eq!(turn.urls[0], "turn:turn.example.com:3478");
    assert_eq!(turn.username.as_deref(), Some("alice"));
    assert_eq!(turn.credential.as_deref(), Some("secret"));

    // Also confirm IceServerConfig::turn constructor works.
    let entry = IceServerConfig::turn("turn:t.example", "u", "p");
    assert!(entry.username.is_some());
    assert!(entry.credential.is_some());
}
