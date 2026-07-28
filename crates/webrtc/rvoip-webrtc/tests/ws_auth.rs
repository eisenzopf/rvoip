//! G2 — WebSocket signaling authentication tests.
//!
//! Tokens may arrive as `?access_token=...` query param or as a
//! `token.<value>` entry in `Sec-WebSocket-Protocol`.

#![cfg(feature = "signaling-ws")]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_webrtc::signaling::auth::{
    AnonymousAuth, AuthContext, AuthRejection, BearerStaticTokenAuth, WsAuthHook,
};
use rvoip_webrtc::signaling::websocket;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, http::header::HeaderValue, Error, Message,
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

async fn start_ws_with_auth(auth: Arc<dyn WsAuthHook>) -> (Arc<WebRtcAdapter>, String) {
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_adapter = Arc::clone(&adapter);
    tokio::spawn(async move {
        websocket::serve_listener_with_auth(listener, serve_adapter, auth)
            .await
            .ok();
    });
    (adapter, format!("ws://{addr}"))
}

struct AlwaysReject(AuthRejection);

#[async_trait]
impl WsAuthHook for AlwaysReject {
    async fn authenticate(
        &self,
        _subprotocols: &[String],
        _query_token: Option<&str>,
        _peer_addr: std::net::SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        Err(self.0.clone())
    }
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
async fn ws_bearer_hook_rejects_before_upgrade_with_challenge() {
    let (_adapter, url) = start_bearer_ws("secret").await;
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&url),
    )
    .await
    .expect("connect timeout")
    .expect_err("unauthorized request must not upgrade");
    match error {
        Error::Http(response) => {
            assert_eq!(response.status(), 401);
            assert_eq!(
                response
                    .headers()
                    .get("www-authenticate")
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer realm=\"rvoip\"")
            );
        }
        other => panic!("expected pre-upgrade HTTP 401, got {other:?}"),
    }
}

#[tokio::test]
async fn ws_forbidden_and_throttled_rejections_preserve_http_status_and_headers() {
    let (_adapter, forbidden_url) =
        start_ws_with_auth(Arc::new(AlwaysReject(AuthRejection::Forbidden))).await;
    let forbidden = tokio_tungstenite::connect_async(forbidden_url)
        .await
        .expect_err("forbidden request must not upgrade");
    match forbidden {
        Error::Http(response) => assert_eq!(response.status(), 403),
        other => panic!("expected pre-upgrade HTTP 403, got {other:?}"),
    }

    let (_adapter, throttled_url) =
        start_ws_with_auth(Arc::new(AlwaysReject(AuthRejection::Throttled {
            retry_after_secs: 17,
        })))
        .await;
    let throttled = tokio_tungstenite::connect_async(throttled_url)
        .await
        .expect_err("throttled request must not upgrade");
    match throttled {
        Error::Http(response) => {
            assert_eq!(response.status(), 429);
            assert_eq!(
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok()),
                Some("17")
            );
        }
        other => panic!("expected pre-upgrade HTTP 429, got {other:?}"),
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
