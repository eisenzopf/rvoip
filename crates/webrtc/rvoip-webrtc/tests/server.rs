//! WebRtcServer facade — dual WHIP + WS listeners on one adapter.

#![cfg(all(feature = "signaling-whip", feature = "signaling-ws"))]

use std::time::Duration;

use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};

#[tokio::test]
async fn server_builder_exposes_whip_and_ws_addrs() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_whip("127.0.0.1:0")
        .with_ws("127.0.0.1:0")
        .build()
        .await
        .expect("build server");

    assert!(server.whip_addr().is_some());
    assert!(server.ws_addr().is_some());

    let adapter = server.adapter();
    let mut events = adapter.subscribe_events();

    let offerer = rvoip_webrtc::WebRtcAdapter::new(WebRtcConfig::loopback());
    let handle = offerer
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: offerer.capabilities(),
            transport: None,
            context: Default::default(),
        })
        .await
        .expect("originate");
    let offer_sdp = offerer.local_sdp(&handle.connection.id).expect("offer");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");
    let whip_base = format!("http://{}", server.whip_addr().expect("whip addr"));
    let resp = client
        .post(format!("{whip_base}/whip/publish"))
        .header("content-type", "application/sdp")
        .body(offer_sdp)
        .send()
        .await
        .expect("whip post");
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("event timeout")
        .expect("event channel");
    assert!(matches!(event, AdapterEvent::InboundConnection { .. }));

    server.shutdown().await;
}
