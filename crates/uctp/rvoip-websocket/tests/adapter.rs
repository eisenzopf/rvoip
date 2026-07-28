//! Adapter integration: assert `AdapterEvent::InboundConnection` fires
//! on inbound `session.invite` over WebSocket.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{AdapterEvent, AdapterKind, ConnectionAdapter, EndReason, RejectReason};
use rvoip_core::connection::Transport;
use rvoip_core::events::Event;
use rvoip_core::{Config, Orchestrator};
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
    let lifecycle = adapter.lifecycle_capabilities();
    assert!(lifecycle.authoritative_liveness);
    assert!(lifecycle.atomic_inbound_handoff);
    assert!(lifecycle.terminal_fallback);
    assert!(!lifecycle.staged_outbound_activation);

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
    let authenticated_participant = session_reply
        .decode_payload::<auth::AuthSession>()
        .expect("decode auth.session")
        .participant_id;

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

    let core_connection_id = match event {
        AdapterEvent::InboundConnection { connection } => {
            assert_eq!(connection.transport, Transport::WebSocket);
            assert_eq!(connection.session_id.as_str(), "sess_ws_adapter_test");
            assert_eq!(
                connection.participant_id.as_str(),
                authenticated_participant.as_str()
            );
            connection.id
        }
        other => panic!("expected InboundConnection, got {:?}", other),
    };

    let wire_connection_id = "conn_ws_wire";
    client
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::to_value(rvoip_uctp::payloads::connection::ConnectionOffer {
                    by_participant: "part_alice".into(),
                    substrate: "websocket".into(),
                    capabilities: serde_json::Value::Object(Default::default()),
                    streams_offered: vec![rvoip_uctp::payloads::connection::StreamOffer {
                        id: "strm_ws_data".into(),
                        kind: "audio".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["opus".into()],
                    }],
                    substrate_setup: serde_json::Value::Null,
                })
                .unwrap(),
            )
            .with_sid("sess_ws_adapter_test")
            .with_connid(wire_connection_id),
        )
        .await
        .expect("send connection offer");
    client
        .send(
            UctpEnvelope::new(
                MessageType::MessageSend,
                serde_json::json!({
                    "msg_id": "msg_ws_data",
                    "from": "part_alice",
                    "to": "all",
                    "content_type": "text/plain",
                    "label": "bridgefu.context.v1",
                    "body": "hello over WebSocket",
                    "body_encoding": "utf8",
                    "attachments": []
                }),
            )
            .with_sid("sess_ws_adapter_test")
            .with_connid(wire_connection_id),
        )
        .await
        .expect("send data message");

    let received_connection = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let AdapterEvent::DataMessage {
                connection_id,
                message,
            } = events.recv().await.expect("event channel closed")
            {
                assert_eq!(message.bytes.as_ref(), b"hello over WebSocket");
                break connection_id;
            }
        }
    })
    .await
    .expect("DataMessage timeout");
    assert_eq!(received_connection, core_connection_id);
    assert_ne!(received_connection.as_str(), wire_connection_id);

    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::SessionInvite,
            id: "env_inv_other".into(),
            ts: Utc::now(),
            cid: Some("conv_y".into()),
            sid: Some("sess_ws_adapter_other".into()),
            connid: None,
            in_reply_to: None,
            payload: serde_json::to_value(SessionInvite {
                from: "part_alice".into(),
                to: vec!["part_bob".into()],
                medium: "voice".into(),
                intent: "synchronous-engagement".into(),
                capabilities_offer: serde_json::Value::Object(Default::default()),
            })
            .unwrap(),
            signature: None,
        })
        .await
        .expect("send second invite");
    let second_core_connection_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let AdapterEvent::InboundConnection { connection } =
                events.recv().await.expect("event channel closed")
            {
                if connection
                    .session_id
                    .as_str()
                    .ends_with("sess_ws_adapter_other")
                {
                    break connection.id;
                }
            }
        }
    })
    .await
    .expect("second inbound connection timeout");
    assert_ne!(second_core_connection_id, core_connection_id);

    let second_wire_connection_id = "conn_ws_wire_other";
    client
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::to_value(rvoip_uctp::payloads::connection::ConnectionOffer {
                    by_participant: "part_alice".into(),
                    substrate: "websocket".into(),
                    capabilities: serde_json::Value::Object(Default::default()),
                    streams_offered: vec![rvoip_uctp::payloads::connection::StreamOffer {
                        id: "strm_ws_data_other".into(),
                        kind: "audio".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["opus".into()],
                    }],
                    substrate_setup: serde_json::Value::Null,
                })
                .unwrap(),
            )
            .with_sid("sess_ws_adapter_other")
            .with_connid(second_wire_connection_id),
        )
        .await
        .expect("bind second wire connection");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.expect("event channel closed"),
                AdapterEvent::Native { kind: "uctp.connection_bound", detail }
                    if detail == second_wire_connection_id
            ) {
                break;
            }
        }
    })
    .await
    .expect("second connection binding timeout");

    adapter
        .send_data_message(
            core_connection_id.clone(),
            rvoip_core::DataMessage::reliable("route.test", "text/plain", "first route payload"),
        )
        .await
        .expect("send first route data");
    adapter
        .send_data_message(
            second_core_connection_id.clone(),
            rvoip_core::DataMessage::reliable("route.test", "text/plain", "second route payload"),
        )
        .await
        .expect("send second route data");

    for (cid, sid, connid, body) in [
        (
            "conv_x",
            "sess_ws_adapter_test",
            wire_connection_id,
            "first route payload",
        ),
        (
            "conv_y",
            "sess_ws_adapter_other",
            second_wire_connection_id,
            "second route payload",
        ),
    ] {
        let envelope = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let envelope = inbound.recv().await.expect("client inbound closed");
                if envelope.msg_type == MessageType::MessageSend {
                    break envelope;
                }
            }
        })
        .await
        .expect("outbound data timeout");
        let payload = envelope
            .decode_payload::<rvoip_uctp::payloads::message::MessageSend>()
            .expect("decode outbound message");
        assert_eq!(envelope.cid.as_deref(), Some(cid));
        assert_eq!(envelope.sid.as_deref(), Some(sid));
        assert_eq!(envelope.connid.as_deref(), Some(connid));
        assert_eq!(payload.body, body);
    }

    adapter
        .end(core_connection_id, EndReason::Normal)
        .await
        .expect("end first route");
    adapter
        .end(second_core_connection_id, EndReason::Normal)
        .await
        .expect("end second route");
}

#[tokio::test]
async fn core_reject_retires_route_and_suppresses_late_peer_terminal() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = listener.local_addr().expect("local_addr");
    let adapter = UctpWsAdapter::new(UctpWsConfig::new(listener, bearer_stub()))
        .await
        .expect("adapter");
    let capabilities = adapter.lifecycle_capabilities();
    assert!(capabilities.authoritative_liveness);
    assert!(capabilities.atomic_inbound_handoff);
    assert!(capabilities.terminal_fallback);
    assert!(!capabilities.staged_outbound_activation);

    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .expect("admission gate");
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register first-party adapter");
    let mut normalized = orchestrator.subscribe_events();

    let url = Url::parse(&format!("ws://{server_addr}")).expect("url");
    let client = UctpWsClient::connect(&url).await.expect("client connect");
    let mut inbound = client.take_inbound().expect("take inbound");
    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthHello,
            id: "reject_hello".into(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: None,
            payload: serde_json::to_value(auth::AuthHello {
                device: auth::Device {
                    id: "dev_ws_reject".into(),
                    kind: "browser".into(),
                    platform: "test".into(),
                    sdk_version: "test/0.1".into(),
                },
                auth_methods: vec!["bearer".into()],
                capabilities: serde_json::Value::Object(Default::default()),
            })
            .expect("hello payload"),
            signature: None,
        })
        .await
        .expect("send hello");
    let challenge = tokio::time::timeout(Duration::from_secs(2), inbound.recv())
        .await
        .expect("challenge timeout")
        .expect("challenge");
    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthResponse,
            id: "reject_response".into(),
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
            .expect("response payload"),
            signature: None,
        })
        .await
        .expect("send auth response");
    let auth_session = tokio::time::timeout(Duration::from_secs(2), inbound.recv())
        .await
        .expect("auth session timeout")
        .expect("auth session");
    assert_eq!(auth_session.msg_type, MessageType::AuthSession);

    client
        .send(
            UctpEnvelope::new(
                MessageType::SessionInvite,
                serde_json::to_value(SessionInvite {
                    from: "part_reject".into(),
                    to: vec!["part_bridge".into()],
                    medium: "voice".into(),
                    intent: "synchronous-engagement".into(),
                    capabilities_offer: serde_json::Value::Object(Default::default()),
                })
                .expect("invite payload"),
            )
            .with_sid("sess_ws_reject"),
        )
        .await
        .expect("send invite");

    let admission = tokio::time::timeout(Duration::from_secs(2), admissions.recv())
        .await
        .expect("admission timeout")
        .expect("admission");
    let connection_id = admission.connection_id().clone();
    admission
        .reject(RejectReason::Forbidden)
        .await
        .expect("core rejection");
    assert!(!adapter.is_connection_live(&connection_id));

    let rejection = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let envelope = inbound.recv().await.expect("inbound remains open");
            if envelope.msg_type == MessageType::SessionReject {
                break envelope;
            }
        }
    })
    .await
    .expect("wire rejection");
    assert_eq!(rejection.sid.as_deref(), Some("sess_ws_reject"));

    client
        .send(
            UctpEnvelope::new(
                MessageType::SessionEnd,
                serde_json::to_value(rvoip_uctp::payloads::session::SessionEnd {
                    by: "part_reject".into(),
                    reason_code: 0,
                    reason: "late duplicate".into(),
                })
                .expect("terminal payload"),
            )
            .with_sid("sess_ws_reject"),
        )
        .await
        .expect("late terminal send");
    let duplicate = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            match normalized.recv().await {
                Ok(
                    Event::ConnectionEnded {
                        connection_id: id, ..
                    }
                    | Event::ConnectionFailed {
                        connection_id: id, ..
                    },
                ) if id == connection_id => {
                    break;
                }
                Ok(_) => {}
                Err(_) => std::future::pending::<()>().await,
            }
        }
    })
    .await;
    assert!(
        duplicate.is_err(),
        "rejected pre-publication route must not emit duplicate normalized terminal"
    );
}
