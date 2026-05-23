//! v0.x MP3b — live multi-party media fanout over the real QUIC wire.
//!
//! Headline test of the v0.x multi-party track: two QUIC peers connect
//! to one Orchestrator. Peer B subscribes to peer A's audio stream
//! via `Orchestrator::add_subscription`; a frame injected at peer A's
//! client-side stream travels over QUIC to the orchestrator, gets
//! fanned out by the adapter's datagram reader (calling
//! `orch.fanout_frame(...)`) into peer B's server-side MediaStream's
//! `frames_out`, which pumps the frame back over QUIC to peer B's
//! client. Assertion: the frame arrives at peer B's client-side
//! stream's `frames_in`.
//!
//! Companion to `cross_transport_bridge.rs` (which proves 1:1
//! orchestrator-orchestrated bridging). This test proves multi-party
//! N-way fanout in the same shape — the only difference is the
//! `bridge_connections` call is replaced with `add_subscription`.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, SessionId};
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_core::{Config, Orchestrator};
use rvoip_quic::{
    spawn_datagram_reader as quic_spawn_datagram_reader, QuicDatagramMediaStream, UctpQuicAdapter,
    UctpQuicClient, UctpQuicConfig,
};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::session::SessionInvite;
use rvoip_uctp::state::OrchestratorSubscriptionHandler;
use rvoip_uctp::substrate::{dev_client_config_trusting, dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;

const ALPN_UCTP: &[u8] = b"uctp/1";

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn server_endpoint(
    addr: std::net::SocketAddr,
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

fn default_codec() -> rvoip_core::capability::CodecInfo {
    rvoip_core::capability::CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48_000,
        channels: 1,
        fmtp: None,
    }
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
            to: vec!["part_server".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    }
}

#[tokio::test]
async fn fanout_routes_media_from_publisher_to_subscriber_over_quic() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    // --- Shared QUIC endpoint with the UCTP ALPN ---
    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP]).expect("dispatcher");
    let quic_accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");

    // --- Orchestrator + multi-party subscription handler + adapter ---
    let orchestrator = Orchestrator::new(Config::default());
    let publishers = orchestrator.publisher_registry();
    let handler = OrchestratorSubscriptionHandler::new(
        Arc::clone(&orchestrator),
        Arc::clone(&publishers),
    );
    let quic_adapter = UctpQuicAdapter::new(
        UctpQuicConfig::new(Arc::clone(&server_ep), quic_accept_rx, bearer_stub())
            .with_subscription_handler(handler)
            .with_orchestrator(Arc::clone(&orchestrator)),
    )
    .await
    .expect("quic adapter");
    orchestrator
        .register(quic_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register");

    let mut events = orchestrator.subscribe_events();

    // --- Two QUIC clients connect + send session.invite ---
    let client_a_ep = client_endpoint();
    let client_b_ep = client_endpoint();
    let cfg = dev_client_config_trusting(&cert_der).expect("client cfg");

    let client_a =
        UctpQuicClient::connect(&client_a_ep, server_addr, "localhost", Arc::new(cfg.clone()))
            .await
            .expect("client a");
    let client_b =
        UctpQuicClient::connect(&client_b_ep, server_addr, "localhost", Arc::new(cfg))
            .await
            .expect("client b");
    // Send invites sequentially so the orchestrator's ConnectionInbound
    // ordering is deterministic — client A is always the publisher.
    client_a.send(invite("sess_a", "part_a_publisher")).await.unwrap();
    let publisher_connid = loop {
        if let Ok(Ok(Event::ConnectionInbound { connection_id, .. })) =
            tokio::time::timeout(Duration::from_secs(5), events.recv()).await
        {
            break connection_id;
        }
    };
    client_b.send(invite("sess_b", "part_b_subscriber")).await.unwrap();
    let subscriber_connid = loop {
        if let Ok(Ok(Event::ConnectionInbound { connection_id, .. })) =
            tokio::time::timeout(Duration::from_secs(5), events.recv()).await
        {
            break connection_id;
        }
    };

    // Look up the publisher's server-side StreamId (the default audio
    // stream the adapter created on InboundInvite).
    let publisher_streams = quic_adapter
        .streams(publisher_connid.clone())
        .await
        .expect("publisher streams");
    let publisher_strm_id = publisher_streams
        .iter()
        .find(|s| s.kind() == StreamKind::Audio)
        .expect("publisher has audio stream")
        .id();

    // The publisher's adapter knows what `SessionId` it belongs to —
    // the fanout context was built at InboundInvite from the wire `sid`.
    // We have to use the SAME sid the adapter knows. We didn't capture
    // it directly, but `add_subscription` is per-(sid, publisher, strm).
    // The adapter's fanout call uses *its* known sid; the subscription
    // table is keyed on the same sid. So as long as we register against
    // that sid, the fanout will route. We learn the publisher's sid
    // from the adapter's internal map indirectly — but the simplest
    // path is: the publisher's session id is sess_a (the wire string
    // from the invite).
    let publisher_sid = SessionId::from_string("sess_a");

    // Register the subscription: subscriber_connid receives publisher's
    // audio stream.
    orchestrator.add_subscription(
        publisher_sid.clone(),
        subscriber_connid.clone(),
        publisher_connid.clone(),
        publisher_strm_id.clone(),
    );

    // --- Client-side stream setup so we can inject + observe ---
    let publisher_client_stream = QuicDatagramMediaStream::start(
        rvoip_core::ids::StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        1,
        client_a.connection.clone(),
    );
    let subscriber_client_stream = QuicDatagramMediaStream::start(
        rvoip_core::ids::StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        1,
        client_b.connection.clone(),
    );

    let publisher_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(
        &publisher_client_stream,
    )]));
    let subscriber_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(
        &subscriber_client_stream,
    )]));
    quic_spawn_datagram_reader(client_a.connection.clone(), publisher_router, None);
    quic_spawn_datagram_reader(client_b.connection.clone(), subscriber_router, None);

    // --- Inject 5 frames at publisher; observe arrival at subscriber ---
    let publisher_out =
        rvoip_core::stream::MediaStream::frames_out(publisher_client_stream.as_ref());
    let mut subscriber_in =
        rvoip_core::stream::MediaStream::frames_in(subscriber_client_stream.as_ref());

    for i in 0u8..5 {
        let frame = MediaFrame {
            stream_id: publisher_client_stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0x4D, 0x50, 0x33, 0x42, i]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
        };
        publisher_out.send(frame).await.expect("inject frame");
    }

    let mut received: Vec<Vec<u8>> = Vec::new();
    while received.len() < 5 {
        let frame = tokio::time::timeout(Duration::from_secs(5), subscriber_in.recv())
            .await
            .expect("timed out waiting for fanout frame on subscriber")
            .expect("subscriber stream closed");
        received.push(frame.payload.to_vec());
    }

    for (i, payload) in received.iter().enumerate() {
        assert_eq!(
            payload,
            &vec![0x4D, 0x50, 0x33, 0x42, i as u8],
            "fanout frame {} corrupted or out of order: {:?}",
            i,
            payload
        );
    }
}

#[tokio::test]
async fn fanout_with_no_subscription_does_not_leak_frames() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP]).expect("dispatcher");
    let quic_accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");

    let orchestrator = Orchestrator::new(Config::default());
    let publishers = orchestrator.publisher_registry();
    let handler = OrchestratorSubscriptionHandler::new(
        Arc::clone(&orchestrator),
        Arc::clone(&publishers),
    );
    let quic_adapter = UctpQuicAdapter::new(
        UctpQuicConfig::new(Arc::clone(&server_ep), quic_accept_rx, bearer_stub())
            .with_subscription_handler(handler)
            .with_orchestrator(Arc::clone(&orchestrator)),
    )
    .await
    .expect("quic adapter");
    orchestrator
        .register(quic_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register");
    let mut events = orchestrator.subscribe_events();

    let client_a_ep = client_endpoint();
    let client_b_ep = client_endpoint();
    let cfg = dev_client_config_trusting(&cert_der).expect("client cfg");
    let client_a =
        UctpQuicClient::connect(&client_a_ep, server_addr, "localhost", Arc::new(cfg.clone()))
            .await
            .expect("client a");
    let client_b =
        UctpQuicClient::connect(&client_b_ep, server_addr, "localhost", Arc::new(cfg))
            .await
            .expect("client b");
    client_a.send(invite("sess_a", "part_a")).await.unwrap();
    client_b.send(invite("sess_b", "part_b")).await.unwrap();

    let mut conn_ids: Vec<ConnectionId> = Vec::new();
    for _ in 0..50 {
        if conn_ids.len() == 2 {
            break;
        }
        if let Ok(Ok(Event::ConnectionInbound { connection_id, .. })) =
            tokio::time::timeout(Duration::from_millis(200), events.recv()).await
        {
            conn_ids.push(connection_id);
        }
    }
    assert_eq!(conn_ids.len(), 2);

    // No subscription registered → no fanout.

    let publisher_client_stream = QuicDatagramMediaStream::start(
        rvoip_core::ids::StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        1,
        client_a.connection.clone(),
    );
    let subscriber_client_stream = QuicDatagramMediaStream::start(
        rvoip_core::ids::StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        1,
        client_b.connection.clone(),
    );
    let publisher_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(
        &publisher_client_stream,
    )]));
    let subscriber_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(
        &subscriber_client_stream,
    )]));
    quic_spawn_datagram_reader(client_a.connection.clone(), publisher_router, None);
    quic_spawn_datagram_reader(client_b.connection.clone(), subscriber_router, None);

    let publisher_out =
        rvoip_core::stream::MediaStream::frames_out(publisher_client_stream.as_ref());
    let mut subscriber_in =
        rvoip_core::stream::MediaStream::frames_in(subscriber_client_stream.as_ref());

    for i in 0u8..5 {
        let frame = MediaFrame {
            stream_id: publisher_client_stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0xDE, 0xAD, 0xBE, 0xEF, i]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
        };
        publisher_out.send(frame).await.expect("inject");
    }

    // Give the wire 500ms — without a subscription, no frame should
    // reach the subscriber's client-side stream.
    assert!(
        tokio::time::timeout(Duration::from_millis(500), subscriber_in.recv())
            .await
            .is_err(),
        "no subscription registered, but a frame leaked to the subscriber"
    );
}
