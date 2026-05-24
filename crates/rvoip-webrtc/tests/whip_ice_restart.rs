//! WHIP PATCH ICE restart returns updated answer SDP.

#![cfg(feature = "signaling-whip")]

use std::time::Duration;

use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_webrtc::signaling::whip;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn whip_patch_ice_restart_returns_new_answer() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let whip_adapter = std::sync::Arc::clone(&adapter);
    let server = tokio::spawn(async move {
        whip::serve_listener(listener, whip_adapter)
            .await
            .expect("whip serve")
    });

    let offerer = WebRtcAdapter::new(config);
    let handle = offerer
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: offerer.capabilities(),
        })
        .await
        .expect("originate");
    let initial_offer = offerer.local_sdp(&handle.connection.id).expect("offer");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");
    let base = format!("http://{addr}");

    let post_resp = client
        .post(format!("{base}/whip/publish"))
        .header("content-type", "application/sdp")
        .body(initial_offer.clone())
        .send()
        .await
        .expect("whip post");
    assert_eq!(post_resp.status(), reqwest::StatusCode::CREATED);
    let first_answer = post_resp.text().await.expect("answer body");
    assert!(first_answer.contains("m=audio"));

    let conn_id = adapter
        .routes()
        .iter()
        .next()
        .map(|e| e.key().clone())
        .expect("route");

    let offerer_route = offerer
        .routes()
        .get(&handle.connection.id)
        .expect("offerer route");
    let restart_offer = offerer_route
        .peer
        .renegotiate_as_offerer()
        .await
        .expect("restart offer");

    let patch_resp = client
        .patch(format!("{base}/whip/{conn_id}"))
        .header("content-type", "application/sdp")
        .body(restart_offer)
        .send()
        .await
        .expect("whip patch");
    assert_eq!(patch_resp.status(), reqwest::StatusCode::OK);
    let restart_answer = patch_resp.text().await.expect("restart answer");
    assert!(restart_answer.contains("m=audio"));

    server.abort();
}
