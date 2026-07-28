//! v0.x MP3b — live multi-party media fanout over the real QUIC wire.
//!
//! Headline test of the v0.x multi-party track: two QUIC peers connect
//! to one Orchestrator. Peer B subscribes to peer A's audio stream
//! with an authenticated `stream.subscribe` envelope; a frame injected at
//! peer A's client-side stream travels over QUIC to the orchestrator, gets
//! fanned out by the adapter's datagram reader (calling
//! `orch.fanout_frame(...)`) into peer B's server-side MediaStream's
//! `frames_out`, which pumps the frame back over QUIC to peer B's
//! client. Assertion: the frame arrives at peer B's client-side
//! stream's `frames_in`.
//!
//! Companion to `cross_transport_bridge.rs` (which proves 1:1
//! orchestrator-orchestrated bridging). This test proves multi-party
//! N-way fanout in the same shape — the only difference is the
//! `bridge_connections` call is replaced by the protocol subscription.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_core::{Config, Orchestrator};
use rvoip_quic::{
    spawn_datagram_reader as quic_spawn_datagram_reader, QuicDatagramMediaStream, UctpQuicAdapter,
    UctpQuicClient, UctpQuicConfig,
};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite, stream};
use rvoip_uctp::state::{
    OrchestratorSubscriptionHandler, ResourceBindingError, SessionBindingResolver,
};
use rvoip_uctp::substrate::{dev_client_config_trusting, dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;

const ALPN_UCTP: &[u8] = b"uctp/1";

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn server_endpoint(
    addr: std::net::SocketAddr,
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

fn default_codec() -> rvoip_core::capability::CodecInfo {
    rvoip_core::capability::CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48_000,
        channels: 1,
        fmtp: None,
    }
}

/// A1: drive the bearer auth handshake on a freshly-opened QUIC client
/// so subsequent session.invite/connection.offer envelopes aren't
/// refused with 401. Caller passes `client` and the inbound channel it
/// already extracted via `take_inbound()`.
async fn drive_auth_quic(
    client: &Arc<UctpQuicClient>,
    inbound: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>,
) {
    let hello = UctpEnvelope::new(
        MessageType::AuthHello,
        serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_mp".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "mp/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    );
    client.send(hello).await.expect("send hello");
    let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.challenge timeout")
        .expect("inbound closed");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    let response = UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "test-token".into(),
            actor_token: None,
        })
        .unwrap(),
    )
    .with_in_reply_to(challenge.id);
    client.send(response).await.expect("send response");
    let session = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.session timeout")
        .expect("inbound closed");
    assert_eq!(session.msg_type, MessageType::AuthSession);
}

/// Drain the inbound envelope channel until a `stream.opened` envelope
/// arrives; return its `stream_local_id`. The MP3c fanout path
/// (plan B1) announces the subscriber's per-publisher local_id this
/// way so the subscriber's client can build a matching MediaStream.
async fn wait_for_stream_opened(
    inbound: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>,
) -> stream::StreamInfo {
    for _ in 0..30 {
        let env = tokio::time::timeout(Duration::from_millis(500), inbound.recv())
            .await
            .expect("stream.opened timeout")
            .expect("inbound closed");
        if env.msg_type != MessageType::StreamOpened {
            continue;
        }
        let payload: rvoip_uctp::payloads::stream::StreamOpened =
            env.decode_payload().expect("decode stream.opened");
        return payload.stream;
    }
    panic!("no stream.opened envelope arrived");
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
        signature: None,
    }
}

fn connection_offer(sid: &str, connid: &str, participant: &str) -> UctpEnvelope {
    UctpEnvelope::new(
        MessageType::ConnectionOffer,
        serde_json::json!({
            "by_participant": participant,
            "substrate": "quic",
            "capabilities": {},
            "streams_offered": [{
                "id": format!("strm_{connid}"),
                "kind": "audio",
                "direction": "sendrecv",
                "codec_preferences": ["opus"]
            }],
            "substrate_setup": null
        }),
    )
    .with_sid(sid)
    .with_connid(connid)
}

fn connection_ready(sid: &str, connid: &str) -> UctpEnvelope {
    UctpEnvelope::new(MessageType::ConnectionReady, serde_json::json!({}))
        .with_sid(sid)
        .with_connid(connid)
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
    let handler =
        OrchestratorSubscriptionHandler::new(Arc::clone(&orchestrator), Arc::clone(&publishers));
    let canonical_session = SessionId::new();
    let resolver: Arc<dyn SessionBindingResolver> = Arc::new({
        let canonical_session = canonical_session.clone();
        move |_: &rvoip_core::identity::AuthenticatedPrincipal,
              _: &SessionId|
              -> Result<SessionId, ResourceBindingError> { Ok(canonical_session.clone()) }
    });
    let quic_adapter = UctpQuicAdapter::new(
        UctpQuicConfig::new(Arc::clone(&server_ep), quic_accept_rx, bearer_stub())
            .with_subscription_handler(handler)
            .with_session_binding_resolver(resolver)
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

    let client_a = UctpQuicClient::connect(
        &client_a_ep,
        server_addr,
        "localhost",
        Arc::new(cfg.clone()),
    )
    .await
    .expect("client a");
    let client_b = UctpQuicClient::connect(&client_b_ep, server_addr, "localhost", Arc::new(cfg))
        .await
        .expect("client b");

    // A1: drive bearer auth on both clients before inviting.
    let mut in_a = client_a.take_inbound().expect("a take_inbound");
    let mut in_b = client_b.take_inbound().expect("b take_inbound");
    drive_auth_quic(&client_a, &mut in_a).await;
    drive_auth_quic(&client_b, &mut in_b).await;

    // Send invites sequentially so the orchestrator's ConnectionInbound
    // ordering is deterministic — client A is always the publisher.
    client_a
        .send(invite("sess_a", "part_a_publisher"))
        .await
        .unwrap();
    let publisher_connid = loop {
        if let Ok(Ok(Event::ConnectionInbound { connection_id, .. })) =
            tokio::time::timeout(Duration::from_secs(5), events.recv()).await
        {
            break connection_id;
        }
    };
    client_a
        .send(connection_offer(
            "sess_a",
            "conn_wire_a",
            "part_a_publisher",
        ))
        .await
        .unwrap();
    client_a
        .send(connection_ready("sess_a", "conn_wire_a"))
        .await
        .unwrap();
    let publisher_stream = wait_for_stream_opened(&mut in_a).await;
    client_b
        .send(invite("sess_b", "part_b_subscriber"))
        .await
        .unwrap();
    let _subscriber_connid = loop {
        if let Ok(Ok(Event::ConnectionInbound { connection_id, .. })) =
            tokio::time::timeout(Duration::from_secs(5), events.recv()).await
        {
            break connection_id;
        }
    };
    client_b
        .send(connection_offer(
            "sess_b",
            "conn_wire_b",
            "part_b_subscriber",
        ))
        .await
        .unwrap();
    client_b
        .send(connection_ready("sess_b", "conn_wire_b"))
        .await
        .unwrap();
    let _subscriber_published_stream = wait_for_stream_opened(&mut in_b).await;

    // The offered wire Stream ID is the actual server-side Stream ID.
    let publisher_streams = quic_adapter
        .streams(publisher_connid.clone())
        .await
        .expect("publisher streams");
    let publisher_strm_id = publisher_streams
        .iter()
        .find(|s| s.kind() == StreamKind::Audio)
        .expect("publisher has audio stream")
        .id();
    assert_eq!(publisher_strm_id.as_str(), publisher_stream.strm_id);

    // Exercise the real authenticated wire API. Both peer-local Session IDs
    // resolve to the same canonical Session through the test resolver.
    let subscribe = UctpEnvelope::new(
        MessageType::StreamSubscribe,
        serde_json::to_value(stream::StreamSubscribe {
            by_participant: "part_b_subscriber".into(),
            subscriptions: vec![stream::StreamSubscription {
                strm_id: Some(publisher_stream.strm_id.clone()),
                ..Default::default()
            }],
        })
        .unwrap(),
    )
    .with_sid("sess_b")
    .with_connid("conn_wire_b");
    client_b.send(subscribe).await.expect("wire subscribe");
    let ack = tokio::time::timeout(Duration::from_secs(5), in_b.recv())
        .await
        .expect("subscribe reply timeout")
        .expect("subscriber signaling closed");
    assert_eq!(ack.msg_type, MessageType::Ack);

    // --- Publisher-side client stream so we can inject ---
    let publisher_client_stream = QuicDatagramMediaStream::start(
        StreamId::from_string(publisher_stream.strm_id.clone()),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        publisher_stream.stream_local_id,
        client_a.connection.clone(),
    );
    let publisher_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(
        &publisher_client_stream,
    )]));
    quic_spawn_datagram_reader(client_a.connection.clone(), publisher_router, None);
    let publisher_out =
        rvoip_core::stream::MediaStream::frames_out(publisher_client_stream.as_ref());

    // --- Trigger the lazy MP3c allocation by injecting a priming frame.
    // The first fanout call into the subscriber's adapter sends
    // `stream.opened` announcing the new subscriber-side local_id; we
    // listen for that envelope before building the matching client-side
    // MediaStream. The priming frame itself may not be received (the
    // subscriber's reader isn't running yet) — that's expected. ---
    publisher_out
        .send(MediaFrame {
            stream_id: publisher_client_stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0x00, 0x00, 0x00, 0x00, 0xFF]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: None,
        })
        .await
        .expect("prime frame");

    // MP3c: server allocates a fresh stream_local_id (>= 2) for this
    // subscription and emits `stream.opened` announcing it. Build the
    // subscriber-side MediaStream with whatever id the server picked,
    // mirroring how a real client would learn it.
    let subscriber_fanout_stream = wait_for_stream_opened(&mut in_b).await;
    let subscriber_local_id = subscriber_fanout_stream.stream_local_id;
    assert!(
        subscriber_local_id >= 2,
        "MP3c must allocate a fresh local_id (got {})",
        subscriber_local_id
    );
    let subscriber_client_stream = QuicDatagramMediaStream::start(
        StreamId::from_string(subscriber_fanout_stream.strm_id),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        subscriber_local_id,
        client_b.connection.clone(),
    );

    let subscriber_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(
        &subscriber_client_stream,
    )]));
    quic_spawn_datagram_reader(client_b.connection.clone(), subscriber_router, None);

    // --- Inject 5 frames at publisher; observe arrival at subscriber ---
    let mut subscriber_in =
        rvoip_core::stream::MediaStream::frames_in(subscriber_client_stream.as_ref());

    for i in 0u8..5 {
        let frame = MediaFrame {
            stream_id: publisher_client_stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0x4D, 0x50, 0x33, 0x42, i]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: None,
        };
        publisher_out.send(frame).await.expect("inject frame");
    }

    let mut received: Vec<Vec<u8>> = Vec::new();
    while received.len() < 5 {
        let frame = tokio::time::timeout(Duration::from_secs(5), subscriber_in.recv())
            .await
            .expect("timed out waiting for fanout frame on subscriber")
            .expect("subscriber stream closed");
        // Skip the priming frame's payload if it happens to slip through
        // after the reader started (depends on QUIC scheduling).
        if frame.payload.as_ref() == &[0x00, 0x00, 0x00, 0x00, 0xFF] {
            continue;
        }
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

    // Real wire unsubscribe removes the canonical route immediately; the
    // already-allocated subscriber MediaStream must not receive later media.
    let unsubscribe = UctpEnvelope::new(
        MessageType::StreamUnsubscribe,
        serde_json::to_value(stream::StreamUnsubscribe {
            strm_ids: vec![publisher_stream.strm_id.clone()],
        })
        .unwrap(),
    )
    .with_sid("sess_b")
    .with_connid("conn_wire_b");
    client_b.send(unsubscribe).await.expect("wire unsubscribe");
    let ack = tokio::time::timeout(Duration::from_secs(5), in_b.recv())
        .await
        .expect("unsubscribe reply timeout")
        .expect("subscriber signaling closed");
    assert_eq!(ack.msg_type, MessageType::Ack);
    publisher_out
        .send(MediaFrame {
            stream_id: publisher_client_stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(b"after-unsubscribe"),
            timestamp_rtp: 960,
            captured_at: Utc::now(),
            payload_type: Some(111),
        })
        .await
        .expect("send after unsubscribe");
    assert!(
        tokio::time::timeout(Duration::from_millis(300), subscriber_in.recv())
            .await
            .is_err(),
        "subscriber received media after stream.unsubscribe"
    );
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
    let handler =
        OrchestratorSubscriptionHandler::new(Arc::clone(&orchestrator), Arc::clone(&publishers));
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
    let client_a = UctpQuicClient::connect(
        &client_a_ep,
        server_addr,
        "localhost",
        Arc::new(cfg.clone()),
    )
    .await
    .expect("client a");
    let client_b = UctpQuicClient::connect(&client_b_ep, server_addr, "localhost", Arc::new(cfg))
        .await
        .expect("client b");
    let mut in_a = client_a.take_inbound().expect("a take_inbound");
    let mut in_b = client_b.take_inbound().expect("b take_inbound");
    drive_auth_quic(&client_a, &mut in_a).await;
    drive_auth_quic(&client_b, &mut in_b).await;
    client_a.send(invite("sess_a", "part_a")).await.unwrap();
    client_b.send(invite("sess_b", "part_b")).await.unwrap();
    client_a
        .send(connection_offer("sess_a", "conn_wire_a", "part_a"))
        .await
        .unwrap();
    client_b
        .send(connection_offer("sess_b", "conn_wire_b", "part_b"))
        .await
        .unwrap();
    client_a
        .send(connection_ready("sess_a", "conn_wire_a"))
        .await
        .unwrap();
    client_b
        .send(connection_ready("sess_b", "conn_wire_b"))
        .await
        .unwrap();
    let publisher_stream = wait_for_stream_opened(&mut in_a).await;
    let subscriber_stream = wait_for_stream_opened(&mut in_b).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

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
        StreamId::from_string(publisher_stream.strm_id),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        publisher_stream.stream_local_id,
        client_a.connection.clone(),
    );
    let subscriber_client_stream = QuicDatagramMediaStream::start(
        StreamId::from_string(subscriber_stream.strm_id),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        subscriber_stream.stream_local_id,
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
            payload_type: None,
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
