//! WebSocket loopback test — equivalent of rvoip-quic's loopback but
//! over plain `ws://` (no TLS, no ALPN, no QUIC handshake to wait on).
//!
//! Asserts the same end-to-end shape: bind on a kernel port, dial via
//! `UctpWsClient`, send `auth.hello`, receive `auth.challenge`.

use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::auth;
use rvoip_uctp::types::MessageType;
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig};
use tokio::net::TcpListener;
use url::Url;

#[tokio::test]
async fn loopback_auth_handshake_via_adapter() {
    let _ = tracing_subscriber::fmt::try_init();

    // --- Server ---
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = listener.local_addr().expect("local_addr");

    let cfg = UctpWsConfig::new(listener, bearer_stub());
    let _adapter = UctpWsAdapter::new(cfg).await.expect("adapter");

    // --- Client ---
    let url = Url::parse(&format!("ws://{}", server_addr)).expect("parse url");
    let client = UctpWsClient::connect(&url).await.expect("client connect");
    let mut inbound = client.take_inbound().expect("first take");

    // Send auth.hello.
    let payload = auth::AuthHello {
        device: auth::Device {
            id: "dev_ws_test".into(),
            kind: "desktop".into(),
            platform: "test".into(),
            sdk_version: "rvoip-ws-test/0.1".into(),
        },
        auth_methods: vec!["bearer".into()],
        capabilities: serde_json::Value::Object(Default::default()),
    };
    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: "env_hello".into(),
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
        .expect("timed out")
        .expect("inbound closed");

    assert_eq!(reply.msg_type, MessageType::AuthChallenge);
}
