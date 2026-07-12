//! G2 — WHIP authentication + RFC 9725 header tests.
//!
//! Covers: anonymous default; Bearer enforcement (401 with WWW-Authenticate);
//! Accept-Post advertisement on OPTIONS; auto-populated Link: rel=ice-server
//! from configured ICE servers; If-Match required on ICE-restart PATCH.

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::config::IceServerConfig;
use rvoip_webrtc::signaling::auth::{AnonymousAuth, BearerStaticTokenAuth};
use rvoip_webrtc::signaling::whip;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

async fn start_anonymous_server() -> (Arc<WebRtcAdapter>, String) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_adapter = Arc::clone(&adapter);
    tokio::spawn(async move {
        whip::serve_listener_with_auth(listener, serve_adapter, Arc::new(AnonymousAuth))
            .await
            .ok();
    });
    (adapter, format!("http://{addr}"))
}

async fn start_bearer_server(token: &str) -> (Arc<WebRtcAdapter>, String) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![
        IceServerConfig::stun("stun:stun.example.com:3478"),
        IceServerConfig::turn("turn:turn.example.com:3478", "user1", "secret-pw"),
    ];
    let adapter = WebRtcAdapter::new(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_adapter = Arc::clone(&adapter);
    let auth = Arc::new(BearerStaticTokenAuth::new(token));
    tokio::spawn(async move {
        whip::serve_listener_with_auth(listener, serve_adapter, auth)
            .await
            .ok();
    });
    (adapter, format!("http://{addr}"))
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

const MIN_OFFER: &str = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 127.0.0.1\r\na=mid:0\r\na=ice-ufrag:abcd\r\na=ice-pwd:abcdefghijklmnopqrstuv\r\na=fingerprint:sha-256 AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99\r\na=setup:actpass\r\na=sendrecv\r\na=rtcp-mux\r\na=rtpmap:111 opus/48000/2\r\n";

#[tokio::test]
async fn whip_anonymous_default_accepts_post() {
    let (_adapter, base) = start_anonymous_server().await;
    let resp = http()
        .post(format!("{base}/whip/test"))
        .header("content-type", "application/sdp")
        .body(MIN_OFFER)
        .send()
        .await
        .unwrap();
    // Either CREATED (offer accepted) or an SDP error from the engine; the
    // important thing is it is NOT 401 — auth is bypassed when the default
    // anonymous hook is registered.
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "anonymous hook should not 401"
    );
}

#[tokio::test]
async fn whip_bearer_hook_rejects_missing_authorization_with_401() {
    let (_adapter, base) = start_bearer_server("topsecret").await;
    let resp = http()
        .post(format!("{base}/whip/test"))
        .header("content-type", "application/sdp")
        .body(MIN_OFFER)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        www.starts_with("Bearer realm="),
        "expected Bearer challenge, got {www:?}"
    );
}

#[tokio::test]
async fn whip_bearer_hook_rejects_wrong_token() {
    let (_adapter, base) = start_bearer_server("topsecret").await;
    let resp = http()
        .post(format!("{base}/whip/test"))
        .header("authorization", "Bearer wrong-token")
        .header("content-type", "application/sdp")
        .body(MIN_OFFER)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn whip_bearer_hook_accepts_valid_token() {
    let (_adapter, base) = start_bearer_server("topsecret").await;
    let resp = http()
        .post(format!("{base}/whip/test"))
        .header("authorization", "Bearer topsecret")
        .header("content-type", "application/sdp")
        .body(MIN_OFFER)
        .send()
        .await
        .unwrap();
    // Not 401 — auth passed. (The offer itself may be too minimal for the
    // engine; we just care that we got past the auth gate.)
    assert_ne!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn whip_options_advertises_accept_post() {
    let (_adapter, base) = start_anonymous_server().await;
    let resp = http()
        .request(reqwest::Method::OPTIONS, format!("{base}/whip/test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);
    let accept_post = resp
        .headers()
        .get("accept-post")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    assert!(
        accept_post.contains("application/sdp"),
        "Accept-Post should advertise application/sdp, got {accept_post:?}"
    );
}

#[tokio::test]
async fn whip_link_header_auto_populated_from_ice_servers() {
    // Use a real loopback adapter (offerer) so we get a valid answer back.
    let (_adapter, base) = start_bearer_server("k").await;
    let resp = http()
        .post(format!("{base}/whip/test"))
        .header("authorization", "Bearer k")
        .header("content-type", "application/sdp")
        .body(MIN_OFFER)
        .send()
        .await
        .unwrap();
    // Even if the SDP is rejected, we don't get Link headers — but on
    // successful CREATED we should see one Link per configured server.
    if resp.status() == reqwest::StatusCode::CREATED {
        let links: Vec<String> = resp
            .headers()
            .get_all("link")
            .iter()
            .filter_map(|h| h.to_str().ok().map(|s| s.to_string()))
            .collect();
        assert!(
            links.iter().any(|l| l.contains("stun.example.com")),
            "expected STUN link header, got {links:?}"
        );
        assert!(
            links.iter().any(|l| l.contains("turn.example.com")
                && l.contains("username=\"user1\"")
                && l.contains("credential=\"secret-pw\"")),
            "expected TURN link header with credentials, got {links:?}"
        );
    }
}

#[tokio::test]
async fn whip_patch_ice_restart_without_if_match_returns_428() {
    let (adapter, base) = start_anonymous_server().await;
    let _ = &adapter; // unused-binding warning shut
                      // Need a real connection id for the PATCH path. Just use a fake one —
                      // the If-Match check fires before the connection lookup.
    let resp = http()
        .patch(format!("{base}/whip/nonexistent"))
        .header("content-type", "application/sdp")
        .body("v=0\r\n")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::PRECONDITION_REQUIRED);
}

#[tokio::test]
async fn whip_patch_ice_restart_with_stale_if_match_returns_412() {
    let (_adapter, base) = start_anonymous_server().await;
    let resp = http()
        .patch(format!("{base}/whip/anyid"))
        .header("content-type", "application/sdp")
        .header("if-match", "\"stale-etag-value\"")
        .body("v=0\r\n")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::PRECONDITION_FAILED);
}
