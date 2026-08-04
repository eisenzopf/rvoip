use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{
    CloseFrame as AxumCloseFrame, Message as AxumWsMessage, WebSocket, WebSocketUpgrade,
};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use chrono::Utc;
use futures::StreamExt;
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_core::stream::{MediaFrame, StreamKind};
use rvoip_vapi::{
    VapiAdapter, VapiApiKey, VapiAssistant, VapiAudioFormat, VapiCallOptions, VapiConfig,
    VapiError, VapiEvent, VAPI_CALL_REFERENCE_KIND,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use url::Url;

#[derive(Clone)]
struct MockState {
    websocket_url: Arc<Mutex<String>>,
    create_count: Arc<AtomicUsize>,
    create_authorization: Arc<Mutex<Option<String>>>,
    websocket_authorization: Arc<Mutex<Option<String>>>,
    create_body: Arc<Mutex<Option<Value>>>,
    create_delay: Duration,
    socket_behavior: SocketBehavior,
    audio_chunks: Arc<Vec<Vec<u8>>>,
    observed: mpsc::UnboundedSender<Observed>,
}

#[derive(Clone, Copy)]
enum SocketBehavior {
    Interactive,
    DelayedEndAck,
    DelayedUpgrade,
    NormalClose,
    Silent,
    RejectUpgrade,
    HttpError,
}

#[derive(Debug)]
enum Observed {
    Binary(Vec<u8>, Instant),
    Json(Value),
}

async fn create_call(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state.create_count.fetch_add(1, Ordering::SeqCst);
    *state
        .create_authorization
        .lock()
        .expect("create authorization lock") = authorization(&headers);
    *state.create_body.lock().expect("create body lock") = Some(body);
    if matches!(state.socket_behavior, SocketBehavior::HttpError) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if !state.create_delay.is_zero() {
        tokio::time::sleep(state.create_delay).await;
    }
    let websocket_url = state
        .websocket_url
        .lock()
        .expect("websocket URL lock")
        .clone();
    Json(json!({
        "id": "call-mock-1",
        "status": "queued",
        "transport": {
            "websocketCallUrl": websocket_url
        }
    }))
    .into_response()
}

async fn websocket(
    State(state): State<MockState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    *state
        .websocket_authorization
        .lock()
        .expect("websocket authorization lock") = authorization(&headers);
    if matches!(state.socket_behavior, SocketBehavior::RejectUpgrade) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if matches!(state.socket_behavior, SocketBehavior::DelayedUpgrade) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    upgrade.on_upgrade(move |socket| mock_websocket(socket, state))
}

async fn mock_websocket(mut socket: WebSocket, state: MockState) {
    if matches!(state.socket_behavior, SocketBehavior::Silent) {
        std::future::pending::<()>().await;
        drop(socket);
        return;
    }
    if matches!(state.socket_behavior, SocketBehavior::NormalClose) {
        let _ = socket
            .send(AxumWsMessage::Close(Some(AxumCloseFrame {
                code: 1000,
                reason: Cow::Borrowed(""),
            })))
            .await;
        return;
    }

    for chunk in state.audio_chunks.iter() {
        let _ = socket.send(AxumWsMessage::Binary(chunk.clone())).await;
    }
    let _ = socket
        .send(AxumWsMessage::Text(
            r#"{"type":"future-event","opaque":"event-canary"}"#.into(),
        ))
        .await;
    let _ = socket.send(AxumWsMessage::Text("{bad".into())).await;

    while let Some(message) = socket.next().await {
        match message {
            Ok(AxumWsMessage::Binary(payload)) => {
                let _ = state
                    .observed
                    .send(Observed::Binary(payload, Instant::now()));
            }
            Ok(AxumWsMessage::Text(text)) => {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    let should_ack_end =
                        matches!(state.socket_behavior, SocketBehavior::DelayedEndAck)
                            && value["type"] == "end-call";
                    let _ = state.observed.send(Observed::Json(value));
                    if should_ack_end {
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        let _ = socket
                            .send(AxumWsMessage::Text(
                                r#"{"type":"status-update","status":"ended"}"#.into(),
                            ))
                            .await;
                        return;
                    }
                }
            }
            Ok(AxumWsMessage::Close(_)) | Err(_) => break,
            Ok(AxumWsMessage::Ping(_)) | Ok(AxumWsMessage::Pong(_)) => {}
        }
    }
}

fn authorization(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

async fn start_mock(
    create_delay: Duration,
) -> (
    Url,
    MockState,
    mpsc::UnboundedReceiver<Observed>,
    tokio::task::JoinHandle<()>,
) {
    start_mock_with_behavior(
        create_delay,
        SocketBehavior::Interactive,
        vec![vec![0x11; 79], vec![0x22; 241]],
    )
    .await
}

async fn start_mock_with_behavior(
    create_delay: Duration,
    socket_behavior: SocketBehavior,
    audio_chunks: Vec<Vec<u8>>,
) -> (
    Url,
    MockState,
    mpsc::UnboundedReceiver<Observed>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock Vapi");
    let address = listener.local_addr().expect("mock local address");
    let (observed, observed_rx) = mpsc::unbounded_channel();
    let state = MockState {
        websocket_url: Arc::new(Mutex::new(format!("ws://{address}/transport"))),
        create_count: Arc::new(AtomicUsize::new(0)),
        create_authorization: Arc::new(Mutex::new(None)),
        websocket_authorization: Arc::new(Mutex::new(None)),
        create_body: Arc::new(Mutex::new(None)),
        create_delay,
        socket_behavior,
        audio_chunks: Arc::new(audio_chunks),
        observed,
    };
    let app = Router::new()
        .route("/call", post(create_call))
        .route("/transport", get(websocket))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock Vapi");
    });
    (
        Url::parse(&format!("http://{address}/")).expect("mock API URL"),
        state,
        observed_rx,
        server,
    )
}

#[tokio::test]
async fn staged_adapter_bridges_binary_events_controls_and_shutdown() {
    let (api_base, state, mut observed, server) = start_mock(Duration::ZERO).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.heartbeat_interval = Duration::from_secs(60);
    config.max_message_bytes = 640;
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut adapter_events = adapter.subscribe_events();
    let mut global_events = adapter.subscribe_vapi_events();
    let options = VapiCallOptions::new(VapiAssistant::saved("assistant-mock"));
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(options);

    let handle = adapter.originate(request).await.expect("prepare route");
    let connection_id = handle.connection.id.clone();
    assert_eq!(state.create_count.load(Ordering::SeqCst), 0);

    let receipt = adapter
        .activate_outbound_with_receipt(connection_id.clone())
        .await
        .expect("activate route");
    assert_eq!(state.create_count.load(Ordering::SeqCst), 1);
    let reference = receipt
        .external_references()
        .first()
        .expect("Vapi call reference");
    assert_eq!(reference.kind(), VAPI_CALL_REFERENCE_KIND);
    assert_eq!(reference.expose_secret(), "call-mock-1");
    assert_eq!(
        state
            .create_authorization
            .lock()
            .expect("create authorization lock")
            .as_deref(),
        Some("Bearer mock-api-key")
    );
    assert_eq!(
        state
            .websocket_authorization
            .lock()
            .expect("websocket authorization lock")
            .as_deref(),
        Some("Bearer mock-api-key")
    );
    let body = state
        .create_body
        .lock()
        .expect("create body lock")
        .clone()
        .expect("create body");
    assert_eq!(body["assistantId"], "assistant-mock");
    assert_eq!(body["transport"]["provider"], "vapi.websocket");
    assert_eq!(body["transport"]["audioFormat"]["format"], "mulaw");
    assert!(body.get("phoneNumber").is_none());
    assert!(matches!(
        adapter_events.recv().await.expect("connected event"),
        AdapterEvent::Connected { .. }
    ));
    // The route retains one bounded receiver so events delivered during
    // activation remain available to the first post-activation subscriber.
    let mut call_events = adapter
        .subscribe_call_events(&connection_id)
        .expect("call events");

    let stream = adapter
        .streams(connection_id.clone())
        .await
        .expect("streams")
        .pop()
        .expect("audio stream");
    let mut incoming = stream.try_frames_in().expect("incoming receiver");
    let first = tokio::time::timeout(Duration::from_secs(1), incoming.recv())
        .await
        .expect("first frame timeout")
        .expect("first frame");
    let first_received_at = Instant::now();
    let second = tokio::time::timeout(Duration::from_secs(1), incoming.recv())
        .await
        .expect("second frame timeout")
        .expect("second frame");
    assert_eq!(first.payload.len(), 160);
    assert_eq!(second.payload.len(), 160);
    assert_eq!(first.timestamp_rtp, 0);
    assert_eq!(second.timestamp_rtp, 160);
    assert!(
        first_received_at.elapsed() >= Duration::from_millis(10),
        "coalesced inbound frames must be released at real-time cadence"
    );
    assert!(first.payload[..79].iter().all(|byte| *byte == 0x11));
    assert!(first.payload[79..].iter().all(|byte| *byte == 0x22));

    let outgoing = stream.try_frames_out().expect("outgoing sender");
    outgoing
        .send(MediaFrame {
            stream_id: stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0x0b; 160]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: Some(101),
        })
        .await
        .expect("queue DTMF frame");
    outgoing
        .send(MediaFrame {
            stream_id: stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0x33; 320]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: Some(0),
        })
        .await
        .expect("queue outgoing frames");
    adapter
        .say(&connection_id, "hello from test", false, true)
        .await
        .expect("say");
    adapter
        .mute_assistant(&connection_id)
        .await
        .expect("mute assistant");
    assert_eq!(
        adapter
            .say(&connection_id, "x".repeat(700), false, false)
            .await,
        Err(VapiError::ControlMessageTooLarge)
    );
    assert!(adapter.is_connection_live(&connection_id));

    let first_event = tokio::time::timeout(Duration::from_secs(1), call_events.recv())
        .await
        .expect("call event timeout")
        .expect("call event");
    let second_event = tokio::time::timeout(Duration::from_secs(1), call_events.recv())
        .await
        .expect("call event timeout")
        .expect("call event");
    assert!(
        matches!(first_event, VapiEvent::Unknown(_))
            || matches!(second_event, VapiEvent::Unknown(_))
    );
    assert!(
        matches!(first_event, VapiEvent::Malformed { .. })
            || matches!(second_event, VapiEvent::Malformed { .. })
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), global_events.recv())
            .await
            .expect("global event timeout")
            .is_ok()
    );

    let mut binary_frames = 0;
    let mut previous_binary_at = None;
    let mut saw_say = false;
    let mut saw_mute = false;
    while binary_frames < 2 || !saw_say || !saw_mute {
        let message = tokio::time::timeout(Duration::from_secs(2), observed.recv())
            .await
            .expect("mock observation timeout")
            .expect("mock observation channel");
        match message {
            Observed::Binary(payload, observed_at) => {
                assert_eq!(payload.len(), 160);
                assert!(payload.iter().all(|byte| *byte == 0x33));
                if let Some(previous) = previous_binary_at {
                    assert!(
                        observed_at.duration_since(previous) >= Duration::from_millis(10),
                        "coalesced outbound frames must be sent at real-time cadence"
                    );
                }
                previous_binary_at = Some(observed_at);
                binary_frames += 1;
            }
            Observed::Json(value) if value["type"] == "say" => {
                assert_eq!(value["content"], "hello from test");
                assert_eq!(value["interruptAssistantEnabled"], true);
                saw_say = true;
            }
            Observed::Json(value) if value["type"] == "control" => {
                assert_eq!(value["control"], "mute-assistant");
                saw_mute = true;
            }
            Observed::Json(_) => {}
        }
    }
    if let Ok(Some(Observed::Binary(_, _))) =
        tokio::time::timeout(Duration::from_millis(100), observed.recv()).await
    {
        panic!("RFC 4733 PT 101 must not be forwarded as Vapi audio");
    }

    adapter
        .end(connection_id.clone(), EndReason::Normal)
        .await
        .expect("end route");
    let mut saw_end_call = false;
    for _ in 0..4 {
        let Ok(Some(message)) = tokio::time::timeout(Duration::from_secs(1), observed.recv()).await
        else {
            break;
        };
        if matches!(message, Observed::Json(ref value) if value["type"] == "end-call") {
            saw_end_call = true;
            break;
        }
    }
    assert!(saw_end_call);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), adapter_events.recv())
            .await
            .expect("terminal event timeout")
            .expect("terminal event"),
        AdapterEvent::Ended { .. }
    ));
    server.abort();
}

#[tokio::test]
async fn pcm_audio_is_reframed_with_wideband_timestamps() {
    let (api_base, state, _observed, server) = start_mock_with_behavior(
        Duration::ZERO,
        SocketBehavior::Interactive,
        vec![vec![0x44; 319], vec![0x55; 961]],
    )
    .await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.heartbeat_interval = Duration::from_secs(60);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let _events = adapter.subscribe_events();
    let options = VapiCallOptions::new(VapiAssistant::saved("assistant-mock"))
        .with_audio_format(VapiAudioFormat::PcmS16Le16Khz);
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::PcmS16Le16Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(options);
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;
    adapter
        .activate_outbound_with_receipt(connection_id.clone())
        .await
        .expect("activate route");

    let body = state
        .create_body
        .lock()
        .expect("create body lock")
        .clone()
        .expect("create body");
    assert_eq!(body["transport"]["audioFormat"]["format"], "pcm_s16le");
    assert_eq!(body["transport"]["audioFormat"]["sampleRate"], 16_000);

    let stream = adapter
        .streams(connection_id.clone())
        .await
        .expect("streams")
        .pop()
        .expect("audio stream");
    let mut incoming = stream.try_frames_in().expect("incoming receiver");
    let first = tokio::time::timeout(Duration::from_secs(1), incoming.recv())
        .await
        .expect("first PCM frame timeout")
        .expect("first PCM frame");
    let second = tokio::time::timeout(Duration::from_secs(1), incoming.recv())
        .await
        .expect("second PCM frame timeout")
        .expect("second PCM frame");
    assert_eq!(first.payload.len(), 640);
    assert_eq!(second.payload.len(), 640);
    assert_eq!(first.timestamp_rtp, 0);
    assert_eq!(second.timestamp_rtp, 320);
    assert!(first.payload[..319].iter().all(|byte| *byte == 0x44));
    assert!(first.payload[319..].iter().all(|byte| *byte == 0x55));
    assert!(second.payload.iter().all(|byte| *byte == 0x55));

    adapter
        .end(connection_id, EndReason::Normal)
        .await
        .expect("end PCM route");
    server.abort();
}

#[tokio::test]
async fn normal_websocket_close_is_a_normal_remote_end() {
    let (api_base, _state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::NormalClose, Vec::new()).await;
    let config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;
    adapter
        .activate_outbound_with_receipt(connection_id)
        .await
        .expect("activate route");

    assert!(matches!(
        events.recv().await.expect("connected event"),
        AdapterEvent::Connected { .. }
    ));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("normal close terminal timeout")
            .expect("normal close terminal"),
        AdapterEvent::Ended {
            reason: EndReason::Normal,
            ..
        }
    ));
    server.abort();
}

#[tokio::test]
async fn local_shutdown_waits_for_terminal_ack_within_grace_period() {
    let (api_base, _state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::DelayedEndAck, Vec::new()).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.graceful_shutdown_timeout = Duration::from_millis(250);
    config.heartbeat_interval = Duration::from_secs(60);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;
    adapter
        .activate_outbound_with_receipt(connection_id.clone())
        .await
        .expect("activate route");
    assert!(matches!(
        events.recv().await.expect("connected event"),
        AdapterEvent::Connected { .. }
    ));

    let started = Instant::now();
    let (first_end, second_end) = tokio::join!(
        adapter.end(connection_id.clone(), EndReason::Normal),
        adapter.end(connection_id, EndReason::Normal),
    );
    first_end.expect("first end route");
    second_end.expect("idempotent concurrent end route");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(30),
        "shutdown returned before the delayed terminal acknowledgement"
    );
    assert!(
        elapsed < Duration::from_millis(250),
        "shutdown exceeded the configured grace period"
    );
    assert!(matches!(
        events.recv().await.expect("terminal event"),
        AdapterEvent::Ended {
            reason: EndReason::Normal,
            ..
        }
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err(),
        "concurrent shutdown paths must publish exactly one terminal event"
    );
    server.abort();
}

#[tokio::test]
async fn websocket_auth_rejection_fails_activation_without_connecting() {
    let (api_base, state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::RejectUpgrade, Vec::new()).await;
    let config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;

    assert!(adapter
        .activate_outbound_with_receipt(connection_id.clone())
        .await
        .is_err());
    assert_eq!(
        state
            .websocket_authorization
            .lock()
            .expect("websocket authorization lock")
            .as_deref(),
        Some("Bearer mock-api-key")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err(),
        "a rejected upgrade must not publish Connected"
    );
    adapter
        .end(connection_id, EndReason::Cancelled)
        .await
        .expect("remove rejected route");
    server.abort();
}

#[tokio::test]
async fn websocket_handshake_timeout_fails_activation_without_connecting() {
    let (api_base, _state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::DelayedUpgrade, Vec::new()).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.websocket_timeout = Duration::from_millis(20);
    config.graceful_shutdown_timeout = Duration::from_millis(20);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;

    assert!(adapter
        .activate_outbound_with_receipt(connection_id.clone())
        .await
        .is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err(),
        "a timed-out handshake must not publish Connected"
    );
    adapter
        .end(connection_id, EndReason::Cancelled)
        .await
        .expect("remove timed-out route");
    server.abort();
}

#[tokio::test]
async fn http_error_fails_activation_without_publishing_connected() {
    let (api_base, state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::HttpError, Vec::new()).await;
    let config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;

    assert!(adapter
        .activate_outbound_with_receipt(connection_id.clone())
        .await
        .is_err());
    assert_eq!(state.create_count.load(Ordering::SeqCst), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err()
    );
    adapter
        .end(connection_id, EndReason::Cancelled)
        .await
        .expect("remove failed route");
    server.abort();
}

#[tokio::test]
async fn oversized_websocket_message_is_terminal() {
    let (api_base, _state, _observed, server) = start_mock_with_behavior(
        Duration::ZERO,
        SocketBehavior::Interactive,
        vec![vec![0x55; 641]],
    )
    .await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.max_message_bytes = 640;
    config.heartbeat_interval = Duration::from_secs(60);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;
    adapter
        .activate_outbound_with_receipt(connection_id)
        .await
        .expect("activate route");

    assert!(matches!(
        events.recv().await.expect("connected event"),
        AdapterEvent::Connected { .. }
    ));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("oversized message terminal timeout")
            .expect("oversized message terminal"),
        AdapterEvent::Failed { .. }
    ));
    server.abort();
}

#[tokio::test]
async fn missing_heartbeat_response_is_terminal() {
    let (api_base, _state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::Silent, Vec::new()).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.heartbeat_interval = Duration::from_millis(20);
    config.websocket_io_timeout = Duration::from_millis(30);
    config.graceful_shutdown_timeout = Duration::from_millis(50);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;
    adapter
        .activate_outbound_with_receipt(connection_id)
        .await
        .expect("activate route");

    assert!(matches!(
        events.recv().await.expect("connected event"),
        AdapterEvent::Connected { .. }
    ));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("heartbeat terminal timeout")
            .expect("heartbeat terminal"),
        AdapterEvent::Failed { .. }
    ));
    server.abort();
}

#[tokio::test]
async fn inbound_audio_burst_degrades_instead_of_terminating_the_session() {
    // Vapi's WebSocket transport is a raw byte stream: its documentation
    // specifies `"container": "raw"` and the sample encoding, but no frame
    // size, no chunk size, and no pacing guarantee. Measured against a live
    // assistant, chunks are 170-743 bytes with a p50 inter-arrival of 50 ms
    // and a minimum of 0 ms, i.e. coalesced bursts.
    //
    // A burst that outruns the 20 ms-per-frame drain must cost audio, not the
    // call. Before this behaviour changed, a transient backlog terminated the
    // media session permanently and silently.
    let (api_base, _state, _observed, server) = start_mock(Duration::ZERO).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    // A jitter buffer far too small for the mock's burst.
    config.inbound_queue_capacity = 1;
    config.heartbeat_interval = Duration::from_secs(60);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;
    adapter
        .activate_outbound_with_receipt(connection_id)
        .await
        .expect("activate route");

    assert!(matches!(
        events.recv().await.expect("connected event"),
        AdapterEvent::Connected { .. }
    ));

    // The session must stay up despite the overflowing burst.
    if let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
        assert!(
            !matches!(event, AdapterEvent::Failed { .. }),
            "an inbound burst terminated the session; it should have dropped \
             frames and continued"
        );
    }
    server.abort();
}

#[tokio::test]
async fn never_activated_route_tears_down_without_publishing_terminal_event() {
    let (api_base, state, _observed, server) = start_mock(Duration::ZERO).await;
    let config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    let adapter = VapiAdapter::new(config).expect("adapter");
    let mut events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;

    adapter
        .end(connection_id, EndReason::Cancelled)
        .await
        .expect("end prepared route");
    assert_eq!(state.create_count.load(Ordering::SeqCst), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err(),
        "an uncommitted staged route must not publish a terminal event"
    );
    server.abort();
}

#[tokio::test]
async fn cancellation_during_post_is_reconciled_with_end_call() {
    let (api_base, state, mut observed, server) = start_mock(Duration::from_millis(150)).await;
    let config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    let adapter = VapiAdapter::new(config).expect("adapter");
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(VapiAssistant::saved("assistant-mock")));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare route")
        .connection
        .id;
    let activation_adapter = Arc::clone(&adapter);
    let activation_id = connection_id.clone();
    let activation = tokio::spawn(async move {
        activation_adapter
            .activate_outbound_with_receipt(activation_id)
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        while state.create_count.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("POST began");
    tokio::time::timeout(
        Duration::from_millis(500),
        adapter.end(connection_id, EndReason::Cancelled),
    )
    .await
    .expect("prepared teardown stayed bounded")
    .expect("prepared teardown");
    assert!(
        activation.await.expect("activation task").is_err(),
        "a cancelled activation must not publish a usable receipt"
    );

    let mut saw_end_call = false;
    for _ in 0..6 {
        let Ok(Some(message)) = tokio::time::timeout(Duration::from_secs(1), observed.recv()).await
        else {
            break;
        };
        if matches!(message, Observed::Json(ref value) if value["type"] == "end-call") {
            saw_end_call = true;
            break;
        }
    }
    assert!(
        saw_end_call,
        "the detached activation owner must reconcile a post-cancel remote call"
    );
    server.abort();
}
