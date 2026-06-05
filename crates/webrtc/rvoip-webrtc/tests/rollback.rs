//! G11 — SDP rollback primitive test.

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{WebRtcConfig, WebRtcError};

#[tokio::test]
async fn rollback_after_local_offer_returns_to_stable() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let offerer = Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("offerer"),
    );
    offerer.add_local_audio_track().await.expect("audio");

    // Create local offer (signaling state moves to have-local-offer).
    let _offer = offerer
        .create_offer_and_gather()
        .await
        .expect("create offer");

    // Rollback — signaling state returns to stable.
    offerer
        .rollback_local()
        .await
        .expect("rollback should succeed from have-local-offer");

    // After rollback we can issue a new offer.
    let second = tokio::time::timeout(Duration::from_secs(5), offerer.create_offer_and_gather())
        .await
        .expect("create second offer timeout")
        .expect("create second offer");
    assert!(second.contains("m=audio"));
}

#[tokio::test]
async fn rollback_in_stable_state_returns_invalid_state() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let offerer = Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("offerer"),
    );

    // Never created a local description — signaling state is stable.
    let res = offerer.rollback_local().await;
    match res {
        Err(WebRtcError::InvalidState(_)) => {}
        Err(WebRtcError::Webrtc(_)) => {
            // Some webrtc-rs builds may return a generic Webrtc error rather
            // than our typed InvalidState. Accept either.
        }
        Ok(()) => panic!("rollback from stable state should fail"),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}
