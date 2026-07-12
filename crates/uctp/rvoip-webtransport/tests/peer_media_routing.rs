//! One physical WebTransport peer carrying two logical Sessions.
//!
//! The UCTP datagram header has no Session/Connection field, so this test is
//! the regression gate that local stream IDs are peer-global and dispatch to
//! the exact negotiated core route rather than the first invited Session.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter};
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_core::{Config, Orchestrator};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, connection, session, stream};
use rvoip_uctp::state::{
    OrchestratorSubscriptionHandler, ResourceBindingError, SessionBindingResolver,
};
use rvoip_uctp::substrate::{
    dispatch_by_alpn, pack_rtp_datagram, self_signed_for_dev, RtpDatagram, RtpMediaPayload,
};
use rvoip_uctp::types::MessageType;
use rvoip_webtransport::{
    spawn_datagram_reader, UctpWtAdapter, UctpWtClient, UctpWtConfig,
    WebTransportDatagramMediaStream,
};
use tokio::sync::mpsc;
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

fn default_codec() -> rvoip_core::capability::CodecInfo {
    rvoip_core::capability::CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48_000,
        channels: 1,
        fmtp: None,
    }
}

async fn authenticate(client: &UctpWtClient, inbound: &mut mpsc::Receiver<UctpEnvelope>) -> String {
    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthHello,
            id: "env_peer_router_hello".into(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: None,
            payload: serde_json::to_value(auth::AuthHello {
                device: auth::Device {
                    id: "dev_peer_router".into(),
                    kind: "browser".into(),
                    platform: "test".into(),
                    sdk_version: "test/0.1".into(),
                },
                auth_methods: vec!["bearer".into()],
                capabilities: serde_json::Value::Object(Default::default()),
            })
            .unwrap(),
            signature: None,
        })
        .await
        .expect("send auth.hello");
    let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.challenge timeout")
        .expect("inbound closed");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthResponse,
            id: "env_peer_router_response".into(),
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
        })
        .await
        .expect("send auth.response");
    let authenticated = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.session timeout")
        .expect("inbound closed");
    authenticated
        .decode_payload::<auth::AuthSession>()
        .expect("decode auth.session")
        .participant_id
}

async fn send_invite(client: &UctpWtClient, sid: &str, participant: &str) {
    client
        .send(
            UctpEnvelope::new(
                MessageType::SessionInvite,
                serde_json::to_value(session::SessionInvite {
                    from: participant.into(),
                    to: vec!["part_target".into()],
                    medium: "voice".into(),
                    intent: "synchronous-engagement".into(),
                    capabilities_offer: serde_json::Value::Object(Default::default()),
                })
                .unwrap(),
            )
            .with_sid(sid),
        )
        .await
        .expect("send session.invite");
}

async fn next_connection(events: &mut mpsc::Receiver<AdapterEvent>) -> rvoip_core::Connection {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let AdapterEvent::InboundConnection { connection } =
                events.recv().await.expect("adapter events closed")
            {
                break connection;
            }
        }
    })
    .await
    .expect("inbound connection timeout")
}

async fn next_orchestrator_connection(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
) -> ConnectionId {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(Event::ConnectionInbound { connection_id, .. }) => break connection_id,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    panic!("orchestrator events closed")
                }
            }
        }
    })
    .await
    .expect("orchestrator inbound connection timeout")
}

async fn next_stream_opened(
    inbound: &mut mpsc::Receiver<UctpEnvelope>,
    sid: &str,
    connid: &str,
) -> stream::StreamOpened {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let envelope = inbound.recv().await.expect("client inbound closed");
            if envelope.msg_type == MessageType::StreamOpened
                && envelope.sid.as_deref() == Some(sid)
                && envelope.connid.as_deref() == Some(connid)
            {
                break envelope
                    .decode_payload::<stream::StreamOpened>()
                    .expect("decode stream.opened");
            }
        }
    })
    .await
    .expect("stream.opened timeout")
}

async fn next_ack(inbound: &mut mpsc::Receiver<UctpEnvelope>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let envelope = inbound.recv().await.expect("client inbound closed");
            match envelope.msg_type {
                MessageType::Ack => break,
                MessageType::Error => panic!("wire command rejected: {envelope:?}"),
                _ => {}
            }
        }
    })
    .await
    .expect("wire command ack timeout");
}

fn stream_subscribe(sid: &str, connid: &str, strm_id: &str) -> UctpEnvelope {
    UctpEnvelope::new(
        MessageType::StreamSubscribe,
        serde_json::to_value(stream::StreamSubscribe {
            by_participant: "authenticated-owner".into(),
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

fn stream_unsubscribe(sid: &str, connid: &str, strm_id: &str) -> UctpEnvelope {
    UctpEnvelope::new(
        MessageType::StreamUnsubscribe,
        serde_json::to_value(stream::StreamUnsubscribe {
            strm_ids: vec![strm_id.into()],
        })
        .unwrap(),
    )
    .with_sid(sid)
    .with_connid(connid)
}

async fn bind_wire_stream(
    client: &UctpWtClient,
    inbound: &mut mpsc::Receiver<UctpEnvelope>,
    sid: &str,
    connid: &str,
    participant: &str,
    stream_id: &str,
) -> stream::StreamOpened {
    client
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::to_value(connection::ConnectionOffer {
                    by_participant: participant.into(),
                    substrate: "webtransport".into(),
                    capabilities: serde_json::Value::Object(Default::default()),
                    streams_offered: vec![connection::StreamOffer {
                        id: stream_id.into(),
                        kind: "audio".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["opus".into()],
                    }],
                    substrate_setup: serde_json::Value::Null,
                })
                .unwrap(),
            )
            .with_sid(sid)
            .with_connid(connid),
        )
        .await
        .expect("send connection.offer");
    client
        .send(
            UctpEnvelope::new(MessageType::ConnectionReady, serde_json::json!({}))
                .with_sid(sid)
                .with_connid(connid),
        )
        .await
        .expect("send connection.ready");

    next_stream_opened(inbound, sid, connid).await
}

async fn wait_for_stream(
    adapter: &Arc<UctpWtAdapter>,
    connection_id: &ConnectionId,
) -> Arc<dyn MediaStream> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let streams = adapter
                .streams(connection_id.clone())
                .await
                .expect("adapter streams");
            if let Some(stream) = streams.into_iter().next() {
                break stream;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bound stream timeout")
}

fn media_datagram(local_id: u16, seq: u32, body: &'static [u8]) -> Bytes {
    pack_rtp_datagram(&RtpDatagram {
        flags: 0,
        stream_local_id: local_id,
        seq,
        rtp: RtpMediaPayload {
            payload: Bytes::from_static(body),
            payload_type: 111,
            sequence_number: seq as u16,
            timestamp: seq * 960,
            ssrc: 0x4455_6677,
        },
    })
    .expect("encode media datagram")
}

async fn recv_frame(receiver: &mut mpsc::Receiver<MediaFrame>) -> MediaFrame {
    tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("media frame timeout")
        .expect("media receiver closed")
}

async fn send_stream_frame(
    stream: &Arc<WebTransportDatagramMediaStream>,
    body: &'static [u8],
    timestamp_rtp: u32,
) {
    stream
        .frames_out()
        .send(MediaFrame {
            stream_id: stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(body),
            timestamp_rtp,
            captured_at: Utc::now(),
            payload_type: Some(111),
        })
        .await
        .expect("send publisher media frame");
}

async fn recv_payload(
    receiver: &mut mpsc::Receiver<MediaFrame>,
    expected: &'static [u8],
) -> MediaFrame {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = receiver.recv().await.expect("media receiver closed");
            if frame.payload.as_ref() == expected {
                break frame;
            }
        }
    })
    .await
    .expect("expected media payload timeout")
}

#[tokio::test]
async fn one_peer_routes_two_sessions_on_distinct_local_ids() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_H3]).expect("dispatcher");
    let accept_rx = routes.take(ALPN_H3).expect("h3 channel");
    let adapter = UctpWtAdapter::new(UctpWtConfig::new(
        Arc::clone(&server_ep),
        accept_rx,
        bearer_stub(),
    ))
    .await
    .expect("adapter");
    let mut events = adapter.subscribe_events();

    let client_ep = client_endpoint();
    let client_cfg =
        rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("client cfg");
    let url = Url::parse(&format!("https://localhost:{}/uctp", server_addr.port())).expect("url");
    let client = UctpWtClient::connect(&client_ep, server_addr, &url, Arc::new(client_cfg))
        .await
        .expect("client connect");
    let mut inbound = client.take_inbound().expect("take inbound");
    let participant = authenticate(&client, &mut inbound).await;

    send_invite(&client, "sess_peer_a", &participant).await;
    let connection_a = next_connection(&mut events).await;
    send_invite(&client, "sess_peer_b", &participant).await;
    let connection_b = next_connection(&mut events).await;
    assert_ne!(connection_a.id, connection_b.id);
    assert_ne!(connection_a.session_id, connection_b.session_id);
    assert!(connection_a.session_id.as_str().ends_with(":sess_peer_a"));
    assert!(connection_b.session_id.as_str().ends_with(":sess_peer_b"));

    let opened_a = bind_wire_stream(
        &client,
        &mut inbound,
        "sess_peer_a",
        "conn_peer_a",
        &participant,
        "strm_shared_name",
    )
    .await;
    let opened_b = bind_wire_stream(
        &client,
        &mut inbound,
        "sess_peer_b",
        "conn_peer_b",
        &participant,
        "strm_shared_name",
    )
    .await;
    assert_ne!(
        opened_a.stream.stream_local_id, opened_b.stream.stream_local_id,
        "one physical peer must never reuse a local ID across Sessions"
    );

    let stream_a = wait_for_stream(&adapter, &connection_a.id).await;
    let stream_b = wait_for_stream(&adapter, &connection_b.id).await;
    assert_eq!(stream_a.id().as_str(), "strm_shared_name");
    assert_eq!(stream_b.id().as_str(), "strm_shared_name");
    let mut frames_a = stream_a.try_frames_in().expect("session A receiver");
    let mut frames_b = stream_b.try_frames_in().expect("session B receiver");

    client
        .session
        .send_datagram(media_datagram(
            opened_a.stream.stream_local_id,
            1,
            b"session-a",
        ))
        .expect("send Session A media");
    let frame_a = recv_frame(&mut frames_a).await;
    assert_eq!(frame_a.payload.as_ref(), b"session-a");
    assert!(
        tokio::time::timeout(Duration::from_millis(50), frames_b.recv())
            .await
            .is_err(),
        "Session A datagram leaked into Session B"
    );

    client
        .session
        .send_datagram(media_datagram(
            opened_b.stream.stream_local_id,
            2,
            b"session-b",
        ))
        .expect("send Session B media");
    let frame_b = recv_frame(&mut frames_b).await;
    assert_eq!(frame_b.payload.as_ref(), b"session-b");

    client
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionEnd,
                serde_json::to_value(connection::ConnectionEnd {
                    reason_code: 200,
                    reason: "test-complete".into(),
                })
                .unwrap(),
            )
            .with_sid("sess_peer_a")
            .with_connid("conn_peer_a"),
        )
        .await
        .expect("end Session A Connection");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.expect("adapter events closed"),
                AdapterEvent::Ended { ref connection_id, .. }
                    if connection_id == &connection_a.id
            ) {
                break;
            }
        }
    })
    .await
    .expect("Session A teardown timeout");
    assert!(adapter.streams(connection_a.id.clone()).await.is_err());
    client
        .session
        .send_datagram(media_datagram(
            opened_b.stream.stream_local_id,
            3,
            b"session-b-after-a-ended",
        ))
        .expect("send surviving Session B media");
    assert_eq!(
        recv_frame(&mut frames_b).await.payload.as_ref(),
        b"session-b-after-a-ended",
        "removing Session A disturbed Session B's peer-global route"
    );

    // A failed multi-stream bind is all-or-nothing. Reusing one Stream ID
    // inside the batch makes the second router commit fail; the first binding
    // must be rolled back, and a corrected offer must receive a fresh ID.
    send_invite(&client, "sess_peer_retry", &participant).await;
    let retry_connection = next_connection(&mut events).await;
    client
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::to_value(connection::ConnectionOffer {
                    by_participant: participant.clone(),
                    substrate: "webtransport".into(),
                    capabilities: serde_json::Value::Object(Default::default()),
                    streams_offered: vec![
                        connection::StreamOffer {
                            id: "strm_duplicate".into(),
                            kind: "audio".into(),
                            direction: "sendrecv".into(),
                            codec_preferences: vec!["opus".into()],
                        },
                        connection::StreamOffer {
                            id: "strm_duplicate".into(),
                            kind: "audio".into(),
                            direction: "sendrecv".into(),
                            codec_preferences: vec!["opus".into()],
                        },
                    ],
                    substrate_setup: serde_json::Value::Null,
                })
                .unwrap(),
            )
            .with_sid("sess_peer_retry")
            .with_connid("conn_peer_retry"),
        )
        .await
        .expect("send duplicate connection.offer");
    client
        .send(
            UctpEnvelope::new(MessageType::ConnectionReady, serde_json::json!({}))
                .with_sid("sess_peer_retry")
                .with_connid("conn_peer_retry"),
        )
        .await
        .expect("send duplicate ready");
    client
        .send(
            UctpEnvelope::new(
                MessageType::MessageSend,
                serde_json::json!({
                    "msg_id": "msg_after_failed_bind",
                    "from": participant,
                    "to": "all",
                    "content_type": "text/plain",
                    "label": "test",
                    "body": "barrier",
                    "body_encoding": "utf8",
                    "attachments": []
                }),
            )
            .with_sid("sess_peer_retry")
            .with_connid("conn_peer_retry"),
        )
        .await
        .expect("send rollback barrier");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.expect("adapter events closed"),
                AdapterEvent::DataMessage { ref connection_id, .. }
                    if connection_id == &retry_connection.id
            ) {
                break;
            }
        }
    })
    .await
    .expect("rollback barrier timeout");
    assert!(
        adapter
            .streams(retry_connection.id.clone())
            .await
            .expect("retry streams")
            .is_empty(),
        "failed batch left a partially committed stream"
    );

    let corrected = bind_wire_stream(
        &client,
        &mut inbound,
        "sess_peer_retry",
        "conn_peer_retry",
        "ignored-and-replaced-from-auth",
        "strm_shared_name",
    )
    .await;
    assert!(
        corrected.stream.stream_local_id
            > opened_a
                .stream
                .stream_local_id
                .max(opened_b.stream.stream_local_id),
        "rolled-back local IDs must never be reused"
    );
    assert_eq!(
        wait_for_stream(&adapter, &retry_connection.id)
            .await
            .id()
            .as_str(),
        "strm_shared_name"
    );
}

#[tokio::test]
async fn wire_subscribe_and_unsubscribe_control_webtransport_fanout() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_H3]).expect("dispatcher");
    let accept_rx = routes.take(ALPN_H3).expect("h3 channel");

    let orchestrator = Orchestrator::new(Config::default());
    let publishers = orchestrator.publisher_registry();
    let handler =
        OrchestratorSubscriptionHandler::new(Arc::clone(&orchestrator), Arc::clone(&publishers));
    let canonical_session = SessionId::from_string("sess_wt_wire_fanout");
    let resolver: Arc<dyn SessionBindingResolver> = Arc::new({
        let canonical_session = canonical_session.clone();
        move |_: &rvoip_core::identity::AuthenticatedPrincipal,
              _: &SessionId|
              -> Result<SessionId, ResourceBindingError> { Ok(canonical_session.clone()) }
    });
    let adapter = UctpWtAdapter::new(
        UctpWtConfig::new(Arc::clone(&server_ep), accept_rx, bearer_stub())
            .with_subscription_handler(handler)
            .with_session_binding_resolver(resolver)
            .with_orchestrator(Arc::clone(&orchestrator)),
    )
    .await
    .expect("adapter");
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");
    let mut events = orchestrator.subscribe_events();

    let publisher_endpoint = client_endpoint();
    let subscriber_endpoint = client_endpoint();
    let client_config =
        rvoip_uctp::substrate::dev_client_config_trusting(&cert_der).expect("client config");
    let url = Url::parse(&format!("https://localhost:{}/uctp", server_addr.port())).expect("url");
    let publisher = UctpWtClient::connect(
        &publisher_endpoint,
        server_addr,
        &url,
        Arc::new(client_config.clone()),
    )
    .await
    .expect("publisher connect");
    let subscriber = UctpWtClient::connect(
        &subscriber_endpoint,
        server_addr,
        &url,
        Arc::new(client_config),
    )
    .await
    .expect("subscriber connect");
    let mut publisher_inbound = publisher.take_inbound().expect("publisher inbound");
    let mut subscriber_inbound = subscriber.take_inbound().expect("subscriber inbound");
    let publisher_participant = authenticate(&publisher, &mut publisher_inbound).await;
    let subscriber_participant = authenticate(&subscriber, &mut subscriber_inbound).await;

    send_invite(&publisher, "sess_wt_pub", &publisher_participant).await;
    let publisher_connection = next_orchestrator_connection(&mut events).await;
    let publisher_opened = bind_wire_stream(
        &publisher,
        &mut publisher_inbound,
        "sess_wt_pub",
        "conn_wt_pub",
        &publisher_participant,
        "strm_wt_publisher",
    )
    .await;

    send_invite(&subscriber, "sess_wt_sub", &subscriber_participant).await;
    let subscriber_connection = next_orchestrator_connection(&mut events).await;
    let subscriber_opened = bind_wire_stream(
        &subscriber,
        &mut subscriber_inbound,
        "sess_wt_sub",
        "conn_wt_sub",
        &subscriber_participant,
        "strm_wt_subscriber",
    )
    .await;

    subscriber
        .send(stream_subscribe(
            "sess_wt_sub",
            "conn_wt_sub",
            &publisher_opened.stream.strm_id,
        ))
        .await
        .expect("wire subscribe");
    next_ack(&mut subscriber_inbound).await;
    let publisher_stream_id = StreamId::from_string(publisher_opened.stream.strm_id.clone());
    assert_eq!(
        orchestrator.subscribers_for(
            &canonical_session,
            &publisher_connection,
            &publisher_stream_id,
        ),
        vec![subscriber_connection.clone()],
        "wire ACK did not create the canonical subscription row"
    );

    let publisher_stream = WebTransportDatagramMediaStream::start(
        publisher_stream_id.clone(),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Outbound,
        publisher_opened.stream.stream_local_id,
        publisher.session.clone(),
    );
    send_stream_frame(&publisher_stream, b"fanout-prime", 0).await;
    let fanout_opened =
        next_stream_opened(&mut subscriber_inbound, "sess_wt_sub", "conn_wt_sub").await;
    assert_ne!(
        fanout_opened.stream.stream_local_id, subscriber_opened.stream.stream_local_id,
        "lazy fanout stream aliased the subscriber's negotiated publisher stream"
    );

    let subscriber_stream = WebTransportDatagramMediaStream::start(
        StreamId::from_string(fanout_opened.stream.strm_id),
        StreamKind::Audio,
        default_codec(),
        rvoip_core::connection::Direction::Inbound,
        fanout_opened.stream.stream_local_id,
        subscriber.session.clone(),
    );
    spawn_datagram_reader(
        subscriber.session.clone(),
        Arc::new(parking_lot::RwLock::new(vec![Arc::clone(
            &subscriber_stream,
        )])),
        None,
    );
    let mut subscriber_media = subscriber_stream
        .try_frames_in()
        .expect("subscriber media receiver");

    send_stream_frame(&publisher_stream, b"before-unsubscribe", 960).await;
    let delivered = recv_payload(&mut subscriber_media, b"before-unsubscribe").await;
    assert_eq!(delivered.stream_id, subscriber_stream.id());

    subscriber
        .send(stream_unsubscribe(
            "sess_wt_sub",
            "conn_wt_sub",
            &publisher_opened.stream.strm_id,
        ))
        .await
        .expect("wire unsubscribe");
    next_ack(&mut subscriber_inbound).await;
    assert!(
        orchestrator
            .subscribers_for(
                &canonical_session,
                &publisher_connection,
                &publisher_stream_id,
            )
            .is_empty(),
        "wire unsubscribe left a stale canonical subscription row"
    );

    // Ignore any pre-unsubscribe datagram that was already queued, then prove
    // three later publisher frames are not delivered after the ACK.
    while matches!(
        tokio::time::timeout(Duration::from_millis(50), subscriber_media.recv()).await,
        Ok(Some(_))
    ) {}
    for timestamp in [1_920, 2_880, 3_840] {
        send_stream_frame(&publisher_stream, b"after-unsubscribe", timestamp).await;
    }
    let post_unsubscribe_delivery = tokio::time::timeout(Duration::from_millis(750), async {
        while let Some(frame) = subscriber_media.recv().await {
            if frame.payload.as_ref() == b"after-unsubscribe" {
                return true;
            }
        }
        false
    })
    .await;
    assert!(
        !matches!(post_unsubscribe_delivery, Ok(true)),
        "publisher media still reached the subscriber after unsubscribe ACK"
    );
}
