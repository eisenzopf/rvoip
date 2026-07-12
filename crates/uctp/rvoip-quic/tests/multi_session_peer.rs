//! One physical QUIC peer may multiplex multiple UCTP Sessions. The media
//! header contains only a peer-local u16, so this verifies that the production
//! adapter allocates one namespace and routes each negotiated Stream exactly.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter};
use rvoip_core::ids::{ConnectionId, StreamId};
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_quic::{QuicDatagramMediaStream, UctpQuicAdapter, UctpQuicClient, UctpQuicConfig};
use rvoip_uctp::payloads::auth;
use rvoip_uctp::substrate::{dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::{MessageType, UctpEnvelope, UCTP_RAW_QUIC_ALPN_BYTES};

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn server_endpoint(
    addr: SocketAddr,
) -> (
    Arc<quinn::Endpoint>,
    rustls::pki_types::CertificateDer<'static>,
) {
    let (certificate, key) = self_signed_for_dev(&["localhost".into()]).expect("self_signed");
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], key)
        .expect("server tls");
    tls.alpn_protocols = vec![UCTP_RAW_QUIC_ALPN_BYTES.to_vec()];
    let endpoint = rvoip_uctp::substrate::make_server_endpoint(
        addr,
        Arc::new(tls),
        quinn::TransportConfig::default(),
    )
    .expect("server endpoint");
    (Arc::new(endpoint), certificate)
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

async fn authenticate(
    client: &Arc<UctpQuicClient>,
    inbound: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>,
) {
    let hello = UctpEnvelope::new(
        MessageType::AuthHello,
        serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_multi_session".into(),
                kind: "test".into(),
                platform: "test".into(),
                sdk_version: "multi-session/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::json!({}),
        })
        .unwrap(),
    );
    client.send(hello).await.unwrap();
    let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth challenge timeout")
        .expect("peer closed");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    client
        .send(
            UctpEnvelope::new(
                MessageType::AuthResponse,
                serde_json::to_value(auth::AuthResponse {
                    method: "bearer".into(),
                    credential: "test-token".into(),
                    actor_token: None,
                })
                .unwrap(),
            )
            .with_in_reply_to(challenge.id),
        )
        .await
        .unwrap();
    let session = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth session timeout")
        .expect("peer closed");
    assert_eq!(session.msg_type, MessageType::AuthSession);
}

fn invite(sid: &str) -> UctpEnvelope {
    UctpEnvelope::new(
        MessageType::SessionInvite,
        serde_json::json!({
            "from": "part_remote",
            "to": ["part_server"],
            "medium": "voice",
            "intent": "synchronous-engagement",
            "capabilities_offer": {}
        }),
    )
    .with_cid(format!("conv_{sid}"))
    .with_sid(sid)
}

fn offer(sid: &str, connid: &str, stream_id: &str) -> UctpEnvelope {
    UctpEnvelope::new(
        MessageType::ConnectionOffer,
        serde_json::json!({
            "by_participant": "part_remote",
            "substrate": "quic",
            "capabilities": {},
            "streams_offered": [{
                "id": stream_id,
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

async fn wait_for_inbound_connection(
    events: &mut tokio::sync::mpsc::Receiver<AdapterEvent>,
    wire_sid: &str,
) -> ConnectionId {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("adapter event timeout")
            .expect("adapter event channel closed");
        if let AdapterEvent::InboundConnection { connection } = event {
            assert!(connection
                .session_id
                .as_str()
                .ends_with(&format!(":{wire_sid}")));
            return connection.id;
        }
    }
}

async fn wait_for_stream_opened(
    inbound: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>,
    expected_stream_id: &str,
) -> u16 {
    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
            .await
            .expect("stream.opened timeout")
            .expect("peer closed");
        if envelope.msg_type != MessageType::StreamOpened {
            continue;
        }
        let opened: rvoip_uctp::payloads::stream::StreamOpened =
            envelope.decode_payload().expect("decode stream.opened");
        assert_eq!(opened.stream.strm_id, expected_stream_id);
        return opened.stream.stream_local_id;
    }
}

async fn negotiate_stream(
    client: &Arc<UctpQuicClient>,
    inbound: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>,
    sid: &str,
    connid: &str,
    stream_id: &str,
) -> u16 {
    client.send(offer(sid, connid, stream_id)).await.unwrap();
    client
        .send(
            UctpEnvelope::new(MessageType::ConnectionReady, serde_json::json!({}))
                .with_sid(sid)
                .with_connid(connid),
        )
        .await
        .unwrap();
    wait_for_stream_opened(inbound, stream_id).await
}

#[tokio::test]
async fn two_sessions_on_one_peer_get_distinct_exact_media_routes() {
    install_crypto_provider();
    let (server_endpoint, certificate) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_endpoint.local_addr().unwrap();
    let mut alpn =
        dispatch_by_alpn(Arc::clone(&server_endpoint), &[UCTP_RAW_QUIC_ALPN_BYTES]).unwrap();
    let accept_rx = alpn.take(UCTP_RAW_QUIC_ALPN_BYTES).unwrap();
    let adapter = UctpQuicAdapter::new(UctpQuicConfig::new(
        Arc::clone(&server_endpoint),
        accept_rx,
        bearer_stub(),
    ))
    .await
    .unwrap();
    let mut events = adapter.subscribe_events();

    let client_endpoint = client_endpoint();
    let client_tls = rvoip_uctp::substrate::dev_client_config_trusting(&certificate).unwrap();
    let client = UctpQuicClient::connect(
        &client_endpoint,
        server_addr,
        "localhost",
        Arc::new(client_tls),
    )
    .await
    .unwrap();
    let mut inbound = client.take_inbound().unwrap();
    authenticate(&client, &mut inbound).await;

    client.send(invite("sess_one")).await.unwrap();
    let core_one = wait_for_inbound_connection(&mut events, "sess_one").await;
    let local_one =
        negotiate_stream(&client, &mut inbound, "sess_one", "conn_one", "strm_one").await;

    client.send(invite("sess_two")).await.unwrap();
    let core_two = wait_for_inbound_connection(&mut events, "sess_two").await;
    let local_two =
        negotiate_stream(&client, &mut inbound, "sess_two", "conn_two", "strm_two").await;

    assert_ne!(local_one, local_two, "local IDs are peer-global");
    let server_one = adapter.streams(core_one).await.unwrap().pop().unwrap();
    let server_two = adapter.streams(core_two).await.unwrap().pop().unwrap();
    assert_eq!(server_one.id().as_str(), "strm_one");
    assert_eq!(server_two.id().as_str(), "strm_two");
    let mut receive_one = server_one.frames_in();
    let mut receive_two = server_two.frames_in();

    let codec = rvoip_core::capability::CodecInfo::from_name_with_defaults("opus");
    let client_one = QuicDatagramMediaStream::start(
        StreamId::from_string("strm_one"),
        StreamKind::Audio,
        codec.clone(),
        rvoip_core::connection::Direction::Outbound,
        local_one,
        client.connection.clone(),
    );
    let client_two = QuicDatagramMediaStream::start(
        StreamId::from_string("strm_two"),
        StreamKind::Audio,
        codec,
        rvoip_core::connection::Direction::Outbound,
        local_two,
        client.connection.clone(),
    );
    client_one
        .frames_out()
        .send(MediaFrame {
            stream_id: client_one.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(b"session-one"),
            timestamp_rtp: 960,
            captured_at: Utc::now(),
            payload_type: Some(111),
        })
        .await
        .unwrap();
    client_two
        .frames_out()
        .send(MediaFrame {
            stream_id: client_two.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(b"session-two"),
            timestamp_rtp: 1920,
            captured_at: Utc::now(),
            payload_type: Some(111),
        })
        .await
        .unwrap();

    let frame_one = tokio::time::timeout(Duration::from_secs(5), receive_one.recv())
        .await
        .expect("session one media timeout")
        .expect("session one stream closed");
    let frame_two = tokio::time::timeout(Duration::from_secs(5), receive_two.recv())
        .await
        .expect("session two media timeout")
        .expect("session two stream closed");
    assert_eq!(frame_one.payload, Bytes::from_static(b"session-one"));
    assert_eq!(frame_two.payload, Bytes::from_static(b"session-two"));
    assert_eq!(frame_one.stream_id.as_str(), "strm_one");
    assert_eq!(frame_two.stream_id.as_str(), "strm_two");
}
