//! Adapter integration test per `UCTP_IMPLEMENTATION_PLAN.md` §5.5.
//!
//! Same shape as `rvoip-quic`'s adapter test, with `Transport::WebTransport`
//! and the WT URL upgrade.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{AdapterEvent, AdapterKind, ConnectionAdapter, EndReason};
use rvoip_core::connection::Transport;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite};
use rvoip_uctp::substrate::{dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;
use rvoip_webtransport::{UctpWtAdapter, UctpWtClient, UctpWtConfig};
use url::Url;

const ALPN_H3: &[u8] = b"h3";

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
    tls.alpn_protocols = vec![ALPN_H3.to_vec()];

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
async fn wt_adapter_emits_inbound_connection_on_session_invite() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");

    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_H3]).expect("dispatcher");
    let accept_rx = routes.take(ALPN_H3).expect("h3 channel");

    let cfg = UctpWtConfig::new(Arc::clone(&server_ep), accept_rx, bearer_stub());
    let adapter = UctpWtAdapter::new(cfg).await.expect("adapter");

    assert_eq!(adapter.transport(), Transport::WebTransport);
    assert_eq!(adapter.kind(), AdapterKind::Substrate);
    let lifecycle = adapter.lifecycle_capabilities();
    assert!(lifecycle.authoritative_liveness);
    assert!(lifecycle.atomic_inbound_handoff);
    assert!(lifecycle.terminal_fallback);
    assert!(!lifecycle.staged_outbound_activation);

    let mut events = adapter.subscribe_events();

    let client_ep = client_endpoint();
    let client_cfg =
        rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("client cfg");

    let url =
        Url::parse(&format!("https://localhost:{}/uctp", server_addr.port())).expect("parse url");
    let client = UctpWtClient::connect(&client_ep, server_addr, &url, Arc::new(client_cfg))
        .await
        .expect("client connect");

    // A1: bearer auth handshake before session.invite.
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
                id: "dev_wt_test".into(),
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
        sid: Some("sess_wt_adapter_test".into()),
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
            .expect("timeout waiting for InboundConnection")
            .expect("event channel closed");
        if matches!(&ev, AdapterEvent::Native { kind, .. } if *kind == "uctp.authenticated") {
            continue;
        }
        break ev;
    };

    let core_connection_id = match event {
        AdapterEvent::InboundConnection { connection } => {
            assert_eq!(connection.transport, Transport::WebTransport);
            assert!(
                connection
                    .session_id
                    .as_str()
                    .ends_with(":sess_wt_adapter_test"),
                "default resolver must retain the wire ID inside a peer-private namespace"
            );
            assert_ne!(connection.session_id.as_str(), "sess_wt_adapter_test");
            assert_eq!(
                connection.participant_id.as_str(),
                authenticated_participant.as_str()
            );

            // Media is bound only after connection.offer/ready supplies the
            // authenticated wire Stream ID and negotiated codec.
            assert_eq!(
                connection.streams.len(),
                0,
                "invite-time synthetic streams would race negotiated binding"
            );

            let via_adapter = adapter
                .streams(connection.id.clone())
                .await
                .expect("streams ok");
            assert!(via_adapter.is_empty());
            connection.id
        }
        other => panic!("expected InboundConnection, got {:?}", other),
    };

    let wire_connection_id = "conn_wt_wire";
    client
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::to_value(rvoip_uctp::payloads::connection::ConnectionOffer {
                    by_participant: "part_alice".into(),
                    substrate: "webtransport".into(),
                    capabilities: serde_json::Value::Object(Default::default()),
                    streams_offered: vec![rvoip_uctp::payloads::connection::StreamOffer {
                        id: "strm_wt_data".into(),
                        kind: "audio".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["opus".into()],
                    }],
                    substrate_setup: serde_json::Value::Null,
                })
                .unwrap(),
            )
            .with_sid("sess_wt_adapter_test")
            .with_connid(wire_connection_id),
        )
        .await
        .expect("send connection offer");
    client
        .send(
            UctpEnvelope::new(MessageType::ConnectionReady, serde_json::json!({}))
                .with_sid("sess_wt_adapter_test")
                .with_connid(wire_connection_id),
        )
        .await
        .expect("send connection ready");

    let bound_streams = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let streams = adapter
                .streams(core_connection_id.clone())
                .await
                .expect("streams ok");
            if !streams.is_empty() {
                break streams;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("negotiated media binding timeout");
    assert_eq!(bound_streams.len(), 1);
    assert_eq!(bound_streams[0].id().as_str(), "strm_wt_data");
    assert_eq!(
        bound_streams[0].kind(),
        rvoip_core::stream::StreamKind::Audio
    );
    assert_eq!(bound_streams[0].codec().name, "opus");

    let opened = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let envelope = inbound.recv().await.expect("client inbound closed");
            if envelope.msg_type == MessageType::StreamOpened {
                break envelope
                    .decode_payload::<rvoip_uctp::payloads::stream::StreamOpened>()
                    .expect("decode stream.opened");
            }
        }
    })
    .await
    .expect("stream.opened timeout");
    assert_eq!(opened.stream.strm_id, "strm_wt_data");
    assert_ne!(opened.stream.stream_local_id, 0);

    let mut media_in = bound_streams[0]
        .try_frames_in()
        .expect("acquire negotiated media receiver");
    let datagram = rvoip_uctp::substrate::pack_rtp_datagram(&rvoip_uctp::substrate::RtpDatagram {
        flags: 0,
        stream_local_id: opened.stream.stream_local_id,
        seq: 9,
        rtp: rvoip_uctp::substrate::RtpMediaPayload {
            payload: bytes::Bytes::from_static(b"negotiated-opus-frame"),
            payload_type: 111,
            sequence_number: 17,
            timestamp: 960,
            ssrc: 0x1020_3040,
        },
    })
    .expect("encode complete RTP datagram");
    client
        .session
        .send_datagram(datagram)
        .expect("send media datagram");
    let media_frame = tokio::time::timeout(Duration::from_secs(5), media_in.recv())
        .await
        .expect("media frame timeout")
        .expect("media receiver closed");
    assert_eq!(media_frame.stream_id.as_str(), "strm_wt_data");
    assert_eq!(media_frame.payload.as_ref(), b"negotiated-opus-frame");
    assert_eq!(media_frame.payload_type, Some(111));
    assert_eq!(media_frame.timestamp_rtp, 960);

    client
        .send(
            UctpEnvelope::new(
                MessageType::MessageSend,
                serde_json::json!({
                    "msg_id": "msg_wt_data",
                    "from": "part_alice",
                    "to": "all",
                    "content_type": "text/plain",
                    "label": "bridgefu.context.v1",
                    "body": "hello over WebTransport",
                    "body_encoding": "utf8",
                    "attachments": []
                }),
            )
            .with_sid("sess_wt_adapter_test")
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
                assert_eq!(message.bytes.as_ref(), b"hello over WebTransport");
                break connection_id;
            }
        }
    })
    .await
    .expect("DataMessage timeout");
    assert_eq!(received_connection, core_connection_id);
    assert_ne!(received_connection.as_str(), wire_connection_id);

    adapter
        .end(core_connection_id.clone(), EndReason::Normal)
        .await
        .expect("local end");
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
        .end(core_connection_id, EndReason::Normal)
        .await
        .expect("repeated end is idempotent");
}
