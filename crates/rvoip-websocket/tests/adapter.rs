//! Adapter integration: assert `AdapterEvent::InboundConnection` fires
//! on inbound `session.invite` over WebSocket.

use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{AdapterEvent, AdapterKind, ConnectionAdapter};
use rvoip_core::connection::Transport;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::session::SessionInvite;
use rvoip_uctp::types::MessageType;
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig};
use tokio::net::TcpListener;
use url::Url;

#[tokio::test]
async fn ws_adapter_emits_inbound_connection_on_session_invite() {
    let _ = tracing_subscriber::fmt::try_init();

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = listener.local_addr().expect("local_addr");

    let cfg = UctpWsConfig::new(listener, bearer_stub());
    let adapter = UctpWsAdapter::new(cfg).await.expect("adapter");

    assert_eq!(adapter.transport(), Transport::WebSocket);
    assert_eq!(adapter.kind(), AdapterKind::Substrate);

    let mut events = adapter.subscribe_events();

    let url = Url::parse(&format!("ws://{}", server_addr)).expect("parse url");
    let client = UctpWsClient::connect(&url).await.expect("client connect");

    let payload = SessionInvite {
        from: "part_alice".into(),
        to: vec!["part_bob".into()],
        medium: "voice".into(),
        intent: "synchronous-engagement".into(),
        capabilities_offer: serde_json::Value::Object(Default::default()),
    };
    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: "env_inv".into(),
        ts: Utc::now(),
        cid: Some("conv_x".into()),
        sid: Some("sess_ws_adapter_test".into()),
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(payload).unwrap(),
    };
    client.send(env).await.expect("send invite");

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout")
        .expect("event channel closed");

    match event {
        AdapterEvent::InboundConnection { connection } => {
            assert_eq!(connection.transport, Transport::WebSocket);
            assert_eq!(connection.session_id.as_str(), "sess_ws_adapter_test");
            assert_eq!(connection.participant_id.as_str(), "part_alice");
        }
        other => panic!("expected InboundConnection, got {:?}", other),
    }
}
