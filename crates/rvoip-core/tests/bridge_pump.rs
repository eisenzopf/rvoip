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
use rvoip_core::connection::{
    Connection, ConnectionState, Direction, Transport, TransportHandle,
};
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_core::{Config, Orchestrator, RvoipError};
use tokio::sync::mpsc;

// =====================================================================
// MockMediaStream
// =====================================================================

struct MockMediaStream {
    id: StreamId,
    codec: CodecInfo,
    /// The "outside" hands us frames via `external_in_tx`; we deliver
    /// them through `frames_in()`.
    external_in_tx: mpsc::Sender<MediaFrame>,
    in_rx: StdMutex<Option<mpsc::Receiver<MediaFrame>>>,
    /// `frames_out()` returns clones of this sender; what the
    /// "outside" reads via `external_out_rx`.
    out_tx: mpsc::Sender<MediaFrame>,
    external_out_rx: StdMutex<Option<mpsc::Receiver<MediaFrame>>>,
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
            in_rx: StdMutex::new(Some(in_rx)),
            out_tx,
            external_out_rx: StdMutex::new(Some(external_out_rx)),
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
        self.in_rx
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }
    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.out_tx.clone()
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
}

impl MockAdapter {
    fn new(transport: Transport) -> Arc<Self> {
        let (events_tx, events_rx) = mpsc::channel(64);
        Arc::new(Self {
            transport,
            streams: dashmap::DashMap::new(),
            events_tx,
            events_rx: StdMutex::new(Some(events_rx)),
        })
    }

    fn register_connection(&self, id: ConnectionId, stream: Arc<MockMediaStream>) {
        self.streams.insert(id, stream);
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
        match self.streams.get(&c) {
            Some(s) => Ok(vec![s.clone() as Arc<dyn MediaStream>]),
            None => Ok(Vec::new()),
        }
    }
    async fn send_message(&self, _c: ConnectionId, _m: Message) -> rvoip_core::error::Result<()> {
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
        Ok(NegotiatedCodecs::default())
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
    }
}

/// Spin up an Orchestrator with one MockAdapter (Quic transport) holding
/// two connections + their streams. Returns the orchestrator + the two
/// streams + their connection ids so tests can inject/observe frames.
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

    (orchestrator, stream_a, stream_b, conn_a, conn_b)
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
async fn bridge_self_returns_error() {
    let (orch, _a, _b, conn_a, _) =
        setup_two_connection_orchestrator("opus", "opus").await;
    let err = orch
        .bridge_connections(conn_a.clone(), conn_a.clone())
        .await
        .unwrap_err();
    matches!(err, RvoipError::AdmissionRejected(_));
}

#[tokio::test]
async fn bridge_connection_not_found_returns_error() {
    let (orch, _a, _b, conn_a, _) =
        setup_two_connection_orchestrator("opus", "opus").await;
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
        setup_two_connection_orchestrator("opus", "opus").await;
    orch.bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .expect("first bridge");
    let err = orch
        .bridge_connections(conn_a.clone(), conn_b.clone())
        .await
        .unwrap_err();
    matches!(err, RvoipError::AdmissionRejected(_));
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

    // The pump task is aborted; subsequent injects don't propagate.
    stream_a.inject(mk_frame(stream_a.id(), 99)).await;
    let result =
        tokio::time::timeout(Duration::from_millis(200), b_out.recv()).await;
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
