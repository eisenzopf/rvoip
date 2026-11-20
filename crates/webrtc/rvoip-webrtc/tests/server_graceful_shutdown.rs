//! H4.4: WebRtcServer graceful shutdown drains active routes.

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};

#[tokio::test]
async fn shutdown_ends_active_routes_then_closes_listener() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_whip("127.0.0.1:0")
        .build()
        .await
        .expect("build server");
    let whip_addr = server.whip_addr().expect("whip addr");
    let adapter = server.adapter();

    // Push one inbound WHIP session.
    let peer = Arc::new(
        RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
            .await
            .expect("offerer"),
    );
    peer.add_local_audio_track().await.expect("audio");
    let offer = peer.create_offer_and_gather().await.expect("offer");

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("http");
    let resp = http
        .post(format!("http://{whip_addr}/whip/live"))
        .header("content-type", "application/sdp")
        .body(offer)
        .send()
        .await
        .expect("post");
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    assert_eq!(adapter.routes().len(), 1, "route inserted after WHIP POST");

    // Shutdown — should drain the route and stop the listener within the deadline.
    let adapter_snapshot = Arc::clone(&adapter);
    server.shutdown_with_deadline(Duration::from_secs(5)).await;

    assert_eq!(
        adapter_snapshot.routes().len(),
        0,
        "graceful shutdown must end all active routes"
    );

    // Listener should be closed: a follow-up request should fail to connect
    // or timeout. We allow up to 2s for the OS to release the port.
    let mut attempts = 0;
    loop {
        attempts += 1;
        let r = http.get(format!("http://{whip_addr}/healthz")).send().await;
        if r.is_err() {
            break;
        }
        if attempts > 4 {
            panic!("listener still accepting after graceful shutdown");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
