//! Adapter integration test per `UCTP_IMPLEMENTATION_PLAN.md` §4.6.
//!
//! Subscribes to the adapter's `AdapterEvent` stream and verifies that
//! `AdapterEvent::InboundConnection` fires when a peer sends
//! `session.invite`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{AdapterEvent, AdapterKind, ConnectionAdapter, EndReason};
use rvoip_core::connection::Transport;
use rvoip_core::error::RvoipError;
use rvoip_quic::{UctpQuicAdapter, UctpQuicClient, UctpQuicConfig};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite};
use rvoip_uctp::substrate::{dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;

const ALPN_UCTP: &[u8] = b"uctp/1";

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn server_endpoint(
    addr: SocketAddr,
) -> (
    Arc<quinn::Endpoint>,
    rustls::pki_types::CertificateDer<'static>,
) {
    let (cert_der, key_der) = self_signed_for_dev(&["localhost".into()]).expect("self_signed");
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server tls");
    tls.alpn_protocols = vec![ALPN_UCTP.to_vec()];

    let endpoint = rvoip_uctp::substrate::make_server_endpoint(
        addr,
        Arc::new(tls),
        quinn::TransportConfig::default(),
    )
    .expect("endpoint");
    (Arc::new(endpoint), cert_der)
}

fn client_endpoint() -> Arc<quinn::Endpoint> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
    Arc::new(
        quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            None,
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .expect("client endpoint"),
    )
}

#[tokio::test]
async fn adapter_emits_inbound_connection_on_session_invite() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");

    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP]).expect("dispatcher");
    let accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");

    let cfg = UctpQuicConfig::new(Arc::clone(&server_ep), accept_rx, bearer_stub());
    let adapter = UctpQuicAdapter::new(cfg).await.expect("adapter");

    // transport() + kind() are real per design doc §4.4.
    assert_eq!(adapter.transport(), Transport::Quic);
    assert_eq!(adapter.kind(), AdapterKind::Substrate);
    let lifecycle = adapter.lifecycle_capabilities();
    assert!(lifecycle.authoritative_liveness);
    assert!(lifecycle.atomic_inbound_handoff);
    assert!(lifecycle.terminal_fallback);
    assert!(!lifecycle.staged_outbound_activation);

    let mut events = adapter.subscribe_events();

    // --- Client side: dial + send session.invite ---
    let client_ep = client_endpoint();
    let client_cfg =
        rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("client cfg");

    let client =
        UctpQuicClient::connect(&client_ep, server_addr, "localhost", Arc::new(client_cfg))
            .await
            .expect("client connect");

    // A1: drive the four-envelope auth handshake before sending the
    // session.invite. The server coordinator now refuses non-auth
    // envelopes from un-authed peers with 401.
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
                id: "dev_test".into(),
                kind: "desktop".into(),
                platform: "test-platform".into(),
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
        .expect("timeout waiting for auth.challenge")
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
        .expect("timeout waiting for auth.session")
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
        sid: Some("sess_adapter_test".into()),
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(payload).unwrap(),
        signature: None,
    };
    client.send(env).await.expect("send invite");

    // The adapter MUST emit InboundConnection (per H2 fix). After A1,
    // a `Native { kind: "uctp.authenticated", .. }` event lands first
    // (from the bearer handshake driven above); skip it.
    let event = loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timeout waiting for InboundConnection")
            .expect("event channel closed");
        if matches!(&ev, AdapterEvent::Native { kind, .. } if *kind == "uctp.authenticated") {
            continue;
        }
        break ev;
    };

    let core_connection_id = match event {
        AdapterEvent::InboundConnection { connection } => {
            assert_eq!(connection.transport, Transport::Quic);
            assert!(
                connection
                    .session_id
                    .as_str()
                    .ends_with(":sess_adapter_test"),
                "the safe default must retain the wire ID inside a peer-scoped namespace"
            );
            assert_ne!(connection.session_id.as_str(), "sess_adapter_test");
            assert_eq!(
                connection.participant_id.as_str(),
                authenticated_participant.as_str()
            );

            // Streams are created from negotiated `connection.offer` data,
            // never synthetically at invite time.
            assert!(connection.streams.is_empty());
            let via_adapter = adapter
                .streams(connection.id.clone())
                .await
                .expect("streams ok");
            assert!(via_adapter.is_empty());
            connection.id
        }
        other => panic!("expected InboundConnection, got {:?}", other),
    };

    let error = adapter
        .send_data_message(
            core_connection_id.clone(),
            rvoip_core::DataMessage::reliable(
                "bridgefu.context.v1",
                "text/plain",
                "must not leak a core connection ID",
            ),
        )
        .await
        .expect_err("outbound data before connection.offer must fail");
    let RvoipError::Adapter(detail) = &error else {
        panic!("expected typed adapter error, got {error:?}");
    };
    assert!(detail.contains("not ready"));
    assert!(!error.to_string().contains(detail));

    let wire_connection_id = "conn_quic_wire_a";
    client
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::to_value(rvoip_uctp::payloads::connection::ConnectionOffer {
                    by_participant: "part_alice".into(),
                    substrate: "quic".into(),
                    capabilities: serde_json::Value::Object(Default::default()),
                    streams_offered: vec![rvoip_uctp::payloads::connection::StreamOffer {
                        id: "strm_quic_data_a".into(),
                        kind: "audio".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["opus".into()],
                    }],
                    substrate_setup: serde_json::Value::Null,
                })
                .unwrap(),
            )
            .with_sid("sess_adapter_test")
            .with_connid(wire_connection_id),
        )
        .await
        .expect("bind first wire connection");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.expect("event channel closed"),
                AdapterEvent::Native { kind: "uctp.connection_bound", detail }
                    if detail == wire_connection_id
            ) {
                break;
            }
        }
    })
    .await
    .expect("first connection binding timeout");

    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::SessionInvite,
            id: "env_inv_other".into(),
            ts: Utc::now(),
            cid: Some("conv_y".into()),
            sid: Some("sess_adapter_other".into()),
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
                    .ends_with(":sess_adapter_other")
                {
                    break connection.id;
                }
            }
        }
    })
    .await
    .expect("second inbound connection timeout");
    assert_ne!(second_core_connection_id, core_connection_id);

    let second_wire_connection_id = "conn_quic_wire_b";
    client
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::to_value(rvoip_uctp::payloads::connection::ConnectionOffer {
                    by_participant: "part_alice".into(),
                    substrate: "quic".into(),
                    capabilities: serde_json::Value::Object(Default::default()),
                    streams_offered: vec![rvoip_uctp::payloads::connection::StreamOffer {
                        id: "strm_quic_data_b".into(),
                        kind: "audio".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["opus".into()],
                    }],
                    substrate_setup: serde_json::Value::Null,
                })
                .unwrap(),
            )
            .with_sid("sess_adapter_other")
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
            "sess_adapter_test",
            wire_connection_id,
            "first route payload",
        ),
        (
            "conv_y",
            "sess_adapter_other",
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

    let peer_terminal = UctpEnvelope::new(
        MessageType::SessionEnd,
        serde_json::to_value(rvoip_uctp::payloads::session::SessionEnd {
            by: "part_alice".into(),
            reason_code: 0,
            reason: "racing peer terminal".into(),
        })
        .expect("terminal payload"),
    )
    .with_sid("sess_adapter_test");
    let (local_end, peer_end) = tokio::join!(
        adapter.end(core_connection_id.clone(), EndReason::Normal),
        client.send(peer_terminal),
    );
    local_end.expect("local end");
    peer_end.expect("peer terminal send");
    assert!(!adapter.is_connection_live(&core_connection_id));
    let terminal = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(AdapterEvent::Ended { connection_id, .. }) = events.recv().await {
                break connection_id;
            }
        }
    })
    .await
    .expect("terminal event");
    assert_eq!(terminal, core_connection_id);
    adapter
        .end(core_connection_id.clone(), EndReason::Normal)
        .await
        .expect("repeated end is idempotent");

    let duplicate = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if matches!(
                events.recv().await,
                Some(AdapterEvent::Ended { connection_id, .. }) if connection_id == core_connection_id
            ) {
                break;
            }
        }
    })
    .await;
    assert!(
        duplicate.is_err(),
        "racing terminal must be emitted exactly once"
    );
    adapter
        .end(second_core_connection_id, EndReason::Normal)
        .await
        .expect("end second route");
}
