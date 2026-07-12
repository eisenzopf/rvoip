//! Gate 4 — real wire-driven two-publisher QUIC fanout.
//!
//! Three authenticated QUIC peers resolve their peer-local Session IDs to one
//! explicit canonical Session: two publishers (A, B) and one subscriber (S).
//! S subscribes to both publishers through `stream.subscribe` envelopes.
//! The server allocates a **distinct** `stream_local_id` per
//! subscription so S can demultiplex A's frames from B's on the wire
//! (CONVERSATION_PROTOCOL.md §10.1 multi-party note).
//!
//! What this asserts:
//! 1. Every offered publisher stream is negotiated by `connection.ready` and
//!    announced with its real Stream ID and peer-local media handle.
//! 2. Real subscriber wire requests create two fresh, distinct fanout handles.
//! 3. Frames injected at A arrive at S on A's allocated handle;
//!    frames injected at B arrive at S on B's allocated local_id; no
//!    cross-talk.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::events::Event;
use rvoip_core::ids::{SessionId, StreamId};
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

fn stream_subscribe(sid: &str, connid: &str, strm_id: &str) -> UctpEnvelope {
    UctpEnvelope::new(
        MessageType::StreamSubscribe,
        serde_json::to_value(stream::StreamSubscribe {
            by_participant: "part_s".into(),
            subscriptions: vec![stream::StreamSubscription {
                strm_id: Some(strm_id.into()),
                ..Default::default()
            }],
        })
        .unwrap(),
    )
    .with_sid(sid)
    .with_connid(connid)
}

async fn drive_auth_quic(
    client: &Arc<UctpQuicClient>,
    inbound: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>,
) {
    let hello = UctpEnvelope::new(
        MessageType::AuthHello,
        serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_3p".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "3p/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    );
    client.send(hello).await.expect("hello");
    let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("challenge timeout")
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
    client.send(response).await.expect("response");
    let session = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("session timeout")
        .expect("inbound closed");
    assert_eq!(session.msg_type, MessageType::AuthSession);
}

/// Drain envelopes until the next `stream.opened` arrives.
async fn next_stream_opened(
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

async fn next_ack(inbound: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>) {
    for _ in 0..30 {
        let envelope = tokio::time::timeout(Duration::from_millis(500), inbound.recv())
            .await
            .expect("ack timeout")
            .expect("inbound closed");
        match envelope.msg_type {
            MessageType::Ack => return,
            MessageType::Error => panic!("stream.subscribe rejected: {envelope:?}"),
            _ => {}
        }
    }
    panic!("no stream.subscribe ack arrived");
}

#[tokio::test]
async fn three_party_subscriber_sees_two_publishers_on_distinct_local_ids() {
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

    // --- Three clients: two publishers (a, b) and one subscriber (s) ---
    let ep_a = client_endpoint();
    let ep_b = client_endpoint();
    let ep_s = client_endpoint();
    let cfg = dev_client_config_trusting(&cert_der).expect("client cfg");

    let client_a = UctpQuicClient::connect(&ep_a, server_addr, "localhost", Arc::new(cfg.clone()))
        .await
        .expect("client a");
    let client_b = UctpQuicClient::connect(&ep_b, server_addr, "localhost", Arc::new(cfg.clone()))
        .await
        .expect("client b");
    let client_s = UctpQuicClient::connect(&ep_s, server_addr, "localhost", Arc::new(cfg))
        .await
        .expect("client s");

    let mut in_a = client_a.take_inbound().expect("a take_inbound");
    let mut in_b = client_b.take_inbound().expect("b take_inbound");
    let mut in_s = client_s.take_inbound().expect("s take_inbound");
    drive_auth_quic(&client_a, &mut in_a).await;
    drive_auth_quic(&client_b, &mut in_b).await;
    drive_auth_quic(&client_s, &mut in_s).await;

    client_a
        .send(invite("sess_3p", "part_a"))
        .await
        .expect("invite a");
    let conn_a = loop {
        if let Ok(Ok(Event::ConnectionInbound { connection_id, .. })) =
            tokio::time::timeout(Duration::from_secs(5), events.recv()).await
        {
            break connection_id;
        }
    };
    client_a
        .send(connection_offer("sess_3p", "conn_wire_a", "part_a"))
        .await
        .expect("offer a");
    client_a
        .send(connection_ready("sess_3p", "conn_wire_a"))
        .await
        .expect("ready a");
    let opened_a = next_stream_opened(&mut in_a).await;
    assert_eq!(opened_a.strm_id, "strm_conn_wire_a");

    client_b
        .send(invite("sess_3p_b", "part_b"))
        .await
        .expect("invite b");
    let conn_b = loop {
        if let Ok(Ok(Event::ConnectionInbound { connection_id, .. })) =
            tokio::time::timeout(Duration::from_secs(5), events.recv()).await
        {
            break connection_id;
        }
    };
    client_b
        .send(connection_offer("sess_3p_b", "conn_wire_b", "part_b"))
        .await
        .expect("offer b");
    client_b
        .send(connection_ready("sess_3p_b", "conn_wire_b"))
        .await
        .expect("ready b");
    let opened_b = next_stream_opened(&mut in_b).await;
    assert_eq!(opened_b.strm_id, "strm_conn_wire_b");

    client_s
        .send(invite("sess_3p_s", "part_s"))
        .await
        .expect("invite s");
    let _conn_s = loop {
        if let Ok(Ok(Event::ConnectionInbound { connection_id, .. })) =
            tokio::time::timeout(Duration::from_secs(5), events.recv()).await
        {
            break connection_id;
        }
    };
    client_s
        .send(connection_offer("sess_3p_s", "conn_wire_s", "part_s"))
        .await
        .expect("offer s");
    client_s
        .send(connection_ready("sess_3p_s", "conn_wire_s"))
        .await
        .expect("ready s");
    let opened_s = next_stream_opened(&mut in_s).await;
    assert_eq!(opened_s.strm_id, "strm_conn_wire_s");

    // The offered wire Stream ID is the server-side Stream ID. This proves
    // bind-before-announce populated each adapter route with the negotiated
    // stream rather than an invite-time synthetic stream.
    let strm_a = quic_adapter
        .streams(conn_a.clone())
        .await
        .expect("streams a")
        .iter()
        .find(|s| s.kind() == StreamKind::Audio)
        .expect("audio stream on a")
        .id();
    let strm_b = quic_adapter
        .streams(conn_b.clone())
        .await
        .expect("streams b")
        .iter()
        .find(|s| s.kind() == StreamKind::Audio)
        .expect("audio stream on b")
        .id();
    assert_eq!(strm_a.as_str(), opened_a.strm_id);
    assert_eq!(strm_b.as_str(), opened_b.strm_id);

    // Exercise the authenticated wire API. All three peer-local Session IDs
    // resolve to `canonical_session`, so the shared publisher registry can
    // resolve both explicit Stream IDs for the subscriber.
    client_s
        .send(stream_subscribe(
            "sess_3p_s",
            "conn_wire_s",
            &opened_a.strm_id,
        ))
        .await
        .expect("subscribe to a");
    next_ack(&mut in_s).await;
    client_s
        .send(stream_subscribe(
            "sess_3p_s",
            "conn_wire_s",
            &opened_b.strm_id,
        ))
        .await
        .expect("subscribe to b");
    next_ack(&mut in_s).await;

    // Publisher-side client streams use the handles negotiated for their own
    // physical peers. No fixed/default local-ID assumption remains.
    let pub_a_stream = QuicDatagramMediaStream::start(
        StreamId::from_string(opened_a.strm_id.clone()),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        opened_a.stream_local_id,
        client_a.connection.clone(),
    );
    let pub_b_stream = QuicDatagramMediaStream::start(
        StreamId::from_string(opened_b.strm_id.clone()),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        opened_b.stream_local_id,
        client_b.connection.clone(),
    );
    quic_spawn_datagram_reader(
        client_a.connection.clone(),
        Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&pub_a_stream)])),
        None,
    );
    quic_spawn_datagram_reader(
        client_b.connection.clone(),
        Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&pub_b_stream)])),
        None,
    );

    // Trigger lazy allocation one publisher at a time so each subscriber-side
    // `stream.opened` can be correlated to its source without inspecting any
    // server-internal subscription state. Priming frames may be lost before
    // the matching client-side reader exists; later test frames are not.
    let prime = Bytes::from(vec![0x00, 0x00, 0x00, 0x00, 0xFF]);
    pub_a_stream
        .frames_out()
        .send(MediaFrame {
            stream_id: pub_a_stream.id(),
            kind: StreamKind::Audio,
            payload: prime.clone(),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: None,
        })
        .await
        .expect("prime a");
    let fanout_a = next_stream_opened(&mut in_s).await;

    pub_b_stream
        .frames_out()
        .send(MediaFrame {
            stream_id: pub_b_stream.id(),
            kind: StreamKind::Audio,
            payload: prime.clone(),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: None,
        })
        .await
        .expect("prime b");
    let fanout_b = next_stream_opened(&mut in_s).await;

    assert_ne!(
        fanout_a.stream_local_id, fanout_b.stream_local_id,
        "each subscription must get a distinct stream_local_id"
    );
    assert!(
        fanout_a.stream_local_id != opened_s.stream_local_id
            && fanout_b.stream_local_id != opened_s.stream_local_id,
        "fanout handles must not alias the subscriber's negotiated publisher handle"
    );

    let sub_stream_a = QuicDatagramMediaStream::start(
        StreamId::from_string(fanout_a.strm_id),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        fanout_a.stream_local_id,
        client_s.connection.clone(),
    );
    let sub_stream_b = QuicDatagramMediaStream::start(
        StreamId::from_string(fanout_b.strm_id),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        fanout_b.stream_local_id,
        client_s.connection.clone(),
    );
    quic_spawn_datagram_reader(
        client_s.connection.clone(),
        Arc::new(parking_lot::RwLock::new(vec![
            Arc::clone(&sub_stream_a),
            Arc::clone(&sub_stream_b),
        ])),
        None,
    );

    let mut rx_a = rvoip_core::stream::MediaStream::frames_in(sub_stream_a.as_ref());
    let mut rx_b = rvoip_core::stream::MediaStream::frames_in(sub_stream_b.as_ref());

    // Inject distinctive payloads: A → 0xAA marker, B → 0xBB marker.
    for i in 0u8..5 {
        pub_a_stream
            .frames_out()
            .send(MediaFrame {
                stream_id: pub_a_stream.id(),
                kind: StreamKind::Audio,
                payload: Bytes::from(vec![0xAA, i]),
                timestamp_rtp: 0,
                captured_at: Utc::now(),
                payload_type: None,
            })
            .await
            .expect("inject a");
        pub_b_stream
            .frames_out()
            .send(MediaFrame {
                stream_id: pub_b_stream.id(),
                kind: StreamKind::Audio,
                payload: Bytes::from(vec![0xBB, i]),
                timestamp_rtp: 0,
                captured_at: Utc::now(),
                payload_type: None,
            })
            .await
            .expect("inject b");
    }

    // Drain both subscriber streams. Because priming was sequential, the
    // subscriber handles are correlated: A must carry only 0xAA and B only
    // 0xBB. Any opposite marker is cross-talk in the peer-local router.
    let mut on_a: Vec<u8> = Vec::new();
    let mut on_b: Vec<u8> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline && (on_a.len() + on_b.len()) < 10 {
        tokio::select! {
            biased;
            frame = rx_a.recv() => {
                if let Some(f) = frame {
                    if f.payload.as_ref() != prime.as_ref() {
                        on_a.push(f.payload[0]);
                    }
                }
            }
            frame = rx_b.recv() => {
                if let Some(f) = frame {
                    if f.payload.as_ref() != prime.as_ref() {
                        on_b.push(f.payload[0]);
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    let a_markers: std::collections::HashSet<u8> = on_a.iter().copied().collect();
    let b_markers: std::collections::HashSet<u8> = on_b.iter().copied().collect();
    assert!(!on_a.is_empty(), "publisher A produced no subscriber media");
    assert!(!on_b.is_empty(), "publisher B produced no subscriber media");
    assert_eq!(
        a_markers,
        std::collections::HashSet::from([0xAA]),
        "A route cross-talk: {on_a:?}"
    );
    assert_eq!(
        b_markers,
        std::collections::HashSet::from([0xBB]),
        "B route cross-talk: {on_b:?}"
    );
}
