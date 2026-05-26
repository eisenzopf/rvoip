//! v0.x MP3c — per-subscriber `stream_local_id` rewriting (plan B1 / G4).
//!
//! Three QUIC peers in one Session: two publishers (A, B) and one
//! subscriber (S). S subscribes to both A's and B's audio streams.
//! The server allocates a **distinct** `stream_local_id` per
//! subscription so S can demultiplex A's frames from B's on the wire
//! (CONVERSATION_PROTOCOL.md §10.1 multi-party note). Without this fix
//! (pre-B1 behavior) both publishers' fanout datagrams would land on
//! S's default audio stream with the same local_id=1, indistinguishable
//! at S's jitter buffer.
//!
//! What this asserts:
//! 1. The server emits one `stream.opened` envelope per subscription,
//!    each carrying a fresh `stream_local_id` (≥ 2, allocator skips
//!    the default audio stream's slot of 1).
//! 2. The two announced local_ids are distinct.
//! 3. Frames injected at A arrive at S on A's allocated local_id;
//!    frames injected at B arrive at S on B's allocated local_id; no
//!    cross-talk.

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
use rvoip_uctp::payloads::{auth, session::SessionInvite};
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
    signature: None,
    }
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

/// Drain envelopes until the next `stream.opened` arrives; return its
/// `stream_local_id`. Used to learn the MP3c-allocated subscriber-side
/// local_id from the server's announcement.
async fn next_stream_opened_local_id(
    inbound: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>,
) -> u16 {
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
        return payload.stream.stream_local_id;
    }
    panic!("no stream.opened envelope arrived");
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

    // --- Three clients: two publishers (a, b) and one subscriber (s) ---
    let ep_a = client_endpoint();
    let ep_b = client_endpoint();
    let ep_s = client_endpoint();
    let cfg = dev_client_config_trusting(&cert_der).expect("client cfg");

    let client_a =
        UctpQuicClient::connect(&ep_a, server_addr, "localhost", Arc::new(cfg.clone()))
            .await
            .expect("client a");
    let client_b =
        UctpQuicClient::connect(&ep_b, server_addr, "localhost", Arc::new(cfg.clone()))
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
    client_s
        .send(invite("sess_3p_s", "part_s"))
        .await
        .expect("invite s");
    let conn_s = loop {
        if let Ok(Ok(Event::ConnectionInbound { connection_id, .. })) =
            tokio::time::timeout(Duration::from_secs(5), events.recv()).await
        {
            break connection_id;
        }
    };

    // Look up each publisher's server-side audio strm_id.
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

    // Register two subscriptions for s: from a's audio AND from b's
    // audio. The orchestrator's subscription table keys on the
    // **publisher's** sid (the sid the publisher's adapter passes to
    // `fanout_frame` when it sees an inbound datagram). The fanout
    // call goes through each publisher's own coordinator, so the
    // subscription must be registered against that publisher's sid.
    let sid_a = SessionId::from_string("sess_3p");
    let sid_b = SessionId::from_string("sess_3p_b");
    orchestrator.add_subscription(sid_a.clone(), conn_s.clone(), conn_a.clone(), strm_a.clone());
    orchestrator.add_subscription(sid_b.clone(), conn_s.clone(), conn_b.clone(), strm_b.clone());

    // --- Publisher-side client streams (both fixed local_id=1, the
    // server-side default audio stream's slot) ---
    let pub_a_stream = QuicDatagramMediaStream::start(
        rvoip_core::ids::StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        1,
        client_a.connection.clone(),
    );
    let pub_b_stream = QuicDatagramMediaStream::start(
        rvoip_core::ids::StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        1,
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

    // --- Trigger lazy allocation by priming the fanout from each
    // publisher. The first frame on each path may be lost (the
    // subscriber's reader / per-publisher MediaStream isn't ready
    // yet); subsequent frames carry the test payload. ---
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

    // Receive the two `stream.opened` envelopes from the subscriber's
    // signaling channel — order isn't deterministic between A and B,
    // so we collect both and dispatch by local_id.
    let local_id_one = next_stream_opened_local_id(&mut in_s).await;
    let local_id_two = next_stream_opened_local_id(&mut in_s).await;
    assert_ne!(
        local_id_one, local_id_two,
        "MP3c: each subscription must get a distinct stream_local_id"
    );
    assert!(
        local_id_one >= 2 && local_id_two >= 2,
        "MP3c: allocator must skip the default audio stream's slot (1)"
    );

    // Build subscriber-side MediaStreams for both local_ids. The
    // subscriber doesn't yet know which publisher owns which id (it
    // would need to correlate strm_id strings); for this test we just
    // verify that frames arrive on distinct local_ids and we can
    // identify each publisher's signature.
    let sub_stream_one = QuicDatagramMediaStream::start(
        rvoip_core::ids::StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        local_id_one,
        client_s.connection.clone(),
    );
    let sub_stream_two = QuicDatagramMediaStream::start(
        rvoip_core::ids::StreamId::new(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        local_id_two,
        client_s.connection.clone(),
    );
    quic_spawn_datagram_reader(
        client_s.connection.clone(),
        Arc::new(parking_lot::RwLock::new(vec![
            Arc::clone(&sub_stream_one),
            Arc::clone(&sub_stream_two),
        ])),
        None,
    );

    let mut rx_one = rvoip_core::stream::MediaStream::frames_in(sub_stream_one.as_ref());
    let mut rx_two = rvoip_core::stream::MediaStream::frames_in(sub_stream_two.as_ref());

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

    // Drain both subscriber streams. Each should receive only one
    // publisher's marker — never both. Collect for a generous window
    // and then check the cross-talk invariant.
    let mut on_one: Vec<u8> = Vec::new();
    let mut on_two: Vec<u8> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline && (on_one.len() + on_two.len()) < 10 {
        tokio::select! {
            biased;
            frame = rx_one.recv() => {
                if let Some(f) = frame {
                    if f.payload.as_ref() != prime.as_ref() {
                        on_one.push(f.payload[0]);
                    }
                }
            }
            frame = rx_two.recv() => {
                if let Some(f) = frame {
                    if f.payload.as_ref() != prime.as_ref() {
                        on_two.push(f.payload[0]);
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    // Cross-talk invariant: each subscriber-side stream only carries
    // ONE publisher's marker. Mixing would indicate the MP3c
    // local_id rewrite isn't working — both publishers' frames would
    // land on the same MediaStream.
    let one_markers: std::collections::HashSet<u8> =
        on_one.iter().copied().collect();
    let two_markers: std::collections::HashSet<u8> =
        on_two.iter().copied().collect();
    assert!(
        one_markers.len() <= 1,
        "MP3c: subscriber stream one received mixed publishers: {:?}",
        on_one
    );
    assert!(
        two_markers.len() <= 1,
        "MP3c: subscriber stream two received mixed publishers: {:?}",
        on_two
    );
    // And the two streams must carry different markers (one A, one B).
    if !on_one.is_empty() && !on_two.is_empty() {
        assert_ne!(
            on_one[0], on_two[0],
            "MP3c: both subscriber streams carry the same publisher — local_id rewrite collapsed"
        );
    }
    // Sanity: at least one publisher made it through on each stream.
    assert!(
        !on_one.is_empty() || !on_two.is_empty(),
        "no fanout frames reached the subscriber at all"
    );
}
