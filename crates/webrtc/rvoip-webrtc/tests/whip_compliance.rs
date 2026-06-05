//! H4: WHIP RFC 9725 surface compliance — Content-Type, ETag, Accept-Patch,
//! Link, health checks, CORS, rate limiting.

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::signaling::whip;
use rvoip_webrtc::{IceServerConfig, WebRtcAdapter, WebRtcConfig};

async fn make_offer() -> String {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let peer = Arc::new(
        RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
            .await
            .expect("offerer"),
    );
    peer.add_local_audio_track().await.expect("audio");
    peer.create_offer_and_gather().await.expect("offer")
}

#[tokio::test]
async fn whip_post_returns_etag_and_accept_patch() {
    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![IceServerConfig::stun("stun:stun.example.com:3478")];
    config.cors_origins = vec!["*".into()];
    let adapter = WebRtcAdapter::new(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve = tokio::spawn({
        let adapter = Arc::clone(&adapter);
        async move {
            let _ = whip::serve_listener(listener, adapter).await;
        }
    });

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("http");
    let offer = make_offer().await;

    let resp = http
        .post(format!("http://{addr}/whip/live"))
        .header("content-type", "application/sdp")
        .body(offer)
        .send()
        .await
        .expect("post");
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);

    let etag = resp
        .headers()
        .get("etag")
        .expect("ETag required for ICE-restart bookkeeping");
    assert!(etag.to_str().expect("ascii").starts_with('"'));

    let accept_patch = resp
        .headers()
        .get("accept-patch")
        .expect("Accept-Patch required by RFC 9725")
        .to_str()
        .expect("ascii");
    assert!(accept_patch.contains("application/sdp"));
    assert!(accept_patch.contains("application/trickle-ice-sdpfrag"));

    let link = resp
        .headers()
        .get_all("link")
        .iter()
        .map(|v| v.to_str().unwrap_or("").to_owned())
        .collect::<Vec<_>>()
        .join(", ");
    assert!(
        link.contains("rel=\"ice-server\""),
        "Link: rel=ice-server expected when ice_servers configured. Got: {link}"
    );

    serve.abort();
}

#[tokio::test]
async fn whip_post_rejects_wrong_content_type() {
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve = tokio::spawn({
        let adapter = Arc::clone(&adapter);
        async move {
            let _ = whip::serve_listener(listener, adapter).await;
        }
    });

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("http");
    let resp = http
        .post(format!("http://{addr}/whip/live"))
        .header("content-type", "text/plain")
        .body("not sdp")
        .send()
        .await
        .expect("post");
    assert_eq!(resp.status(), reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE);

    serve.abort();
}

#[tokio::test]
async fn healthz_and_readyz_respond() {
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve = tokio::spawn({
        let adapter = Arc::clone(&adapter);
        async move {
            let _ = whip::serve_listener(listener, adapter).await;
        }
    });

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("http");
    let h = http
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .expect("healthz");
    assert_eq!(h.status(), reqwest::StatusCode::OK);
    assert_eq!(h.text().await.unwrap(), "ok");

    let r = http
        .get(format!("http://{addr}/readyz"))
        .send()
        .await
        .expect("readyz");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body = r.text().await.unwrap();
    assert!(body.contains("active_sessions"));

    serve.abort();
}

#[tokio::test]
async fn whip_per_ip_rate_limit_429s_after_cap() {
    let mut config = WebRtcConfig::loopback();
    config.whip_per_ip_per_min = 2; // very small cap
    let adapter = WebRtcAdapter::new(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve = tokio::spawn({
        let adapter = Arc::clone(&adapter);
        async move {
            let _ = whip::serve_listener(listener, adapter).await;
        }
    });

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("http");

    let mut statuses = Vec::new();
    for _ in 0..5 {
        let resp = http
            .post(format!("http://{addr}/whip/live"))
            .header("content-type", "application/sdp")
            .body("v=0\r\nm=audio 0 RTP/AVP 0\r\n")
            .send()
            .await
            .expect("post");
        statuses.push(resp.status());
    }
    let too_many = statuses
        .iter()
        .filter(|s| **s == reqwest::StatusCode::TOO_MANY_REQUESTS)
        .count();
    assert!(
        too_many >= 1,
        "expected at least one 429 after cap exhausted, got {statuses:?}"
    );

    serve.abort();
}

#[tokio::test]
async fn session_cap_returns_503() {
    let mut config = WebRtcConfig::loopback();
    config.max_concurrent_sessions = 1;
    let adapter = WebRtcAdapter::new(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve = tokio::spawn({
        let adapter = Arc::clone(&adapter);
        async move {
            let _ = whip::serve_listener(listener, adapter).await;
        }
    });

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http");

    let offer1 = make_offer().await;
    let r1 = http
        .post(format!("http://{addr}/whip/live"))
        .header("content-type", "application/sdp")
        .body(offer1)
        .send()
        .await
        .expect("first post");
    assert_eq!(r1.status(), reqwest::StatusCode::CREATED);

    let offer2 = make_offer().await;
    let r2 = http
        .post(format!("http://{addr}/whip/live"))
        .header("content-type", "application/sdp")
        .body(offer2)
        .send()
        .await
        .expect("second post");
    assert_eq!(
        r2.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "second WHIP POST should 503 when session cap hit; got {}",
        r2.status()
    );

    let m = adapter.metrics();
    assert!(
        m.sessions_rejected_over_cap >= 1,
        "metrics should track rejections"
    );

    serve.abort();
}
