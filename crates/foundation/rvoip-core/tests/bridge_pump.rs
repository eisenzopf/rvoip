//! Integration tests for `Orchestrator::bridge_connections` /
//! `unbridge_connections` (the cross-transport frame-pump path).
//!
//! Uses an inline `MockAdapter` + `MockMediaStream` so the test is
//! self-contained — no SIP / QUIC / WebSocket setup needed.

use std::future::{poll_fn, Future};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::task::Poll;
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
use rvoip_core::ids::{BridgeId, ConnectionId, MessageId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::DEFAULT_BRIDGED_DATA_MESSAGE_QUEUE_CAPACITY;
#[cfg(feature = "test-hooks")]
use rvoip_core::orchestrator::{ReplacementPreparationBoundary, ReplacementPreparationTestGate};
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

// The default-feature shard intentionally has no native codec dependency.
// Keep its transport/graph behavior tests on the always-available G.711
// backend; the all-features codec gate separately exercises Opus and AMR.
const DEFAULT_TEST_CODEC: &str = "PCMU";

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
        Self::with_output_capacity(codec_name, 64)
    }

    fn with_output_capacity(codec_name: &str, output_capacity: usize) -> Arc<Self> {
        let (external_in_tx, in_rx) = mpsc::channel::<MediaFrame>(64);
        let (out_tx, external_out_rx) = mpsc::channel::<MediaFrame>(output_capacity);
        let clock_rate_hz = match codec_name.to_ascii_lowercase().as_str() {
            "opus" => 48_000,
            "amr-wb" => 16_000,
            _ => 8_000,
        };
        Arc::new(Self {
            id: StreamId::new(),
            codec: CodecInfo {
                name: codec_name.to_string(),
                clock_rate_hz,
                channels: 1,
                fmtp: None,
                payload_type: None,
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
    empty_stream_lookups: dashmap::DashSet<ConnectionId>,
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
            empty_stream_lookups: dashmap::DashSet::new(),
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

    #[cfg(feature = "test-hooks")]
    fn return_no_stream_once(&self, id: ConnectionId) {
        self.empty_stream_lookups.insert(id);
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

    async fn announce_end(&self, id: ConnectionId) {
        self.events_tx
            .send(AdapterEvent::Ended {
                connection_id: id,
                reason: AdapterEndReason::Normal,
            })
            .await
            .expect("announce mock connection end");
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
        if self.empty_stream_lookups.remove(&c).is_some() {
            return Ok(Vec::new());
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

fn active_bridge_count(orchestrator: &Orchestrator) -> usize {
    match orchestrator.capacity_report() {
        Event::CapacityReport { active_bridges, .. } => active_bridges as usize,
        _ => unreachable!("capacity_report returns a capacity event"),
    }
}

async fn wait_for_active_bridge_count(orchestrator: &Orchestrator, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while active_bridge_count(orchestrator) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("active bridge count did not converge");
}

async fn wait_for_connection_retirement(orchestrator: &Orchestrator, connection_id: &ConnectionId) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while orchestrator.connection_transport(connection_id).is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("connection retirement did not converge");
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

async fn setup_amazon_connect_bridge_orchestrator() -> (
    Arc<Orchestrator>,
    Arc<MockMediaStream>,
    Arc<MockMediaStream>,
    ConnectionId,
    ConnectionId,
) {
    let sip_adapter = MockAdapter::new(Transport::Sip);
    let connect_adapter = MockAdapter::new(Transport::AmazonConnect);
    let sip_connection = ConnectionId::new();
    let connect_connection = ConnectionId::new();
    let sip_stream = MockMediaStream::new(DEFAULT_TEST_CODEC);
    let connect_stream = MockMediaStream::with_output_capacity(DEFAULT_TEST_CODEC, 1);
    sip_adapter.register_connection(sip_connection.clone(), Arc::clone(&sip_stream));
    connect_adapter.register_connection(connect_connection.clone(), Arc::clone(&connect_stream));

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(sip_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register SIP adapter");
    orchestrator
        .register(connect_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register Amazon Connect adapter");

    let session = SessionId::new();
    sip_adapter
        .announce(sip_connection.clone(), session.clone())
        .await;
    connect_adapter
        .announce(connect_connection.clone(), session)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    (
        orchestrator,
        sip_stream,
        connect_stream,
        sip_connection,
        connect_connection,
    )
}

/// Build one Session whose ingress, current service destination, and
/// replacement service destination are owned by three different adapters.
/// This mirrors the SIP -> WebRTC -> in-process-AI handoff that exercises
/// destination replacement in production without requiring live transports.
async fn setup_cross_transport_replacement_orchestrator() -> (
    Arc<Orchestrator>,
    Arc<MockMediaStream>,
    Arc<MockMediaStream>,
    Arc<MockMediaStream>,
    ConnectionId,
    ConnectionId,
    ConnectionId,
) {
    let sip_adapter = MockAdapter::new(Transport::Sip);
    let webrtc_adapter = MockAdapter::new(Transport::WebRtc);
    let ai_adapter = MockAdapter::new(Transport::InProcessAi);
    let ingress = ConnectionId::new();
    let current_destination = ConnectionId::new();
    let replacement_destination = ConnectionId::new();
    let ingress_stream = MockMediaStream::new(DEFAULT_TEST_CODEC);
    let current_stream = MockMediaStream::new(DEFAULT_TEST_CODEC);
    let replacement_stream = MockMediaStream::new(DEFAULT_TEST_CODEC);
    sip_adapter.register_connection(ingress.clone(), Arc::clone(&ingress_stream));
    webrtc_adapter.register_connection(current_destination.clone(), Arc::clone(&current_stream));
    ai_adapter.register_connection(
        replacement_destination.clone(),
        Arc::clone(&replacement_stream),
    );

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(sip_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register SIP adapter");
    orchestrator
        .register(webrtc_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register WebRTC adapter");
    orchestrator
        .register(ai_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register in-process AI adapter");

    let session = SessionId::new();
    sip_adapter.announce(ingress.clone(), session.clone()).await;
    webrtc_adapter
        .announce(current_destination.clone(), session.clone())
        .await;
    ai_adapter
        .announce(replacement_destination.clone(), session)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    (
        orchestrator,
        ingress_stream,
        current_stream,
        replacement_stream,
        ingress,
        current_destination,
        replacement_destination,
    )
}

struct ReplacementFaultFixture {
    orchestrator: Arc<Orchestrator>,
    ingress_adapter: Arc<MockAdapter>,
    current_adapter: Arc<MockAdapter>,
    replacement_adapter: Arc<MockAdapter>,
    ingress_stream: Arc<MockMediaStream>,
    current_stream: Arc<MockMediaStream>,
    replacement_stream: Arc<MockMediaStream>,
    ingress: ConnectionId,
    current_destination: ConnectionId,
    replacement_destination: ConnectionId,
}

impl ReplacementFaultFixture {
    fn adapter_for(&self, target: ReplacementEndpoint) -> &Arc<MockAdapter> {
        match target {
            ReplacementEndpoint::Source => &self.ingress_adapter,
            ReplacementEndpoint::OldDestination => &self.current_adapter,
            ReplacementEndpoint::PendingDestination => &self.replacement_adapter,
        }
    }

    fn connection_for(&self, target: ReplacementEndpoint) -> &ConnectionId {
        match target {
            ReplacementEndpoint::Source => &self.ingress,
            ReplacementEndpoint::OldDestination => &self.current_destination,
            ReplacementEndpoint::PendingDestination => &self.replacement_destination,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ReplacementEndpoint {
    Source,
    OldDestination,
    PendingDestination,
}

#[derive(Clone, Copy, Debug)]
enum ReplacementAwaitBoundary {
    SourceStreamLookup,
    PendingStreamLookup,
    #[cfg(feature = "test-hooks")]
    StreamAvailabilityRetryDelay,
    #[cfg(feature = "test-hooks")]
    SourceMediaGraphInitLock,
    #[cfg(feature = "test-hooks")]
    PendingMediaGraphInitLock,
    #[cfg(feature = "test-hooks")]
    SourceToPendingRouteActivation,
    #[cfg(feature = "test-hooks")]
    PendingToSourceRouteActivation,
}

enum ReplacementAwaitGate {
    Stream(Arc<StreamLookupGate>),
    #[cfg(feature = "test-hooks")]
    Core(Arc<ReplacementPreparationTestGate>),
}

impl ReplacementAwaitGate {
    async fn wait_until_blocked(&self) {
        match self {
            Self::Stream(gate) => {
                tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
                    .await
                    .expect("replacement did not reach stream lookup gate");
            }
            #[cfg(feature = "test-hooks")]
            Self::Core(gate) => gate
                .wait_until_blocked()
                .await
                .expect("replacement did not reach core preparation gate"),
        }
    }

    fn release(&self) {
        match self {
            Self::Stream(gate) => gate.release.notify_one(),
            #[cfg(feature = "test-hooks")]
            Self::Core(gate) => gate.release(),
        }
    }
}

async fn setup_replacement_fault_fixture() -> ReplacementFaultFixture {
    let ingress_adapter = MockAdapter::new(Transport::Sip);
    let current_adapter = MockAdapter::new(Transport::WebRtc);
    let replacement_adapter = MockAdapter::new(Transport::InProcessAi);
    let ingress = ConnectionId::new();
    let current_destination = ConnectionId::new();
    let replacement_destination = ConnectionId::new();
    let ingress_stream = MockMediaStream::new(DEFAULT_TEST_CODEC);
    let current_stream = MockMediaStream::new(DEFAULT_TEST_CODEC);
    let replacement_stream = MockMediaStream::new(DEFAULT_TEST_CODEC);
    ingress_adapter.register_connection(ingress.clone(), Arc::clone(&ingress_stream));
    current_adapter.register_connection(current_destination.clone(), Arc::clone(&current_stream));
    replacement_adapter.register_connection(
        replacement_destination.clone(),
        Arc::clone(&replacement_stream),
    );

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(ingress_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register fault-fixture source adapter");
    orchestrator
        .register(current_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register fault-fixture old adapter");
    orchestrator
        .register(replacement_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register fault-fixture pending adapter");
    let mut events = orchestrator.subscribe_events();

    let session = SessionId::new();
    ingress_adapter
        .announce(ingress.clone(), session.clone())
        .await;
    current_adapter
        .announce(current_destination.clone(), session.clone())
        .await;
    replacement_adapter
        .announce(replacement_destination.clone(), session)
        .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut ingress_published = false;
        let mut current_published = false;
        let mut replacement_published = false;
        while !(ingress_published && current_published && replacement_published) {
            if let Event::ConnectionInbound { connection_id, .. } =
                events.recv().await.expect("core event bus closed")
            {
                ingress_published |= connection_id == ingress;
                current_published |= connection_id == current_destination;
                replacement_published |= connection_id == replacement_destination;
            }
        }
    })
    .await
    .expect("replacement fault fixture was not admitted");

    ReplacementFaultFixture {
        orchestrator,
        ingress_adapter,
        current_adapter,
        replacement_adapter,
        ingress_stream,
        current_stream,
        replacement_stream,
        ingress,
        current_destination,
        replacement_destination,
    }
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;

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
async fn bridge_destination_replacement_cuts_media_over_and_fences_stale_generation() {
    let (
        orch,
        ingress_stream,
        current_stream,
        replacement_stream,
        ingress,
        current_destination,
        replacement_destination,
    ) = setup_cross_transport_replacement_orchestrator().await;
    let mut ingress_out = ingress_stream.take_external_out();
    let mut current_out = current_stream.take_external_out();
    let mut replacement_out = replacement_stream.take_external_out();
    let original_bridge = orch
        .bridge_connections(ingress.clone(), current_destination.clone())
        .await
        .expect("initial SIP-to-WebRTC bridge");

    ingress_stream
        .inject(mk_frame(ingress_stream.id(), 1))
        .await;
    let before_cutover = tokio::time::timeout(Duration::from_secs(2), current_out.recv())
        .await
        .expect("current destination did not receive pre-cutover media")
        .expect("current destination output closed");
    assert_eq!(before_cutover.payload[0], 1);

    let replacement = orch
        .replace_bridge_destination(
            original_bridge.clone(),
            ingress.clone(),
            current_destination.clone(),
            replacement_destination.clone(),
        )
        .await
        .expect("replace WebRTC destination with in-process AI");
    assert_ne!(replacement.bridge_id, original_bridge);
    assert_eq!(replacement.previous_bridge_id, original_bridge);
    assert_eq!(replacement.ingress, ingress);
    assert_eq!(replacement.previous_destination, current_destination);
    assert_eq!(replacement.destination, replacement_destination);

    ingress_stream
        .inject(mk_frame(ingress_stream.id(), 2))
        .await;
    let after_cutover = tokio::time::timeout(Duration::from_secs(2), replacement_out.recv())
        .await
        .expect("replacement destination did not receive post-cutover media")
        .expect("replacement destination output closed");
    assert_eq!(after_cutover.payload[0], 2);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), current_out.recv())
            .await
            .is_err(),
        "retired destination continued receiving ingress media"
    );

    replacement_stream
        .inject(mk_frame(replacement_stream.id(), 3))
        .await;
    let reverse = tokio::time::timeout(Duration::from_secs(2), ingress_out.recv())
        .await
        .expect("ingress did not receive replacement return media")
        .expect("ingress output closed");
    assert_eq!(reverse.payload[0], 3);

    assert!(matches!(
        orch.replace_bridge_destination(
            original_bridge.clone(),
            ingress.clone(),
            current_destination,
            replacement_destination.clone(),
        )
        .await,
        Err(RvoipError::BridgeNotFound(id)) if id == original_bridge
    ));

    ingress_stream
        .inject(mk_frame(ingress_stream.id(), 4))
        .await;
    let after_stale_attempt = tokio::time::timeout(Duration::from_secs(2), replacement_out.recv())
        .await
        .expect("stale replacement disturbed the committed bridge")
        .expect("replacement destination output closed");
    assert_eq!(after_stale_attempt.payload[0], 4);

    orch.unbridge_connections(replacement.bridge_id)
        .await
        .expect("remove replacement bridge");
}

#[tokio::test(flavor = "current_thread")]
async fn committed_bridge_replacement_returns_before_retired_cleanup_can_be_cancelled() {
    let (
        orch,
        ingress_stream,
        _current_stream,
        replacement_stream,
        ingress,
        current_destination,
        replacement_destination,
    ) = setup_cross_transport_replacement_orchestrator().await;
    let mut replacement_out = replacement_stream.take_external_out();
    let original_bridge = orch
        .bridge_connections(ingress.clone(), current_destination.clone())
        .await
        .expect("initial bridge");
    // Subscribe after the initial generation so the first relevant event is
    // produced by replacement itself.
    let mut events = orch.subscribe_events();
    let replacement = orch.replace_bridge_destination(
        original_bridge.clone(),
        ingress.clone(),
        current_destination,
        replacement_destination,
    );
    tokio::pin!(replacement);

    let receipt = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                biased;
                result = &mut replacement => break result.expect("replacement"),
                event = events.recv() => {
                    if matches!(
                        event.expect("core event bus"),
                        Event::ConnectionsUnbridged { bridge_id, .. }
                            if bridge_id == original_bridge
                    ) {
                        panic!(
                            "a committed replacement yielded before returning its generation receipt"
                        );
                    }
                }
            }
        }
    })
    .await
    .expect("replacement did not return its commit receipt");

    ingress_stream
        .inject(mk_frame(ingress_stream.id(), 9))
        .await;
    let forwarded = tokio::time::timeout(Duration::from_secs(2), replacement_out.recv())
        .await
        .expect("replacement did not receive media")
        .expect("replacement output closed");
    assert_eq!(forwarded.payload[0], 9);

    orch.unbridge_connections(receipt.bridge_id)
        .await
        .expect("remove replacement bridge");
    orch.drain_connection_lifecycle_tasks().await;
}

#[tokio::test(flavor = "current_thread")]
async fn bridge_replacement_fences_blocked_data_for_the_retired_destination() {
    let adapter = MockAdapter::new(Transport::Quic);
    let ingress = ConnectionId::new();
    let retired_destination = ConnectionId::new();
    let replacement_destination = ConnectionId::new();
    for connection_id in [
        ingress.clone(),
        retired_destination.clone(),
        replacement_destination.clone(),
    ] {
        adapter.register_connection(connection_id, MockMediaStream::new(DEFAULT_TEST_CODEC));
    }

    let orch = Orchestrator::new(Config::default());
    orch.register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");
    let session = SessionId::new();
    adapter.announce(ingress.clone(), session.clone()).await;
    adapter
        .announce(retired_destination.clone(), session.clone())
        .await;
    adapter
        .announce(replacement_destination.clone(), session)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let original_bridge = orch
        .bridge_connections(ingress.clone(), retired_destination.clone())
        .await
        .expect("initial bridge");
    let blocked_send = adapter.gate_data_send(retired_destination.clone());
    let entered = blocked_send.entered.notified();
    tokio::pin!(entered);
    entered.as_mut().enable();
    let stale = DataMessage::reliable("stale", "text/plain", "old generation");
    adapter
        .events_tx
        .send(AdapterEvent::DataMessage {
            connection_id: ingress.clone(),
            message: stale,
        })
        .await
        .expect("queue data for retired destination");
    tokio::time::timeout(Duration::from_secs(2), &mut entered)
        .await
        .expect("old data worker did not enter blocked send");

    let replacement = orch
        .replace_bridge_destination(
            original_bridge,
            ingress.clone(),
            retired_destination.clone(),
            replacement_destination.clone(),
        )
        .await
        .expect("replace destination");

    // Releasing the adapter-side send after the replacement receipt must not
    // allow work retained by the old generation to reach its former peer.
    blocked_send.release();
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert!(
        adapter.sent_data_messages().is_empty(),
        "retired bridge data crossed the replacement boundary"
    );

    let fresh = DataMessage::reliable("fresh", "text/plain", "new generation");
    adapter
        .events_tx
        .send(AdapterEvent::DataMessage {
            connection_id: ingress,
            message: fresh.clone(),
        })
        .await
        .expect("queue data for replacement destination");
    wait_for_data_message_count(&adapter, 1).await;
    assert_eq!(
        adapter.sent_data_messages(),
        vec![(replacement_destination, fresh)]
    );

    orch.unbridge_connections(replacement.bridge_id)
        .await
        .expect("remove replacement bridge");
    orch.drain_connection_lifecycle_tasks().await;
}

#[tokio::test]
async fn failed_bridge_destination_replacement_rolls_back_and_can_retry() {
    let (
        orch,
        ingress_stream,
        current_stream,
        replacement_stream,
        ingress,
        current_destination,
        replacement_destination,
    ) = setup_cross_transport_replacement_orchestrator().await;
    let mut current_out = current_stream.take_external_out();
    replacement_stream.set_writable(false);
    let original_bridge = orch
        .bridge_connections(ingress.clone(), current_destination.clone())
        .await
        .expect("initial bridge");

    assert!(matches!(
        orch.replace_bridge_destination(
            original_bridge.clone(),
            ingress.clone(),
            current_destination.clone(),
            replacement_destination.clone(),
        )
        .await,
        Err(RvoipError::InvalidState(
            "mock media stream is not activated"
        ))
    ));

    ingress_stream
        .inject(mk_frame(ingress_stream.id(), 5))
        .await;
    let retained = tokio::time::timeout(Duration::from_secs(2), current_out.recv())
        .await
        .expect("failed replacement disturbed the original bridge")
        .expect("current destination output closed");
    assert_eq!(retained.payload[0], 5);

    replacement_stream.set_writable(true);
    let replacement = orch
        .replace_bridge_destination(
            original_bridge,
            ingress,
            current_destination,
            replacement_destination,
        )
        .await
        .expect("failed preflight must release replacement reservation");
    orch.unbridge_connections(replacement.bridge_id)
        .await
        .expect("remove retried replacement bridge");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_endpoint_termination_matrix_compensates_precommit_awaits() {
    #[cfg(not(feature = "test-hooks"))]
    let boundaries = vec![
        ReplacementAwaitBoundary::SourceStreamLookup,
        ReplacementAwaitBoundary::PendingStreamLookup,
    ];
    #[cfg(feature = "test-hooks")]
    let boundaries = vec![
        ReplacementAwaitBoundary::SourceStreamLookup,
        ReplacementAwaitBoundary::PendingStreamLookup,
        ReplacementAwaitBoundary::StreamAvailabilityRetryDelay,
        ReplacementAwaitBoundary::SourceMediaGraphInitLock,
        ReplacementAwaitBoundary::PendingMediaGraphInitLock,
        ReplacementAwaitBoundary::SourceToPendingRouteActivation,
        ReplacementAwaitBoundary::PendingToSourceRouteActivation,
    ];
    let mut cases = 0_usize;
    for boundary in boundaries {
        for endpoint in [
            ReplacementEndpoint::Source,
            ReplacementEndpoint::OldDestination,
            ReplacementEndpoint::PendingDestination,
        ] {
            let fixture = setup_replacement_fault_fixture().await;
            let _ingress_output = fixture.ingress_stream.take_external_out();
            let mut current_output = fixture.current_stream.take_external_out();
            let _replacement_output = fixture.replacement_stream.take_external_out();
            let original_bridge = fixture
                .orchestrator
                .bridge_connections(fixture.ingress.clone(), fixture.current_destination.clone())
                .await
                .unwrap_or_else(|error| match error {
                    RvoipError::AdmissionRejected(reason) => {
                        panic!("initial bridge for termination matrix was rejected: {reason}")
                    }
                    error => panic!("initial bridge for termination matrix: {error:?}"),
                });
            let ingress_graph = fixture
                .orchestrator
                .media_graph_for_connection(fixture.ingress.clone())
                .await
                .expect("source graph");
            let current_graph = fixture
                .orchestrator
                .media_graph_for_connection(fixture.current_destination.clone())
                .await
                .expect("old destination graph");
            let gate = match boundary {
                ReplacementAwaitBoundary::SourceStreamLookup => ReplacementAwaitGate::Stream(
                    fixture
                        .ingress_adapter
                        .gate_next_stream_lookup(fixture.ingress.clone()),
                ),
                ReplacementAwaitBoundary::PendingStreamLookup => ReplacementAwaitGate::Stream(
                    fixture
                        .replacement_adapter
                        .gate_next_stream_lookup(fixture.replacement_destination.clone()),
                ),
                #[cfg(feature = "test-hooks")]
                ReplacementAwaitBoundary::StreamAvailabilityRetryDelay => {
                    fixture
                        .replacement_adapter
                        .return_no_stream_once(fixture.replacement_destination.clone());
                    ReplacementAwaitGate::Core(
                        fixture
                            .orchestrator
                            .install_replacement_preparation_test_gate(
                                ReplacementPreparationBoundary::StreamAvailabilityRetryDelay,
                                Duration::from_secs(5),
                            )
                            .expect("install stream-availability retry gate"),
                    )
                }
                #[cfg(feature = "test-hooks")]
                ReplacementAwaitBoundary::SourceMediaGraphInitLock => ReplacementAwaitGate::Core(
                    fixture
                        .orchestrator
                        .install_replacement_preparation_test_gate(
                            ReplacementPreparationBoundary::SourceMediaGraphInitLock,
                            Duration::from_secs(5),
                        )
                        .expect("install source graph-init gate"),
                ),
                #[cfg(feature = "test-hooks")]
                ReplacementAwaitBoundary::PendingMediaGraphInitLock => ReplacementAwaitGate::Core(
                    fixture
                        .orchestrator
                        .install_replacement_preparation_test_gate(
                            ReplacementPreparationBoundary::PendingMediaGraphInitLock,
                            Duration::from_secs(5),
                        )
                        .expect("install pending graph-init gate"),
                ),
                #[cfg(feature = "test-hooks")]
                ReplacementAwaitBoundary::SourceToPendingRouteActivation => {
                    ReplacementAwaitGate::Core(
                        fixture
                            .orchestrator
                            .install_replacement_preparation_test_gate(
                                ReplacementPreparationBoundary::SourceToPendingRouteActivation,
                                Duration::from_secs(5),
                            )
                            .expect("install source-to-pending route gate"),
                    )
                }
                #[cfg(feature = "test-hooks")]
                ReplacementAwaitBoundary::PendingToSourceRouteActivation => {
                    ReplacementAwaitGate::Core(
                        fixture
                            .orchestrator
                            .install_replacement_preparation_test_gate(
                                ReplacementPreparationBoundary::PendingToSourceRouteActivation,
                                Duration::from_secs(5),
                            )
                            .expect("install pending-to-source route gate"),
                    )
                }
            };
            let replacement = {
                let orchestrator = Arc::clone(&fixture.orchestrator);
                let ingress = fixture.ingress.clone();
                let old = fixture.current_destination.clone();
                let pending = fixture.replacement_destination.clone();
                let bridge = original_bridge.clone();
                tokio::spawn(async move {
                    orchestrator
                        .replace_bridge_destination(bridge, ingress, old, pending)
                        .await
                })
            };
            gate.wait_until_blocked().await;

            let ended_connection = fixture.connection_for(endpoint).clone();
            fixture
                .adapter_for(endpoint)
                .announce_end(ended_connection.clone())
                .await;
            wait_for_connection_retirement(&fixture.orchestrator, &ended_connection).await;
            gate.release();

            let result = tokio::time::timeout(Duration::from_secs(2), replacement)
                .await
                .unwrap_or_else(|_| {
                    panic!("replacement hung after {endpoint:?} ended at {boundary:?}")
                })
                .expect("replacement task panicked");
            assert!(
                result.is_err(),
                "replacement committed after {endpoint:?} ended at {boundary:?}"
            );

            match endpoint {
                ReplacementEndpoint::PendingDestination => {
                    wait_for_active_bridge_count(&fixture.orchestrator, 1).await;
                    fixture
                        .ingress_stream
                        .inject(mk_frame(fixture.ingress_stream.id(), 91))
                        .await;
                    let retained =
                        tokio::time::timeout(Duration::from_secs(2), current_output.recv())
                            .await
                            .expect("old destination did not retain media after pending loss")
                            .expect("old destination output closed");
                    assert_eq!(retained.payload[0], 91);
                    wait_for_sink_count(&ingress_graph, 1).await;
                    wait_for_sink_count(&current_graph, 1).await;
                    fixture
                        .orchestrator
                        .unbridge_connections(original_bridge)
                        .await
                        .expect("remove retained original bridge");
                }
                ReplacementEndpoint::Source | ReplacementEndpoint::OldDestination => {
                    wait_for_active_bridge_count(&fixture.orchestrator, 0).await;
                    wait_for_sink_count(&ingress_graph, 0).await;
                    wait_for_sink_count(&current_graph, 0).await;
                    let (surviving_source, surviving_destination) = match endpoint {
                        ReplacementEndpoint::Source => (
                            fixture.current_destination.clone(),
                            fixture.replacement_destination.clone(),
                        ),
                        ReplacementEndpoint::OldDestination => (
                            fixture.ingress.clone(),
                            fixture.replacement_destination.clone(),
                        ),
                        ReplacementEndpoint::PendingDestination => unreachable!(),
                    };
                    let retry = fixture
                        .orchestrator
                        .bridge_connections(surviving_source, surviving_destination)
                        .await
                        .unwrap_or_else(|error| {
                            panic!(
                                "replacement reservation leaked after {endpoint:?} ended at \
                                 {boundary:?}: {error:?}"
                            )
                        });
                    fixture
                        .orchestrator
                        .unbridge_connections(retry)
                        .await
                        .expect("remove reservation-compensation probe bridge");
                    wait_for_active_bridge_count(&fixture.orchestrator, 0).await;
                }
            }
            fixture
                .orchestrator
                .drain_connection_lifecycle_tasks()
                .await;
            assert_eq!(fixture.orchestrator.connection_lifecycle_task_count(), 0);
            cases += 1;
        }
    }
    #[cfg(feature = "test-hooks")]
    assert_eq!(cases, 21);
    #[cfg(not(feature = "test-hooks"))]
    assert_eq!(cases, 6);
    eprintln!(
        "{{\"kind\":\"replacement_endpoint_termination_precommit_await_matrix\",\"cases\":{cases},\"boundaries\":{},\"actors\":3}}",
        cases / 3
    );
}

#[tokio::test]
async fn replacement_second_direction_failure_removes_activated_candidate_route() {
    let fixture = setup_replacement_fault_fixture().await;
    let _ingress_output = fixture.ingress_stream.take_external_out();
    let mut current_output = fixture.current_stream.take_external_out();
    let _replacement_output = fixture.replacement_stream.take_external_out();
    let original_bridge = fixture
        .orchestrator
        .bridge_connections(fixture.ingress.clone(), fixture.current_destination.clone())
        .await
        .expect("initial bridge for media compensation");
    let ingress_graph = fixture
        .orchestrator
        .media_graph_for_connection(fixture.ingress.clone())
        .await
        .expect("source graph");
    let replacement_graph = fixture
        .orchestrator
        .media_graph_for_connection(fixture.replacement_destination.clone())
        .await
        .expect("replacement source graph");

    let mut saturation_routes = Vec::new();
    let mut saturation_receivers = Vec::new();
    loop {
        let (target, receiver) = mpsc::channel(1);
        match replacement_graph.add_sink(fixture.replacement_stream.codec(), target) {
            Ok(route) => {
                saturation_routes.push(route);
                saturation_receivers.push(receiver);
                wait_for_sink_count(&replacement_graph, saturation_routes.len()).await;
            }
            Err(RvoipError::AdmissionRejected("media graph maximum sink count reached")) => break,
            Err(error) => panic!("unexpected media graph saturation error: {error}"),
        }
    }
    let saturated_sink_count = saturation_routes.len();
    assert!(saturated_sink_count > 0);
    wait_for_sink_count(&replacement_graph, saturated_sink_count).await;

    assert!(matches!(
        fixture
            .orchestrator
            .replace_bridge_destination(
                original_bridge.clone(),
                fixture.ingress.clone(),
                fixture.current_destination.clone(),
                fixture.replacement_destination.clone(),
            )
            .await,
        Err(RvoipError::AdmissionRejected(
            "media graph maximum sink count reached"
        ))
    ));
    wait_for_sink_count(&ingress_graph, 1).await;
    wait_for_sink_count(&replacement_graph, saturated_sink_count).await;
    wait_for_active_bridge_count(&fixture.orchestrator, 1).await;

    fixture
        .ingress_stream
        .inject(mk_frame(fixture.ingress_stream.id(), 92))
        .await;
    let retained = tokio::time::timeout(Duration::from_secs(2), current_output.recv())
        .await
        .expect("old destination did not retain media after route compensation")
        .expect("old destination output closed");
    assert_eq!(retained.payload[0], 92);

    for route in saturation_routes {
        replacement_graph
            .remove_sink_and_wait(route)
            .await
            .expect("remove saturation route");
    }
    drop(saturation_receivers);
    fixture
        .orchestrator
        .unbridge_connections(original_bridge)
        .await
        .expect("remove original bridge after compensation test");
    wait_for_sink_count(&ingress_graph, 0).await;
    wait_for_sink_count(&replacement_graph, 0).await;
}

#[tokio::test]
async fn promoted_bridge_survives_exact_old_destination_retirement() {
    let fixture = setup_replacement_fault_fixture().await;
    let _ingress_output = fixture.ingress_stream.take_external_out();
    let _current_output = fixture.current_stream.take_external_out();
    let mut replacement_output = fixture.replacement_stream.take_external_out();
    let original_bridge = fixture
        .orchestrator
        .bridge_connections(fixture.ingress.clone(), fixture.current_destination.clone())
        .await
        .expect("initial bridge for exact retirement");
    let replacement = fixture
        .orchestrator
        .replace_bridge_destination(
            original_bridge,
            fixture.ingress.clone(),
            fixture.current_destination.clone(),
            fixture.replacement_destination.clone(),
        )
        .await
        .expect("promote replacement destination");

    fixture
        .current_adapter
        .announce_end(fixture.current_destination.clone())
        .await;
    wait_for_connection_retirement(&fixture.orchestrator, &fixture.current_destination).await;
    wait_for_active_bridge_count(&fixture.orchestrator, 1).await;
    assert!(fixture
        .orchestrator
        .connection_transport(&fixture.ingress)
        .is_ok());
    assert!(fixture
        .orchestrator
        .connection_transport(&fixture.replacement_destination)
        .is_ok());

    fixture
        .ingress_stream
        .inject(mk_frame(fixture.ingress_stream.id(), 93))
        .await;
    let forwarded = tokio::time::timeout(Duration::from_secs(2), replacement_output.recv())
        .await
        .expect("replacement stopped after exact old retirement")
        .expect("replacement output closed");
    assert_eq!(forwarded.payload[0], 93);

    fixture
        .replacement_adapter
        .announce_end(fixture.replacement_destination.clone())
        .await;
    wait_for_connection_retirement(&fixture.orchestrator, &fixture.replacement_destination).await;
    wait_for_active_bridge_count(&fixture.orchestrator, 0).await;
    assert!(fixture
        .orchestrator
        .connection_transport(&fixture.ingress)
        .is_ok());
    assert!(matches!(
        fixture
            .orchestrator
            .unbridge_connections(replacement.bridge_id)
            .await,
        Err(RvoipError::BridgeNotFound(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seeded_replacement_glare_keeps_one_generation_and_converges() {
    const BASE_SEED: u64 = 0x5eed_c0de_d15c_a11e;
    const ROUNDS: usize = 64;

    let adapter = MockAdapter::new(Transport::Quic);
    let ingress = ConnectionId::new();
    let ingress_stream = MockMediaStream::new(DEFAULT_TEST_CODEC);
    adapter.register_connection(ingress.clone(), Arc::clone(&ingress_stream));
    let destinations = (0..3).map(|_| ConnectionId::new()).collect::<Vec<_>>();
    let destination_streams = (0..3)
        .map(|_| MockMediaStream::new(DEFAULT_TEST_CODEC))
        .collect::<Vec<_>>();
    for (connection_id, stream) in destinations.iter().zip(&destination_streams) {
        adapter.register_connection(connection_id.clone(), Arc::clone(stream));
    }

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register glare adapter");
    let session = SessionId::new();
    adapter.announce(ingress.clone(), session.clone()).await;
    for connection_id in &destinations {
        adapter
            .announce(connection_id.clone(), session.clone())
            .await;
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while std::iter::once(&ingress)
            .chain(destinations.iter())
            .any(|connection_id| orchestrator.connection_transport(connection_id).is_err())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("glare fixture was not admitted");

    let mut ingress_output = ingress_stream.take_external_out();
    let mut destination_outputs = destination_streams
        .iter()
        .map(|stream| stream.take_external_out())
        .collect::<Vec<_>>();
    let ingress_graph = orchestrator
        .media_graph_for_connection(ingress.clone())
        .await
        .expect("glare ingress graph");
    let mut destination_graphs = Vec::new();
    for connection_id in &destinations {
        destination_graphs.push(
            orchestrator
                .media_graph_for_connection(connection_id.clone())
                .await
                .expect("glare destination graph"),
        );
    }

    let mut current = 0_usize;
    let mut bridge_id = orchestrator
        .bridge_connections(ingress.clone(), destinations[current].clone())
        .await
        .expect("initial glare bridge");
    let sampling = Arc::new(AtomicBool::new(true));
    let max_total_sinks = Arc::new(AtomicUsize::new(2));
    let max_lifecycle_tasks = Arc::new(AtomicUsize::new(
        orchestrator.connection_lifecycle_task_count(),
    ));
    let sampler = {
        let sampling = Arc::clone(&sampling);
        let max_total_sinks = Arc::clone(&max_total_sinks);
        let max_lifecycle_tasks = Arc::clone(&max_lifecycle_tasks);
        let orchestrator = Arc::clone(&orchestrator);
        let ingress_graph = ingress_graph.clone();
        let destination_graphs = destination_graphs.clone();
        tokio::spawn(async move {
            while sampling.load(Ordering::Acquire) {
                let total_sinks = ingress_graph.latest_snapshot().sinks.len()
                    + destination_graphs
                        .iter()
                        .map(|graph| graph.latest_snapshot().sinks.len())
                        .sum::<usize>();
                max_total_sinks.fetch_max(total_sinks, Ordering::AcqRel);
                max_lifecycle_tasks.fetch_max(
                    orchestrator.connection_lifecycle_task_count(),
                    Ordering::AcqRel,
                );
                tokio::task::yield_now().await;
            }
        })
    };

    for round in 0..ROUNDS {
        let seed = BASE_SEED
            .wrapping_add(round as u64)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let candidates = (0..destinations.len())
            .filter(|index| *index != current)
            .collect::<Vec<_>>();
        let barrier = Arc::new(Barrier::new(3));
        let mut attempts = tokio::task::JoinSet::new();
        for (lane, candidate) in candidates.iter().copied().enumerate() {
            let orchestrator = Arc::clone(&orchestrator);
            let barrier = Arc::clone(&barrier);
            let bridge_id = bridge_id.clone();
            let ingress = ingress.clone();
            let old = destinations[current].clone();
            let pending = destinations[candidate].clone();
            let yields = ((seed.rotate_left((lane * 17) as u32) >> 60) & 0x7) as usize;
            attempts.spawn(async move {
                barrier.wait().await;
                for _ in 0..yields {
                    tokio::task::yield_now().await;
                }
                (
                    candidate,
                    orchestrator
                        .replace_bridge_destination(bridge_id, ingress, old, pending)
                        .await,
                )
            });
        }
        barrier.wait().await;

        let mut winner = None;
        let mut rejected = 0;
        while let Some(result) = attempts.join_next().await {
            let (candidate, result) = result.expect("replacement contender task");
            match result {
                Ok(receipt) => {
                    assert!(winner.replace((candidate, receipt)).is_none());
                }
                Err(RvoipError::BridgeNotFound(id)) if id == bridge_id => rejected += 1,
                Err(RvoipError::InvalidState(reason)) => {
                    panic!("seed {seed:#x} produced unexpected invalid state: {reason}")
                }
                Err(RvoipError::AdmissionRejected(reason)) => {
                    panic!("seed {seed:#x} produced unexpected admission rejection: {reason}")
                }
                Err(error) => panic!("seed {seed:#x} produced unexpected glare error: {error}"),
            }
        }
        assert_eq!(rejected, 1, "seed {seed:#x} did not fence one contender");
        let (next, receipt) = winner
            .unwrap_or_else(|| panic!("seed {seed:#x} did not commit exactly one replacement"));
        current = next;
        bridge_id = receipt.bridge_id;
        wait_for_active_bridge_count(&orchestrator, 1).await;
        wait_for_sink_count(&ingress_graph, 1).await;
        for (index, graph) in destination_graphs.iter().enumerate() {
            wait_for_sink_count(graph, usize::from(index == current)).await;
        }

        while ingress_output.try_recv().is_ok() {}
        for output in &mut destination_outputs {
            while output.try_recv().is_ok() {}
        }
        let marker = (round as u8).wrapping_add(1);
        ingress_stream
            .inject(mk_frame(ingress_stream.id(), marker))
            .await;
        let forwarded =
            tokio::time::timeout(Duration::from_secs(2), destination_outputs[current].recv())
                .await
                .unwrap_or_else(|_| panic!("seed {seed:#x} did not forward to its winner"))
                .expect("winning destination output closed");
        assert_eq!(forwarded.payload[0], marker);
        for (index, output) in destination_outputs.iter_mut().enumerate() {
            if index != current {
                assert!(
                    output.try_recv().is_err(),
                    "seed {seed:#x} mixed media into retired destination {index}"
                );
            }
        }
        destination_streams[current]
            .inject(mk_frame(destination_streams[current].id(), marker))
            .await;
        let reverse = tokio::time::timeout(Duration::from_secs(2), ingress_output.recv())
            .await
            .unwrap_or_else(|_| panic!("seed {seed:#x} lost reverse media"))
            .expect("ingress output closed");
        assert_eq!(reverse.payload[0], marker);
    }

    orchestrator
        .unbridge_connections(bridge_id)
        .await
        .expect("remove final glare bridge");
    wait_for_sink_count(&ingress_graph, 0).await;
    for graph in &destination_graphs {
        wait_for_sink_count(graph, 0).await;
    }
    sampling.store(false, Ordering::Release);
    sampler.await.expect("resource sampler task");
    orchestrator.drain_connection_lifecycle_tasks().await;
    assert_eq!(orchestrator.connection_lifecycle_task_count(), 0);
    assert_eq!(active_bridge_count(&orchestrator), 0);
    eprintln!(
        "{{\"kind\":\"seeded_core_bridge_replacement_glare\",\"baseSeed\":\"{BASE_SEED:#x}\",\"rounds\":{ROUNDS},\"resourceHighWater\":{{\"mediaGraphSinks\":{},\"lifecycleTasks\":{}}},\"final\":{{\"activeBridges\":0,\"mediaGraphSinks\":0,\"lifecycleTasks\":0}}}}",
        max_total_sinks.load(Ordering::Acquire),
        max_lifecycle_tasks.load(Ordering::Acquire),
    );
}

#[cfg(feature = "opus")]
#[tokio::test]
async fn native_codec_bundle_bridges_opus_frames() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let mut output = stream_b.take_external_out();
    let bridge = orch
        .bridge_connections(conn_a, conn_b)
        .await
        .expect("Opus bridge with native codec feature");

    stream_a.inject(mk_frame(stream_a.id(), 42)).await;
    let frame = tokio::time::timeout(Duration::from_secs(2), output.recv())
        .await
        .expect("Opus bridge timed out")
        .expect("Opus bridge output closed");
    assert_eq!(frame.payload[0], 42);

    orch.unbridge_connections(bridge)
        .await
        .expect("remove Opus bridge");
}

#[tokio::test]
async fn data_messages_preserve_all_fields_and_route_to_only_the_exact_connection() {
    let (orch, _stream_a, _stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC)
            .await;
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
        setup_two_connection_orchestrator_with_adapter(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC)
            .await;
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
        setup_two_connection_orchestrator_with_adapter(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC)
            .await;
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
        setup_two_connection_orchestrator_with_adapter(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC)
            .await;
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
        setup_two_connection_orchestrator_with_adapter(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC)
            .await;
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
async fn amazon_connect_cross_transport_bridge_survives_startup_backpressure() {
    const STARTUP_FRAMES: usize = 75;
    let (orchestrator, sip_stream, connect_stream, sip_connection, connect_connection) =
        setup_amazon_connect_bridge_orchestrator().await;
    let mut connect_output = connect_stream.take_external_out();
    let bridge = orchestrator
        .bridge_connections_directional(
            sip_connection.clone(),
            connect_connection,
            DirectionalMediaBridgePlan::new(true, false).unwrap(),
        )
        .await
        .expect("SIP-to-Connect directional bridge");
    let graph = orchestrator
        .media_graph_for_connection(sip_connection)
        .await
        .expect("SIP source graph");

    for value in 0..STARTUP_FRAMES {
        sip_stream
            .inject(mk_frame(sip_stream.id(), value as u8))
            .await;
    }
    let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = graph.snapshot().await;
            if snapshot.source_frames >= STARTUP_FRAMES as u64 {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("graph consumes startup burst");
    assert_eq!(snapshot.evictions, 0);
    assert_eq!(snapshot.sinks.len(), 1);
    assert!(snapshot.dropped_frames > 0);

    connect_output.recv().await.expect("release startup stall");
    sip_stream.inject(mk_frame(sip_stream.id(), 255)).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(frame) = connect_output.recv().await {
            if frame.payload[0] == 255 {
                return;
            }
        }
        panic!("Connect output closed after startup stall");
    })
    .await
    .expect("post-stall media reaches Connect");

    orchestrator
        .unbridge_connections(bridge)
        .await
        .expect("remove Connect bridge");
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator_with_adapter("PCMU", "PCMA").await;
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
        payload_type: None,
    });

    orch.renegotiate_media(conn_a, CapabilityDescriptor::default())
        .await
        .expect("one-way graph swap");
    let snapshot = graph.snapshot().await;
    assert_eq!(snapshot.source_payload_type, 8);
    assert_eq!(snapshot.sinks.len(), 1);
    assert_eq!(snapshot.sinks[0].target_payload_type, 8);
}

#[tokio::test]
async fn bridge_renegotiation_uses_the_transports_negotiated_dynamic_payload_type() {
    let (orch, _stream_a, _stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("PCMU", "PCMU").await;
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
        payload_type: Some(96),
    });

    orch.renegotiate_media(conn_a, CapabilityDescriptor::default())
        .await
        .expect("dynamic payload graph swap");
    let snapshot = graph.snapshot().await;
    assert_eq!(
        snapshot.source_payload_type, 96,
        "the negotiated dynamic PT must win over PCMA's static payload type 8"
    );
    assert_eq!(snapshot.sinks[0].target_payload_type, 0);
}

#[tokio::test]
async fn one_way_bridge_renegotiation_rejects_an_unsupported_codec() {
    let (orch, _stream_a, _stream_b, conn_a, conn_b, adapter) =
        setup_two_connection_orchestrator_with_adapter("PCMU", "PCMA").await;
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
        payload_type: None,
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
    let (orch, _a, _b, conn_a, _) =
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
    let err = orch
        .bridge_connections(conn_a.clone(), conn_a.clone())
        .await
        .unwrap_err();
    matches!(err, RvoipError::AdmissionRejected(_));
}

#[tokio::test]
async fn bridge_connection_not_found_returns_error() {
    let (orch, _a, _b, conn_a, _) =
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
    let unknown = ConnectionId::new();
    let err = orch
        .bridge_connections(conn_a, unknown.clone())
        .await
        .unwrap_err();
    matches!(err, RvoipError::ConnectionNotFound(_));
}

#[tokio::test]
async fn bridge_already_bridged_returns_error() {
    let (orch, _a, _b, conn_a, conn_b) =
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
    let (orch, _a, _b, conn_a, conn_b) =
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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

#[tokio::test(flavor = "current_thread")]
async fn cancelled_unbridge_releases_ownership_before_cleanup_awaits() {
    let (orch, _stream_a, _stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
    let first = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect("first bridge");
    let mut unbridge = Box::pin(orch.unbridge_connections(first));

    // Poll exactly once. Managed route removal has queued actor work and is
    // pending on its acknowledgement, so dropping here models cancellation at
    // the first cleanup await after the synchronous detach commit.
    poll_fn(|context| {
        assert!(
            unbridge.as_mut().poll(context).is_pending(),
            "managed bridge cleanup unexpectedly completed in one poll"
        );
        Poll::Ready(())
    })
    .await;
    drop(unbridge);

    let second = orch
        .bridge_connections(conn_a, conn_b)
        .await
        .expect("cancelled cleanup must not strand endpoint ownership");
    orch.unbridge_connections(second)
        .await
        .expect("remove replacement bridge");
}

#[tokio::test]
async fn full_media_target_is_bounded_and_never_backpressures_the_source() {
    let (orch, stream_a, stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator_with_adapter(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC)
            .await;
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

    let replacement_b = MockMediaStream::new(DEFAULT_TEST_CODEC);
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
async fn lifecycle_drain_aborts_and_joins_cross_bridge_terminal_supervisor() {
    let (orch, _stream_a, _stream_b, conn_a, conn_b) =
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
    let baseline = orch.connection_lifecycle_task_count();
    let bridge = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect("bridge");
    assert_eq!(
        orch.connection_lifecycle_task_count(),
        baseline + 1,
        "one combined terminal supervisor must be owned per bridge"
    );

    tokio::time::timeout(
        Duration::from_secs(1),
        orch.drain_connection_lifecycle_tasks(),
    )
    .await
    .expect("lifecycle supervisor drain");
    assert_eq!(orch.connection_lifecycle_task_count(), 0);
    assert!(matches!(
        orch.bridge_connections(conn_a, conn_b).await,
        Err(RvoipError::InvalidState(
            "connection lifecycle supervisor is draining"
        ))
    ));
    orch.unbridge_connections(bridge)
        .await
        .expect("explicitly remove bridge after terminal watcher drain");
}

#[tokio::test]
async fn lifecycle_drain_rejects_bridge_destination_replacement() {
    let (
        orch,
        _ingress_stream,
        _current_stream,
        _replacement_stream,
        ingress,
        current_destination,
        replacement_destination,
    ) = setup_cross_transport_replacement_orchestrator().await;
    let bridge = orch
        .bridge_connections(ingress.clone(), current_destination.clone())
        .await
        .expect("initial bridge");

    orch.drain_connection_lifecycle_tasks().await;
    assert!(matches!(
        orch.replace_bridge_destination(
            bridge.clone(),
            ingress,
            current_destination,
            replacement_destination,
        )
        .await,
        Err(RvoipError::InvalidState(
            "connection lifecycle supervisor is draining"
        ))
    ));
    orch.unbridge_connections(bridge)
        .await
        .expect("explicitly remove bridge after rejected replacement");
}

#[tokio::test]
async fn concurrent_graph_initialization_takes_source_once() {
    let (orch, stream_a, _stream_b, conn_a, _conn_b) =
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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
    let stream_a = MockMediaStream::new(DEFAULT_TEST_CODEC);
    let stream_b = MockMediaStream::new(DEFAULT_TEST_CODEC);
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
        setup_two_connection_orchestrator(DEFAULT_TEST_CODEC, DEFAULT_TEST_CODEC).await;
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

// =====================================================================
// Two-phase peer handoff (prepare / commit)
// =====================================================================

/// One Quic adapter holding `count` connections in a single Session, each
/// with a G.711 mock stream, admitted before the fixture returns.
async fn setup_shared_session_orchestrator(
    count: usize,
) -> (
    Arc<Orchestrator>,
    Vec<Arc<MockMediaStream>>,
    Vec<ConnectionId>,
    Arc<MockAdapter>,
) {
    let adapter = MockAdapter::new(Transport::Quic);
    let connections = (0..count).map(|_| ConnectionId::new()).collect::<Vec<_>>();
    let streams = (0..count)
        .map(|_| MockMediaStream::new(DEFAULT_TEST_CODEC))
        .collect::<Vec<_>>();
    for (connection_id, stream) in connections.iter().zip(&streams) {
        adapter.register_connection(connection_id.clone(), Arc::clone(stream));
    }
    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register handoff adapter");
    let session = SessionId::new();
    for connection_id in &connections {
        adapter
            .announce(connection_id.clone(), session.clone())
            .await;
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while connections
            .iter()
            .any(|connection_id| orchestrator.connection_transport(connection_id).is_err())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("handoff fixture was not admitted");
    (orchestrator, streams, connections, adapter)
}

fn bidirectional_plan() -> DirectionalMediaBridgePlan {
    DirectionalMediaBridgePlan::new(true, true).expect("bidirectional plan")
}

async fn expect_frame(receiver: &mut mpsc::Receiver<MediaFrame>, expected: u8) {
    let frame = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap_or_else(|_| panic!("frame {expected} was not forwarded"))
        .expect("media output closed");
    assert_eq!(frame.payload[0], expected);
}

async fn expect_silence(receiver: &mut mpsc::Receiver<MediaFrame>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(50), receiver.recv())
            .await
            .is_err(),
        "a silent route forwarded media"
    );
}

/// Skip unrelated bus traffic until `matches` accepts an event.
async fn next_event_matching(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    matches: impl Fn(&Event) -> bool,
) -> Event {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.expect("core event bus closed");
            if matches(&event) {
                break event;
            }
        }
    })
    .await
    .expect("expected core event was not published")
}

fn assert_send_ref<T: Send>(_: &T) {}

#[tokio::test]
async fn staged_peer_handoff_is_silent_and_rollback_preserves_original_bridge() {
    let (orch, streams, connections, _adapter) = setup_shared_session_orchestrator(3).await;
    let (a, b, c) = (&streams[0], &streams[1], &streams[2]);
    let (ca, cb, cc) = (&connections[0], &connections[1], &connections[2]);
    let mut a_out = a.take_external_out();
    let mut b_out = b.take_external_out();
    let mut c_out = c.take_external_out();
    let bridge = orch
        .bridge_connections(ca.clone(), cb.clone())
        .await
        .expect("original bridge");
    let c_graph = orch
        .media_graph_for_connection(cc.clone())
        .await
        .expect("target graph");

    for await_rollback in [false, true] {
        let staged = orch
            .prepare_peer_handoff(
                bridge.clone(),
                ca.clone(),
                cc.clone(),
                bidirectional_plan(),
                Arc::new(SelectiveDataPolicy::default()),
            )
            .await
            .expect("stage (or restage after rollback) the target");
        assert_eq!(staged.previous_bridge_id(), &bridge);
        assert_eq!(staged.retained_connection(), ca);
        assert_eq!(staged.source_connection(), cb);
        assert_eq!(staged.target_connection(), cc);
        assert_ne!(staged.replacement_bridge_id(), &bridge);
        assert_eq!(
            active_bridge_count(&orch),
            1,
            "staging must not publish a bridge"
        );

        a.inject(mk_frame(a.id(), 71)).await;
        b.inject(mk_frame(b.id(), 72)).await;
        c.inject(mk_frame(c.id(), 73)).await;
        expect_frame(&mut b_out, 71).await;
        expect_frame(&mut a_out, 72).await;
        expect_silence(&mut c_out).await;
        expect_silence(&mut a_out).await;

        if await_rollback {
            staged.abandon().await;
            assert!(
                c_graph.latest_snapshot().sinks.is_empty(),
                "awaited rollback must remove the dormant target route"
            );
        } else {
            drop(staged);
        }
        wait_for_sink_count(&c_graph, 0).await;
        assert_eq!(active_bridge_count(&orch), 1);
    }

    a.inject(mk_frame(a.id(), 74)).await;
    expect_frame(&mut b_out, 74).await;
    orch.unbridge_connections(bridge)
        .await
        .expect("remove original bridge");
    let retry = orch
        .bridge_connections(ca.clone(), cc.clone())
        .await
        .expect("rolled-back handoff released the target reservation");
    orch.unbridge_connections(retry).await.expect("cleanup");
}

#[tokio::test]
async fn committed_peer_handoff_switches_both_directions_and_can_switch_back() {
    let (orch, streams, connections, _adapter) = setup_shared_session_orchestrator(3).await;
    let (a, b, c) = (&streams[0], &streams[1], &streams[2]);
    let (ca, cb, cc) = (&connections[0], &connections[1], &connections[2]);
    let mut a_out = a.take_external_out();
    let mut b_out = b.take_external_out();
    let mut c_out = c.take_external_out();
    let mut bridge = orch
        .bridge_connections(ca.clone(), cb.clone())
        .await
        .expect("original bridge");

    // Legacy mock queues carry no transport delivery fence, so the strict
    // variant must refuse before reserving anything.
    assert!(matches!(
        orch.prepare_transport_fenced_peer_handoff(
            bridge.clone(),
            ca.clone(),
            cc.clone(),
            bidirectional_plan(),
            Arc::new(rvoip_core::stream::PassThroughDataMessageBridgePolicy),
        )
        .await,
        Err(RvoipError::NotImplemented(_))
    ));

    let recording = Arc::new(VecRecordingSink::new("memory:rec/handoff"));
    orch.register_recording_sink("handoff", recording.clone());
    let recording_id = orch
        .start_recording(RecordingTarget::Connection(ca.clone()), "handoff")
        .await
        .expect("record the retained connection");
    a.inject(mk_frame(a.id(), 60)).await;
    expect_frame(&mut b_out, 60).await;

    let mut events = orch.subscribe_events();
    let staged = orch
        .prepare_peer_handoff(
            bridge.clone(),
            ca.clone(),
            cc.clone(),
            bidirectional_plan(),
            Arc::new(SelectiveDataPolicy::default()),
        )
        .await
        .expect("stage the target");
    assert_eq!(staged.source_connection(), cb);
    let previous = bridge.clone();
    let expected_replacement = staged.replacement_bridge_id().clone();
    let commit = orch.commit_peer_handoff_with_timeout_and_receipt(staged, Duration::from_secs(1));
    assert_send_ref(&commit);
    let receipt = commit.await.expect("commit the target");
    bridge = receipt.bridge_id.clone();
    assert_eq!(bridge, expected_replacement);
    assert_eq!(receipt.previous_bridge_id, previous);
    assert_eq!(receipt.retained, *ca);
    assert_eq!(receipt.source, *cb);
    assert_eq!(receipt.target, *cc);

    let handoff = next_event_matching(&mut events, |event| {
        matches!(
            event,
            Event::PeerHandoffCommitted { .. }
                | Event::ConnectionsUnbridged { .. }
                | Event::ConnectionsBridged { .. }
        )
    })
    .await;
    match handoff {
        Event::PeerHandoffCommitted {
            previous_bridge_id,
            bridge_id,
            retained,
            source,
            target,
            at,
        } => {
            assert_eq!(at, receipt.committed_at);
            assert_eq!(previous_bridge_id, previous);
            assert_eq!(bridge_id, bridge);
            assert_eq!(retained, *ca);
            assert_eq!(source, *cb);
            assert_eq!(target, *cc);
        }
        other => panic!("handoff must precede compatibility events: {other:?}"),
    }
    assert!(matches!(
        next_event_matching(&mut events, |event| matches!(
            event,
            Event::ConnectionsUnbridged { .. } | Event::ConnectionsBridged { .. }
        ))
        .await,
        Event::ConnectionsUnbridged { bridge_id, .. } if bridge_id == previous
    ));
    assert!(matches!(
        next_event_matching(&mut events, |event| matches!(
            event,
            Event::ConnectionsBridged { .. }
        ))
        .await,
        Event::ConnectionsBridged { bridge_id, a, b, .. }
            if bridge_id == bridge && a == *ca && b == *cc
    ));

    a.inject(mk_frame(a.id(), 81)).await;
    c.inject(mk_frame(c.id(), 82)).await;
    b.inject(mk_frame(b.id(), 83)).await;
    expect_frame(&mut c_out, 81).await;
    expect_frame(&mut a_out, 82).await;
    expect_silence(&mut b_out).await;
    expect_silence(&mut a_out).await;

    // The retired peer is unowned again and can be handed back in.
    let staged = orch
        .prepare_peer_handoff(
            bridge.clone(),
            ca.clone(),
            cb.clone(),
            bidirectional_plan(),
            Arc::new(SelectiveDataPolicy::default()),
        )
        .await
        .expect("old peer released for return");
    assert_eq!(staged.source_connection(), cc);
    let returned = orch
        .commit_peer_handoff_with_timeout(staged, Duration::from_secs(1))
        .await
        .expect("commit the return");
    a.inject(mk_frame(a.id(), 91)).await;
    b.inject(mk_frame(b.id(), 92)).await;
    expect_frame(&mut b_out, 91).await;
    expect_frame(&mut a_out, 92).await;
    expect_silence(&mut c_out).await;

    assert_eq!(a.source_acquisitions(), 1);
    assert_eq!(b.source_acquisitions(), 1);
    assert_eq!(c.source_acquisitions(), 1);
    let expected: Vec<u8> = [60, 81, 91]
        .into_iter()
        .flat_map(|byte| mk_frame(a.id(), byte).payload.to_vec())
        .collect();
    tokio::time::timeout(Duration::from_secs(2), async {
        while recording.bytes().len() < expected.len() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("retained recording receives frames across both commits");
    orch.stop_recording(recording_id)
        .await
        .expect("same recording remains controllable");
    assert_eq!(recording.bytes(), expected);
    orch.unbridge_connections(returned).await.expect("cleanup");
}

#[tokio::test]
async fn peer_handoff_derives_source_from_either_bridge_side() {
    let (orch, streams, connections, _adapter) = setup_shared_session_orchestrator(3).await;
    let (a, b, c) = (&streams[0], &streams[1], &streams[2]);
    let (ca, cb, cc) = (&connections[0], &connections[1], &connections[2]);
    let mut a_out = a.take_external_out();
    let _b_out = b.take_external_out();
    let mut c_out = c.take_external_out();
    // Retain the `b` side of the original bridge.
    let bridge = orch
        .bridge_connections(cb.clone(), ca.clone())
        .await
        .expect("original bridge");
    let mut events = orch.subscribe_events();
    let staged = orch
        .prepare_peer_handoff(
            bridge.clone(),
            ca.clone(),
            cc.clone(),
            bidirectional_plan(),
            Arc::new(SelectiveDataPolicy::default()),
        )
        .await
        .expect("stage from the b side");
    assert_eq!(staged.source_connection(), cb);
    let receipt = orch
        .commit_peer_handoff_with_receipt(staged)
        .expect("synchronous commit");
    assert_eq!(receipt.retained, *ca);
    assert_eq!(receipt.source, *cb);
    assert_eq!(receipt.target, *cc);
    // The replacement keeps the original endpoint orientation.
    assert!(matches!(
        next_event_matching(&mut events, |event| matches!(
            event,
            Event::ConnectionsBridged { .. }
        ))
        .await,
        Event::ConnectionsBridged { bridge_id, a, b, .. }
            if bridge_id == receipt.bridge_id && a == *cc && b == *ca
    ));
    a.inject(mk_frame(a.id(), 51)).await;
    c.inject(mk_frame(c.id(), 52)).await;
    expect_frame(&mut c_out, 51).await;
    expect_frame(&mut a_out, 52).await;
    orch.unbridge_connections(receipt.bridge_id)
        .await
        .expect("cleanup");
    orch.drain_connection_lifecycle_tasks().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_handoff_commit_rejects_foreign_and_stale_preparations_and_releases_target() {
    let (orch, streams, connections, _adapter) = setup_shared_session_orchestrator(3).await;
    let (a, b) = (&streams[0], &streams[1]);
    let (ca, cb, cc) = (&connections[0], &connections[1], &connections[2]);
    let mut b_out = b.take_external_out();
    let original = orch
        .bridge_connections(ca.clone(), cb.clone())
        .await
        .expect("original bridge");
    let prepare = || {
        orch.prepare_peer_handoff(
            original.clone(),
            ca.clone(),
            cc.clone(),
            bidirectional_plan(),
            Arc::new(SelectiveDataPolicy::default()),
        )
    };

    let foreign = Orchestrator::new(Config::default());
    assert!(matches!(
        foreign.commit_peer_handoff(prepare().await.expect("stage")),
        Err(RvoipError::InvalidState(
            "handoff belongs to another orchestrator"
        ))
    ));
    assert!(matches!(
        foreign
            .commit_peer_handoff_with_timeout(
                prepare().await.expect("restage"),
                Duration::from_secs(1)
            )
            .await,
        Err(RvoipError::InvalidState(
            "handoff belongs to another orchestrator"
        ))
    ));
    a.inject(mk_frame(a.id(), 61)).await;
    expect_frame(&mut b_out, 61).await;

    let stale = prepare()
        .await
        .expect("foreign rejection released the target");
    orch.unbridge_connections(original.clone())
        .await
        .expect("remove original bridge");
    let committing = {
        let orch = Arc::clone(&orch);
        tokio::spawn(async move { orch.commit_peer_handoff(stale) })
    };
    let result = tokio::time::timeout(Duration::from_secs(2), committing)
        .await
        .expect("failed commit must not deadlock reservation cleanup")
        .expect("commit task panicked");
    assert!(
        matches!(result, Err(RvoipError::BridgeNotFound(id)) if id == original),
        "a retired original bridge is a lost generation, not an invalid state"
    );
    let retry = orch
        .bridge_connections(ca.clone(), cc.clone())
        .await
        .expect("stale rejection released the target");
    orch.unbridge_connections(retry).await.expect("cleanup");
    orch.drain_connection_lifecycle_tasks().await;
}

#[tokio::test]
async fn peer_handoff_commit_after_concurrent_replacement_reports_bridge_not_found() {
    for bounded in [false, true] {
        let (orch, streams, connections, _adapter) = setup_shared_session_orchestrator(4).await;
        let (a, d) = (&streams[0], &streams[3]);
        let (ca, cb, cc, cd) = (
            &connections[0],
            &connections[1],
            &connections[2],
            &connections[3],
        );
        let mut d_out = d.take_external_out();
        let original = orch
            .bridge_connections(ca.clone(), cb.clone())
            .await
            .expect("original bridge");
        let staged = orch
            .prepare_peer_handoff(
                original.clone(),
                ca.clone(),
                cc.clone(),
                bidirectional_plan(),
                Arc::new(SelectiveDataPolicy::default()),
            )
            .await
            .expect("stage the first contender");

        // A second contender wins the generation while the first is staged.
        let winner = orch
            .replace_bridge_destination(original.clone(), ca.clone(), cb.clone(), cd.clone())
            .await
            .expect("concurrent one-shot replacement");

        let result = if bounded {
            orch.commit_peer_handoff_with_timeout(staged, Duration::from_secs(1))
                .await
        } else {
            orch.commit_peer_handoff(staged)
        };
        assert!(
            matches!(result, Err(RvoipError::BridgeNotFound(id)) if id == original),
            "bounded={bounded}: losing the generation must classify as BridgeNotFound"
        );
        a.inject(mk_frame(a.id(), 41)).await;
        expect_frame(&mut d_out, 41).await;
        let released = orch
            .bridge_connections(cc.clone(), cb.clone())
            .await
            .expect("losing contender released its target");
        orch.unbridge_connections(released).await.expect("cleanup");
        orch.unbridge_connections(winner.bridge_id)
            .await
            .expect("cleanup");
        orch.drain_connection_lifecycle_tasks().await;
    }
}

#[tokio::test]
async fn peer_handoff_preparation_rejects_original_bridge_removed_during_setup() {
    let (orch, _streams, connections, adapter) = setup_shared_session_orchestrator(3).await;
    let (ca, cb, cc) = (&connections[0], &connections[1], &connections[2]);
    let original = orch
        .bridge_connections(ca.clone(), cb.clone())
        .await
        .expect("original bridge");
    let gate = adapter.gate_next_stream_lookup(cc.clone());
    let pending = {
        let orch = Arc::clone(&orch);
        let original = original.clone();
        let ca = ca.clone();
        let cc = cc.clone();
        tokio::spawn(async move {
            orch.prepare_peer_handoff(
                original,
                ca,
                cc,
                bidirectional_plan(),
                Arc::new(SelectiveDataPolicy::default()),
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .expect("preparation did not reach the target stream lookup");
    orch.unbridge_connections(original.clone())
        .await
        .expect("remove original bridge mid-preparation");
    gate.release.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .expect("preparation hung")
        .expect("preparation task panicked");
    assert!(matches!(result, Err(RvoipError::BridgeNotFound(id)) if id == original));
    let retry = orch
        .bridge_connections(ca.clone(), cc.clone())
        .await
        .expect("failed preparation released the target");
    orch.unbridge_connections(retry).await.expect("cleanup");
}

#[tokio::test]
async fn peer_handoff_preparation_validates_endpoints_before_reserving() {
    let (orch, _streams, connections, _adapter) = setup_shared_session_orchestrator(4).await;
    let (ca, cb, cc, cd) = (
        &connections[0],
        &connections[1],
        &connections[2],
        &connections[3],
    );
    let original = orch
        .bridge_connections(ca.clone(), cb.clone())
        .await
        .expect("original bridge");
    let policy = || Arc::new(SelectiveDataPolicy::default());
    // The retained connection must be on the expected bridge.
    assert!(matches!(
        orch.prepare_peer_handoff(
            original.clone(),
            cc.clone(),
            cd.clone(),
            bidirectional_plan(),
            policy()
        )
        .await,
        Err(RvoipError::AdmissionRejected(_))
    ));
    // The target must be distinct from both bridge endpoints.
    assert!(matches!(
        orch.prepare_peer_handoff(
            original.clone(),
            ca.clone(),
            cb.clone(),
            bidirectional_plan(),
            policy()
        )
        .await,
        Err(RvoipError::AdmissionRejected(_))
    ));
    assert!(matches!(
        orch.prepare_peer_handoff(
            original.clone(),
            ca.clone(),
            ca.clone(),
            bidirectional_plan(),
            policy()
        )
        .await,
        Err(RvoipError::AdmissionRejected(_))
    ));
    // An unknown generation is a lost fence.
    let stale = BridgeId::new();
    assert!(matches!(
        orch.prepare_peer_handoff(stale.clone(), ca.clone(), cc.clone(), bidirectional_plan(), policy())
            .await,
        Err(RvoipError::BridgeNotFound(id)) if id == stale
    ));
    // Nothing above reserved the spare connections.
    let probe = orch
        .bridge_connections(cc.clone(), cd.clone())
        .await
        .expect("rejected preparations reserved nothing");
    orch.unbridge_connections(probe).await.expect("cleanup");

    orch.drain_connection_lifecycle_tasks().await;
    assert!(matches!(
        orch.prepare_peer_handoff(
            original.clone(),
            ca.clone(),
            cc.clone(),
            bidirectional_plan(),
            policy()
        )
        .await,
        Err(RvoipError::InvalidState(
            "connection lifecycle supervisor is draining"
        ))
    ));
    orch.unbridge_connections(original)
        .await
        .expect("explicitly remove bridge after rejected preparation");
}

/// Drain everything currently on the bus, returning the events in order.
async fn drain_events(events: &mut tokio::sync::broadcast::Receiver<Event>) -> Vec<Event> {
    let mut drained = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(100), events.recv()).await
    {
        drained.push(event);
    }
    drained
}

#[tokio::test]
async fn peer_handoff_committed_is_published_once_after_commit_and_never_on_failure() {
    let (orch, streams, connections, _adapter) = setup_shared_session_orchestrator(3).await;
    let (a, c) = (&streams[0], &streams[2]);
    let (ca, cb, cc) = (&connections[0], &connections[1], &connections[2]);
    let _a_out = a.take_external_out();
    let mut c_out = c.take_external_out();
    let original = orch
        .bridge_connections(ca.clone(), cb.clone())
        .await
        .expect("original bridge");
    let mut events = orch.subscribe_events();
    let prepare = |bridge: BridgeId, target: ConnectionId| {
        orch.prepare_peer_handoff(
            bridge,
            ca.clone(),
            target,
            bidirectional_plan(),
            Arc::new(SelectiveDataPolicy::default()),
        )
    };

    // Rejected commits publish nothing: a foreign orchestrator, and a
    // generation lost to a one-shot contender while the handoff was staged.
    let foreign = Orchestrator::new(Config::default());
    let staged = prepare(original.clone(), cc.clone()).await.expect("stage");
    assert!(foreign
        .commit_peer_handoff_with_timeout(staged, Duration::from_secs(1))
        .await
        .is_err());
    let lost = prepare(original.clone(), cc.clone())
        .await
        .expect("restage after foreign rejection");
    // Rolling the lost handoff back must not publish either.
    lost.abandon().await;
    assert!(
        !drain_events(&mut events)
            .await
            .iter()
            .any(|event| matches!(event, Event::PeerHandoffCommitted { .. })),
        "a rejected or abandoned handoff must not publish PeerHandoffCommitted"
    );

    // Two-phase commit: exactly one PeerHandoffCommitted, carrying the
    // receipt's identities, published before the replacement's
    // ConnectionsBridged.
    let staged = prepare(original.clone(), cc.clone()).await.expect("stage");
    let receipt = orch
        .commit_peer_handoff_with_timeout_and_receipt(staged, Duration::from_secs(1))
        .await
        .expect("commit");
    a.inject(mk_frame(a.id(), 21)).await;
    expect_frame(&mut c_out, 21).await;
    let published = drain_events(&mut events).await;
    let handoffs = published
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event, Event::PeerHandoffCommitted { .. }))
        .collect::<Vec<_>>();
    assert_eq!(
        handoffs.len(),
        1,
        "exactly one handoff event: {published:?}"
    );
    let (handoff_index, handoff) = handoffs[0];
    match handoff {
        Event::PeerHandoffCommitted {
            previous_bridge_id,
            bridge_id,
            retained,
            source,
            target,
            at,
        } => {
            assert_eq!(previous_bridge_id, &receipt.previous_bridge_id);
            assert_eq!(bridge_id, &receipt.bridge_id);
            assert_eq!(retained, &receipt.retained);
            assert_eq!(source, &receipt.source);
            assert_eq!(target, &receipt.target);
            assert_eq!(at, &receipt.committed_at);
            assert_eq!(previous_bridge_id, &original);
            assert_eq!((retained, source, target), (ca, cb, cc));
        }
        other => unreachable!("filtered handoff event: {other:?}"),
    }
    let unbridged_index = published
        .iter()
        .position(|event| {
            matches!(event, Event::ConnectionsUnbridged { bridge_id, .. } if bridge_id == &original)
        })
        .expect("retired generation publishes ConnectionsUnbridged");
    let bridged_index = published
        .iter()
        .position(|event| {
            matches!(event, Event::ConnectionsBridged { bridge_id, .. } if bridge_id == &receipt.bridge_id)
        })
        .expect("replacement generation publishes ConnectionsBridged");
    assert!(
        handoff_index < unbridged_index && unbridged_index < bridged_index,
        "handoff must precede the compatibility events: {published:?}"
    );

    // The one-shot path shares the commit and publishes the same event.
    let replacement = orch
        .replace_bridge_destination(
            receipt.bridge_id.clone(),
            ca.clone(),
            cc.clone(),
            cb.clone(),
        )
        .await
        .expect("one-shot replacement back to the first peer");
    let published = drain_events(&mut events).await;
    let handoffs = published
        .iter()
        .filter(|event| matches!(event, Event::PeerHandoffCommitted { .. }))
        .collect::<Vec<_>>();
    assert_eq!(
        handoffs.len(),
        1,
        "one-shot path publishes one handoff event"
    );
    assert!(matches!(
        handoffs[0],
        Event::PeerHandoffCommitted { previous_bridge_id, bridge_id, retained, source, target, .. }
            if previous_bridge_id == &replacement.previous_bridge_id
                && bridge_id == &replacement.bridge_id
                && retained == &replacement.ingress
                && source == &replacement.previous_destination
                && target == &replacement.destination
    ));
    orch.unbridge_connections(replacement.bridge_id)
        .await
        .expect("cleanup");
    orch.drain_connection_lifecycle_tasks().await;
}
