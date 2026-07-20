//! Minimal WHEP-04 prerequisite: the pinned alpha engine must support a
//! subscriber rolling back its local offer, accepting a server counter-offer,
//! and returning an answer on the same PeerConnection.

#![cfg(feature = "signaling-whip")]

use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;

#[tokio::test]
async fn offer_rollback_counter_offer_answer_is_supported_by_the_alpha_engine() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = WebRtcConfig::loopback();
    config.trickle_ice = true;

    let subscriber = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("subscriber peer");
    subscriber
        .add_local_audio_track()
        .await
        .expect("subscriber audio");
    subscriber
        .create_offer_and_gather()
        .await
        .expect("subscriber offer");
    subscriber
        .rollback_local()
        .await
        .expect("rollback local subscriber offer");

    let origin = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("origin peer");
    let counter_offer_sdp = origin
        .create_offer_and_gather()
        .await
        .expect("origin counter-offer");
    counter_offer_sdp
        .parse::<rvoip_sip_core::types::sdp::SdpSession>()
        .expect("counter-offer parses through the shared SDP boundary");
    let counter_offer = rtc::peer_connection::sdp::RTCSessionDescription::offer(counter_offer_sdp)
        .expect("parse counter-offer");

    subscriber
        .peer_connection()
        .set_remote_description(counter_offer)
        .await
        .expect("apply counter-offer after rollback");
    let answer = subscriber
        .peer_connection()
        .create_answer(None)
        .await
        .expect("create answer to counter-offer");
    subscriber
        .peer_connection()
        .set_local_description(answer)
        .await
        .expect("apply local counter-offer answer");
    let answer = subscriber
        .peer_connection()
        .local_description()
        .await
        .expect("retained answer");
    origin
        .peer_connection()
        .set_remote_description(answer)
        .await
        .expect("origin accepts counter-offer answer");

    assert!(subscriber.signaling_is_stable().await);
    assert!(origin.signaling_is_stable().await);
    subscriber.close().await.expect("close subscriber");
    origin.close().await.expect("close origin");
}

#[tokio::test]
async fn rvoip_wrapper_completes_the_same_counter_offer_transition() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = WebRtcConfig::loopback();
    config.trickle_ice = false;

    let subscriber = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("subscriber peer");
    subscriber
        .prepare_receive_only_offer()
        .await
        .expect("receive-only media");
    subscriber
        .create_offer_and_gather()
        .await
        .expect("subscriber offer");

    let origin = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("origin peer");
    let counter_offer_sdp = origin
        .create_offer_and_gather()
        .await
        .expect("origin counter-offer");
    let answer = subscriber
        .answer_counter_offer_after_rollback(&counter_offer_sdp)
        .await
        .expect("wrapper counter-offer answer");
    origin
        .set_remote_answer(&answer)
        .await
        .expect("origin accepts wrapper answer");

    assert!(answer.contains("a=recvonly"));
    assert!(subscriber.signaling_is_stable().await);
    assert!(origin.signaling_is_stable().await);
    subscriber.close().await.expect("close subscriber");
    origin.close().await.expect("close origin");
}
