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
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config as CoreConfig;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId, TenantId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::participant::{ParticipantKind, ParticipantRole};
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::{
    MediaFrame, MediaReceiverReservation, MediaStream, QualitySnapshot, StreamKind,
};
use rvoip_vapi::{
    VapiAdapter, VapiAgentOutcome, VapiApiKey, VapiAssistant, VapiAudioFormat, VapiCallOptions,
    VapiConfig, VapiError, VapiEvent, VapiExistingCall, VAPI_CALL_REFERENCE_KIND,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::{broadcast, mpsc};
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
    injected: broadcast::Sender<Injected>,
}

#[derive(Clone, Copy)]
enum SocketBehavior {
    Interactive,
    DelayedEndAck,
    DelayedUpgrade,
    NormalClose,
    /// A close frame carrying no body. Explicitly legal per RFC 6455 §5.5.1.
    BodylessClose,
    Silent,
    RejectUpgrade,
    HttpError,
}

#[derive(Debug)]
enum Observed {
    Binary(Vec<u8>, Instant),
    Json(Value),
}

#[derive(Clone, Debug)]
enum Injected {
    Binary(Vec<u8>),
    Text(String),
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
    if matches!(state.socket_behavior, SocketBehavior::BodylessClose) {
        let _ = socket.send(AxumWsMessage::Close(None)).await;
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

    let mut injected = state.injected.subscribe();
    loop {
        let message = tokio::select! {
            message = socket.next() => {
                let Some(message) = message else { break };
                message
            }
            injected = injected.recv() => {
                let message = match injected {
                    Ok(Injected::Binary(payload)) => AxumWsMessage::Binary(payload),
                    Ok(Injected::Text(text)) => AxumWsMessage::Text(text.into()),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if socket.send(message).await.is_err() {
                    break;
                }
                continue;
            }
        };
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
    let (injected, _) = broadcast::channel(32);
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
        injected,
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

struct CallerAdapter {
    events: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    /// One media stream per admitted caller Connection. A shared adapter can
    /// carry several callers at once, and each bridge acquires its own
    /// single-consumer receiver.
    streams: Mutex<HashMap<ConnectionId, Arc<CallerStream>>>,
    /// Opt-in. When set, every stream this adapter hands out exposes a
    /// generation-fenced peer queue (`try_peer_frames_out`), which makes any
    /// bridge over it transport-fenced. Off by default so the other tests keep
    /// exercising the legacy sink path.
    transport_fenced: bool,
}

struct CallerStream {
    id: StreamId,
    _inbound_tx: mpsc::Sender<MediaFrame>,
    inbound_rx: Arc<Mutex<Option<mpsc::Receiver<MediaFrame>>>>,
    outbound_tx: mpsc::Sender<MediaFrame>,
    _outbound_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    transport_fenced: bool,
}

#[async_trait::async_trait]
impl MediaStream for CallerStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }

    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }

    fn codec(&self) -> CodecInfo {
        CodecInfo {
            name: "PCMU".into(),
            clock_rate_hz: 8_000,
            channels: 1,
            fmtp: None,
            payload_type: Some(0),
        }
    }

    fn direction(&self) -> Direction {
        Direction::Inbound
    }

    #[allow(deprecated)]
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.inbound_rx
            .lock()
            .expect("caller inbound lock")
            .take()
            .expect("caller media receiver acquired once")
    }

    fn try_frames_in(&self) -> RvoipResult<mpsc::Receiver<MediaFrame>> {
        Ok(self.reserve_frames_in()?.commit())
    }

    fn reserve_frames_in(&self) -> RvoipResult<MediaReceiverReservation> {
        let receiver = self
            .inbound_rx
            .lock()
            .expect("caller inbound lock")
            .take()
            .ok_or(RvoipError::InvalidState(
                "caller media receiver already acquired",
            ))?;
        let slot = Arc::clone(&self.inbound_rx);
        Ok(MediaReceiverReservation::new(receiver, move |receiver| {
            let mut slot = slot.lock().expect("caller inbound restore lock");
            if slot.is_none() {
                *slot = Some(receiver);
            }
        }))
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.outbound_tx.clone()
    }

    /// A legacy fixture (the default) reports the trait's `NotImplemented`,
    /// so core treats it as an unfenced queue. A transport-fenced fixture
    /// honours the contract: each entry is re-checked after dequeue and its
    /// delivery guard is held through the local send.
    fn try_peer_frames_out(
        &self,
    ) -> RvoipResult<mpsc::Sender<rvoip_core::peer_media::PeerMediaFrame>> {
        if !self.transport_fenced {
            return Err(RvoipError::NotImplemented(
                "caller fixture is a legacy media queue",
            ));
        }
        let (tx, mut rx) = mpsc::channel::<rvoip_core::peer_media::PeerMediaFrame>(1);
        let outgoing = self.outbound_tx.clone();
        tokio::spawn(async move {
            while let Some(entry) = rx.recv().await {
                if let Some((frame, guard)) = entry.into_delivery() {
                    let _ = outgoing.send(frame).await;
                    drop(guard);
                }
            }
        });
        Ok(tx)
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        Ok(())
    }
}

impl CallerAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        Self::with_transport_fence(false)
    }

    fn new_transport_fenced() -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        Self::with_transport_fence(true)
    }

    fn with_transport_fence(transport_fenced: bool) -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        let (event_tx, event_rx) = mpsc::channel(16);
        (
            Arc::new(Self {
                events: Mutex::new(Some(event_rx)),
                streams: Mutex::new(HashMap::new()),
                transport_fenced,
            }),
            event_tx,
        )
    }
}

impl CallerStream {
    fn new(transport_fenced: bool) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(32);
        // One frame can enter the caller transport; later frames must remain
        // in the graph sink queue until the test deliberately flushes it.
        let (outbound_tx, outbound_rx) = mpsc::channel(1);
        Self {
            id: StreamId::new(),
            _inbound_tx: inbound_tx,
            inbound_rx: Arc::new(Mutex::new(Some(inbound_rx))),
            outbound_tx,
            _outbound_rx: Mutex::new(Some(outbound_rx)),
            transport_fenced,
        }
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for CallerAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    async fn originate(&self, _: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        Err(RvoipError::NotImplemented("test caller origination"))
    }

    async fn accept(&self, _: ConnectionId) -> RvoipResult<()> {
        Ok(())
    }

    async fn reject(&self, _: ConnectionId, _: RejectReason) -> RvoipResult<()> {
        Ok(())
    }

    async fn end(&self, _: ConnectionId, _: EndReason) -> RvoipResult<()> {
        Ok(())
    }

    async fn hold(&self, _: ConnectionId) -> RvoipResult<()> {
        Ok(())
    }

    async fn resume(&self, _: ConnectionId) -> RvoipResult<()> {
        Ok(())
    }

    async fn transfer(&self, _: ConnectionId, _: TransferTarget) -> RvoipResult<()> {
        Ok(())
    }

    async fn streams(&self, connection: ConnectionId) -> RvoipResult<Vec<Arc<dyn MediaStream>>> {
        let mut streams = self.streams.lock().expect("caller streams lock");
        let stream = streams
            .entry(connection)
            .or_insert_with(|| Arc::new(CallerStream::new(self.transport_fenced)));
        Ok(vec![Arc::clone(stream) as Arc<dyn MediaStream>])
    }

    async fn send_message(&self, _: ConnectionId, _: Message) -> RvoipResult<()> {
        Ok(())
    }

    async fn send_dtmf(&self, _: ConnectionId, _: &str, _: u32) -> RvoipResult<()> {
        Ok(())
    }

    async fn renegotiate_media(
        &self,
        _: ConnectionId,
        _: CapabilityDescriptor,
    ) -> RvoipResult<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.events
            .lock()
            .expect("caller event lock")
            .take()
            .expect("caller events subscribed once")
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }

    async fn verify_request_signature(
        &self,
        _: ConnectionId,
        _: SignatureHeaders,
    ) -> RvoipResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

async fn setup_caller() -> (Arc<Orchestrator>, ConnectionId) {
    let (orchestrator, connection_id, _) = setup_caller_with_events().await;
    (orchestrator, connection_id)
}

/// Like [`setup_caller`], but also returns the caller adapter's event sender so
/// a test can admit further callers or end the caller leg from the outside.
async fn setup_caller_with_events() -> (Arc<Orchestrator>, ConnectionId, mpsc::Sender<AdapterEvent>)
{
    setup_caller_with_events_on(CallerAdapter::new()).await
}

/// [`setup_caller_with_events`] over a caller adapter whose streams expose a
/// transport delivery fence, for tests that need a fenced caller bridge.
async fn setup_fenced_caller_with_events(
) -> (Arc<Orchestrator>, ConnectionId, mpsc::Sender<AdapterEvent>) {
    setup_caller_with_events_on(CallerAdapter::new_transport_fenced()).await
}

async fn setup_caller_with_events_on(
    (adapter, events): (Arc<CallerAdapter>, mpsc::Sender<AdapterEvent>),
) -> (Arc<Orchestrator>, ConnectionId, mpsc::Sender<AdapterEvent>) {
    let orchestrator = Orchestrator::new(CoreConfig::default());
    orchestrator
        .register(adapter)
        .expect("register caller adapter");
    let connection_id = admit_caller_on(&orchestrator, &events).await;
    (orchestrator, connection_id, events)
}

/// Open a fresh Conversation and Session on `orchestrator`, publish one inbound
/// caller Connection through the caller adapter, and accept it.
async fn admit_caller_on(
    orchestrator: &Arc<Orchestrator>,
    events: &mpsc::Sender<AdapterEvent>,
) -> ConnectionId {
    let conversation_id = orchestrator
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open caller conversation");
    let session_id = orchestrator
        .start_session(conversation_id, SessionMedium::Voice, vec![])
        .await
        .expect("start caller session");
    admit_caller_into_session(orchestrator, events, session_id).await
}

/// Publish one inbound caller Connection into an existing Session through the
/// caller adapter and accept it there.
async fn admit_caller_into_session(
    orchestrator: &Arc<Orchestrator>,
    events: &mpsc::Sender<AdapterEvent>,
    session_id: SessionId,
) -> ConnectionId {
    let connection_id = ConnectionId::new();
    events
        .send(AdapterEvent::InboundConnection {
            connection: Connection {
                id: connection_id.clone(),
                session_id: session_id.clone(),
                participant_id: ParticipantId::new(),
                transport: Transport::Sip,
                direction: Direction::Inbound,
                state: ConnectionState::Connecting,
                capabilities: VapiAudioFormat::MuLaw8Khz.capabilities(),
                negotiated_codecs: NegotiatedCodecs::default(),
                streams: vec![],
                messaging_enabled: false,
                transport_handle: TransportHandle(Arc::new(())),
                opened_at: Utc::now(),
                closed_at: None,
            },
        })
        .await
        .expect("publish inbound caller");
    tokio::time::sleep(Duration::from_millis(30)).await;
    orchestrator
        .route_inbound_connection(
            connection_id.clone(),
            InboundAction::Accept {
                session_id,
                participant_id: ParticipantId::new(),
            },
        )
        .await
        .expect("accept caller");
    connection_id
}

/// An externally created provider call is attached through the canonical
/// staged connection/bridge lifecycle without a second `POST /call`: the
/// receipt carries the handed-off call ID, the adapter's default key
/// authenticates the WebSocket, media is graphed, and the caller hanging up
/// tears the pair down as `CallerEnded` with the create counter still at zero.
#[tokio::test]
async fn existing_call_bridges_and_ends_without_creating_a_provider_call() {
    let (api_base, state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::Interactive, Vec::new()).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.heartbeat_interval = Duration::from_secs(60);
    config.graceful_shutdown_timeout = Duration::from_millis(100);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let (orchestrator, caller, caller_events) = setup_caller_with_events().await;
    let websocket_url = Url::parse(&state.websocket_url.lock().expect("websocket URL lock"))
        .expect("mock websocket URL");
    let existing = VapiExistingCall::new(
        VapiCallOptions::new(VapiAssistant::saved("assistant-mock"))
            .with_audio_format(VapiAudioFormat::PcmS16Le16Khz),
        "already-created-call".into(),
        websocket_url,
    )
    .expect("existing call");
    assert_eq!(format!("{existing:?}"), "VapiExistingCall([redacted])");

    let mut call = adapter
        .attach_existing_agent(&orchestrator, caller.clone(), existing)
        .await
        .expect("attach existing Vapi call");
    assert_eq!(state.create_count.load(Ordering::SeqCst), 0);

    // Re-requesting the receipt after attachment returns the committed
    // activation rather than starting another one.
    let receipt = adapter
        .activate_outbound_with_receipt(call.vapi_connection_id().clone())
        .await
        .expect("activation receipt");
    let reference = receipt
        .external_references()
        .first()
        .expect("Vapi call reference");
    assert_eq!(reference.kind(), VAPI_CALL_REFERENCE_KIND);
    assert_eq!(reference.expose_secret(), "already-created-call");
    assert_eq!(
        state
            .websocket_authorization
            .lock()
            .expect("websocket authorization lock")
            .as_deref(),
        Some("Bearer mock-api-key")
    );
    assert!(orchestrator
        .media_graph_snapshot(call.vapi_connection_id())
        .await
        .is_some());

    caller_events
        .send(AdapterEvent::Ended {
            connection_id: caller,
            reason: EndReason::Normal,
        })
        .await
        .expect("publish caller hangup");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), call.wait())
            .await
            .expect("paired teardown timeout"),
        VapiAgentOutcome::CallerEnded
    );
    assert!(!adapter.is_connection_live(call.vapi_connection_id()));
    assert_eq!(state.create_count.load(Ordering::SeqCst), 0);
    server.abort();
}

/// One credential-free transport adapter (`existing_calls_only`) serves two
/// concurrent handoffs to two different providers. Each WebSocket is
/// authenticated with exactly its own call's key, neither key is retained as
/// an adapter default, neither provider ever sees a create request, and a
/// plain `attach_agent` on the shared adapter is refused before any HTTP.
#[tokio::test]
async fn shared_existing_transport_uses_each_call_key_and_cannot_create() {
    let (api_base, first, _first_observed, first_server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::Interactive, Vec::new()).await;
    let (_, second, _second_observed, second_server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::Interactive, Vec::new()).await;
    let mut config = VapiConfig::existing_calls_only()
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.heartbeat_interval = Duration::from_secs(60);
    assert!(config.api_key.is_none());
    let adapter = VapiAdapter::new(config).expect("adapter");
    let (orchestrator, caller, caller_events) = setup_caller_with_events().await;

    let mut calls = Vec::new();
    for (state, key, call_id) in [
        (&first, "first-tenant-key", "first-call"),
        (&second, "second-tenant-key", "second-call"),
    ] {
        let websocket_url = Url::parse(&state.websocket_url.lock().expect("websocket URL lock"))
            .expect("mock websocket URL");
        let existing = VapiExistingCall::new(
            VapiCallOptions::new(VapiAssistant::saved("assistant-mock"))
                .with_audio_format(VapiAudioFormat::PcmS16Le16Khz),
            call_id.into(),
            websocket_url,
        )
        .expect("existing call")
        .with_api_key(VapiApiKey::new(key).expect("tenant key"));
        assert!(
            !format!("{existing:?}").contains(key),
            "a call-local credential must be redacted from Debug output"
        );
        let tenant_caller = admit_caller_on(&orchestrator, &caller_events).await;
        calls.push(
            adapter
                .attach_existing_agent(&orchestrator, tenant_caller, existing)
                .await
                .expect("attach tenant call"),
        );
        assert_eq!(
            state
                .websocket_authorization
                .lock()
                .expect("websocket authorization lock")
                .as_deref(),
            Some(format!("Bearer {key}").as_str()),
            "each handoff authenticates with exactly its own key"
        );
        assert_eq!(state.create_count.load(Ordering::SeqCst), 0);
    }
    // The second provider never saw the first tenant's key, and vice versa.
    assert_eq!(
        first
            .websocket_authorization
            .lock()
            .expect("websocket authorization lock")
            .as_deref(),
        Some("Bearer first-tenant-key")
    );
    assert_eq!(
        second
            .websocket_authorization
            .lock()
            .expect("websocket authorization lock")
            .as_deref(),
        Some("Bearer second-tenant-key")
    );
    for call in &calls {
        assert!(adapter.is_connection_live(call.vapi_connection_id()));
    }

    // A caller cannot turn the shared media adapter into a provider creator,
    // even after it has handled credentialed calls.
    assert!(adapter
        .attach_agent(
            &orchestrator,
            caller,
            VapiCallOptions::new(VapiAssistant::saved("assistant-mock"))
        )
        .await
        .is_err());
    assert_eq!(first.create_count.load(Ordering::SeqCst), 0);
    assert_eq!(second.create_count.load(Ordering::SeqCst), 0);
    assert!(
        first.create_authorization.lock().expect("lock").is_none()
            && second.create_authorization.lock().expect("lock").is_none(),
        "the refused create request must fail before any HTTP"
    );

    for call in calls {
        let _ = call.end().await;
    }
    first_server.abort();
    second_server.abort();
}

/// Production-shaped strict peer handoff: the caller's Vapi bridge is
/// transport-fenced on both legs, a human target in the same Session is staged
/// with `prepare_transport_fenced_peer_handoff` and committed with
/// `commit_peer_handoff_with_timeout_and_receipt`. The Vapi supervisor observes
/// `PeerHandoffCommitted`, retires only the detached AI leg (`HandedOff` from
/// `wait_shared`, `end-call` on the wire, core `ConnectionEnded`, socket no
/// longer live) and leaves the retained caller and the new peer in their
/// Session. No provider call is created at any point.
#[tokio::test]
async fn committed_peer_handoff_retires_ai_without_ending_retained_caller() {
    let (api_base, state, mut observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::Interactive, Vec::new()).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.heartbeat_interval = Duration::from_secs(60);
    config.graceful_shutdown_timeout = Duration::from_millis(100);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let (orchestrator, caller, caller_events) = setup_fenced_caller_with_events().await;
    let session_id = orchestrator
        .session_of(&caller)
        .expect("retained caller session");
    let target = admit_caller_into_session(&orchestrator, &caller_events, session_id.clone()).await;
    let websocket_url = Url::parse(&state.websocket_url.lock().expect("websocket URL lock"))
        .expect("mock websocket URL");
    let existing = VapiExistingCall::new(
        VapiCallOptions::new(VapiAssistant::saved("assistant-mock")),
        "handoff-call".into(),
        websocket_url,
    )
    .expect("existing call");
    let call = adapter
        .attach_existing_agent(&orchestrator, caller.clone(), existing)
        .await
        .expect("attach existing Vapi call");
    let vapi = call.vapi_connection_id().clone();
    let original_bridge = call.bridge_id().clone();
    assert!(adapter.is_connection_live(&vapi));
    assert_eq!(state.create_count.load(Ordering::SeqCst), 0);

    let mut core_events = orchestrator.subscribe_events();
    let staged = orchestrator
        .prepare_transport_fenced_peer_handoff(
            original_bridge.clone(),
            caller.clone(),
            target.clone(),
            rvoip_core::DirectionalMediaBridgePlan::new(true, true).expect("bidirectional plan"),
            Arc::new(rvoip_core::stream::PassThroughDataMessageBridgePolicy),
        )
        .await
        .expect("stage the human target against a fenced caller bridge");
    assert_eq!(staged.previous_bridge_id(), &original_bridge);
    assert_eq!(staged.retained_connection(), &caller);
    assert_eq!(staged.source_connection(), &vapi);
    assert_eq!(staged.target_connection(), &target);
    let receipt = orchestrator
        .commit_peer_handoff_with_timeout_and_receipt(staged, Duration::from_secs(2))
        .await
        .expect("commit the handoff");
    assert_eq!(receipt.previous_bridge_id, original_bridge);
    assert_ne!(receipt.bridge_id, original_bridge, "a fresh generation is minted");
    assert_eq!(receipt.retained, caller);
    assert_eq!(receipt.source, vapi);
    assert_eq!(receipt.target, target);

    // The supervisor retires the AI leg without ending the retained caller.
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), call.wait_shared())
            .await
            .expect("handoff outcome timeout"),
        VapiAgentOutcome::HandedOff
    );
    // The outcome is latched, so shared observers can read it again while the
    // handle is still held for call control.
    assert_eq!(call.wait_shared().await, VapiAgentOutcome::HandedOff);

    // Core published the handoff once, then ended exactly the Vapi leg.
    let mut saw_handoff = false;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match core_events.recv().await.expect("core event bus") {
                rvoip_core::events::Event::PeerHandoffCommitted {
                    previous_bridge_id,
                    bridge_id,
                    retained,
                    source,
                    target: handoff_target,
                    ..
                } => {
                    assert!(!saw_handoff, "PeerHandoffCommitted must be published once");
                    assert_eq!(previous_bridge_id, original_bridge);
                    assert_eq!(bridge_id, receipt.bridge_id);
                    assert_eq!(retained, caller);
                    assert_eq!(source, vapi);
                    assert_eq!(handoff_target, target);
                    saw_handoff = true;
                }
                rvoip_core::events::Event::ConnectionEnded { connection_id, .. }
                | rvoip_core::events::Event::ConnectionFailed { connection_id, .. } => {
                    assert_ne!(connection_id, caller, "the retained caller was ended");
                    assert_ne!(connection_id, target, "the new peer was ended");
                    if connection_id == vapi {
                        break;
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .expect("the retired Vapi leg never reached a terminal state");
    assert!(saw_handoff, "PeerHandoffCommitted precedes the AI leg's terminal");

    // The old Vapi WebSocket was closed through the normal end protocol.
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
    assert!(saw_end_call, "the retired Vapi leg must send end-call on its socket");
    assert!(!adapter.is_connection_live(&vapi));

    // The retained caller and the new peer both remain in their Session.
    assert_eq!(orchestrator.session_of(&caller), Some(session_id.clone()));
    assert_eq!(orchestrator.session_of(&target), Some(session_id));
    assert_eq!(state.create_count.load(Ordering::SeqCst), 0);
    server.abort();
}

/// A handoff whose WebSocket is rejected fails the attachment outright. It
/// never falls back to creating a replacement provider call, even on an adapter
/// that holds a default key and therefore could.
#[tokio::test]
async fn existing_call_failed_socket_never_falls_back_to_creation() {
    let (api_base, state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::RejectUpgrade, Vec::new()).await;
    let config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    let adapter = VapiAdapter::new(config).expect("adapter");
    let (orchestrator, caller) = setup_caller().await;
    let websocket_url = Url::parse(&state.websocket_url.lock().expect("websocket URL lock"))
        .expect("mock websocket URL");
    let existing = VapiExistingCall::new(
        VapiCallOptions::new(VapiAssistant::saved("assistant-mock")),
        "already-created-call".into(),
        websocket_url,
    )
    .expect("existing call");

    assert!(adapter
        .attach_existing_agent(&orchestrator, caller.clone(), existing)
        .await
        .is_err());
    assert_eq!(
        state
            .websocket_authorization
            .lock()
            .expect("websocket authorization lock")
            .as_deref(),
        Some("Bearer mock-api-key"),
        "the handoff was attempted against the provider socket"
    );
    assert_eq!(state.create_count.load(Ordering::SeqCst), 0);
    assert!(
        orchestrator.session_of(&caller).is_some(),
        "a failed handoff must not end the retained caller"
    );
    server.abort();
}

#[tokio::test]
async fn user_speech_flushes_real_graph_sink_queue_and_counts_drops() {
    let (api_base, state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::Interactive, Vec::new()).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.heartbeat_interval = Duration::from_secs(60);
    config.inbound_queue_capacity = 32;
    config.media_queue_capacity = 32;
    let adapter = VapiAdapter::new(config).expect("adapter");
    let (orchestrator, caller_connection_id) = setup_caller().await;
    let call_result = adapter
        .attach_agent(
            &orchestrator,
            caller_connection_id,
            VapiCallOptions::new(VapiAssistant::saved("assistant-mock")),
        )
        .await;
    let call = match call_result {
        Ok(call) => call,
        Err(RvoipError::InvalidState(reason)) => panic!("attach invalid state: {reason}"),
        Err(RvoipError::NotImplemented(operation)) => {
            panic!("attach operation not implemented: {operation}")
        }
        Err(error) => panic!("attach Vapi agent: {error:?}"),
    };
    let vapi_connection_id = call.vapi_connection_id().clone();

    state
        .injected
        .send(Injected::Binary(vec![0x55; 160 * 16]))
        .expect("inject queued assistant audio");

    let queued_before = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let snapshot = orchestrator
                .media_graph_snapshot(&vapi_connection_id)
                .await
                .expect("Vapi source graph");
            let queued = snapshot
                .sinks
                .iter()
                .map(|sink| sink.queue_depth)
                .sum::<usize>();
            if queued >= 3 {
                break queued;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("assistant frames never parked in the graph sink queue");

    state
        .injected
        .send(Injected::Text(
            r#"{"type":"speech-update","status":"started","role":"user"}"#.into(),
        ))
        .expect("inject user speech start");

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = orchestrator
                .media_graph_snapshot(&vapi_connection_id)
                .await
                .expect("Vapi graph after barge-in");
            let queued = snapshot
                .sinks
                .iter()
                .map(|sink| sink.queue_depth)
                .sum::<usize>();
            let health = adapter
                .media_health(&vapi_connection_id)
                .expect("Vapi media health");
            if queued == 0 && health.barge_in_dropped >= queued_before as u64 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("barge-in did not flush and count the real graph queue");

    call.end().await.expect("end Vapi test call");
    server.abort();
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
async fn bodyless_websocket_close_is_a_normal_remote_end() {
    // RFC 6455 §5.5.1 permits a Close frame with no body, and this adapter's
    // own close is bodyless. Reporting it as AdapterEvent::Failed marked calls
    // as failures when the peer had simply hung up and the audio was fine.
    let (api_base, _state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::BodylessClose, Vec::new()).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
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
    match tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("terminal event timeout")
        .expect("terminal event")
    {
        AdapterEvent::Ended { .. } => {}
        AdapterEvent::Failed { detail, .. } => {
            panic!("a bodyless close was reported as a failure: {detail}")
        }
        other => panic!("unexpected terminal event: {other:?}"),
    }
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

#[tokio::test]
async fn attach_agent_attributes_vapi_to_distinct_ai_participant() {
    let (api_base, _state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::Interactive, Vec::new()).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.heartbeat_interval = Duration::from_secs(60);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let (orchestrator, caller_connection_id) = setup_caller().await;
    let call = adapter
        .attach_agent(
            &orchestrator,
            caller_connection_id.clone(),
            VapiCallOptions::new(VapiAssistant::saved("assistant-mock")),
        )
        .await
        .expect("attach Vapi agent");

    let session_id = orchestrator
        .session_of(call.caller_connection_id())
        .expect("caller session");
    let session = orchestrator.session(&session_id).expect("session handle");
    let session = session.read().expect("session lock");
    let caller_participant = session
        .connections
        .get(call.caller_connection_id())
        .expect("caller connection")
        .participant_id
        .clone();
    let vapi_participant = session
        .connections
        .get(call.vapi_connection_id())
        .expect("vapi connection")
        .participant_id
        .clone();
    assert_ne!(
        caller_participant, vapi_participant,
        "Vapi Connection must not reuse the caller's participant_id"
    );
    assert_eq!(&vapi_participant, call.ai_participant_id());

    let conversation = orchestrator
        .conversation(&session.conversation_id)
        .expect("conversation");
    let conversation = conversation.read().expect("conversation lock");
    let ai = conversation
        .participants
        .iter()
        .find(|participant| participant.id == vapi_participant)
        .expect("AI participant on conversation");
    assert_eq!(ai.kind, ParticipantKind::Ai);
    assert_eq!(ai.role, ParticipantRole::Agent);
    server.abort();
}

#[tokio::test]
async fn attach_agent_for_participant_rejects_existing_human() {
    let (api_base, _state, _observed, server) =
        start_mock_with_behavior(Duration::ZERO, SocketBehavior::Interactive, Vec::new()).await;
    let mut config = VapiConfig::new(VapiApiKey::new("mock-api-key").expect("mock key"))
        .with_api_base(api_base)
        .with_loopback_test_transport();
    config.heartbeat_interval = Duration::from_secs(60);
    let adapter = VapiAdapter::new(config).expect("adapter");
    let (orchestrator, caller_connection_id) = setup_caller().await;
    let session_id = orchestrator
        .session_of(&caller_connection_id)
        .expect("caller session");
    let human = ParticipantId::new();
    orchestrator
        .join_session(
            session_id,
            human.clone(),
            ParticipantKind::Human,
            ParticipantRole::Customer,
        )
        .await
        .expect("join human");

    let error = adapter
        .attach_agent_for_participant(
            &orchestrator,
            caller_connection_id,
            human,
            VapiCallOptions::new(VapiAssistant::saved("assistant-mock")),
        )
        .await
        .expect_err("human cannot be the AI participant");
    assert!(matches!(error, RvoipError::InvalidState(_)));
    server.abort();
}
