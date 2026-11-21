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
    let initial_etag = resp
        .headers()
        .get("etag")
        .expect("etag header")
        .to_str()
        .expect("etag ascii")
        .to_owned();
    // Drain the answer body.
    let _ = resp.text().await;

    // Send a trickle PATCH with a host candidate fragment.
    let sdpfrag = "a=mid:0\r\na=candidate:1 1 udp 2130706431 127.0.0.1 50001 typ host\r\n";
    let resource_url = format!("http://{addr}{location}");
    let wrong_resource_kind = http
        .patch(format!(
            "http://{addr}{}",
            location.replacen("/whip/", "/whep/", 1)
        ))
        .header("content-type", "application/trickle-ice-sdpfrag")
        .header("if-match", &initial_etag)
        .body(sdpfrag)
        .send()
        .await
        .expect("cross-protocol resource mutation");
    assert_eq!(wrong_resource_kind.status(), reqwest::StatusCode::NOT_FOUND);
    for inexact in ["*".to_owned(), format!("W/{initial_etag}")] {
        let rejected = http
            .patch(&resource_url)
            .header("content-type", "application/trickle-ice-sdpfrag")
            .header("if-match", inexact)
            .body(sdpfrag)
            .send()
            .await
            .expect("patch with inexact precondition");
        assert_eq!(rejected.status(), reqwest::StatusCode::PRECONDITION_FAILED);
    }
    let patch = |client: reqwest::Client| {
        let resource_url = resource_url.clone();
        let initial_etag = initial_etag.clone();
        async move {
            client
                .patch(resource_url)
                .header("content-type", "application/trickle-ice-sdpfrag")
                .header("if-match", initial_etag)
                .body(sdpfrag)
                .send()
                .await
                .expect("patch trickle")
        }
    };
    let (first, second) = tokio::join!(patch(http.clone()), patch(http.clone()));
    let (winner, stale) = if first.status() == reqwest::StatusCode::NO_CONTENT {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(winner.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(stale.status(), reqwest::StatusCode::PRECONDITION_FAILED);
    let current_etag = winner
        .headers()
        .get("etag")
        .expect("rotated etag")
        .to_str()
        .expect("etag ascii")
        .to_owned();
    assert_ne!(current_etag, initial_etag);

    // Empty trickle body → 400.
    let bad = http
        .patch(resource_url)
        .header("content-type", "application/trickle-ice-sdpfrag")
        .header("if-match", &current_etag)
        .body("")
        .send()
        .await
        .expect("patch empty");
    assert_eq!(bad.status(), reqwest::StatusCode::BAD_REQUEST);

    // Trickle for unknown route → 404.
    let bogus = http
        .patch(format!("http://{addr}/whip/does-not-exist"))
        .header("content-type", "application/trickle-ice-sdpfrag")
        .header("if-match", "\"unknown\"")
        .body(sdpfrag)
        .send()
        .await
        .expect("patch unknown");
    assert_eq!(bogus.status(), reqwest::StatusCode::NOT_FOUND);

    let missing = http
        .delete(format!("http://{addr}{location}"))
        .send()
        .await
        .expect("delete without precondition");
    assert_eq!(missing.status(), reqwest::StatusCode::PRECONDITION_REQUIRED);
    let stale_delete = http
        .delete(format!("http://{addr}{location}"))
        .header("if-match", initial_etag)
        .send()
        .await
        .expect("delete with stale tag");
    assert_eq!(
        stale_delete.status(),
        reqwest::StatusCode::PRECONDITION_FAILED
    );
    let deleted = http
        .delete(format!("http://{addr}{location}"))
        .header("if-match", current_etag)
        .send()
        .await
        .expect("delete with current tag");
    assert_eq!(deleted.status(), reqwest::StatusCode::OK);

    server.abort();
}
