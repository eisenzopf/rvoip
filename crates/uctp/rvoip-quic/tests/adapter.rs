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
use rvoip_core::adapter::{AdapterEvent, AdapterKind, ConnectionAdapter};
use rvoip_core::connection::Transport;
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
            assert_eq!(connection.session_id.as_str(), "sess_adapter_test");
            assert_eq!(connection.participant_id.as_str(), "part_alice");

            // SP-D: default audio stream is now populated at InboundInvite
            // time so `Orchestrator::bridge_connections` has something to
            // bridge.
            assert_eq!(
                connection.streams.len(),
                1,
                "expected one default audio stream populated at InboundInvite"
            );
            assert_eq!(
                rvoip_core::stream::MediaStream::kind(connection.streams[0].stream().as_ref()),
                rvoip_core::stream::StreamKind::Audio
            );
            let codec =
                rvoip_core::stream::MediaStream::codec(connection.streams[0].stream().as_ref());
            assert_eq!(codec.name, "opus");

            // The adapter's `streams(id)` returns the same stream by id.
            let via_adapter = adapter
                .streams(connection.id.clone())
                .await
                .expect("streams ok");
            assert_eq!(via_adapter.len(), 1);
            assert_eq!(
                rvoip_core::stream::MediaStream::kind(via_adapter[0].as_ref()),
                rvoip_core::stream::StreamKind::Audio
            );
            connection.id
        }
        other => panic!("expected InboundConnection, got {:?}", other),
    };

    let error = adapter
        .send_data_message(
            core_connection_id,
            rvoip_core::DataMessage::reliable(
                "bridgefu.context.v1",
                "text/plain",
                "must not leak a core connection ID",
            ),
        )
        .await
        .expect_err("outbound data before connection.offer must fail");
    assert!(error.to_string().contains("not ready"));
}
