use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
use rvoip_core::stream::{
    MediaFrame, MediaReadiness, MediaStream, QualitySnapshot, StreamKind, StreamSelector,
    StreamWaitError,
};
use rvoip_core::{Config, Orchestrator, Result, RvoipError};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct ReadinessStream {
    id: StreamId,
    codec: CodecInfo,
    direction: Direction,
    source_ready: AtomicBool,
    outbound_ready: AtomicBool,
    inbound: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    outbound: mpsc::Sender<MediaFrame>,
}

impl ReadinessStream {
    fn new(
        codec_name: &str,
        direction: Direction,
        source_ready: bool,
        outbound_ready: bool,
    ) -> Arc<Self> {
        let (_source, inbound) = mpsc::channel(1);
        let (outbound, _sink) = mpsc::channel(1);
        Arc::new(Self {
            id: StreamId::new(),
            codec: CodecInfo {
                name: codec_name.to_owned(),
                clock_rate_hz: 8_000,
                channels: 1,
                fmtp: None,
                payload_type: Some(0),
            },
            direction,
            source_ready: AtomicBool::new(source_ready),
            outbound_ready: AtomicBool::new(outbound_ready),
            inbound: Mutex::new(Some(inbound)),
            outbound,
        })
    }

    fn set_source_ready(&self) {
        self.source_ready.store(true, Ordering::Release);
    }

    fn set_outbound_ready(&self) {
        self.outbound_ready.store(true, Ordering::Release);
    }
}

#[async_trait::async_trait]
impl MediaStream for ReadinessStream {
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
        self.direction
    }

    fn source_ready(&self) -> bool {
        self.source_ready.load(Ordering::Acquire)
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.inbound
            .lock()
            .expect("readiness stream receiver lock")
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.outbound.clone()
    }

    fn try_frames_out(&self) -> Result<mpsc::Sender<MediaFrame>> {
        self.outbound_ready
            .load(Ordering::Acquire)
            .then(|| self.outbound.clone())
            .ok_or(RvoipError::InvalidState("outbound media is not ready"))
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }

    async fn close(self: Arc<Self>) -> Result<()> {
        Ok(())
    }
}

struct ReadinessAdapter {
    transport: Transport,
    streams: Mutex<HashMap<ConnectionId, Vec<Arc<dyn MediaStream>>>>,
    live: Mutex<HashSet<ConnectionId>>,
    fail_queries: AtomicBool,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
    receiver: Mutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
}

impl ReadinessAdapter {
    fn new(transport: Transport) -> Arc<Self> {
        let (events, receiver) = mpsc::channel(16);
        Arc::new(Self {
            transport,
            streams: Mutex::new(HashMap::new()),
            live: Mutex::new(HashSet::new()),
            fail_queries: AtomicBool::new(false),
            events,
            receiver: Mutex::new(Some(receiver)),
        })
    }

    fn add_live(&self, connection_id: ConnectionId) {
        self.live
            .lock()
            .expect("live route lock")
            .insert(connection_id);
    }

    fn add_stream(&self, connection_id: ConnectionId, stream: Arc<dyn MediaStream>) {
        self.streams
            .lock()
            .expect("stream registry lock")
            .entry(connection_id)
            .or_default()
            .push(stream);
    }

    async fn announce(&self, connection_id: ConnectionId) {
        self.events
            .send(
                AdapterEvent::InboundConnection {
                    connection: connection(connection_id, self.transport),
                }
                .into(),
            )
            .await
            .expect("announce connection");
    }

    async fn terminate(&self, connection_id: ConnectionId) {
        self.events
            .send(
                AdapterEvent::Ended {
                    connection_id,
                    reason: EndReason::Normal,
                }
                .into(),
            )
            .await
            .expect("terminate connection");
    }

    fn remove_route(&self, connection_id: &ConnectionId) {
        self.live
            .lock()
            .expect("live route lock")
            .remove(connection_id);
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for ReadinessAdapter {
    fn transport(&self) -> Transport {
        self.transport
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    fn is_connection_live(&self, connection_id: &ConnectionId) -> bool {
        self.live
            .lock()
            .expect("live route lock")
            .contains(connection_id)
    }

    async fn originate(&self, _: OriginateRequest) -> Result<ConnectionHandle> {
        Err(RvoipError::NotImplemented("readiness test originate"))
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
        if self.fail_queries.load(Ordering::Acquire) {
            return Err(RvoipError::InvalidState("readiness test query failure"));
        }
        Ok(self
            .streams
            .lock()
            .expect("stream registry lock")
            .get(&connection_id)
            .cloned()
            .unwrap_or_default())
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

fn connection(connection_id: ConnectionId, transport: Transport) -> Connection {
    Connection {
        id: connection_id,
        session_id: SessionId::new(),
        participant_id: ParticipantId::new(),
        transport,
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

async fn wait_until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition deadline");
}

fn deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(2)
}

fn assert_wait_error(
    result: std::result::Result<Arc<dyn MediaStream>, StreamWaitError>,
    expected: StreamWaitError,
) {
    match result {
        Ok(_) => panic!("stream wait unexpectedly succeeded"),
        Err(actual) => assert_eq!(actual, expected),
    }
}

#[tokio::test]
async fn adapter_wait_is_transport_neutral_immediate_and_readiness_aware() {
    for transport in [Transport::Sip, Transport::WebRtc, Transport::Quic] {
        let adapter = ReadinessAdapter::new(transport);
        let connection_id = ConnectionId::new();
        adapter.add_live(connection_id.clone());
        let stream = ReadinessStream::new("PCMU", Direction::Inbound, false, false);
        adapter.add_stream(connection_id.clone(), stream.clone());

        let registered = adapter
            .wait_for_stream(
                connection_id.clone(),
                StreamSelector::new(StreamKind::Audio)
                    .with_codec("pcmu")
                    .with_direction(Direction::Inbound),
                deadline(),
                CancellationToken::new(),
            )
            .await
            .expect("registered stream is immediate");
        assert_eq!(registered.id(), stream.id());

        let source = stream.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            source.set_source_ready();
        });
        adapter
            .wait_for_stream(
                connection_id.clone(),
                StreamSelector::new(StreamKind::Audio).with_readiness(MediaReadiness::SourceReady),
                deadline(),
                CancellationToken::new(),
            )
            .await
            .expect("source readiness is observed");

        let sink = stream.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            sink.set_outbound_ready();
        });
        adapter
            .wait_for_stream(
                connection_id,
                StreamSelector::new(StreamKind::Audio)
                    .with_readiness(MediaReadiness::Bidirectional),
                deadline(),
                CancellationToken::new(),
            )
            .await
            .expect("bidirectional readiness is observed");
    }
}

#[tokio::test]
async fn adapter_wait_reports_cancel_deadline_route_loss_and_query_failure() {
    let adapter = ReadinessAdapter::new(Transport::Sip);
    let connection_id = ConnectionId::new();
    adapter.add_live(connection_id.clone());
    let selector = StreamSelector::new(StreamKind::Audio);

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_wait_error(
        adapter
            .wait_for_stream(
                connection_id.clone(),
                selector.clone(),
                deadline(),
                cancellation,
            )
            .await,
        StreamWaitError::Cancelled,
    );
    assert_wait_error(
        adapter
            .wait_for_stream(
                connection_id.clone(),
                selector.clone(),
                tokio::time::Instant::now(),
                CancellationToken::new(),
            )
            .await,
        StreamWaitError::DeadlineExceeded,
    );
    assert_wait_error(
        adapter
            .wait_for_stream(
                ConnectionId::new(),
                selector.clone(),
                deadline(),
                CancellationToken::new(),
            )
            .await,
        StreamWaitError::AdapterUnavailable,
    );
    adapter.fail_queries.store(true, Ordering::Release);
    assert_wait_error(
        adapter
            .wait_for_stream(
                connection_id,
                selector,
                deadline(),
                CancellationToken::new(),
            )
            .await,
        StreamWaitError::AdapterFailure,
    );
}

#[tokio::test]
async fn orchestrator_wait_distinguishes_missing_and_terminal_lifecycles() {
    let orchestrator = Arc::new(Orchestrator::new(Config::default()));
    let adapter = ReadinessAdapter::new(Transport::Sip);
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register readiness adapter");

    assert_wait_error(
        orchestrator
            .wait_for_stream(
                ConnectionId::new(),
                StreamSelector::new(StreamKind::Audio),
                deadline(),
                CancellationToken::new(),
            )
            .await,
        StreamWaitError::ConnectionNotFound,
    );

    let connection_id = ConnectionId::new();
    adapter.add_live(connection_id.clone());
    adapter.announce(connection_id.clone()).await;
    wait_until(|| orchestrator.connection_transport(&connection_id).is_ok()).await;

    let waiting_orchestrator = orchestrator.clone();
    let waiting_connection = connection_id.clone();
    let waiting = tokio::spawn(async move {
        waiting_orchestrator
            .wait_for_stream(
                waiting_connection,
                StreamSelector::new(StreamKind::Audio),
                deadline(),
                CancellationToken::new(),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    adapter.terminate(connection_id.clone()).await;
    wait_until(|| orchestrator.connection_transport(&connection_id).is_err()).await;
    adapter.remove_route(&connection_id);
    assert_wait_error(
        waiting.await.expect("wait task"),
        StreamWaitError::ConnectionTerminated,
    );
}
