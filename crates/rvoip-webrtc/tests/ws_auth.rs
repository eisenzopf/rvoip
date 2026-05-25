//! G2 — WebSocket signaling authentication tests.
//!
//! Tokens may arrive as `?access_token=...` query param or as a
//! `token.<value>` entry in `Sec-WebSocket-Protocol`.

#![cfg(feature = "signaling-ws")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::signaling::auth::{AnonymousAuth, BearerStaticTokenAuth};
use rvoip_webrtc::signaling::websocket;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, http::header::HeaderValue, Message,
};

async fn start_anonymous_ws() -> (Arc<WebRtcAdapter>, String) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_adapter = Arc::clone(&adapter);
    tokio::spawn(async move {
        websocket::serve_listener_with_auth(listener, serve_adapter, Arc::new(AnonymousAuth))
            .await
            .ok();
    });
    (adapter, format!("ws://{addr}"))
}

async fn start_bearer_ws(token: &str) -> (Arc<WebRtcAdapter>, String) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_adapter = Arc::clone(&adapter);
    let auth = Arc::new(BearerStaticTokenAuth::new(token));
    tokio::spawn(async move {
        websocket::serve_listener_with_auth(listener, serve_adapter, auth)
            .await
            .ok();
    });
    (adapter, format!("ws://{addr}"))
}

#[tokio::test]
async fn ws_anonymous_default_allows_upgrade() {
    let (_adapter, url) = start_anonymous_ws().await;
    let res = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&url),
    )
    .await
    .expect("connect timeout");
    assert!(res.is_ok(), "anonymous upgrade should succeed: {res:?}");
}

#[tokio::test]
async fn ws_bearer_hook_closes_connection_when_no_token() {
    let (_adapter, url) = start_bearer_ws("secret").await;
    let (mut ws, _resp) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&url),
    )
    .await
    .expect("connect timeout")
    .expect("tcp + upgrade");

    // Hook runs async after upgrade; we should see a Close frame promptly.
    use futures::StreamExt;
    let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("timeout waiting for close")
        .expect("stream ended");
    match msg {
        Ok(Message::Close(Some(frame))) => {
            assert_eq!(
                u16::from(frame.code),
                4401,
                "expected custom close code 4401, got {:?}",
                frame.code
            );
        }
        other => panic!("expected Close(4401), got {other:?}"),
    }
}

#[tokio::test]
async fn ws_bearer_hook_accepts_token_via_subprotocol() {
    let (_adapter, url) = start_bearer_ws("secret").await;
    let mut req = url.into_client_request().expect("client req");
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static("rvoip.webrtc.v1, token.secret"),
    );
    let (mut ws, _resp) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(req),
    )
    .await
    .expect("connect timeout")
    .expect("upgrade + auth");

    // Send a no-op bye to confirm the connection is live.
    use futures::SinkExt;
    let body = r#"{"type":"bye"}"#;
    ws.send(Message::Text(body.into())).await.expect("send");
}

#[tokio::test]
async fn ws_bearer_hook_accepts_token_via_query_param() {
    let (_adapter, url) = start_bearer_ws("qsecret").await;
    let full = format!("{url}/?access_token=qsecret");
    let res = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&full),
    )
    .await
    .expect("connect timeout");
    assert!(res.is_ok(), "query-token upgrade should succeed: {res:?}");
}
