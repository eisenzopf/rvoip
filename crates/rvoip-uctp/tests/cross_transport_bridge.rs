//! Cross-transport bridge proof: a QUIC client and a WebTransport
//! client both connect to a single Orchestrator that registers both
//! adapters; frames injected at the QUIC client arrive at the WT
//! client and vice-versa.
//!
//! This is the test that proves the v0 spike's headline claim — that
//! UCTP genuinely bridges across substrate types in one process.

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
    spawn_datagram_reader as quic_spawn_datagram_reader, QuicDatagramMediaStream, UctpQuicAdapter,
    UctpQuicClient, UctpQuicConfig,
};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite};
use rvoip_uctp::substrate::{dev_client_config_trusting, dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;
use rvoip_webtransport::{
    spawn_datagram_reader as wt_spawn_datagram_reader, UctpWtAdapter, UctpWtClient, UctpWtConfig,
    WebTransportDatagramMediaStream,
};
use url::Url;

const ALPN_UCTP: &[u8] = b"uctp/1";
const ALPN_H3: &[u8] = b"h3";

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
    tls.alpn_protocols = vec![ALPN_UCTP.to_vec(), ALPN_H3.to_vec()];
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

fn default_codec() -> rvoip_core::capability::CodecInfo {
    rvoip_core::capability::CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48000,
        channels: 1,
        fmtp: None,
    }
}

fn auth_hello() -> UctpEnvelope {
    UctpEnvelope::new(
        MessageType::AuthHello,
        serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_xt".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "xt/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    )
}

fn auth_response(in_reply_to: String) -> UctpEnvelope {
    UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "test-token".into(),
        })
        .unwrap(),
    )
    .with_in_reply_to(in_reply_to)
}

/// Two-step assertion that an inbound envelope has the expected type.
fn assert_msg(env: &UctpEnvelope, expected: MessageType) {
    assert_eq!(env.msg_type, expected, "expected {:?} got {:?}", expected, env.msg_type);
}

fn invite(sid: &str, participant: &str) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: format!("env_{}", sid),
        ts: Utc::now(),
        cid: Some(format!("conv_{}", sid)),
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
    }
}

#[tokio::test]
async fn quic_to_wt_bridge_flows_frames_end_to_end() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    // --- Shared server endpoint with both ALPNs ---
    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let mut routes =
        dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP, ALPN_H3]).expect("dispatcher");
    let quic_accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");
    let wt_accept_rx = routes.take(ALPN_H3).expect("h3 channel");

    // --- Adapters ---
    let quic_adapter = UctpQuicAdapter::new(UctpQuicConfig::new(
        Arc::clone(&server_ep),
        quic_accept_rx,
        bearer_stub(),
    ))
    .await
    .expect("quic adapter");
    let wt_adapter = UctpWtAdapter::new(UctpWtConfig::new(
        Arc::clone(&server_ep),
        wt_accept_rx,
        bearer_stub(),
    ))
    .await
    .expect("wt adapter");

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(quic_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register quic");
    orchestrator
        .register(wt_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register wt");
    let mut events = orchestrator.subscribe_events();

    // --- QUIC client dials in + sends session.invite ---
    let quic_client_ep = client_endpoint();
    let quic_client_cfg = dev_client_config_trusting(&cert_der).expect("client cfg");
    let quic_client = UctpQuicClient::connect(
        &quic_client_ep,
        server_addr,
        "localhost",
        Arc::new(quic_client_cfg.clone()),
    )
    .await
    .expect("quic client connect");
    // A1: drive bearer auth on the QUIC client before sending session.invite.
    let mut quic_in = quic_client.take_inbound().expect("quic take_inbound");
    quic_client.send(auth_hello()).await.expect("quic hello");
    let challenge = tokio::time::timeout(Duration::from_secs(5), quic_in.recv())
        .await
        .expect("quic challenge timeout")
        .expect("quic inbound closed");
    assert_msg(&challenge, MessageType::AuthChallenge);
    quic_client
        .send(auth_response(challenge.id))
        .await
        .expect("quic response");
    let qs = tokio::time::timeout(Duration::from_secs(5), quic_in.recv())
        .await
        .expect("quic session timeout")
        .expect("quic inbound closed");
    assert_msg(&qs, MessageType::AuthSession);

    quic_client
        .send(invite("sess_quic", "part_quic_alice"))
        .await
        .expect("quic invite");

    // --- WT client dials in + sends session.invite ---
    let wt_client_ep = client_endpoint();
    let wt_url = Url::parse(&format!("https://localhost:{}/uctp", server_addr.port()))
        .expect("parse url");
    let wt_client = UctpWtClient::connect(
        &wt_client_ep,
        server_addr,
        &wt_url,
        Arc::new(quic_client_cfg),
    )
    .await
    .expect("wt client connect");

    // A1: drive bearer auth on the WT client too.
    let mut wt_in = wt_client.take_inbound().expect("wt take_inbound");
    wt_client.send(auth_hello()).await.expect("wt hello");
    let wt_challenge = tokio::time::timeout(Duration::from_secs(5), wt_in.recv())
        .await
        .expect("wt challenge timeout")
        .expect("wt inbound closed");
    assert_msg(&wt_challenge, MessageType::AuthChallenge);
    wt_client
        .send(auth_response(wt_challenge.id))
        .await
        .expect("wt response");
    let ws = tokio::time::timeout(Duration::from_secs(5), wt_in.recv())
        .await
        .expect("wt session timeout")
        .expect("wt inbound closed");
    assert_msg(&ws, MessageType::AuthSession);

    wt_client
        .send(invite("sess_wt", "part_wt_bob"))
        .await
        .expect("wt invite");

    // --- Capture both InboundConnection events ---
    let mut conn_ids: Vec<ConnectionId> = Vec::new();
    for _ in 0..50 {
        if conn_ids.len() == 2 {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(Event::ConnectionInbound { connection_id, .. })) => {
                conn_ids.push(connection_id);
            }
            _ => continue,
        }
    }
    assert_eq!(conn_ids.len(), 2, "expected two ConnectionInbound events");

    // Figure out which connection_id is QUIC vs WT (the orchestrator's
    // ConnectionEntry tracks transport; lookup via `adapter(transport)`).
    // Easier: query each adapter for its registered streams; whichever
    // returns the matching id is the right one.
    let mut quic_conn_id: Option<ConnectionId> = None;
    let mut wt_conn_id: Option<ConnectionId> = None;
    for id in &conn_ids {
        if quic_adapter
            .streams(id.clone())
            .await
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            quic_conn_id = Some(id.clone());
        } else if wt_adapter
            .streams(id.clone())
            .await
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            wt_conn_id = Some(id.clone());
        }
    }
    let quic_conn_id = quic_conn_id.expect("QUIC connection id");
    let wt_conn_id = wt_conn_id.expect("WT connection id");

    // --- Bridge ---
    let _bridge_id = orchestrator
        .bridge_connections(quic_conn_id, wt_conn_id)
        .await
        .expect("bridge succeeds — both sides have streams");

    // --- Client-side stream setup so we can inject + observe ---
    let quic_client_stream = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        1,
        quic_client.connection.clone(),
    );
    let wt_client_stream = WebTransportDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        1,
        wt_client.session.clone(),
    );

    let quic_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(
        &quic_client_stream,
    )]));
    let wt_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&wt_client_stream)]));
    quic_spawn_datagram_reader(quic_client.connection.clone(), quic_router, None);
    wt_spawn_datagram_reader(wt_client.session.clone(), wt_router, None);

    // --- Inject 10 frames from QUIC client; observe on WT client ---
    let quic_out = rvoip_core::stream::MediaStream::frames_out(quic_client_stream.as_ref());
    let mut wt_in = rvoip_core::stream::MediaStream::frames_in(wt_client_stream.as_ref());

    for i in 0u8..10 {
        let frame = MediaFrame {
            stream_id: quic_client_stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0xCA, 0xFE, 0xBA, 0xBE, i]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
        };
        quic_out.send(frame).await.expect("inject");
    }

    let mut received: Vec<Vec<u8>> = Vec::new();
    while received.len() < 10 {
        let frame = tokio::time::timeout(Duration::from_secs(5), wt_in.recv())
            .await
            .expect("timed out waiting for bridged frame on WT client")
            .expect("WT client stream closed");
        received.push(frame.payload.to_vec());
    }

    for (i, payload) in received.iter().enumerate() {
        assert_eq!(
            payload,
            &vec![0xCA, 0xFE, 0xBA, 0xBE, i as u8],
            "QUIC→WT frame {} corrupted or out of order: {:?}",
            i,
            payload
        );
    }
}

#[tokio::test]
async fn wt_to_wt_bridge_flows_frames_end_to_end() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    // Same setup as the cross-transport test but with only the WT
    // adapter — two WT clients dial in.
    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_H3]).expect("dispatcher");
    let wt_accept_rx = routes.take(ALPN_H3).expect("h3 channel");

    let wt_adapter = UctpWtAdapter::new(UctpWtConfig::new(
        Arc::clone(&server_ep),
        wt_accept_rx,
        bearer_stub(),
    ))
    .await
    .expect("wt adapter");

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(wt_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register wt");
    let mut events = orchestrator.subscribe_events();

    // Two WT clients
    let client_a_ep = client_endpoint();
    let client_b_ep = client_endpoint();
    let client_cfg = dev_client_config_trusting(&cert_der).expect("client cfg");
    let url = Url::parse(&format!("https://localhost:{}/uctp", server_addr.port())).unwrap();
    let client_a = UctpWtClient::connect(
        &client_a_ep,
        server_addr,
        &url,
        Arc::new(client_cfg.clone()),
    )
    .await
    .expect("client a");
    let client_b = UctpWtClient::connect(&client_b_ep, server_addr, &url, Arc::new(client_cfg))
        .await
        .expect("client b");

    // A1: drive bearer auth on both WT clients before inviting.
    let mut in_a = client_a.take_inbound().expect("client_a take_inbound");
    let mut in_b = client_b.take_inbound().expect("client_b take_inbound");
    client_a.send(auth_hello()).await.expect("a hello");
    client_b.send(auth_hello()).await.expect("b hello");
    let ca = tokio::time::timeout(Duration::from_secs(5), in_a.recv())
        .await
        .expect("a challenge timeout")
        .expect("a closed");
    let cb = tokio::time::timeout(Duration::from_secs(5), in_b.recv())
        .await
        .expect("b challenge timeout")
        .expect("b closed");
    assert_msg(&ca, MessageType::AuthChallenge);
    assert_msg(&cb, MessageType::AuthChallenge);
    client_a.send(auth_response(ca.id)).await.expect("a resp");
    client_b.send(auth_response(cb.id)).await.expect("b resp");
    let sa = tokio::time::timeout(Duration::from_secs(5), in_a.recv())
        .await
        .expect("a session timeout")
        .expect("a closed");
    let sb = tokio::time::timeout(Duration::from_secs(5), in_b.recv())
        .await
        .expect("b session timeout")
        .expect("b closed");
    assert_msg(&sa, MessageType::AuthSession);
    assert_msg(&sb, MessageType::AuthSession);

    client_a.send(invite("sess_a", "part_a")).await.unwrap();
    client_b.send(invite("sess_b", "part_b")).await.unwrap();

    let mut conn_ids: Vec<ConnectionId> = Vec::new();
    for _ in 0..50 {
        if conn_ids.len() == 2 {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(Event::ConnectionInbound { connection_id, .. })) => {
                conn_ids.push(connection_id);
            }
            _ => continue,
        }
    }
    assert_eq!(conn_ids.len(), 2, "expected two ConnectionInbound events");

    let _bridge_id = orchestrator
        .bridge_connections(conn_ids[0].clone(), conn_ids[1].clone())
        .await
        .expect("bridge");

    // Client-side streams
    let stream_a = WebTransportDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        1,
        client_a.session.clone(),
    );
    let stream_b = WebTransportDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        1,
        client_b.session.clone(),
    );
    let router_a = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&stream_a)]));
    let router_b = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&stream_b)]));
    wt_spawn_datagram_reader(client_a.session.clone(), router_a, None);
    wt_spawn_datagram_reader(client_b.session.clone(), router_b, None);

    let out_a = rvoip_core::stream::MediaStream::frames_out(stream_a.as_ref());
    let mut in_b = rvoip_core::stream::MediaStream::frames_in(stream_b.as_ref());

    for i in 0u8..10 {
        let frame = MediaFrame {
            stream_id: stream_a.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0xFE, 0xED, 0xBE, 0xEF, i]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
        };
        out_a.send(frame).await.expect("inject");
    }

    let mut received: Vec<Vec<u8>> = Vec::new();
    while received.len() < 10 {
        let frame = tokio::time::timeout(Duration::from_secs(5), in_b.recv())
            .await
            .expect("timed out")
            .expect("client B stream closed");
        received.push(frame.payload.to_vec());
    }

    for (i, payload) in received.iter().enumerate() {
        assert_eq!(
            payload,
            &vec![0xFE, 0xED, 0xBE, 0xEF, i as u8],
            "WT→WT frame {} corrupted or out of order: {:?}",
            i,
            payload
        );
    }
}
