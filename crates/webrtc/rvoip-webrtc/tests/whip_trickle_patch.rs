//! H2: WHIP `PATCH application/trickle-ice-sdpfrag` per RFC 9725 §4.4.

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::signaling::whip;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn whip_patch_trickle_sdpfrag_accepts_candidate() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Server-side adapter behind WHIP listener.
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve_adapter = Arc::clone(&adapter);
    let server = tokio::spawn(async move {
        whip::serve_listener(listener, serve_adapter).await.ok();
    });

    // Client-side offerer just to produce a real SDP offer.
    let client = Arc::new(
        RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
            .await
            .expect("client peer"),
    );
    client.add_local_audio_track().await.expect("audio");
    let offer = client.create_offer_and_gather().await.expect("offer");

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("http");

    // POST the offer.
    let resp = http
        .post(format!("http://{addr}/whip/test"))
        .header("content-type", "application/sdp")
        .body(offer)
        .send()
        .await
        .expect("post offer");
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    let location = resp
        .headers()
        .get("location")
        .expect("location header")
        .to_str()
        .expect("location ascii")
        .to_owned();
    // Drain the answer body.
    let _ = resp.text().await;

    // Send a trickle PATCH with a host candidate fragment.
    let sdpfrag = "a=mid:0\r\na=candidate:1 1 udp 2130706431 127.0.0.1 50001 typ host\r\n";
    let patch_resp = http
        .patch(format!("http://{addr}{location}"))
        .header("content-type", "application/trickle-ice-sdpfrag")
        .body(sdpfrag)
        .send()
        .await
        .expect("patch trickle");
    assert_eq!(
        patch_resp.status(),
        reqwest::StatusCode::NO_CONTENT,
        "valid trickle PATCH should return 204"
    );

    // Empty trickle body → 400.
    let bad = http
        .patch(format!("http://{addr}{location}"))
        .header("content-type", "application/trickle-ice-sdpfrag")
        .body("")
        .send()
        .await
        .expect("patch empty");
    assert_eq!(bad.status(), reqwest::StatusCode::BAD_REQUEST);

    // Trickle for unknown route → 404.
    let bogus = http
        .patch(format!("http://{addr}/whip/does-not-exist"))
        .header("content-type", "application/trickle-ice-sdpfrag")
        .body(sdpfrag)
        .send()
        .await
        .expect("patch unknown");
    assert_eq!(bogus.status(), reqwest::StatusCode::NOT_FOUND);

    server.abort();
}
