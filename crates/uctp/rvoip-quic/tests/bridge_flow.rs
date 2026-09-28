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
use rvoip_core::adapter::{ConnectionAdapter, EndReason};
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

async fn dial_and_invite(
    client_ep: &quinn::Endpoint,
    server_addr: SocketAddr,
    cert: &rustls::pki_types::CertificateDer<'static>,
    sid: &str,
    participant: &str,
) -> (Arc<UctpQuicClient>, u16) {
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
    let wire_connid = format!("conn_{participant}");
    let wire_stream_id = format!("strm_{participant}");
    client
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::json!({
                    "by_participant": participant,
                    "substrate": "quic",
                    "capabilities": {},
                    "streams_offered": [{
                        "id": wire_stream_id,
                        "kind": "audio",
                        "direction": "sendrecv",
                        "codec_preferences": ["g.711-mu"]
                    }],
                    "substrate_setup": null
                }),
            )
            .with_sid(sid)
            .with_connid(wire_connid.clone()),
        )
        .await
        .expect("send connection.offer");
    client
        .send(
            UctpEnvelope::new(MessageType::ConnectionReady, serde_json::json!({}))
                .with_sid(sid)
                .with_connid(wire_connid),
        )
        .await
        .expect("send connection.ready");

    let stream_local_id = loop {
        let envelope = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
            .await
            .expect("stream.opened timeout")
            .expect("inbound closed");
        if envelope.msg_type == MessageType::StreamOpened {
            let opened: rvoip_uctp::payloads::stream::StreamOpened =
                envelope.decode_payload().expect("decode stream.opened");
            break opened.stream.stream_local_id;
        }
    };
    (client, stream_local_id)
}

/// Drain core events until `want` inbound Connections have been observed,
/// accumulating the paired `ConnectionAuthenticated` /
/// `ConnectionPrincipalAuthenticated` events along the way so none is lost
/// between two dial steps. Bounded so a missing event fails the test
/// instead of hanging it.
async fn collect_admissions(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    conn_ids: &mut Vec<ConnectionId>,
    authenticated: &mut Vec<(ConnectionId, String, String)>,
    principals: &mut Vec<(ConnectionId, rvoip_core::identity::AuthenticatedPrincipal)>,
    want: usize,
) {
    for _ in 0..60 {
        if conn_ids.len() >= want && authenticated.len() >= want && principals.len() >= want {
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
            Ok(Ok(Event::ConnectionPrincipalAuthenticated {
                connection_id,
                principal,
                ..
            })) => {
                principals.push((connection_id, principal));
            }
            _ => continue,
        }
    }
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
    // A3: every UCTP InboundConnection should now be followed by a
    // ConnectionAuthenticated carrying the auth handshake's identity_id
    // / participant_id / assurance triple. We accumulate both and
    // assert pairing after the loop.
    //
    // Client A is admitted and its core ConnectionId pinned as `retained`
    // before client B dials, so the later handoff names A by identity rather
    // than by event order.
    let mut conn_ids: Vec<ConnectionId> = Vec::new();
    let mut authenticated: Vec<(ConnectionId, String, String)> = Vec::new();
    let mut principals = Vec::new();
    let client_ep_a = client_endpoint();
    let client_ep_b = client_endpoint();
    let (client_a, client_a_local_id) =
        dial_and_invite(&client_ep_a, server_addr, &cert_der, "sess_a", "part_alice").await;
    collect_admissions(
        &mut events,
        &mut conn_ids,
        &mut authenticated,
        &mut principals,
        1,
    )
    .await;
    assert_eq!(conn_ids.len(), 1, "expected client A's ConnectionInbound");
    let retained = conn_ids[0].clone();
    let (client_b, client_b_local_id) =
        dial_and_invite(&client_ep_b, server_addr, &cert_der, "sess_b", "part_bob").await;
    collect_admissions(
        &mut events,
        &mut conn_ids,
        &mut authenticated,
        &mut principals,
        2,
    )
    .await;
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
    assert_eq!(
        principals.len(),
        2,
        "expected each UCTP connection to retain its complete principal"
    );
    for (connection_id, principal) in &principals {
        let legacy_subject = authenticated
            .iter()
            .find(|(candidate, _, _)| candidate == connection_id)
            .map(|(_, identity_id, _)| identity_id)
            .expect("rich principal has matching legacy auth event");
        assert_eq!(&principal.subject, legacy_subject);
        assert_eq!(
            orchestrator
                .connection_principal(connection_id)
                .expect("principal retained on orchestrator route")
                .ownership_key(),
            principal.ownership_key()
        );
    }

    // --- Bridge ---
    let bridge_id = orchestrator
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
        // This test runs in the default, system-library-free shard. Exercise
        // the bridge with the default G.711 codec; Opus is qualified by the
        // native facade bundle and codec feature matrix.
        name: "g.711-mu".into(),
        clock_rate_hz: 8_000,
        channels: 1,
        fmtp: None,
        payload_type: Some(0),
    };

    // Client A is sendrecv (`Inbound` on a local peer; see the loopback
    // test): it injects now and must also be able to receive from client C
    // after the handoff below. An `Outbound` binding would reject C's
    // datagrams at A's reader.
    let client_a_stream = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec.clone(),
        rvoip_core::connection::Direction::Inbound,
        client_a_local_id,
        client_a.connection.clone(),
    );
    let client_b_stream = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec,
        rvoip_core::connection::Direction::Inbound,
        client_b_local_id,
        client_b.connection.clone(),
    );

    // Spawn datagram readers on both client connections.
    let router_a = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&client_a_stream)]));
    let router_b = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&client_b_stream)]));
    spawn_datagram_reader(client_a.connection.clone(), router_a, None);
    spawn_datagram_reader(client_b.connection.clone(), router_b, None);

    // --- Inject 10 frames from client A; observe all of them on client B in order. ---
    let client_a_out = rvoip_core::stream::MediaStream::frames_out(client_a_stream.as_ref());
    let mut client_b_in = client_b_stream
        .try_frames_in()
        .expect("client B media receiver");

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

    // --- Peer handoff over real transports: A–B becomes A–C ---
    // A third QUIC client is admitted, then core's transport-fenced
    // replacement swaps B for C while A stays put. Afterwards both new
    // directions carry audio over the wire, B receives nothing, and A is
    // still live. The property under test is the transport fence, not the
    // entry point that drives it, so the one-shot core API is used here.
    let client_ep_c = client_endpoint();
    let (client_c, client_c_local_id) =
        dial_and_invite(&client_ep_c, server_addr, &cert_der, "sess_c", "part_carol").await;
    collect_admissions(
        &mut events,
        &mut conn_ids,
        &mut authenticated,
        &mut principals,
        3,
    )
    .await;
    assert_eq!(conn_ids.len(), 3, "expected client C's ConnectionInbound");
    let previous_destination = conn_ids[1].clone();
    let target = conn_ids[2].clone();
    assert_ne!(target, retained);
    assert_ne!(target, previous_destination);

    let client_c_stream = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        client_b_stream.codec(),
        rvoip_core::connection::Direction::Inbound,
        client_c_local_id,
        client_c.connection.clone(),
    );
    let router_c = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&client_c_stream)]));
    spawn_datagram_reader(client_c.connection.clone(), router_c, None);
    let mut client_c_in = client_c_stream
        .try_frames_in()
        .expect("client C media receiver");
    let mut client_a_in = client_a_stream
        .try_frames_in()
        .expect("client A media receiver");

    let replacement = tokio::time::timeout(
        Duration::from_secs(5),
        orchestrator.replace_bridge_destination_transport_fenced(
            bridge_id.clone(),
            retained.clone(),
            previous_destination.clone(),
            target.clone(),
        ),
    )
    .await
    .expect("real transport handoff timed out")
    .expect("real transport handoff");
    assert_eq!(replacement.previous_bridge_id, bridge_id);
    assert_ne!(
        replacement.bridge_id, bridge_id,
        "a fresh generation is minted"
    );
    assert_eq!(replacement.ingress, retained);
    assert_eq!(replacement.previous_destination, previous_destination);
    assert_eq!(replacement.destination, target);

    for (stream, value) in [(&client_a_stream, 71u8), (&client_c_stream, 72u8)] {
        stream
            .frames_out()
            .send(MediaFrame {
                stream_id: stream.id(),
                kind: StreamKind::Audio,
                payload: Bytes::from(vec![value]),
                timestamp_rtp: 2_000,
                captured_at: Utc::now(),
                payload_type: Some(0),
            })
            .await
            .expect("inject post-handoff frame");
    }
    for (receiver, expected, label) in [
        (&mut client_c_in, 71u8, "A→C"),
        (&mut client_a_in, 72u8, "C→A"),
    ] {
        let received = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for the post-handoff {label} frame"))
            .unwrap_or_else(|| panic!("{label} stream closed unexpectedly"));
        assert_eq!(
            received.payload[0], expected,
            "{label} carried the wrong frame"
        );
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(200), client_b_in.recv())
            .await
            .is_err(),
        "the retired peer received post-handoff audio"
    );
    assert!(adapter.is_connection_live(&retained));
    assert!(adapter.is_connection_live(&target));

    // Core owns terminal dispatch. The adapter removes route ownership before
    // wire teardown, then reports exactly one normalized terminal even if a
    // late peer terminal races in afterwards.
    let ended_connection = conn_ids[0].clone();
    orchestrator
        .end_connection(ended_connection.clone(), EndReason::Normal)
        .await
        .expect("core normal end");
    assert!(!adapter.is_connection_live(&ended_connection));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                events.recv().await,
                Ok(Event::ConnectionEnded { connection_id, .. }) if connection_id == ended_connection
            ) {
                break;
            }
        }
    })
    .await
    .expect("one normalized terminal event");

    client_a
        .send(
            UctpEnvelope::new(
                MessageType::SessionEnd,
                serde_json::to_value(rvoip_uctp::payloads::session::SessionEnd {
                    by: "part_alice".into(),
                    reason_code: 0,
                    reason: "late duplicate".into(),
                })
                .expect("serialize peer terminal"),
            )
            .with_sid("sess_a"),
        )
        .await
        .expect("send late duplicate terminal");
    let duplicate = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if matches!(
                events.recv().await,
                Ok(Event::ConnectionEnded { connection_id, .. }) if connection_id == ended_connection
            ) {
                break;
            }
        }
    })
    .await;
    assert!(duplicate.is_err(), "late peer terminal must be suppressed");
}
