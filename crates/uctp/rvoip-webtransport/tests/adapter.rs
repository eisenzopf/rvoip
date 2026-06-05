//! Adapter integration test per `UCTP_IMPLEMENTATION_PLAN.md` §5.5.
//!
//! Same shape as `rvoip-quic`'s adapter test, with `Transport::WebTransport`
//! and the WT URL upgrade.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{AdapterEvent, AdapterKind, ConnectionAdapter};
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

    match event {
        AdapterEvent::InboundConnection { connection } => {
            assert_eq!(connection.transport, Transport::WebTransport);
            assert_eq!(connection.session_id.as_str(), "sess_wt_adapter_test");
            assert_eq!(connection.participant_id.as_str(), "part_alice");

            // SP-D: default audio stream is now populated at InboundInvite.
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

            let via_adapter = adapter
                .streams(connection.id.clone())
                .await
                .expect("streams ok");
            assert_eq!(via_adapter.len(), 1);
            assert_eq!(
                rvoip_core::stream::MediaStream::kind(via_adapter[0].as_ref()),
                rvoip_core::stream::StreamKind::Audio
            );
        }
        other => panic!("expected InboundConnection, got {:?}", other),
    }
}
