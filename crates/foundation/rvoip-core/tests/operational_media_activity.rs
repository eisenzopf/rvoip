use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason,
    OrchestratorAdapterEvent, OriginateRequest, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::operational_events::{
    OperationalEvent, OperationalEventKind, OperationalEventStreamHealth,
};
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_core::{Config, Orchestrator, Result, RvoipError};
use tokio::sync::mpsc;

fn pcmu() -> CodecInfo {
    CodecInfo {
        name: "pcmu".into(),
        clock_rate_hz: 8_000,
        channels: 1,
        fmtp: None,
    }
}

struct TestStream {
    id: StreamId,
    inbound: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    outbound: mpsc::Sender<MediaFrame>,
}

impl TestStream {
    fn new() -> (Arc<Self>, mpsc::Sender<MediaFrame>) {
        let (source, inbound) = mpsc::channel(512);
        let (outbound, _sink) = mpsc::channel(1);
        (
            Arc::new(Self {
                id: StreamId::new(),
                inbound: Mutex::new(Some(inbound)),
                outbound,
            }),
            source,
        )
    }
}

#[async_trait::async_trait]
impl MediaStream for TestStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }

    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }

    fn codec(&self) -> CodecInfo {
        pcmu()
    }

    fn direction(&self) -> Direction {
        Direction::Inbound
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.inbound
            .lock()
            .expect("stream receiver lock")
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> Result<mpsc::Receiver<MediaFrame>> {
        self.inbound
            .lock()
            .expect("stream receiver lock")
            .take()
            .ok_or(RvoipError::InvalidState(
                "test stream receiver already acquired",
            ))
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.outbound.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }

    async fn close(self: Arc<Self>) -> Result<()> {
        Ok(())
    }
}

struct TestAdapter {
    streams: Mutex<HashMap<ConnectionId, Arc<dyn MediaStream>>>,
    live: Mutex<HashSet<ConnectionId>>,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
    receiver: Mutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
}

impl TestAdapter {
    fn new() -> Arc<Self> {
        let (events, receiver) = mpsc::channel(64);
        Arc::new(Self {
            streams: Mutex::new(HashMap::new()),
            live: Mutex::new(HashSet::new()),
            events,
            receiver: Mutex::new(Some(receiver)),
        })
    }

    fn add_connection(&self, connection_id: ConnectionId, stream: Arc<dyn MediaStream>) {
        self.streams
            .lock()
            .expect("streams lock")
            .insert(connection_id.clone(), stream);
        self.live
            .lock()
            .expect("live routes lock")
            .insert(connection_id);
    }

    async fn send(&self, event: AdapterEvent) {
        self.events.send(event.into()).await.expect("send event");
    }

    async fn end(&self, connection_id: ConnectionId) {
        self.live
            .lock()
            .expect("live routes lock")
            .remove(&connection_id);
        self.send(AdapterEvent::Ended {
            connection_id,
            reason: EndReason::Normal,
        })
        .await;
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for TestAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    fn is_connection_live(&self, connection_id: &ConnectionId) -> bool {
        self.live
            .lock()
            .expect("live routes lock")
            .contains(connection_id)
    }

    async fn originate(&self, _: OriginateRequest) -> Result<ConnectionHandle> {
        Err(RvoipError::NotImplemented("test originate"))
    }

    async fn accept(&self, _: ConnectionId) -> Result<()> {
        Ok(())
    }

    async fn reject(&self, _: ConnectionId, _: RejectReason) -> Result<()> {
        Ok(())
    }

    async fn end(&self, _: ConnectionId, _: EndReason) -> Result<()> {
        Ok(())
    }

    async fn hold(&self, _: ConnectionId) -> Result<()> {
        Ok(())
    }

    async fn resume(&self, _: ConnectionId) -> Result<()> {
        Ok(())
    }

    async fn transfer(&self, _: ConnectionId, _: TransferTarget) -> Result<()> {
        Ok(())
    }

    async fn streams(&self, connection_id: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>> {
        Ok(self
            .streams
            .lock()
            .expect("streams lock")
            .get(&connection_id)
            .cloned()
            .into_iter()
            .collect())
    }

    async fn send_message(&self, _: ConnectionId, _: Message) -> Result<()> {
        Ok(())
    }

    async fn send_dtmf(&self, _: ConnectionId, _: &str, _: u32) -> Result<()> {
        Ok(())
    }

    async fn renegotiate_media(
        &self,
        _: ConnectionId,
        _: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        mpsc::channel(1).1
    }

    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        self.receiver
            .lock()
            .expect("event receiver lock")
            .take()
            .expect("orchestrator subscribes once")
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }

    async fn verify_request_signature(
        &self,
        _: ConnectionId,
        _: SignatureHeaders,
    ) -> Result<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

fn connection(connection_id: ConnectionId) -> Connection {
    Connection {
        id: connection_id,
        session_id: SessionId::new(),
        participant_id: ParticipantId::new(),
        transport: Transport::Sip,
        direction: Direction::Inbound,
        state: ConnectionState::Connecting,
        capabilities: CapabilityDescriptor::default(),
        negotiated_codecs: NegotiatedCodecs::default(),
        streams: Vec::new(),
        messaging_enabled: false,
        transport_handle: TransportHandle(Arc::new(())),
        opened_at: Utc::now(),
        closed_at: None,
    }
}

fn frame(stream_id: &StreamId, sequence: u32) -> MediaFrame {
    MediaFrame {
        stream_id: stream_id.clone(),
        kind: StreamKind::Audio,
        payload: Bytes::from_static(&[0_u8; 160]),
        timestamp_rtp: sequence.saturating_mul(160),
        captured_at: Utc::now(),
        payload_type: Some(0),
    }
}

async fn wait_until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition deadline");
}

async fn connect(
    orchestrator: &Orchestrator,
    adapter: &TestAdapter,
    operational: &mut mpsc::Receiver<OperationalEvent>,
    connection_id: ConnectionId,
) {
    adapter
        .send(AdapterEvent::InboundConnection {
            connection: connection(connection_id.clone()),
        })
        .await;
    wait_until(|| orchestrator.connection_transport(&connection_id).is_ok()).await;
    adapter
        .send(AdapterEvent::Connected {
            connection_id: connection_id.clone(),
        })
        .await;
    let connected = operational.recv().await.expect("connected event");
    assert_eq!(connected.connection_id, connection_id);
    assert!(matches!(connected.kind, OperationalEventKind::Connected));
}

async fn next_activity(operational: &mut mpsc::Receiver<OperationalEvent>) -> OperationalEvent {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = operational.recv().await.expect("operational stream");
            if matches!(event.kind, OperationalEventKind::MediaActivity { .. }) {
                return event;
            }
        }
    })
    .await
    .expect("activity deadline")
}

#[tokio::test]
async fn media_activity_is_coalesced_and_generations_are_consecutive() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut operational = orchestrator.install_operational_event_stream(8).unwrap();
    let adapter = TestAdapter::new();
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .unwrap();
    let connection_id = ConnectionId::new();
    let (stream, source) = TestStream::new();
    let stream_id = stream.id();
    adapter.add_connection(connection_id.clone(), stream);
    connect(
        &orchestrator,
        &adapter,
        &mut operational,
        connection_id.clone(),
    )
    .await;
    let graph = orchestrator
        .media_graph_for_connection(connection_id.clone())
        .await
        .unwrap();

    for sequence in 0..100 {
        source.send(frame(&stream_id, sequence)).await.unwrap();
    }
    let first = next_activity(&mut operational).await;
    assert_eq!(first.connection_id, connection_id);
    assert!(matches!(
        first.kind,
        OperationalEventKind::MediaActivity { generation: 1 }
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), operational.recv())
            .await
            .is_err(),
        "one packet burst produces one authoritative activity event"
    );

    source.send(frame(&stream_id, 100)).await.unwrap();
    let second = next_activity(&mut operational).await;
    assert_eq!(second.sequence, first.sequence + 1);
    assert!(second.at >= first.at);
    assert!(matches!(
        second.kind,
        OperationalEventKind::MediaActivity { generation: 2 }
    ));
    wait_until(|| graph.latest_snapshot().source_frames == 101).await;
    assert_eq!(graph.latest_snapshot().source_frames, 101);

    adapter.end(connection_id).await;
    wait_until(|| orchestrator.connection_lifecycle_task_count() == 1).await;
    orchestrator.drain_connection_lifecycle_tasks().await;
    assert_eq!(orchestrator.connection_lifecycle_task_count(), 0);
}

#[tokio::test]
async fn terminal_retirement_suppresses_pending_activity_and_drain_reaps_observer() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut operational = orchestrator.install_operational_event_stream(8).unwrap();
    let adapter = TestAdapter::new();
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .unwrap();
    let connection_id = ConnectionId::new();
    let (stream, source) = TestStream::new();
    let stream_id = stream.id();
    adapter.add_connection(connection_id.clone(), stream);
    connect(
        &orchestrator,
        &adapter,
        &mut operational,
        connection_id.clone(),
    )
    .await;
    orchestrator
        .media_graph_for_connection(connection_id.clone())
        .await
        .unwrap();
    assert_eq!(orchestrator.connection_lifecycle_task_count(), 2);

    source.send(frame(&stream_id, 1)).await.unwrap();
    adapter.end(connection_id.clone()).await;
    let terminal = operational.recv().await.expect("terminal event");
    assert_eq!(terminal.connection_id, connection_id);
    assert!(matches!(terminal.kind, OperationalEventKind::Ended { .. }));
    assert!(
        tokio::time::timeout(Duration::from_millis(150), operational.recv())
            .await
            .is_err(),
        "retired lifecycle cannot publish retained graph activity"
    );
    wait_until(|| orchestrator.connection_lifecycle_task_count() == 1).await;
    orchestrator.drain_connection_lifecycle_tasks().await;
    assert_eq!(orchestrator.connection_lifecycle_task_count(), 0);
}

#[tokio::test]
async fn operational_backpressure_coalesces_without_stalling_the_media_graph() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut operational = orchestrator.install_operational_event_stream(1).unwrap();
    let adapter = TestAdapter::new();
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .unwrap();
    let connection_id = ConnectionId::new();
    let (stream, source) = TestStream::new();
    let stream_id = stream.id();
    adapter.add_connection(connection_id.clone(), stream);
    connect(
        &orchestrator,
        &adapter,
        &mut operational,
        connection_id.clone(),
    )
    .await;
    let graph = orchestrator
        .media_graph_for_connection(connection_id.clone())
        .await
        .unwrap();

    adapter
        .send(AdapterEvent::Dtmf {
            connection_id: connection_id.clone(),
            digits: "5".into(),
            duration_ms: 80,
        })
        .await;
    wait_until(|| operational.len() == 1).await;

    for sequence in 0..100 {
        source.send(frame(&stream_id, sequence)).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    for sequence in 100..200 {
        source.send(frame(&stream_id, sequence)).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    for sequence in 200..300 {
        source.send(frame(&stream_id, sequence)).await.unwrap();
    }
    // Wait beyond the third graph observation tick while the authoritative
    // receiver remains saturated. The watch slot must retain only the newest
    // of the second and third observations.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    wait_until(|| graph.latest_snapshot().source_frames == 300).await;
    assert_eq!(orchestrator.connection_lifecycle_task_count(), 2);
    assert_eq!(
        orchestrator.operational_event_stream_health(),
        OperationalEventStreamHealth::Healthy
    );

    assert!(matches!(
        operational.recv().await.unwrap().kind,
        OperationalEventKind::Dtmf { .. }
    ));
    let first = next_activity(&mut operational).await;
    assert!(matches!(
        first.kind,
        OperationalEventKind::MediaActivity { generation: 1 }
    ));
    let second = next_activity(&mut operational).await;
    assert!(matches!(
        second.kind,
        OperationalEventKind::MediaActivity { generation: 2 }
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), operational.recv())
            .await
            .is_err(),
        "three activity windows collapse to two deliveries while blocked"
    );
    assert_eq!(
        orchestrator.operational_event_stream_health(),
        OperationalEventStreamHealth::Healthy
    );

    adapter.end(connection_id).await;
    let _ = operational.recv().await;
    orchestrator.drain_connection_lifecycle_tasks().await;
}
