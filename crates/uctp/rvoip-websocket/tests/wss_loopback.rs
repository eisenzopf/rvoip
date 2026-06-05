//! Gap plan §2.3 — WSS substrate (TLS on the WS signaling channel).
//!
//! Mirrors `tests/loopback.rs` but the WS server terminates TLS via
//! `tokio_rustls::TlsAcceptor` (gated by the `wss` feature) and the
//! client dials `wss://` pinning the self-signed cert.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::auth;
use rvoip_uctp::substrate::tls::{dev_client_config_trusting, self_signed_for_dev};
use rvoip_uctp::types::MessageType;
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig};
use tokio::net::TcpListener;
use url::Url;

#[tokio::test]
async fn loopback_auth_handshake_over_wss() {
    let _ = tracing_subscriber::fmt::try_init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    // --- TLS cert (self-signed for `localhost`) ---
    let (cert, key) = self_signed_for_dev(&["localhost".into()]).expect("self-signed");
    let mut server_tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)
        .expect("server tls cfg");
    // No ALPN required for WebSocket-over-TLS.
    server_tls.alpn_protocols = vec![];
    let server_tls = Arc::new(server_tls);

    // --- Server (WSS) ---
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = listener.local_addr().expect("local_addr");

    let cfg = UctpWsConfig::new(listener, bearer_stub()).with_tls(server_tls);
    let _adapter = UctpWsAdapter::new(cfg).await.expect("adapter");

    // --- Client (wss://, pinning the server cert) ---
    let client_tls = dev_client_config_trusting(&cert).expect("client tls cfg");
    let url = Url::parse(&format!("wss://localhost:{}", server_addr.port())).expect("parse url");
    let client = UctpWsClient::connect_with_tls(&url, Arc::new(client_tls))
        .await
        .expect("client connect over wss");
    let mut inbound = client.take_inbound().expect("first take");

    // Drive auth.hello → assert auth.challenge over the TLS-wrapped link.
    let payload = auth::AuthHello {
        device: auth::Device {
            id: "dev_wss_test".into(),
            kind: "desktop".into(),
            platform: "test".into(),
            sdk_version: "rvoip-wss-test/0.1".into(),
        },
        auth_methods: vec!["bearer".into()],
        capabilities: serde_json::Value::Object(Default::default()),
    };
    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: "env_wss_hello".into(),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(payload).unwrap(),
        signature: None,
    };
    client.send(env).await.expect("send");

    let reply = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("timed out waiting for auth.challenge over wss")
        .expect("inbound closed");

    assert_eq!(
        reply.msg_type,
        MessageType::AuthChallenge,
        "expected auth.challenge from server over wss; got {:?}",
        reply.msg_type
    );
}
