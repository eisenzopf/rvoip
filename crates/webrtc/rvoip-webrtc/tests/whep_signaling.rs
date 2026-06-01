//! WHEP POST + PATCH round-trip (feature `signaling-whip`).

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter};
use rvoip_webrtc::signaling::whip;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn whep_post_patch_reaches_connected() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config);

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
    let base = format!("http://{addr}");

    let offer_resp = client
        .post(format!("{base}/whep/subscriber"))
        .send()
        .await
        .expect("whep post");
    assert_eq!(offer_resp.status(), reqwest::StatusCode::CREATED);
    let offer_sdp = offer_resp.text().await.expect("offer body");
    assert!(offer_sdp.contains("m=audio"));

    let conn_id = adapter
        .routes()
        .iter()
        .next()
        .map(|e| e.key().clone())
        .expect("whep route");

    let answerer = WebRtcAdapter::new(WebRtcConfig::loopback());
    let inbound_id = answerer
        .apply_remote_offer(&offer_sdp)
        .await
        .expect("apply whep offer");
    let answer_sdp = answerer.local_sdp(&inbound_id).expect("answer sdp");

    let patch_resp = client
        .patch(format!("{base}/whep/{conn_id}"))
        .header("content-type", "application/sdp")
        .body(answer_sdp)
        .send()
        .await
        .expect("whep patch");
    assert_eq!(patch_resp.status(), reqwest::StatusCode::NO_CONTENT);

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("event timeout")
        .expect("event channel");
    assert!(matches!(event, AdapterEvent::Connected { .. }));

    server.abort();
}
