//! Adapter integration: assert `AdapterEvent::InboundConnection` fires
//! on inbound `session.invite` over WebSocket.

use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{AdapterEvent, AdapterKind, ConnectionAdapter};
use rvoip_core::connection::Transport;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite};
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

    // A1: drive bearer auth before session.invite. Without this the
    // coordinator refuses the invite with 401.
    let mut inbound = client.take_inbound().expect("take_inbound");
    let hello = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: "env_hello".into(),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_ws_test".into(),
                kind: "browser".into(),
                platform: "test".into(),
                sdk_version: "test/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    signature: None,
    };
    client.send(hello).await.expect("send hello");
    let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.challenge timeout")
        .expect("inbound closed");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    let response = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthResponse,
        id: "env_response".into(),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: Some(challenge.id),
        payload: serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "test-token".into(),
        actor_token: None,
        })
        .unwrap(),
    signature: None,
    };
    client.send(response).await.expect("send response");
    let session_reply = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.session timeout")
        .expect("inbound closed");
    assert_eq!(session_reply.msg_type, MessageType::AuthSession);

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
    signature: None,
    };
    client.send(env).await.expect("send invite");

    // The first AdapterEvent is now `Native { kind: "uctp.authenticated" }`
    // (from the bearer handshake driven above); skip past it.
    let event = loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timeout")
            .expect("event channel closed");
        if matches!(&ev, AdapterEvent::Native { kind, .. } if *kind == "uctp.authenticated") {
            continue;
        }
        break ev;
    };

    match event {
        AdapterEvent::InboundConnection { connection } => {
            assert_eq!(connection.transport, Transport::WebSocket);
            assert_eq!(connection.session_id.as_str(), "sess_ws_adapter_test");
            assert_eq!(connection.participant_id.as_str(), "part_alice");
        }
        other => panic!("expected InboundConnection, got {:?}", other),
    }
}
