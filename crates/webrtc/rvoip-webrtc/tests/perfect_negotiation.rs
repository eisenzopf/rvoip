//! G3 — Perfect-negotiation helper tests.

#![cfg(feature = "client")]

use std::sync::Arc;

use rvoip_webrtc::client::{NegotiationAction, PerfectNegotiation, SignalingPool};
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;

#[tokio::test]
async fn polite_yields_on_collision_with_local_pending_offer() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pn = PerfectNegotiation::new(true);
    let config = WebRtcConfig::loopback();
    let peer = Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("offerer"),
    );
    peer.add_local_audio_track().await.unwrap();

    // Create local offer → signaling state is no longer stable.
    pn.begin_local_offer();
    let _ = peer.create_offer_and_gather().await.expect("offer");
    pn.end_local_offer();

    // A remote offer arrives. Polite peer rolls back.
    let action = pn.decide_remote_offer(&peer).await;
    assert_eq!(action, NegotiationAction::Rollback);
}

#[tokio::test]
async fn impolite_ignores_collision() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pn = PerfectNegotiation::new(false);
    let config = WebRtcConfig::loopback();
    let peer = Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("offerer"),
    );
    peer.add_local_audio_track().await.unwrap();

    // Local offer pending.
    pn.begin_local_offer();
    let _ = peer.create_offer_and_gather().await.expect("offer");
    pn.end_local_offer();

    let action = pn.decide_remote_offer(&peer).await;
    assert_eq!(action, NegotiationAction::Ignore);
    assert!(pn.was_offer_ignored());
}

#[tokio::test]
async fn no_collision_applies_remote_offer() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pn = PerfectNegotiation::new(true);
    let config = WebRtcConfig::loopback();
    // Fresh answerer — no local offer pending.
    let peer = Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Answerer)
            .await
            .expect("answerer"),
    );
    let action = pn.decide_remote_offer(&peer).await;
    assert_eq!(action, NegotiationAction::Apply);
}

#[tokio::test]
async fn signaling_pool_dedupes_per_url() {
    let pool = SignalingPool::new(std::time::Duration::from_secs(60));
    let a = pool.get_ws("ws://example.org:9999/sig").await.unwrap();
    let b = pool.get_ws("ws://example.org:9999/sig").await.unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    let c = pool.get_ws("ws://example.org:9998/sig").await.unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
    assert_eq!(pool.ws_len(), 2);
    pool.evict("ws://example.org:9999/sig");
    assert_eq!(pool.ws_len(), 1);
}
