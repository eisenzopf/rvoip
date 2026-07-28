//! Integration tests for `Orchestrator::bridge_connections` /
//! `unbridge_connections` (the cross-transport frame-pump path).
//!
//! Uses an inline `MockAdapter` + `MockMediaStream` so the test is
//! self-contained — no SIP / QUIC / WebSocket setup needed.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason as AdapterEndReason,
    OriginateRequest, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::commands::{AttachmentRef, ListenerSink, ListenerTarget, RecordingTarget};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, MessageId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::DEFAULT_BRIDGED_DATA_MESSAGE_QUEUE_CAPACITY;
use rvoip_core::stream::{
    BridgedDataMessageDecision, DataMessageBridgePolicy, MediaFrame, MediaReceiverReservation,
    MediaStream, QualitySnapshot, StreamKind,
};
use rvoip_core::{
    Config, DataMessage, DataReliability, DirectionalMediaBridgePlan, Orchestrator, RvoipError,
};
use rvoip_harness::{
    AsrConfig, AsrProvider, AsrResult, AsrStream, DialogAction, DialogManager, ListenOnlyDialog,
    NoOpTtsProvider, TtsPlayback, TtsProvider, TtsRequest, VecRecordingSink,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{mpsc, Barrier, Notify};

// =====================================================================
// MockMediaStream
// =====================================================================

struct MockMediaStream {
    id: StreamId,
    codec: CodecInfo,
    /// The "outside" hands us frames via `external_in_tx`; we deliver
    /// them through `frames_in()`.
    external_in_tx: mpsc::Sender<MediaFrame>,
    in_rx: Arc<StdMutex<Option<mpsc::Receiver<MediaFrame>>>>,
    source_acquisitions: Arc<AtomicUsize>,
    /// `frames_out()` returns clones of this sender; what the
    /// "outside" reads via `external_out_rx`.
    out_tx: mpsc::Sender<MediaFrame>,
    external_out_rx: StdMutex<Option<mpsc::Receiver<MediaFrame>>>,
    writable: AtomicBool,
}

impl MockMediaStream {
    fn new(codec_name: &str) -> Arc<Self> {
        let (external_in_tx, in_rx) = mpsc::channel::<MediaFrame>(64);
        let (out_tx, external_out_rx) = mpsc::channel::<MediaFrame>(64);
        Arc::new(Self {
            id: StreamId::new(),
            codec: CodecInfo {
                name: codec_name.to_string(),
                clock_rate_hz: 48000,
                channels: 1,
                fmtp: None,
            },
            external_in_tx,
            in_rx: Arc::new(StdMutex::new(Some(in_rx))),
            source_acquisitions: Arc::new(AtomicUsize::new(0)),
            out_tx,
            external_out_rx: StdMutex::new(Some(external_out_rx)),
            writable: AtomicBool::new(true),
        })
    }

    /// Push a frame from "outside" — the bridge sees it via `frames_in()`.
    async fn inject(&self, frame: MediaFrame) {
        let _ = self.external_in_tx.send(frame).await;
    }

    /// Take the external-side receiver for the outbound stream.
    fn take_external_out(&self) -> mpsc::Receiver<MediaFrame> {
        self.external_out_rx
            .lock()
            .unwrap()
            .take()
            .expect("first take")
    }

    fn set_writable(&self, writable: bool) {
        self.writable.store(writable, Ordering::Release);
    }

    fn source_acquisitions(&self) -> usize {
        self.source_acquisitions.load(Ordering::Acquire)
    }

    fn try_take_source(&self) -> rvoip_core::error::Result<mpsc::Receiver<MediaFrame>> {
        let receiver = self
            .in_rx
            .lock()
            .unwrap()
            .take()
            .ok_or(RvoipError::InvalidState(
                "mock media source receiver was already acquired",
            ))?;
        self.source_acquisitions.fetch_add(1, Ordering::AcqRel);
        Ok(receiver)
    }
}

#[async_trait]
impl MediaStream for MockMediaStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }
    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }
    fn codec(&self) -> CodecInfo {
        self.codec.clone()
    }
    fn direction(&self) -> Direction {
        Direction::Inbound
    }
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.try_take_source()
            .unwrap_or_else(|_| mpsc::channel(1).1)
    }
    fn try_frames_in(&self) -> rvoip_core::error::Result<mpsc::Receiver<MediaFrame>> {
        self.try_take_source()
    }
    fn reserve_frames_in(&self) -> rvoip_core::error::Result<MediaReceiverReservation> {
        let receiver = self
            .in_rx
            .lock()
            .unwrap()
            .take()
            .ok_or(RvoipError::InvalidState(
                "mock media source receiver was already acquired",
            ))?;
        let slot = Arc::clone(&self.in_rx);
        let acquisitions = Arc::clone(&self.source_acquisitions);
        Ok(MediaReceiverReservation::new(receiver, move |receiver| {
            let mut slot = slot.lock().unwrap();
            debug_assert!(slot.is_none(), "reserved mock receiver slot was replaced");
            if slot.is_none() {
                *slot = Some(receiver);
            }
        })
        .with_commit_hook(move || {
            acquisitions.fetch_add(1, Ordering::AcqRel);
        }))
    }
    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.out_tx.clone()
    }
    fn try_frames_out(&self) -> rvoip_core::error::Result<mpsc::Sender<MediaFrame>> {
        if self.writable.load(Ordering::Acquire) {
            Ok(self.out_tx.clone())
        } else {
            Err(RvoipError::InvalidState(
                "mock media stream is not activated",
            ))
        }
    }
    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }
    async fn close(self: Arc<Self>) -> rvoip_core::error::Result<()> {
        Ok(())
    }
}

// =====================================================================
// MockAdapter
// =====================================================================

struct MockAdapter {
    transport: Transport,
    /// One stream per ConnectionId (audio).
    streams: dashmap::DashMap<ConnectionId, Arc<MockMediaStream>>,
    events_tx: mpsc::Sender<AdapterEvent>,
    events_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
    stream_gates: dashmap::DashMap<ConnectionId, Arc<StreamLookupGate>>,
    data_send_gates: dashmap::DashMap<ConnectionId, Arc<DataSendGate>>,
    sent_data_messages: StdMutex<Vec<(ConnectionId, DataMessage)>>,
    renegotiated_audio: StdMutex<Option<CodecInfo>>,
}

struct StreamLookupGate {
    armed: AtomicBool,
    entered: Notify,
    release: Notify,
}

struct DataSendGate {
    entered: Notify,
    release: Notify,
    released: AtomicBool,
}

impl DataSendGate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            release: Notify::new(),
            released: AtomicBool::new(false),
        })
    }

    async fn wait(&self) {
        self.entered.notify_waiters();
        while !self.released.load(Ordering::Acquire) {
            self.release.notified().await;
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.release.notify_waiters();
    }
}

impl StreamLookupGate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            armed: AtomicBool::new(true),
            entered: Notify::new(),
            release: Notify::new(),
        })
    }
}

impl MockAdapter {
    fn new(transport: Transport) -> Arc<Self> {
        let (events_tx, events_rx) = mpsc::channel(64);
        Arc::new(Self {
            transport,
            streams: dashmap::DashMap::new(),
            events_tx,
            events_rx: StdMutex::new(Some(events_rx)),
            stream_gates: dashmap::DashMap::new(),
            data_send_gates: dashmap::DashMap::new(),
            sent_data_messages: StdMutex::new(Vec::new()),
            renegotiated_audio: StdMutex::new(None),
        })
    }

    fn register_connection(&self, id: ConnectionId, stream: Arc<MockMediaStream>) {
        self.streams.insert(id, stream);
    }

    fn gate_next_stream_lookup(&self, id: ConnectionId) -> Arc<StreamLookupGate> {
        let gate = StreamLookupGate::new();
        self.stream_gates.insert(id, Arc::clone(&gate));
        gate
    }

    async fn announce(&self, id: ConnectionId, session_id: SessionId) {
        let conn = Connection {
            id: id.clone(),
            session_id,
            participant_id: ParticipantId::new(),
            transport: self.transport,
            direction: Direction::Inbound,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor::default(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: Vec::new(),
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        };
        let _ = self
            .events_tx
            .send(AdapterEvent::InboundConnection { connection: conn })
            .await;
    }

    fn sent_data_messages(&self) -> Vec<(ConnectionId, DataMessage)> {
        self.sent_data_messages.lock().unwrap().clone()
    }

    fn gate_data_send(&self, id: ConnectionId) -> Arc<DataSendGate> {
        let gate = DataSendGate::new();
        self.data_send_gates.insert(id, Arc::clone(&gate));
        gate
    }

    fn set_renegotiated_audio(&self, codec: CodecInfo) {
        *self.renegotiated_audio.lock().unwrap() = Some(codec);
    }
}

#[async_trait]
impl ConnectionAdapter for MockAdapter {
    fn transport(&self) -> Transport {
        self.transport
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Substrate
    }

    async fn originate(&self, _r: OriginateRequest) -> rvoip_core::error::Result<ConnectionHandle> {
        Err(RvoipError::NotImplemented("mock"))
    }
    async fn accept(&self, _c: ConnectionId) -> rvoip_core::error::Result<()> {
        Ok(())
    }
    async fn reject(&self, _c: ConnectionId, _r: RejectReason) -> rvoip_core::error::Result<()> {
        Ok(())
    }
    async fn end(&self, _c: ConnectionId, _r: AdapterEndReason) -> rvoip_core::error::Result<()> {
        Ok(())
    }
    async fn hold(&self, _c: ConnectionId) -> rvoip_core::error::Result<()> {
        Ok(())
    }
    async fn resume(&self, _c: ConnectionId) -> rvoip_core::error::Result<()> {
        Ok(())
    }
    async fn transfer(
        &self,
        _c: ConnectionId,
        _t: TransferTarget,
    ) -> rvoip_core::error::Result<()> {
        Ok(())
    }
    async fn streams(
        &self,
        c: ConnectionId,
    ) -> rvoip_core::error::Result<Vec<Arc<dyn MediaStream>>> {
        let gate = self
            .stream_gates
            .get(&c)
            .map(|entry| Arc::clone(entry.value()));
        if let Some(gate) = gate {
            if gate.armed.swap(false, Ordering::SeqCst) {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
        }
        match self.streams.get(&c) {
            Some(s) => Ok(vec![s.clone() as Arc<dyn MediaStream>]),
            None => Ok(Vec::new()),
        }
    }
    async fn send_message(&self, _c: ConnectionId, _m: Message) -> rvoip_core::error::Result<()> {
        Ok(())
    }
    async fn send_data_message(
        &self,
        connection_id: ConnectionId,
        message: DataMessage,
    ) -> rvoip_core::error::Result<()> {
        let gate = self
            .data_send_gates
            .get(&connection_id)
            .map(|gate| Arc::clone(gate.value()));
        if let Some(gate) = gate {
            gate.wait().await;
        }
        self.sent_data_messages
            .lock()
            .unwrap()
            .push((connection_id, message));
        Ok(())
    }
    async fn send_dtmf(
        &self,
        _c: ConnectionId,
        _digits: &str,
        _ms: u32,
    ) -> rvoip_core::error::Result<()> {
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _c: ConnectionId,
        _caps: CapabilityDescriptor,
    ) -> rvoip_core::error::Result<rvoip_core::capability::NegotiatedCodecs> {
        Ok(NegotiatedCodecs {
            audio: self.renegotiated_audio.lock().unwrap().clone(),
            video: None,
        })
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.events_rx
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }
    async fn verify_request_signature(
        &self,
        _c: ConnectionId,
        _sig: SignatureHeaders,
    ) -> rvoip_core::error::Result<IdentityAssurance> {
        Err(RvoipError::NotImplemented("mock"))
    }
}

// =====================================================================
// Helpers
// =====================================================================

fn mk_frame(stream_id: StreamId, byte: u8) -> MediaFrame {
    MediaFrame {
        stream_id,
        kind: StreamKind::Audio,
        payload: Bytes::from(vec![byte]),
        timestamp_rtp: 0,
        captured_at: Utc::now(),
        payload_type: None,
    }
}

struct CountingAsrProvider {
    pushes: Arc<AtomicUsize>,
}

struct CountingAsrStream {
    pushes: Arc<AtomicUsize>,
}

struct OneResultAsrProvider;

struct OneResultAsrStream {
    delivered: AtomicBool,
}

struct SayDialog;

struct CountingTtsProvider {
    cancellations: Arc<AtomicUsize>,
}

struct CountingTtsPlayback {
    cancellations: Arc<AtomicUsize>,
    frame_delivered: AtomicBool,
}

#[async_trait]
impl AsrProvider for CountingAsrProvider {
    async fn open_stream(
        &self,
        _conn: ConnectionId,
        _config: AsrConfig,
    ) -> rvoip_core::error::Result<Box<dyn AsrStream>> {
        Ok(Box::new(CountingAsrStream {
            pushes: Arc::clone(&self.pushes),
        }))
    }
}

#[async_trait]
impl AsrStream for CountingAsrStream {
    async fn push(&self, _frame: MediaFrame) -> rvoip_core::error::Result<()> {
        self.pushes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn next(&self) -> Option<AsrResult> {
        std::future::pending().await
    }

    async fn close(&self) -> rvoip_core::error::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl AsrProvider for OneResultAsrProvider {
    async fn open_stream(
        &self,
        _conn: ConnectionId,
        _config: AsrConfig,
    ) -> rvoip_core::error::Result<Box<dyn AsrStream>> {
        Ok(Box::new(OneResultAsrStream {
            delivered: AtomicBool::new(false),
        }))
    }
}

#[async_trait]
impl AsrStream for OneResultAsrStream {
    async fn push(&self, _frame: MediaFrame) -> rvoip_core::error::Result<()> {
        Ok(())
    }

    async fn next(&self) -> Option<AsrResult> {
        if !self.delivered.swap(true, Ordering::AcqRel) {
            return Some(AsrResult {
                stream_id: StreamId::new(),
                speaker: None,
                text: "speak".to_string(),
                confidence: 1.0,
                is_final: true,
            });
        }
        std::future::pending().await
    }

    async fn close(&self) -> rvoip_core::error::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl DialogManager for SayDialog {
    async fn turn(&self, _transcript: &AsrResult) -> rvoip_core::error::Result<DialogAction> {
        Ok(DialogAction::Say {
            text: "response".to_string(),
            voice: None,
        })
    }
}

#[async_trait]
impl TtsProvider for CountingTtsProvider {
    async fn synthesize(
        &self,
        _request: TtsRequest,
    ) -> rvoip_core::error::Result<Box<dyn TtsPlayback>> {
        Ok(Box::new(CountingTtsPlayback {
            cancellations: Arc::clone(&self.cancellations),
            frame_delivered: AtomicBool::new(false),
        }))
    }
}

#[async_trait]
impl TtsPlayback for CountingTtsPlayback {
    async fn next_frame(&self) -> Option<MediaFrame> {
        if !self.frame_delivered.swap(true, Ordering::AcqRel) {
            return Some(mk_frame(StreamId::new(), 7));
        }
        std::future::pending().await
    }

    async fn cancel(&self) -> rvoip_core::error::Result<()> {
        self.cancellations.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

async fn wait_for_cancellations(cancellations: &AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while cancellations.load(Ordering::Acquire) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("TTS cancellation count did not converge");
}

async fn wait_for_sink_count(graph: &rvoip_core::media_graph::MediaGraphHandle, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if graph.snapshot().await.sinks.len() == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("media graph sink count did not converge");
}

/// Spin up an Orchestrator with one MockAdapter (Quic transport) holding
/// two connections + their streams. Returns the orchestrator + the two
/// streams + their connection ids so tests can inject/observe frames.
async fn setup_two_connection_orchestrator_with_adapter(
    codec_a: &str,
    codec_b: &str,
) -> (
    Arc<Orchestrator>,
    Arc<MockMediaStream>,
    Arc<MockMediaStream>,
    ConnectionId,
    ConnectionId,
    Arc<MockAdapter>,
) {
    let adapter = MockAdapter::new(Transport::Quic);
    let conn_a = ConnectionId::new();
    let conn_b = ConnectionId::new();
    let stream_a = MockMediaStream::new(codec_a);
    let stream_b = MockMediaStream::new(codec_b);
    adapter.register_connection(conn_a.clone(), Arc::clone(&stream_a));
    adapter.register_connection(conn_b.clone(), Arc::clone(&stream_b));

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register");

    // The orchestrator's adapter-event-pump loop runs in a spawned
    // task; it needs to observe `InboundConnection` events so the
    // connection registry is populated. Drive that by announcing two
    // inbound connections via the adapter's event channel.
    let session = SessionId::new();
    adapter.announce(conn_a.clone(), session.clone()).await;
    adapter.announce(conn_b.clone(), session).await;
    // Give the pump a beat to consume both events.
    tokio::time::sleep(Duration::from_millis(50)).await;

    (orchestrator, stream_a, stream_b, conn_a, conn_b, adapter)
}

async fn setup_two_connection_orchestrator(
    codec_a: &str,
    codec_b: &str,
) -> (
    Arc<Orchestrator>,
    Arc<MockMediaStream>,
    Arc<MockMediaStream>,
    ConnectionId,
    ConnectionId,
) {
    let (orchestrator, stream_a, stream_b, conn_a, conn_b, _adapter) =
        setup_two_connection_orchestrator_with_adapter(codec_a, codec_b).await;
    (orchestrator, stream_a, stream_b, conn_a, conn_b)
}

async fn wait_for_data_message_count(adapter: &MockAdapter, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while adapter.sent_data_messages().len() < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bridged data message count did not converge");
}

#[derive(Default)]
struct SelectiveDataPolicy {
    seen: StdMutex<Vec<(ConnectionId, ConnectionId, String)>>,
}

impl SelectiveDataPolicy {
    fn seen(&self) -> Vec<(ConnectionId, ConnectionId, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl DataMessageBridgePolicy for SelectiveDataPolicy {
    fn decide(
        &self,
        source: &ConnectionId,
        target: &ConnectionId,
        mut message: DataMessage,
    ) -> BridgedDataMessageDecision {
        self.seen
            .lock()
            .unwrap()
            .push((source.clone(), target.clone(), message.label.clone()));
        match message.label.as_str() {
            "policy.drop" => BridgedDataMessageDecision::Drop,
            "policy.transform" => {
                message.label = "policy.transformed".to_string();
                message.content_type = "application/octet-stream".to_string();
                BridgedDataMessageDecision::Forward(message)
            }
            "policy.invalid-transform" => {
                message.label.clear();
                BridgedDataMessageDecision::Forward(message)
            }
            "policy.panic" => panic!("deterministic test policy panic"),
            _ => BridgedDataMessageDecision::Forward(message),
        }
    }
}

async fn wait_for_policy_count(policy: &SelectiveDataPolicy, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while policy.seen().len() < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bridge data policy count did not converge");
}

fn assert_send<T: Send>(_: T) {}

#[test]
fn public_bridge_futures_remain_send_for_multithreaded_call_actors() {
    let orchestrator = Orchestrator::new(Config::default());
    let left = ConnectionId::new();
    let right = ConnectionId::new();
    let policy: Arc<dyn DataMessageBridgePolicy> = Arc::new(SelectiveDataPolicy::default());
    assert_send(orchestrator.bridge_connections_with_data_policy(
        left.clone(),
        right.clone(),
        policy,
    ));
    assert_send(orchestrator.bridge_connections_directional(
        left.clone(),
        right.clone(),
        DirectionalMediaBridgePlan::new(true, false).unwrap(),
    ));
    assert_send(orchestrator.bridge_connections(left, right));
}

// =====================================================================
// Tests
// =====================================================================

#[tokio::test]
async fn bridge_passes_frames_through_when_codecs_match() {
    let _ = tracing_subscriber::fmt::try_init();
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;

    let mut b_out = stream_b.take_external_out();
    let _bridge_id = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect("bridge");

    // Inject 5 frames into A; they should arrive on B unchanged.
    for i in 0u8..5 {
        stream_a.inject(mk_frame(stream_a.id(), i)).await;
    }

    let mut received = Vec::new();
    while received.len() < 5 {
        let frame = tokio::time::timeout(Duration::from_secs(2), b_out.recv())
            .await
            .expect("timeout")
            .expect("closed");
        received.push(frame.payload[0]);
    }
    assert_eq!(received, (0u8..5).collect::<Vec<_>>());
}

#[tokio::test]
async fn data_messages_preserve_all_fields_and_route_to_only_the_exact_connection() {
    let (orch, _stream_a, _stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("opus", "opus").await;
    let mut events = orch.subscribe_events();
    let text = DataMessage {
        label: "customer.control/text-v7".to_string(),
        content_type: "text/plain".to_string(),
        bytes: Bytes::from_static("hello, data channel".as_bytes()),
        reliability: DataReliability::MaxLifetime {
            ordered: false,
            milliseconds: 1_500,
        },
        message_id: MessageId::from_string("message-text-exact-target"),
    };
    let binary = DataMessage {
        label: "opaque.binary/custom".to_string(),
        content_type: "application/octet-stream".to_string(),
        bytes: Bytes::from_static(&[0, 0xff, 1, 2, 0x80, 42]),
        reliability: DataReliability::MaxRetransmits {
            ordered: true,
            count: 7,
        },
        message_id: MessageId::from_string("message-binary-exact-target"),
    };

    adapter
        .events_tx
        .send(AdapterEvent::DataMessage {
            connection_id: conn_a.clone(),
            message: text.clone(),
        })
        .await
        .expect("inbound text data message");
    adapter
        .events_tx
        .send(AdapterEvent::DataMessage {
            connection_id: conn_b.clone(),
            message: binary.clone(),
        })
        .await
        .expect("inbound binary data message");

    let mut received = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), async {
        while received.len() < 2 {
            match events.recv().await {
                Ok(Event::DataMessageReceived {
                    connection_id,
                    message,
                    ..
                }) => received.push((connection_id, message)),
                Ok(_) => {}
                Err(error) => panic!("event bus closed: {error}"),
            }
        }
    })
    .await
    .expect("inbound data messages were not normalized");
    assert!(received.contains(&(conn_a.clone(), text.clone())));
    assert!(received.contains(&(conn_b.clone(), binary.clone())));

    orch.send_data_message_to_connection(conn_b.clone(), text.clone())
        .await
        .expect("outbound text message");
    orch.send_data_message(conn_a.clone(), binary.clone())
        .await
        .expect("outbound binary message through compatibility wrapper");
    assert_eq!(
        adapter.sent_data_messages(),
        vec![(conn_b, text), (conn_a, binary)],
        "the Orchestrator must neither rewrite data metadata nor fan it to a peer connection"
    );

    let unknown = ConnectionId::new();
    assert!(matches!(
        orch.send_data_message(
            unknown.clone(),
            DataMessage::reliable("unknown-target", "text/plain", "ignored"),
        )
        .await,
        Err(RvoipError::ConnectionNotFound(id)) if id == unknown
    ));
    assert_eq!(adapter.sent_data_messages().len(), 2);
}

#[tokio::test]
async fn legacy_bridge_passes_arbitrary_data_labels_in_both_exact_directions() {
    let (orch, stream_a, stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("opus", "opus").await;
    let bridge = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect("legacy bridge");
    let from_a = DataMessage {
        label: "arbitrary.customer/text-v9".to_string(),
        content_type: "text/plain".to_string(),
        bytes: Bytes::from_static(b"left-to-right"),
        reliability: DataReliability::ReliableUnordered,
        message_id: MessageId::from_string("legacy-a-to-b"),
    };
    let from_b = DataMessage {
        label: "opaque.vendor/binary".to_string(),
        content_type: "application/octet-stream".to_string(),
        bytes: Bytes::from_static(&[0, 0xff, 7]),
        reliability: DataReliability::MaxRetransmits {
            ordered: false,
            count: 3,
        },
        message_id: MessageId::from_string("legacy-b-to-a"),
    };

    adapter
        .events_tx
        .send(AdapterEvent::DataMessage {
            connection_id: conn_a.clone(),
            message: from_a.clone(),
        })
        .await
        .expect("A inbound data");
    adapter
        .events_tx
        .send(AdapterEvent::DataMessage {
            connection_id: conn_b.clone(),
            message: from_b.clone(),
        })
        .await
        .expect("B inbound data");
    wait_for_data_message_count(&adapter, 2).await;

    let sent = adapter.sent_data_messages();
    assert!(sent.contains(&(conn_b, from_a)));
    assert!(sent.contains(&(conn_a, from_b)));
    assert_eq!(stream_a.source_acquisitions(), 1);
    assert_eq!(stream_b.source_acquisitions(), 1);
    orch.unbridge_connections(bridge)
        .await
        .expect("remove legacy bridge");
}

#[tokio::test]
async fn bridge_policy_gets_exact_direction_and_can_drop_transform_or_fail_validation() {
    let (orch, _stream_a, _stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("opus", "opus").await;
    let policy = Arc::new(SelectiveDataPolicy::default());
    let bridge = orch
        .bridge_connections_with_data_policy(conn_a.clone(), conn_b.clone(), policy.clone())
        .await
        .expect("policy bridge");
    let drop_message = DataMessage::reliable("policy.drop", "text/plain", "drop me");
    let transform_message = DataMessage::reliable(
        "policy.transform",
        "text/plain",
        Bytes::from_static(&[0, 1, 2]),
    );
    let invalid_message =
        DataMessage::reliable("policy.invalid-transform", "text/plain", "invalid");
    for (connection_id, message) in [
        (conn_a.clone(), drop_message),
        (conn_b.clone(), transform_message.clone()),
        (conn_a.clone(), invalid_message),
    ] {
        adapter
            .events_tx
            .send(AdapterEvent::DataMessage {
                connection_id,
                message,
            })
            .await
            .expect("inbound policy data");
    }
    wait_for_policy_count(&policy, 3).await;
    wait_for_data_message_count(&adapter, 1).await;
    tokio::time::sleep(Duration::from_millis(25)).await;

    let mut transformed = transform_message;
    transformed.label = "policy.transformed".to_string();
    transformed.content_type = "application/octet-stream".to_string();
    assert_eq!(
        adapter.sent_data_messages(),
        vec![(conn_a.clone(), transformed)]
    );
    let seen = policy.seen();
    assert!(seen.contains(&(conn_a.clone(), conn_b.clone(), "policy.drop".to_string())));
    assert!(seen.contains(&(
        conn_b.clone(),
        conn_a.clone(),
        "policy.transform".to_string()
    )));
    assert!(seen.contains(&(conn_a, conn_b, "policy.invalid-transform".to_string())));
    orch.unbridge_connections(bridge)
        .await
        .expect("remove policy bridge");
}

#[tokio::test]
async fn slow_data_direction_is_bounded_does_not_stall_peer_and_unbridge_aborts_workers() {
    let (orch, stream_a, stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("opus", "opus").await;
    let gate = adapter.gate_data_send(conn_b.clone());
    let bridge = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect("bridge");

    let entered = gate.entered.notified();
    tokio::pin!(entered);
    adapter
        .events_tx
        .send(AdapterEvent::DataMessage {
            connection_id: conn_a.clone(),
            message: DataMessage::reliable("blocked", "text/plain", "first"),
        })
        .await
        .expect("first blocked message");
    tokio::time::timeout(Duration::from_secs(2), &mut entered)
        .await
        .expect("target data send did not enter gate");

    let offered = DEFAULT_BRIDGED_DATA_MESSAGE_QUEUE_CAPACITY * 4;
    tokio::time::timeout(Duration::from_secs(2), async {
        for index in 0..offered {
            adapter
                .events_tx
                .send(AdapterEvent::DataMessage {
                    connection_id: conn_a.clone(),
                    message: DataMessage {
                        label: "blocked".to_string(),
                        content_type: "application/octet-stream".to_string(),
                        bytes: Bytes::from(vec![index as u8]),
                        reliability: DataReliability::ReliableOrdered,
                        message_id: MessageId::from_string(format!("blocked-{index}")),
                    },
                })
                .await
                .expect("bounded offer");
        }
    })
    .await
    .expect("a blocked target must not backpressure adapter-event ingest");

    let reverse = DataMessage::reliable("reverse", "text/plain", "still live");
    adapter
        .events_tx
        .send(AdapterEvent::DataMessage {
            connection_id: conn_b.clone(),
            message: reverse.clone(),
        })
        .await
        .expect("reverse message");
    wait_for_data_message_count(&adapter, 1).await;
    assert_eq!(adapter.sent_data_messages(), vec![(conn_a, reverse)]);
    assert_eq!(stream_a.source_acquisitions(), 1);
    assert_eq!(stream_b.source_acquisitions(), 1);

    tokio::time::timeout(Duration::from_secs(2), orch.unbridge_connections(bridge))
        .await
        .expect("unbridge must abort and join blocked directional workers")
        .expect("unbridge");
    gate.release();
}

#[tokio::test]
async fn policy_panic_tears_down_only_its_bridge_without_unwinding_or_reacquiring_media() {
    let (orch, stream_a, stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("opus", "opus").await;
    let mut events = orch.subscribe_events();
    let policy = Arc::new(SelectiveDataPolicy::default());
    let first = orch
        .bridge_connections_with_data_policy(conn_a.clone(), conn_b.clone(), policy)
        .await
        .expect("policy bridge");
    adapter
        .events_tx
        .send(AdapterEvent::DataMessage {
            connection_id: conn_a.clone(),
            message: DataMessage::reliable("policy.panic", "text/plain", "secret body"),
        })
        .await
        .expect("panic trigger");

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                events.recv().await,
                Ok(Event::ConnectionsUnbridged { bridge_id, .. }) if bridge_id == first
            ) {
                return;
            }
        }
    })
    .await
    .expect("policy panic did not converge bridge teardown");

    let replacement = orch
        .bridge_connections(conn_a, conn_b)
        .await
        .expect("panic teardown must release exact bridge ownership");
    assert_eq!(stream_a.source_acquisitions(), 1);
    assert_eq!(stream_b.source_acquisitions(), 1);
    orch.unbridge_connections(replacement)
        .await
        .expect("remove replacement bridge");
}

#[tokio::test]
async fn bridge_propagates_typed_unwritable_sink_before_consuming_sources() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    stream_b.set_writable(false);

    let error = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect_err("dormant sink must reject bridge setup");
    assert!(matches!(
        error,
        RvoipError::InvalidState("mock media stream is not activated")
    ));
    assert!(
        stream_a.in_rx.lock().unwrap().is_some(),
        "source A receiver must remain available after preflight rejection"
    );
    assert!(
        stream_b.in_rx.lock().unwrap().is_some(),
        "source B receiver must remain available after preflight rejection"
    );

    stream_b.set_writable(true);
    let bridge = orch
        .bridge_connections(conn_a, conn_b)
        .await
        .expect("failed preflight must release bridge admission");
    orch.unbridge_connections(bridge)
        .await
        .expect("remove replacement bridge");
}

#[tokio::test]
async fn directional_bridge_consumes_only_enabled_sources_and_routes_each_half_independently() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let mut a_out = stream_a.take_external_out();
    let mut b_out = stream_b.take_external_out();
    let bridge = orch
        .bridge_connections_directional(
            conn_a,
            conn_b,
            DirectionalMediaBridgePlan::new(true, false).unwrap(),
        )
        .await
        .expect("A-to-B bridge");

    assert_eq!(stream_a.source_acquisitions(), 1);
    assert_eq!(
        stream_b.source_acquisitions(),
        0,
        "disabled B source must remain available"
    );
    assert!(stream_b.in_rx.lock().unwrap().is_some());

    stream_a.inject(mk_frame(stream_a.id(), 41)).await;
    let delivered = tokio::time::timeout(Duration::from_secs(2), b_out.recv())
        .await
        .expect("A-to-B delivery timed out")
        .expect("B output closed");
    assert_eq!(delivered.payload[0], 41);

    stream_b.inject(mk_frame(stream_b.id(), 42)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), a_out.recv())
            .await
            .is_err(),
        "disabled B-to-A direction delivered media"
    );
    orch.unbridge_connections(bridge)
        .await
        .expect("unbridge A-to-B");

    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let mut a_out = stream_a.take_external_out();
    let mut b_out = stream_b.take_external_out();
    let bridge = orch
        .bridge_connections_directional(
            conn_a,
            conn_b,
            DirectionalMediaBridgePlan::new(false, true).unwrap(),
        )
        .await
        .expect("B-to-A bridge");

    assert_eq!(stream_a.source_acquisitions(), 0);
    assert_eq!(stream_b.source_acquisitions(), 1);
    assert!(stream_a.in_rx.lock().unwrap().is_some());

    stream_b.inject(mk_frame(stream_b.id(), 43)).await;
    let delivered = tokio::time::timeout(Duration::from_secs(2), a_out.recv())
        .await
        .expect("B-to-A delivery timed out")
        .expect("A output closed");
    assert_eq!(delivered.payload[0], 43);

    stream_a.inject(mk_frame(stream_a.id(), 44)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), b_out.recv())
            .await
            .is_err(),
        "disabled A-to-B direction delivered media"
    );
    orch.unbridge_connections(bridge)
        .await
        .expect("unbridge B-to-A");
}

#[tokio::test]
async fn directional_bridge_validates_required_sink_before_any_source_acquisition() {
    assert!(matches!(
        DirectionalMediaBridgePlan::new(false, false),
        Err(RvoipError::AdmissionRejected(
            "media bridge must enable at least one direction"
        ))
    ));

    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    // A is not a target in an A-to-B plan, so its dormant output must not
    // reject the plan. B is required and must fail before A is consumed.
    stream_a.set_writable(false);
    stream_b.set_writable(false);
    let plan = DirectionalMediaBridgePlan::new(true, false).unwrap();
    assert!(matches!(
        orch.bridge_connections_directional(conn_a.clone(), conn_b.clone(), plan)
            .await,
        Err(RvoipError::InvalidState(
            "mock media stream is not activated"
        ))
    ));
    assert_eq!(stream_a.source_acquisitions(), 0);
    assert_eq!(stream_b.source_acquisitions(), 0);
    assert!(stream_a.in_rx.lock().unwrap().is_some());
    assert!(stream_b.in_rx.lock().unwrap().is_some());

    stream_b.set_writable(true);
    let bridge = orch
        .bridge_connections_directional(conn_a, conn_b, plan)
        .await
        .expect("disabled A target must not be preflighted");
    assert_eq!(stream_a.source_acquisitions(), 1);
    assert_eq!(stream_b.source_acquisitions(), 0);
    orch.unbridge_connections(bridge)
        .await
        .expect("remove directional bridge");
}

#[tokio::test]
async fn one_way_bridge_renegotiation_updates_the_enabled_graph_route() {
    let (orch, _stream_a, _stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("PCMU", "opus").await;
    orch.bridge_connections_directional(
        conn_a.clone(),
        conn_b,
        DirectionalMediaBridgePlan::new(true, false).expect("one-way plan"),
    )
    .await
    .expect("directional bridge");
    let graph = orch
        .media_graph_for_connection(conn_a.clone())
        .await
        .expect("source graph");
    adapter.set_renegotiated_audio(CodecInfo {
        name: "PCMA".into(),
        clock_rate_hz: 8_000,
        channels: 1,
        fmtp: None,
    });

    orch.renegotiate_media(conn_a, CapabilityDescriptor::default())
        .await
        .expect("one-way graph swap");
    let snapshot = graph.snapshot().await;
    assert_eq!(snapshot.source_payload_type, 8);
    assert_eq!(snapshot.sinks.len(), 1);
    assert_eq!(snapshot.sinks[0].target_payload_type, 111);
}

#[tokio::test]
async fn one_way_bridge_renegotiation_rejects_an_unsupported_codec() {
    let (orch, _stream_a, _stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("PCMU", "opus").await;
    orch.bridge_connections_directional(
        conn_a.clone(),
        conn_b,
        DirectionalMediaBridgePlan::new(true, false).expect("one-way plan"),
    )
    .await
    .expect("directional bridge");
    adapter.set_renegotiated_audio(CodecInfo {
        name: "unsupported-test-codec".into(),
        clock_rate_hz: 16_000,
        channels: 1,
        fmtp: None,
    });

    assert!(matches!(
        orch.renegotiate_media(conn_a, CapabilityDescriptor::default())
            .await,
        Err(RvoipError::UnsupportedCodec(codec)) if codec == "unsupported-test-codec"
    ));
}

#[tokio::test]
async fn bridge_rejects_an_already_closed_target_before_consuming_sources() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    drop(stream_b.take_external_out());

    assert!(matches!(
        orch.bridge_connections(conn_a, conn_b).await,
        Err(RvoipError::InvalidState(
            "bridge media target is already closed"
        ))
    ));
    assert!(stream_a.in_rx.lock().unwrap().is_some());
    assert!(stream_b.in_rx.lock().unwrap().is_some());
    assert_eq!(stream_a.source_acquisitions(), 0);
    assert_eq!(stream_b.source_acquisitions(), 0);
}

#[tokio::test]
async fn unavailable_second_source_rolls_back_first_receiver_reservation() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let (first_stream, unavailable_stream) = if conn_a < conn_b {
        (&stream_a, &stream_b)
    } else {
        (&stream_b, &stream_a)
    };
    let _unavailable_receiver = unavailable_stream
        .try_frames_in()
        .expect("pre-acquire the stable-order second receiver");

    assert!(matches!(
        orch.bridge_connections(conn_a, conn_b).await,
        Err(RvoipError::InvalidState(
            "mock media source receiver was already acquired"
        ))
    ));
    assert_eq!(
        first_stream.source_acquisitions(),
        0,
        "a rolled-back reservation is not a destructive acquisition"
    );
    assert!(
        first_stream.in_rx.lock().unwrap().is_some(),
        "the first receiver must be restored when the second reservation fails"
    );

    first_stream.inject(mk_frame(first_stream.id(), 73)).await;
    let mut restored = first_stream
        .try_frames_in()
        .expect("restored receiver remains usable");
    let frame = tokio::time::timeout(Duration::from_secs(2), restored.recv())
        .await
        .expect("restored receiver timed out")
        .expect("restored receiver closed");
    assert_eq!(frame.payload[0], 73);
}

#[tokio::test]
async fn ai_cancels_synthesized_playback_when_media_output_is_not_writable() {
    let (orch, stream, _other, conn, _other_conn) =
        setup_two_connection_orchestrator("opus", "opus").await;
    stream.set_writable(false);
    let cancellations = Arc::new(AtomicUsize::new(0));
    orch.register_asr_provider("cancel-output", Arc::new(OneResultAsrProvider));
    orch.register_tts_provider(
        "cancel-output",
        Arc::new(CountingTtsProvider {
            cancellations: Arc::clone(&cancellations),
        }),
    );
    orch.register_dialog_manager("cancel-output", Arc::new(SayDialog));

    let attachment = orch
        .attach_ai(conn, "cancel-output", std::collections::HashMap::new())
        .await
        .expect("AI attachment");
    wait_for_cancellations(&cancellations, 1).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(cancellations.load(Ordering::Acquire), 1);
    orch.detach(AttachmentRef::Ai(attachment))
        .await
        .expect("detach AI");
}

#[tokio::test]
async fn ai_cancels_synthesized_playback_when_media_output_closes() {
    let (orch, stream, _other, conn, _other_conn) =
        setup_two_connection_orchestrator("opus", "opus").await;
    drop(stream.take_external_out());
    let cancellations = Arc::new(AtomicUsize::new(0));
    orch.register_asr_provider("cancel-closed", Arc::new(OneResultAsrProvider));
    orch.register_tts_provider(
        "cancel-closed",
        Arc::new(CountingTtsProvider {
            cancellations: Arc::clone(&cancellations),
        }),
    );
    orch.register_dialog_manager("cancel-closed", Arc::new(SayDialog));

    let attachment = orch
        .attach_ai(conn, "cancel-closed", std::collections::HashMap::new())
        .await
        .expect("AI attachment");
    wait_for_cancellations(&cancellations, 1).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(cancellations.load(Ordering::Acquire), 1);
    orch.detach(AttachmentRef::Ai(attachment))
        .await
        .expect("detach AI");
}

#[tokio::test]
async fn ai_detach_cancels_in_flight_synthesized_playback() {
    let (orch, stream, _other, conn, _other_conn) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let mut output = stream.take_external_out();
    let cancellations = Arc::new(AtomicUsize::new(0));
    orch.register_asr_provider("cancel-detach", Arc::new(OneResultAsrProvider));
    orch.register_tts_provider(
        "cancel-detach",
        Arc::new(CountingTtsProvider {
            cancellations: Arc::clone(&cancellations),
        }),
    );
    orch.register_dialog_manager("cancel-detach", Arc::new(SayDialog));

    let attachment = orch
        .attach_ai(conn, "cancel-detach", std::collections::HashMap::new())
        .await
        .expect("AI attachment");
    tokio::time::timeout(Duration::from_secs(2), output.recv())
        .await
        .expect("playback frame deadline")
        .expect("playback output closed");
    orch.detach(AttachmentRef::Ai(attachment))
        .await
        .expect("detach AI");
    wait_for_cancellations(&cancellations, 1).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(cancellations.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn bridge_self_returns_error() {
    let (orch, _a, _b, conn_a, _) = setup_two_connection_orchestrator("opus", "opus").await;
    let err = orch
        .bridge_connections(conn_a.clone(), conn_a.clone())
        .await
        .unwrap_err();
    matches!(err, RvoipError::AdmissionRejected(_));
}

#[tokio::test]
async fn bridge_connection_not_found_returns_error() {
    let (orch, _a, _b, conn_a, _) = setup_two_connection_orchestrator("opus", "opus").await;
    let unknown = ConnectionId::new();
    let err = orch
        .bridge_connections(conn_a, unknown.clone())
        .await
        .unwrap_err();
    matches!(err, RvoipError::ConnectionNotFound(_));
}

#[tokio::test]
async fn bridge_already_bridged_returns_error() {
    let (orch, _a, _b, conn_a, conn_b) = setup_two_connection_orchestrator("opus", "opus").await;
    orch.bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect("first bridge");
    let err = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .unwrap_err();
    matches!(err, RvoipError::AdmissionRejected(_));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_bridge_attempts_reserve_both_connections_atomically() {
    let (orch, _a, _b, conn_a, conn_b) = setup_two_connection_orchestrator("opus", "opus").await;
    let barrier = Arc::new(Barrier::new(33));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let orch = Arc::clone(&orch);
        let conn_a = conn_a.clone();
        let conn_b = conn_b.clone();
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            orch.bridge_connections(conn_a, conn_b).await
        });
    }
    barrier.wait().await;

    let mut successes = Vec::new();
    let mut rejected = 0;
    while let Some(result) = tasks.join_next().await {
        match result.expect("bridge task") {
            Ok(bridge_id) => successes.push(bridge_id),
            Err(RvoipError::AdmissionRejected("connection already bridged")) => rejected += 1,
            Err(error) => panic!("unexpected bridge result: {error}"),
        }
    }
    assert_eq!(successes.len(), 1);
    assert_eq!(rejected, 31);

    let a_graph = orch
        .media_graph_for_connection(conn_a)
        .await
        .expect("A graph");
    let b_graph = orch
        .media_graph_for_connection(conn_b)
        .await
        .expect("B graph");
    assert_eq!(a_graph.snapshot().await.sinks.len(), 1);
    assert_eq!(b_graph.snapshot().await.sinks.len(), 1);
    orch.unbridge_connections(successes.pop().unwrap())
        .await
        .expect("unbridge winner");
    assert!(a_graph.latest_snapshot().sinks.is_empty());
    assert!(b_graph.latest_snapshot().sinks.is_empty());
}

#[tokio::test]
async fn unbridge_aborts_pumps_and_emits_event() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let mut events = orch.subscribe_events();
    let mut b_out = stream_b.take_external_out();

    let bridge_id = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect("bridge");
    let a_graph = orch
        .media_graph_for_connection(conn_a.clone())
        .await
        .expect("A graph");
    let b_graph = orch
        .media_graph_for_connection(conn_b.clone())
        .await
        .expect("B graph");

    // Confirm one frame propagates.
    stream_a.inject(mk_frame(stream_a.id(), 7)).await;
    let frame = tokio::time::timeout(Duration::from_secs(2), b_out.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(frame.payload[0], 7);

    // Unbridge.
    orch.unbridge_connections(bridge_id.clone())
        .await
        .expect("unbridge");
    assert!(
        a_graph.latest_snapshot().sinks.is_empty(),
        "unbridge acknowledgement must follow A route removal"
    );
    assert!(
        b_graph.latest_snapshot().sinks.is_empty(),
        "unbridge acknowledgement must follow B route removal"
    );

    // The pump task is aborted; subsequent injects don't propagate.
    stream_a.inject(mk_frame(stream_a.id(), 99)).await;
    let result = tokio::time::timeout(Duration::from_millis(200), b_out.recv()).await;
    assert!(
        result.is_err(),
        "no frame should arrive after unbridge (got {:?})",
        result.ok().flatten().map(|f| f.payload[0])
    );

    // Look for ConnectionsUnbridged on the event bus.
    let mut saw = false;
    for _ in 0..30 {
        match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Ok(Event::ConnectionsUnbridged { bridge_id: bid, .. })) if bid == bridge_id => {
                saw = true;
                break;
            }
            Ok(Ok(_)) => continue,
            Ok(Err(_)) | Err(_) => continue,
        }
    }
    assert!(saw, "expected Event::ConnectionsUnbridged within 3s");
}

#[tokio::test]
async fn full_media_target_is_bounded_and_never_backpressures_the_source() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let b_out = stream_b.take_external_out();
    let bridge_id = orch
        .bridge_connections(conn_a.clone(), conn_b)
        .await
        .expect("bridge");
    let source_graph = orch
        .media_graph_for_connection(conn_a)
        .await
        .expect("source graph");

    const FRAME_COUNT: usize = 400;
    tokio::time::timeout(Duration::from_secs(2), async {
        for value in 0..FRAME_COUNT {
            stream_a.inject(mk_frame(stream_a.id(), value as u8)).await;
        }
    })
    .await
    .expect("a full target must not apply unbounded backpressure to the source");

    let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = source_graph.snapshot().await;
            if snapshot.source_frames >= FRAME_COUNT as u64
                && (snapshot.dropped_frames > 0 || snapshot.evictions > 0)
            {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bounded sink drop/eviction diagnostics did not converge");
    assert!(snapshot.source_frames >= FRAME_COUNT as u64);
    assert!(snapshot.dropped_frames > 0 || snapshot.evictions > 0);
    assert!(
        b_out.len() <= 64,
        "the transport-facing queue must remain at its configured bound"
    );
    assert_eq!(stream_a.source_acquisitions(), 1);

    match orch.unbridge_connections(bridge_id).await {
        Ok(()) | Err(RvoipError::BridgeNotFound(_)) => {}
        Err(error) => panic!("unexpected bridge cleanup error: {error}"),
    }
}

#[tokio::test]
async fn terminal_bridge_route_removes_owner_and_allows_rebridge() {
    let (orch, stream_a, stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("opus", "opus").await;
    let mut events = orch.subscribe_events();
    let closed_target = stream_b.take_external_out();
    let first = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect("first bridge");
    drop(closed_target);

    stream_a.inject(mk_frame(stream_a.id(), 1)).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await {
                Ok(Event::ConnectionsUnbridged { bridge_id, .. }) if bridge_id == first => return,
                Ok(_) => continue,
                Err(error) => panic!("event stream closed: {error}"),
            }
        }
    })
    .await
    .expect("terminal route did not remove bridge owner");

    let replacement_b = MockMediaStream::new("opus");
    let _replacement_out = replacement_b.take_external_out();
    adapter.register_connection(conn_b.clone(), replacement_b);
    let second = orch
        .bridge_connections(conn_a, conn_b)
        .await
        .expect("bridge ownership was not released");
    orch.unbridge_connections(second)
        .await
        .expect("remove replacement bridge");
}

#[tokio::test]
async fn concurrent_graph_initialization_takes_source_once() {
    let (orch, stream_a, _stream_b, conn_a, _conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let orch = Arc::clone(&orch);
        let conn_a = conn_a.clone();
        tasks.spawn(async move {
            orch.media_graph_for_connection(conn_a)
                .await
                .expect("graph")
                .id()
                .to_string()
        });
    }

    let mut graph_ids = Vec::new();
    while let Some(result) = tasks.join_next().await {
        graph_ids.push(result.expect("join"));
    }
    assert_eq!(graph_ids.len(), 32);
    assert!(graph_ids.iter().all(|id| id == &graph_ids[0]));
    assert_eq!(
        stream_a.source_acquisitions(),
        1,
        "concurrent graph users must share the one authoritative source receiver"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn directional_bridge_and_concurrent_graph_users_share_each_source_once() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let barrier = Arc::new(Barrier::new(34));
    let mut graph_tasks = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let orch = Arc::clone(&orch);
        let conn_a = conn_a.clone();
        let barrier = Arc::clone(&barrier);
        graph_tasks.spawn(async move {
            barrier.wait().await;
            orch.media_graph_for_connection(conn_a)
                .await
                .expect("concurrent graph")
                .id()
                .to_string()
        });
    }
    let bridge_task = {
        let orch = Arc::clone(&orch);
        let conn_a = conn_a.clone();
        let conn_b = conn_b.clone();
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            orch.bridge_connections_directional(
                conn_a,
                conn_b,
                DirectionalMediaBridgePlan::bidirectional(),
            )
            .await
        })
    };
    barrier.wait().await;

    let bridge = bridge_task
        .await
        .expect("bridge task")
        .expect("directional bridge");
    let mut graph_ids = Vec::new();
    while let Some(result) = graph_tasks.join_next().await {
        graph_ids.push(result.expect("graph task"));
    }
    assert_eq!(graph_ids.len(), 32);
    assert!(graph_ids.iter().all(|id| id == &graph_ids[0]));
    assert_eq!(stream_a.source_acquisitions(), 1);
    assert_eq!(stream_b.source_acquisitions(), 1);

    let a_graph = orch
        .media_graph_for_connection(conn_a)
        .await
        .expect("authoritative A graph");
    assert_eq!(a_graph.id().to_string(), graph_ids[0]);
    orch.unbridge_connections(bridge)
        .await
        .expect("remove concurrent bridge");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_graph_init_for_one_connection_does_not_block_another() {
    let adapter = MockAdapter::new(Transport::Quic);
    let conn_a = ConnectionId::new();
    let conn_b = ConnectionId::new();
    let stream_a = MockMediaStream::new("opus");
    let stream_b = MockMediaStream::new("opus");
    adapter.register_connection(conn_a.clone(), stream_a);
    adapter.register_connection(conn_b.clone(), stream_b);
    let gate = adapter.gate_next_stream_lookup(conn_a.clone());

    let orch = Orchestrator::new(Config::default());
    orch.register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register");
    let session = SessionId::new();
    adapter.announce(conn_a.clone(), session.clone()).await;
    adapter.announce(conn_b.clone(), session).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let slow = {
        let orch = Arc::clone(&orch);
        tokio::spawn(async move { orch.media_graph_for_connection(conn_a).await })
    };
    gate.entered.notified().await;
    tokio::time::timeout(
        Duration::from_millis(250),
        orch.media_graph_for_connection(conn_b),
    )
    .await
    .expect("independent graph init was blocked by connection A")
    .expect("B graph");
    gate.release.notify_one();
    slow.await.expect("slow init task").expect("A graph");
}

#[tokio::test]
async fn bridge_recording_ai_and_listener_share_one_source_and_cleanup_routes() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let mut b_out = stream_b.take_external_out();
    let bridge_id = orch
        .bridge_connections(conn_a.clone(), conn_b)
        .await
        .expect("bridge");

    let recording = Arc::new(VecRecordingSink::new("memory:rec/fanout"));
    orch.register_recording_sink("fanout", recording.clone());
    let recording_id = orch
        .start_recording(RecordingTarget::Connection(conn_a.clone()), "fanout")
        .await
        .expect("recording");

    let asr_pushes = Arc::new(AtomicUsize::new(0));
    orch.register_asr_provider(
        "fanout",
        Arc::new(CountingAsrProvider {
            pushes: Arc::clone(&asr_pushes),
        }),
    );
    orch.register_tts_provider("fanout", Arc::new(NoOpTtsProvider));
    orch.register_dialog_manager("fanout", Arc::new(ListenOnlyDialog));
    let ai_id = orch
        .attach_ai(conn_a.clone(), "fanout", std::collections::HashMap::new())
        .await
        .expect("AI attachment");

    let listener_id = orch
        .attach_listener(
            ListenerTarget::Connection(conn_a.clone()),
            ListenerSink::Channel,
        )
        .expect("listener");
    let mut listener = orch
        .listener_channel(&listener_id)
        .expect("listener channel");

    let graph = orch
        .media_graph_for_connection(conn_a.clone())
        .await
        .expect("source graph");
    wait_for_sink_count(&graph, 4).await;
    assert_eq!(stream_a.source_acquisitions(), 1);

    stream_a.inject(mk_frame(stream_a.id(), 42)).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), b_out.recv())
            .await
            .expect("bridge timeout")
            .expect("bridge closed")
            .payload[0],
        42
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), listener.recv())
            .await
            .expect("listener timeout")
            .expect("listener closed")
            .payload[0],
        42
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while recording.bytes().is_empty() || asr_pushes.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("observer fanout timeout");

    orch.stop_recording(recording_id)
        .await
        .expect("stop recording");
    orch.detach(AttachmentRef::Ai(ai_id))
        .await
        .expect("detach AI");
    orch.detach(AttachmentRef::Listener(listener_id))
        .await
        .expect("detach listener");
    wait_for_sink_count(&graph, 1).await;

    orch.unbridge_connections(bridge_id)
        .await
        .expect("unbridge");
    wait_for_sink_count(&graph, 0).await;
}
