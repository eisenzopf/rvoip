//! Gate 7 target-contacting RFC 9725 WHIP origination.

#![cfg(feature = "signaling-whip")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::header::{CONTENT_TYPE, ETAG, LOCATION};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, post};
use axum::Router;
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::stream::{MediaFrame, StreamKind};
use rvoip_webrtc::signaling::auth::BearerStaticTokenAuth;
use rvoip_webrtc::signaling::whip::serve_listener_with_auth;
use rvoip_webrtc::{
    StaticWebRtcBearerCredentialProvider, WebRtcAdapter, WebRtcBearerCredential, WebRtcConfig,
    WebRtcIceExchangePolicy, WebRtcOriginateContext, WebRtcSignalingMode, WebRtcTargetPolicy,
};

#[derive(Clone)]
struct StalledOrigin {
    started: Arc<tokio::sync::Notify>,
    requests: Arc<AtomicUsize>,
}

async fn stall_creation(State(state): State<StalledOrigin>) -> StatusCode {
    state.requests.fetch_add(1, Ordering::SeqCst);
    state.started.notify_one();
    std::future::pending().await
}

#[derive(Clone)]
struct MismatchedAnswerOrigin {
    adapter: Arc<WebRtcAdapter>,
    connection: Arc<tokio::sync::Mutex<Option<ConnectionId>>>,
    corruption: AnswerCorruption,
    resource_location: &'static str,
}

#[derive(Clone, Copy)]
enum AnswerCorruption {
    NoIceCandidates,
    DtlsFingerprint,
}

async fn create_mismatched_answer(
    State(state): State<MismatchedAnswerOrigin>,
    body: axum::body::Bytes,
) -> Response {
    let Ok(offer) = std::str::from_utf8(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let offer = match state.corruption {
        AnswerCorruption::NoIceCandidates => strip_ice_candidates(offer),
        AnswerCorruption::DtlsFingerprint => offer.to_owned(),
    };
    let Ok(connection) = state.adapter.apply_remote_offer(&offer).await else {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    };
    let Ok(answer) = state.adapter.local_sdp(&connection) else {
        let _ = state
            .adapter
            .end(
                connection,
                EndReason::Failed {
                    detail: "local answer unavailable".into(),
                },
            )
            .await;
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    *state.connection.lock().await = Some(connection);

    let answer = match state.corruption {
        AnswerCorruption::NoIceCandidates => strip_ice_candidates(&answer),
        AnswerCorruption::DtlsFingerprint => corrupt_first_dtls_fingerprint(&answer),
    };
    let mut response = (StatusCode::CREATED, answer).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/sdp"));
    response
        .headers_mut()
        .insert(LOCATION, HeaderValue::from_static(state.resource_location));
    response
        .headers_mut()
        .insert(ETAG, HeaderValue::from_static("\"mismatch-1\""));
    response
}

async fn delete_mismatched_answer_resource(
    State(state): State<MismatchedAnswerOrigin>,
) -> StatusCode {
    if let Some(connection) = state.connection.lock().await.take() {
        let _ = state.adapter.end(connection, EndReason::Normal).await;
    }
    StatusCode::NO_CONTENT
}

#[tokio::test]
async fn whip_creation_retains_resource_and_conditional_delete_tears_it_down() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // The WHIP answer carries fully gathered server candidates. The client
    // uses trickle and sends its candidates through retained resource PATCHes.
    let server_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WHIP loopback");
    let address = listener.local_addr().expect("listener address");
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        tokio::spawn(async move {
            serve_listener_with_auth(
                listener,
                adapter,
                Arc::new(BearerStaticTokenAuth::new("secret")),
            )
            .await
            .expect("serve WHIP loopback")
        })
    };
    let mut server_events = server_adapter.subscribe_events();

    let client_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let endpoint = format!("http://{address}/whip/loopback");
    let policy = WebRtcTargetPolicy::default()
        .allow_port(address.port())
        .allow_insecure(true)
        .allow_loopback(true)
        .with_timeouts(Duration::from_secs(3), Duration::from_secs(10))
        .expect("bounded timeouts");
    let provider = Arc::new(StaticWebRtcBearerCredentialProvider::new(
        WebRtcBearerCredential::new("secret").expect("bearer"),
    ));
    let context = WebRtcOriginateContext::new(
        &endpoint,
        WebRtcSignalingMode::Whip,
        WebRtcIceExchangePolicy::Trickle,
        policy,
        Some(provider),
    )
    .expect("validated WHIP context");
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        endpoint,
        Direction::Outbound,
        client_adapter.capabilities(),
    )
    .with_context(context);
    let handle = client_adapter
        .originate(request)
        .await
        .expect("prepare WHIP route");
    let client_connection = handle.connection.id.clone();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), server_events.recv())
            .await
            .is_err(),
        "preparation must not issue the WHIP POST"
    );

    client_adapter
        .activate_outbound(client_connection.clone())
        .await
        .expect("create WHIP resource and apply answer");
    let server_connection = wait_for_inbound_connection(&mut server_events).await;
    let (client_accept, server_accept) = tokio::join!(
        client_adapter.accept(client_connection.clone()),
        server_adapter.accept(server_connection.clone()),
    );
    client_accept.expect("client ICE/DTLS connected");
    server_accept.expect("server ICE/DTLS connected");
    exchange_bidirectional_audio(
        &client_adapter,
        &client_connection,
        &server_adapter,
        &server_connection,
    )
    .await;

    client_adapter
        .end(client_connection.clone(), EndReason::Normal)
        .await
        .expect("conditional resource delete");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if !server_adapter.is_connection_live(&server_connection) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("WHIP DELETE cleanup");
    assert!(!client_adapter.is_connection_live(&client_connection));

    server_task.abort();
}

#[tokio::test]
async fn candidate_less_ice_has_a_bounded_failure_and_releases_both_routes() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let origin = MismatchedAnswerOrigin {
        adapter: Arc::clone(&server_adapter),
        connection: Arc::new(tokio::sync::Mutex::new(None)),
        corruption: AnswerCorruption::NoIceCandidates,
        resource_location: "/whip/ice-mismatch-resource",
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ICE-failure WHIP origin");
    let address = listener.local_addr().expect("ICE-failure origin address");
    let app = Router::new()
        .route("/whip/ice-failure", post(create_mismatched_answer))
        .route(
            "/whip/ice-mismatch-resource",
            delete(delete_mismatched_answer_resource),
        )
        .with_state(origin.clone());
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve ICE-failure WHIP origin")
    });

    let mut client_config = WebRtcConfig::loopback();
    client_config.connection_timeout_secs = 1;
    let client_adapter = WebRtcAdapter::new(client_config);
    let endpoint = format!("http://{address}/whip/ice-failure");
    let context = WebRtcOriginateContext::new(
        &endpoint,
        WebRtcSignalingMode::Whip,
        WebRtcIceExchangePolicy::FullGather,
        WebRtcTargetPolicy::default()
            .allow_port(address.port())
            .allow_insecure(true)
            .allow_loopback(true)
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(10))
            .expect("bounded ICE-failure target policy"),
        None,
    )
    .expect("ICE-failure WHIP context");
    let connection = client_adapter
        .originate(
            OriginateRequest::new(
                SessionId::new(),
                ParticipantId::new(),
                endpoint,
                Direction::Outbound,
                client_adapter.capabilities(),
            )
            .with_context(context),
        )
        .await
        .expect("prepare candidate-less ICE route")
        .connection
        .id;

    client_adapter
        .activate_outbound(connection.clone())
        .await
        .expect("WHIP signaling succeeds before ICE is evaluated");
    let server_connection = wait_for_origin_connection(&origin).await;
    let failure = tokio::time::timeout(
        Duration::from_secs(3),
        client_adapter.accept(connection.clone()),
    )
    .await
    .expect("candidate-less ICE failure exceeded the configured bound")
    .expect_err("candidate-less ICE unexpectedly connected");
    assert!(
        !failure.to_string().is_empty(),
        "ICE failure must remain diagnosable"
    );

    client_adapter
        .end(connection.clone(), EndReason::Timeout)
        .await
        .expect("ICE-failure route cleanup");
    wait_for_route_release(&server_adapter, &server_connection).await;
    assert!(!client_adapter.is_connection_live(&connection));
    assert_eq!(client_adapter.outbound_signaling_task_count(), 0);
    assert!(client_adapter.routes().is_empty());
    assert!(server_adapter.routes().is_empty());

    server_task.abort();
    let _ = server_task.await;
}

#[tokio::test]
async fn mismatched_answer_fingerprint_has_a_bounded_dtls_failure_and_releases_both_routes() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let origin = MismatchedAnswerOrigin {
        adapter: Arc::clone(&server_adapter),
        connection: Arc::new(tokio::sync::Mutex::new(None)),
        corruption: AnswerCorruption::DtlsFingerprint,
        resource_location: "/whip/mismatch-resource",
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind DTLS-failure WHIP origin");
    let address = listener.local_addr().expect("DTLS-failure origin address");
    let app = Router::new()
        .route("/whip/dtls-failure", post(create_mismatched_answer))
        .route(
            "/whip/mismatch-resource",
            delete(delete_mismatched_answer_resource),
        )
        .with_state(origin.clone());
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve DTLS-failure WHIP origin")
    });

    let mut client_config = WebRtcConfig::loopback();
    client_config.connection_timeout_secs = 1;
    let client_adapter = WebRtcAdapter::new(client_config);
    let endpoint = format!("http://{address}/whip/dtls-failure");
    let context = WebRtcOriginateContext::new(
        &endpoint,
        WebRtcSignalingMode::Whip,
        WebRtcIceExchangePolicy::FullGather,
        WebRtcTargetPolicy::default()
            .allow_port(address.port())
            .allow_insecure(true)
            .allow_loopback(true)
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(10))
            .expect("bounded DTLS-failure target policy"),
        None,
    )
    .expect("DTLS-failure WHIP context");
    let connection = client_adapter
        .originate(
            OriginateRequest::new(
                SessionId::new(),
                ParticipantId::new(),
                endpoint,
                Direction::Outbound,
                client_adapter.capabilities(),
            )
            .with_context(context),
        )
        .await
        .expect("prepare DTLS-failure route")
        .connection
        .id;

    client_adapter
        .activate_outbound(connection.clone())
        .await
        .expect("the syntactically valid mismatched answer is applied");
    let server_connection = wait_for_origin_connection(&origin).await;
    let failure = tokio::time::timeout(
        Duration::from_secs(3),
        client_adapter.accept(connection.clone()),
    )
    .await
    .expect("DTLS fingerprint failure exceeded the configured bound")
    .expect_err("mismatched certificate fingerprint unexpectedly connected");
    assert!(
        !failure.to_string().is_empty(),
        "DTLS failure must remain diagnosable"
    );

    client_adapter
        .end(connection.clone(), EndReason::Timeout)
        .await
        .expect("DTLS-failure route cleanup and conditional DELETE");
    wait_for_route_release(&server_adapter, &server_connection).await;
    assert!(!client_adapter.is_connection_live(&connection));
    assert_eq!(client_adapter.outbound_signaling_task_count(), 0);
    assert!(client_adapter.routes().is_empty());
    assert!(server_adapter.routes().is_empty());

    server_task.abort();
    let _ = server_task.await;
}

#[tokio::test]
async fn stalled_creation_is_aborted_and_joined_at_the_route_shutdown_deadline() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let origin = StalledOrigin {
        started: Arc::new(tokio::sync::Notify::new()),
        requests: Arc::new(AtomicUsize::new(0)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled WHIP origin");
    let address = listener.local_addr().expect("origin address");
    let app = Router::new()
        .route("/whip/stalled", post(stall_creation))
        .with_state(origin.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve stalled origin")
    });

    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let endpoint = format!("http://{address}/whip/stalled");
    let context = WebRtcOriginateContext::new(
        &endpoint,
        WebRtcSignalingMode::Whip,
        WebRtcIceExchangePolicy::Trickle,
        WebRtcTargetPolicy::default()
            .allow_port(address.port())
            .allow_insecure(true)
            .allow_loopback(true)
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(20))
            .expect("bounded timeouts"),
        None,
    )
    .expect("validated WHIP context");
    let connection = adapter
        .originate(
            OriginateRequest::new(
                SessionId::new(),
                ParticipantId::new(),
                endpoint,
                Direction::Outbound,
                adapter.capabilities(),
            )
            .with_context(context),
        )
        .await
        .expect("prepare stalled route")
        .connection
        .id;

    let activation = {
        let adapter = Arc::clone(&adapter);
        let connection = connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    tokio::time::timeout(Duration::from_secs(5), origin.started.notified())
        .await
        .expect("origin received the one-shot POST");
    assert_eq!(origin.requests.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.outbound_signaling_task_count(), 1);

    tokio::time::timeout(
        Duration::from_secs(5),
        adapter.end(connection.clone(), EndReason::Normal),
    )
    .await
    .expect("route shutdown exceeded its abort deadline")
    .expect("route shutdown");
    let activation_result = tokio::time::timeout(Duration::from_secs(1), activation)
        .await
        .expect("activation waiter remained blocked after forced shutdown")
        .expect("activation task");
    assert!(activation_result.is_err());
    assert_eq!(adapter.outbound_signaling_task_count(), 0);
    assert!(!adapter.is_connection_live(&connection));

    server.abort();
}

async fn exchange_bidirectional_audio(
    first_adapter: &Arc<WebRtcAdapter>,
    first_connection: &ConnectionId,
    second_adapter: &Arc<WebRtcAdapter>,
    second_connection: &ConnectionId,
) {
    let first = first_adapter
        .streams(first_connection.clone())
        .await
        .expect("first media streams")
        .into_iter()
        .find(|stream| stream.kind() == StreamKind::Audio)
        .expect("first audio stream");
    let second = second_adapter
        .streams(second_connection.clone())
        .await
        .expect("second media streams")
        .into_iter()
        .find(|stream| stream.kind() == StreamKind::Audio)
        .expect("second audio stream");
    assert_eq!(first.codec().name.to_ascii_lowercase(), "opus");
    assert_eq!(second.codec().name.to_ascii_lowercase(), "opus");

    let mut first_inbound = first.try_frames_in().expect("first inbound receiver");
    let mut second_inbound = second.try_frames_in().expect("second inbound receiver");
    send_audio_burst(&first, 10).await;
    let second_frame = tokio::time::timeout(Duration::from_secs(5), second_inbound.recv())
        .await
        .expect("first-to-second media timed out")
        .expect("first-to-second media closed");
    assert_eq!(
        second_frame.payload,
        rvoip_webrtc::media::silent_opus_payload()
    );

    send_audio_burst(&second, 30).await;
    let first_frame = tokio::time::timeout(Duration::from_secs(5), first_inbound.recv())
        .await
        .expect("second-to-first media timed out")
        .expect("second-to-first media closed");
    assert_eq!(
        first_frame.payload,
        rvoip_webrtc::media::silent_opus_payload()
    );
}

async fn send_audio_burst(stream: &Arc<dyn rvoip_core::stream::MediaStream>, sequence_base: u32) {
    let output = stream.frames_out();
    for sequence in 0..10u32 {
        output
            .send(MediaFrame {
                stream_id: stream.id(),
                kind: StreamKind::Audio,
                payload: rvoip_webrtc::media::silent_opus_payload(),
                timestamp_rtp: (sequence_base + sequence) * 960,
                captured_at: chrono::Utc::now(),
                payload_type: None,
            })
            .await
            .expect("send loopback audio");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_inbound_connection(
    events: &mut tokio::sync::mpsc::Receiver<AdapterEvent>,
) -> ConnectionId {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::InboundConnection { connection }) => return connection.id,
                Some(_) => {}
                None => panic!("server event stream closed"),
            }
        }
    })
    .await
    .expect("inbound connection event")
}

async fn wait_for_route_release(adapter: &WebRtcAdapter, connection: &ConnectionId) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while adapter.is_connection_live(connection) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("remote WHIP resource cleanup");
}

async fn wait_for_origin_connection(origin: &MismatchedAnswerOrigin) -> ConnectionId {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(connection) = origin.connection.lock().await.clone() {
                return connection;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("mismatched-answer origin route publication")
}

fn strip_ice_candidates(sdp: &str) -> String {
    let mut stripped = String::with_capacity(sdp.len());
    for line in sdp.split_inclusive('\n') {
        let attribute = line.trim_end_matches(['\r', '\n']);
        if attribute.starts_with("a=candidate:") || attribute == "a=end-of-candidates" {
            continue;
        }
        stripped.push_str(line);
    }
    stripped
}

fn corrupt_first_dtls_fingerprint(sdp: &str) -> String {
    const BOGUS_SHA256: &str = "00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:\
         00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00";

    let start = sdp
        .find("a=fingerprint:")
        .expect("answer contains a DTLS fingerprint");
    let end = start
        + sdp[start..]
            .find("\r\n")
            .unwrap_or_else(|| sdp.len() - start);
    let algorithm = sdp[start..end]
        .split_ascii_whitespace()
        .next()
        .expect("fingerprint algorithm");
    replace_first_sdp_attribute(
        sdp,
        "a=fingerprint:",
        &format!("{algorithm} {BOGUS_SHA256}"),
    )
}

fn replace_first_sdp_attribute(sdp: &str, marker: &str, replacement: &str) -> String {
    let start = sdp
        .find(marker)
        .unwrap_or_else(|| panic!("answer contains {marker}"));
    let end = start
        + sdp[start..]
            .find("\r\n")
            .unwrap_or_else(|| sdp.len() - start);
    let mut replaced = sdp.to_owned();
    replaced.replace_range(start..end, replacement);
    replaced
}
