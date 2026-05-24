//! WHIP POST round-trip (feature `signaling-whip`).

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_webrtc::signaling::whip;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn whip_post_returns_sdp_answer_and_inbound_event() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config.clone());

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
    let offer_sdp = offerer.local_sdp(&handle.connection.id).expect("offer sdp");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let whip_adapter = Arc::clone(&adapter);
    let server = tokio::spawn(async move {
        whip::serve_listener(listener, whip_adapter)
            .await
            .expect("whip serve")
    });

    let mut events = adapter.subscribe_events();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");
    let url = format!("http://{addr}/whip/test-session");
    let resp = client
        .post(&url)
        .header("content-type", "application/sdp")
        .body(offer_sdp)
        .send()
        .await
        .expect("whip post");

    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    let answer_sdp = resp.text().await.expect("answer body");
    assert!(!answer_sdp.is_empty());
    assert!(answer_sdp.contains("m=audio"));

    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("event timeout")
        .expect("event channel");
    assert!(matches!(event, AdapterEvent::InboundConnection { .. }));

    server.abort();
}
