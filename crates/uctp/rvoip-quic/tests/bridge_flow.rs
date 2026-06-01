//! End-to-end proof: two QUIC clients connect, server bridges them,
//! an RTP frame injected from one client arrives at the other.
//!
//! Without this test we only had channel-level evidence; this exercises
//! the full path including QUIC datagram transmission, the server-side
//! `spawn_datagram_reader`, the cross-transport frame-pump, and the
//! outbound datagram pump on the destination side.
//!
//! Topology (single process):
//!
//!     client A                  ┌── server ──┐                  client B
//!     ┌──────┐                  │            │                  ┌──────┐
//!     │ quinn│──datagrams──────►│ conn A     │                  │ quinn│
//!     │ conn │                  │  stream A  │                  │ conn │
//!     │      │                  │   ▲        │                  │      │
//!     │      │                  │   │ bridge │                  │      │
//!     │      │                  │   ▼        │                  │      │
//!     │      │                  │  stream B  │──datagrams──────►│      │
//!     └──────┘                  │ conn B     │                  └──────┘
//!                               └────────────┘

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, StreamId};
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_core::{Config, Orchestrator};
use rvoip_quic::{
    spawn_datagram_reader, QuicDatagramMediaStream, UctpQuicAdapter, UctpQuicClient, UctpQuicConfig,
};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite};
use rvoip_uctp::substrate::{dev_client_config_trusting, dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;

const ALPN_UCTP: &[u8] = b"uctp/1";

fn rand_hex() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("{:016x}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn server_endpoint(
    addr: SocketAddr,
) -> (Arc<quinn::Endpoint>, rustls::pki_types::CertificateDer<'static>) {
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

async fn dial_and_invite(
    client_ep: &quinn::Endpoint,
    server_addr: SocketAddr,
    cert: &rustls::pki_types::CertificateDer<'static>,
    sid: &str,
    participant: &str,
) -> Arc<UctpQuicClient> {
    let client_cfg = dev_client_config_trusting(cert).expect("client cfg");
    let client = UctpQuicClient::connect(client_ep, server_addr, "localhost", Arc::new(client_cfg))
        .await
        .expect("client connect");

    // A1: drive the auth handshake before sending session.invite.
    let mut inbound = client.take_inbound().expect("take_inbound");
    let hello = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: format!("env_{}", rand_hex()),
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
        .expect("auth.challenge timeout")
        .expect("inbound closed");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    let response = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthResponse,
        id: format!("env_{}", rand_hex()),
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

    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: format!("env_{}", rand_hex()),
        ts: Utc::now(),
        cid: Some(format!("conv_{}", rand_hex())),
        sid: Some(sid.into()),
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(SessionInvite {
            from: participant.into(),
            to: vec!["part_bridge".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    signature: None,
    };
    client.send(env).await.expect("send invite");
    client
}

#[tokio::test]
async fn quic_bridge_flows_real_audio_frame_end_to_end() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    // --- Server (with adapter + orchestrator) ---
    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP]).expect("dispatcher");
    let accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");

    let cfg = UctpQuicConfig::new(Arc::clone(&server_ep), accept_rx, bearer_stub());
    let adapter = UctpQuicAdapter::new(cfg).await.expect("adapter");
    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register");
    let mut events = orchestrator.subscribe_events();

    // --- Two clients ---
    let client_ep_a = client_endpoint();
    let client_ep_b = client_endpoint();
    let client_a =
        dial_and_invite(&client_ep_a, server_addr, &cert_der, "sess_a", "part_alice").await;
    let client_b =
        dial_and_invite(&client_ep_b, server_addr, &cert_der, "sess_b", "part_bob").await;

    // --- Wait for two InboundConnection events + paired ConnectionAuthenticated ---
    // A3: every UCTP InboundConnection should now be followed by a
    // ConnectionAuthenticated carrying the auth handshake's identity_id
    // / participant_id / assurance triple. We accumulate both and
    // assert pairing after the loop.
    let mut conn_ids: Vec<ConnectionId> = Vec::new();
    let mut authenticated: Vec<(ConnectionId, String, String)> = Vec::new();
    for _ in 0..60 {
        if conn_ids.len() == 2 && authenticated.len() == 2 {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(Event::ConnectionInbound { connection_id, .. })) => {
                conn_ids.push(connection_id);
            }
            Ok(Ok(Event::ConnectionAuthenticated {
                connection_id,
                identity_id,
                participant_id,
                ..
            })) => {
                authenticated.push((connection_id, identity_id, participant_id));
            }
            _ => continue,
        }
    }
    assert_eq!(conn_ids.len(), 2, "expected two ConnectionInbound events");
    assert_eq!(
        authenticated.len(),
        2,
        "A3: expected a ConnectionAuthenticated event paired with each InboundConnection"
    );
    // Each ConnectionAuthenticated must match one of the inbound
    // connection_ids — no orphan auth events.
    for (auth_connid, _id_id, _part_id) in &authenticated {
        assert!(
            conn_ids.contains(auth_connid),
            "ConnectionAuthenticated connection_id {:?} does not match any InboundConnection",
            auth_connid
        );
    }

    // --- Bridge ---
    let _bridge_id = orchestrator
        .bridge_connections(conn_ids[0].clone(), conn_ids[1].clone())
        .await
        .expect("bridge succeeds — both sides have streams");

    // --- Client-side stream setup so we can inject + observe ---
    // The server-side QUIC adapter creates streams with stream_local_id = 1
    // (per SP-A) and now also spawns a datagram reader (per this PR).
    // For the client side, we create matching `QuicDatagramMediaStream`s
    // manually + spawn readers, mirroring the loopback-datagram test
    // pattern. Client A injects on its outbound side; client B observes
    // on its inbound side.
    let codec = rvoip_core::capability::CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48000,
        channels: 1,
        fmtp: None,
    };

    let client_a_stream = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec.clone(),
        rvoip_core::connection::Direction::Outbound,
        1, // matches server-side stream_local_id
        client_a.connection.clone(),
    );
    let client_b_stream = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec,
        rvoip_core::connection::Direction::Inbound,
        1,
        client_b.connection.clone(),
    );

    // Spawn datagram readers on both client connections.
    let router_a = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&client_a_stream)]));
    let router_b = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&client_b_stream)]));
    spawn_datagram_reader(client_a.connection.clone(), router_a, None);
    spawn_datagram_reader(client_b.connection.clone(), router_b, None);

    // --- Inject 10 frames from client A; observe all of them on client B in order. ---
    let client_a_out =
        rvoip_core::stream::MediaStream::frames_out(client_a_stream.as_ref());
    let mut client_b_in =
        rvoip_core::stream::MediaStream::frames_in(client_b_stream.as_ref());

    for i in 0u8..10 {
        let frame = MediaFrame {
            stream_id: client_a_stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0xDE, 0xAD, 0xBE, 0xEF, i]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
        payload_type: None,
        };
        client_a_out.send(frame).await.expect("inject frame");
    }

    let mut received = Vec::with_capacity(10);
    while received.len() < 10 {
        let frame = tokio::time::timeout(Duration::from_secs(5), client_b_in.recv())
            .await
            .expect("timed out waiting for bridged frame on client B")
            .expect("client B's stream closed unexpectedly");
        received.push(frame.payload.to_vec());
    }

    // Bytes-identical pass-through (Opus↔Opus, no transcode) and ordering preserved.
    for (i, payload) in received.iter().enumerate() {
        assert_eq!(
            payload,
            &vec![0xDE, 0xAD, 0xBE, 0xEF, i as u8],
            "frame {} arrived corrupted or out of order: {:?}",
            i,
            payload
        );
    }
}
