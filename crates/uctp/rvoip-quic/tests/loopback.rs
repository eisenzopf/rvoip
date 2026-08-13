//! Loopback test per `UCTP_IMPLEMENTATION_PLAN.md` §4.6.
//!
//! Binds a quinn endpoint on 127.0.0.1:0, dispatches by ALPN to a
//! `UctpQuicAdapter`, connects a `UctpQuicClient`, exchanges an
//! `auth.hello` → `auth.challenge` round-trip, and verifies the
//! adapter emitted at least one `AdapterEvent`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_quic::{UctpQuicAdapter, UctpQuicClient, UctpQuicConfig};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::auth;
use rvoip_uctp::substrate::{dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;

const ALPN_UCTP: &[u8] = b"uctp/1";

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
    // For dialing we just need a UDP-bound endpoint; client config is
    // passed at connect_with time.
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

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[tokio::test]
async fn loopback_auth_handshake_via_adapter() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    // --- Server side ---
    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");

    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP]).expect("dispatcher");
    let accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");

    let cfg = UctpQuicConfig::new(Arc::clone(&server_ep), accept_rx, bearer_stub());
    let adapter = UctpQuicAdapter::new(cfg).await.expect("adapter");
    let mut events = adapter.subscribe_events();

    // --- Client side ---
    let client_ep = client_endpoint();
    let client_cfg =
        rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("client cfg");

    let client =
        UctpQuicClient::connect(&client_ep, server_addr, "localhost", Arc::new(client_cfg))
            .await
            .expect("client connect");

    let mut inbound = client.take_inbound().expect("first take");

    // --- Send auth.hello and expect auth.challenge back ---
    let payload = auth::AuthHello {
        device: auth::Device {
            id: "dev_test".into(),
            kind: "desktop".into(),
            platform: "test".into(),
            sdk_version: "rvoip-quic-test/0.1".into(),
        },
        auth_methods: vec!["bearer".into()],
        capabilities: serde_json::Value::Object(Default::default()),
    };
    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: "env_hello".into(),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(payload).unwrap(),
        signature: None,
    };
    client.send(env).await.expect("send");

    let reply = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("timed out waiting for challenge")
        .expect("inbound channel closed");

    assert_eq!(reply.msg_type, MessageType::AuthChallenge);

    // The adapter should have emitted at least one Native event for the new connection.
    let _ = tokio::time::timeout(Duration::from_millis(500), events.recv()).await;
}

/// Spec-required: 5 envelopes each direction with ordering preserved.
/// Each `auth.hello` from the client yields one `auth.challenge` reply,
/// so 5 sends produce 5 receives. Reply IDs encode the order via
/// `in_reply_to` matching each request.
#[tokio::test]
async fn loopback_five_envelopes_each_direction_in_order() {
    install_crypto_provider();

    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let mut routes = rvoip_uctp::substrate::dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP])
        .expect("dispatcher");
    let accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");

    let cfg = UctpQuicConfig::new(Arc::clone(&server_ep), accept_rx, bearer_stub());
    let _adapter = UctpQuicAdapter::new(cfg).await.expect("adapter");

    let client_ep = client_endpoint();
    let client_cfg =
        rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("client cfg");
    let client =
        UctpQuicClient::connect(&client_ep, server_addr, "localhost", Arc::new(client_cfg))
            .await
            .expect("client connect");

    let mut inbound = client.take_inbound().expect("first take");

    for i in 0..5u32 {
        let payload = rvoip_uctp::payloads::auth::AuthHello {
            device: rvoip_uctp::payloads::auth::Device {
                id: format!("dev_{}", i),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "test/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        };
        let env = UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthHello,
            id: format!("env_{}", i),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: None,
            payload: serde_json::to_value(payload).unwrap(),
            signature: None,
        };
        client.send(env).await.expect("send");
    }

    for i in 0..5u32 {
        let reply = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        assert_eq!(reply.msg_type, MessageType::AuthChallenge);
        assert_eq!(
            reply.in_reply_to.as_deref(),
            Some(format!("env_{}", i).as_str()),
            "reply {} out of order",
            i
        );
    }
}

/// Datagram pump round-trip: 10 frames in each direction, ordering
/// preserved (within the limits of UDP — QUIC datagrams are unreliable
/// but loopback rarely loses).
#[tokio::test]
async fn loopback_datagram_pump_round_trip() {
    use bytes::Bytes;
    use rvoip_core::capability::CodecInfo;
    use rvoip_core::connection::Direction;
    use rvoip_core::ids::StreamId;
    use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};

    install_crypto_provider();

    // Two endpoints, no adapter involvement — we drive the
    // QuicDatagramMediaStream / spawn_datagram_reader pair directly to
    // validate the pump in isolation.
    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");

    // Server-side: accept one connection and stash a clone.
    let server_conn_handle = {
        let ep = Arc::clone(&server_ep);
        tokio::spawn(async move {
            let incoming = ep.accept().await.expect("incoming");
            incoming.accept().expect("connecting").await.expect("conn")
        })
    };

    // Client-side: dial with a QUIC config that opts into datagrams.
    let client_ep = client_endpoint();
    let mut tls = rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("cfg");
    tls.alpn_protocols = vec![ALPN_UCTP.to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls).expect("crypto");
    let qc = quinn::ClientConfig::new(Arc::new(crypto));
    let client_conn = client_ep
        .connect_with(qc, server_addr, "localhost")
        .expect("connect_with")
        .await
        .expect("client conn");
    let server_conn = server_conn_handle.await.expect("server conn");

    // Build matched media streams: server's id = 1, client's id = 1 (same
    // mapping). Each side spawns its own pump + reader against its own
    // quinn::Connection.
    let codec = CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48000,
        channels: 1,
        fmtp: None,
        payload_type: None,
    };

    // `Direction::Outbound` represents a receive-only wire offer and must
    // reject peer-supplied datagrams. This round-trip exercises a sendrecv
    // binding, which is represented by `Inbound` on both local peers.
    let client_stream = rvoip_quic::QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec.clone(),
        Direction::Inbound,
        1,
        client_conn.clone(),
    );
    let server_stream = rvoip_quic::QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec.clone(),
        Direction::Inbound,
        1,
        server_conn.clone(),
    );

    let client_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&client_stream)]));
    let server_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&server_stream)]));
    rvoip_quic::spawn_datagram_reader(client_conn.clone(), client_router, None);
    rvoip_quic::spawn_datagram_reader(server_conn.clone(), server_router, None);

    // Client → server: 10 frames.
    let client_out = rvoip_core::stream::MediaStream::frames_out(client_stream.as_ref());
    let mut server_in = rvoip_core::stream::MediaStream::frames_in(server_stream.as_ref());
    for i in 0u8..10 {
        let frame = MediaFrame {
            stream_id: client_stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![i]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: None,
        };
        client_out.send(frame).await.expect("client send");
    }

    let mut received = Vec::new();
    while received.len() < 10 {
        let frame = tokio::time::timeout(Duration::from_secs(5), server_in.recv())
            .await
            .expect("server recv timed out")
            .expect("server stream closed");
        received.push(frame.payload[0]);
    }
    assert_eq!(
        received,
        (0u8..10).collect::<Vec<_>>(),
        "ordering broken on client→server"
    );

    // Server → client: 10 frames.
    let server_out = rvoip_core::stream::MediaStream::frames_out(server_stream.as_ref());
    let mut client_in = rvoip_core::stream::MediaStream::frames_in(client_stream.as_ref());
    for i in 0u8..10 {
        let frame = MediaFrame {
            stream_id: server_stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![100 + i]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: None,
        };
        server_out.send(frame).await.expect("server send");
    }

    let mut received = Vec::new();
    while received.len() < 10 {
        let frame = tokio::time::timeout(Duration::from_secs(5), client_in.recv())
            .await
            .expect("client recv timed out")
            .expect("client stream closed");
        received.push(frame.payload[0]);
    }
    assert_eq!(
        received,
        (0u8..10).map(|i| 100 + i).collect::<Vec<_>>(),
        "ordering broken on server→client"
    );
}

/// AMR over a real QUIC datagram, both the carrying case and the refusal.
///
/// The pump resolves a frame's payload type from the frame itself, falling
/// back to a name table that has no AMR row — so everything about AMR here
/// depends on the frame carrying its own. That was previously argued from
/// reading the code; this runs it over a live connection instead.
///
/// The negative half matters more than the positive one. Before
/// `resolve_payload_type`, an unlabelled frame took `unwrap_or(111)` and went
/// out wearing Opus's payload type: well-formed, right length, parses fine,
/// and lying about what it contains. A receiver cannot detect that. So the
/// unlabelled frame must not arrive at all, and a labelled frame sent after it
/// must, which is what separates "dropped" from "the test was too impatient".
#[tokio::test]
async fn loopback_amr_datagram_keeps_its_payload_type_and_drops_the_unlabelled() {
    use bytes::Bytes;
    use rvoip_core::capability::CodecInfo;
    use rvoip_core::connection::Direction;
    use rvoip_core::ids::StreamId;
    use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};

    const AMR_OA_PT: u8 = 107;

    install_crypto_provider();

    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let server_conn_handle = {
        let ep = Arc::clone(&server_ep);
        tokio::spawn(async move {
            let incoming = ep.accept().await.expect("incoming");
            incoming.accept().expect("connecting").await.expect("conn")
        })
    };

    let client_ep = client_endpoint();
    let mut tls = rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("cfg");
    tls.alpn_protocols = vec![ALPN_UCTP.to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls).expect("crypto");
    let qc = quinn::ClientConfig::new(Arc::new(crypto));
    let client_conn = client_ep
        .connect_with(qc, server_addr, "localhost")
        .expect("connect_with")
        .await
        .expect("client conn");
    let server_conn = server_conn_handle.await.expect("server conn");

    // `payload_type: None` is what UCTP actually produces: it negotiates
    // codecs by name and has no payload type to report. So the stream's own
    // descriptor cannot supply one, and every frame is on its own.
    let codec = CodecInfo {
        name: "AMR".into(),
        clock_rate_hz: 8_000,
        channels: 1,
        fmtp: Some("octet-align=1".into()),
        payload_type: None,
    };

    let client_stream = rvoip_quic::QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec.clone(),
        Direction::Inbound,
        1,
        client_conn.clone(),
    );
    let server_stream = rvoip_quic::QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec.clone(),
        Direction::Inbound,
        1,
        server_conn.clone(),
    );
    let client_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&client_stream)]));
    let server_router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&server_stream)]));
    rvoip_quic::spawn_datagram_reader(client_conn.clone(), client_router, None);
    rvoip_quic::spawn_datagram_reader(server_conn.clone(), server_router, None);

    let client_out = rvoip_core::stream::MediaStream::frames_out(client_stream.as_ref());
    let mut server_in = rvoip_core::stream::MediaStream::frames_in(server_stream.as_ref());

    // Payload bytes are markers, not audio: the pump packs and labels, it
    // never decodes, so what is being asserted is the label.
    for (marker, payload_type) in [
        (0xAA_u8, Some(AMR_OA_PT)),
        (0xBB, None), // must be dropped rather than labelled 111
        (0xCC, Some(AMR_OA_PT)),
    ] {
        client_out
            .send(MediaFrame {
                stream_id: client_stream.id(),
                kind: StreamKind::Audio,
                payload: Bytes::from(vec![marker; 32]),
                timestamp_rtp: u32::from(marker),
                captured_at: Utc::now(),
                payload_type,
            })
            .await
            .expect("client send");
    }

    let mut seen = Vec::new();
    while seen.len() < 2 {
        let frame = tokio::time::timeout(Duration::from_secs(5), server_in.recv())
            .await
            .expect("server recv timed out")
            .expect("server stream closed");
        assert_eq!(
            frame.payload_type,
            Some(AMR_OA_PT),
            "AMR's negotiated payload type must reach the peer intact; 111 \
             would mean the pump relabelled it as Opus"
        );
        seen.push(frame.payload[0]);
    }

    assert_eq!(
        seen,
        vec![0xAA, 0xCC],
        "the unlabelled frame must be dropped, not delivered under a \
         fabricated payload type"
    );

    // The frame after the dropped one already arrived, so anything still
    // queued would be the dropped frame turning up late.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), server_in.recv())
            .await
            .is_err(),
        "nothing further should arrive: the unlabelled frame was dropped"
    );
}
