//! Focused coverage for connection-scoped progress and provisional one-way
//! media routing through an unresolved inbound admission.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, AdapterLifecycleCapabilities, AdapterLifecycleSink,
    AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    OrchestratorAdapterEvent, OriginateRequest, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::events::{ConnectionProgressKind, Event};
use rvoip_core::identity::{AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::operational_events::{OperationalEvent, OperationalEventKind};
use rvoip_core::stream::{
    MediaFrame, MediaReceiverReservation, MediaStream, QualitySnapshot, StreamKind,
};
use rvoip_core::{Config, DataMessage, Orchestrator, Result, RvoipError};
use tokio::sync::mpsc;

struct TestMediaStream {
    id: StreamId,
    codec: CodecInfo,
    inbound_sender: mpsc::Sender<MediaFrame>,
    inbound_receiver: Arc<Mutex<Option<mpsc::Receiver<MediaFrame>>>>,
    outbound_sender: mpsc::Sender<MediaFrame>,
    outbound_receiver: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    source_acquisitions: Arc<AtomicUsize>,
    source_ready: AtomicBool,
    writable: AtomicBool,
}

impl TestMediaStream {
    fn new(writable: bool) -> Arc<Self> {
        let (inbound_sender, inbound_receiver) = mpsc::channel(16);
        let (outbound_sender, outbound_receiver) = mpsc::channel(16);
        Arc::new(Self {
            id: StreamId::new(),
            codec: CodecInfo {
                name: "g.711-mu".into(),
                clock_rate_hz: 8_000,
                channels: 1,
                fmtp: None,
            },
            inbound_sender,
            inbound_receiver: Arc::new(Mutex::new(Some(inbound_receiver))),
            outbound_sender,
            outbound_receiver: Mutex::new(Some(outbound_receiver)),
            source_acquisitions: Arc::new(AtomicUsize::new(0)),
            source_ready: AtomicBool::new(true),
            writable: AtomicBool::new(writable),
        })
    }

    async fn inject(&self, frame: MediaFrame) {
        self.inbound_sender
            .send(frame)
            .await
            .expect("source graph remains live");
    }

    fn take_output(&self) -> mpsc::Receiver<MediaFrame> {
        self.outbound_receiver
            .lock()
            .unwrap()
            .take()
            .expect("output receiver is single-take")
    }

    fn source_acquisitions(&self) -> usize {
        self.source_acquisitions.load(Ordering::Acquire)
    }

    fn set_writable(&self) {
        self.writable.store(true, Ordering::Release);
    }

    fn set_source_ready(&self, ready: bool) {
        self.source_ready.store(ready, Ordering::Release);
    }
}

#[async_trait]
impl MediaStream for TestMediaStream {
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

    fn source_ready(&self) -> bool {
        self.source_ready.load(Ordering::Acquire)
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> Result<mpsc::Receiver<MediaFrame>> {
        Ok(self.reserve_frames_in()?.commit())
    }

    fn reserve_frames_in(&self) -> Result<MediaReceiverReservation> {
        let receiver =
            self.inbound_receiver
                .lock()
                .unwrap()
                .take()
                .ok_or(RvoipError::InvalidState(
                    "test media source was already acquired",
                ))?;
        let slot = Arc::clone(&self.inbound_receiver);
        let acquisitions = Arc::clone(&self.source_acquisitions);
        Ok(MediaReceiverReservation::new(receiver, move |receiver| {
            let mut slot = slot.lock().unwrap();
            if slot.is_none() {
                *slot = Some(receiver);
            }
        })
        .with_commit_hook(move || {
            acquisitions.fetch_add(1, Ordering::AcqRel);
        }))
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.try_frames_out().unwrap_or_else(|_| mpsc::channel(1).0)
    }

    fn try_frames_out(&self) -> Result<mpsc::Sender<MediaFrame>> {
        self.writable
            .load(Ordering::Acquire)
            .then(|| self.outbound_sender.clone())
            .ok_or(RvoipError::InvalidState(
                "test media stream is not writable",
            ))
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }

    async fn close(self: Arc<Self>) -> Result<()> {
        Ok(())
    }
}

struct TestAdapter {
    receiver: Mutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    live: Mutex<HashSet<ConnectionId>>,
    streams: Mutex<HashMap<ConnectionId, Arc<TestMediaStream>>>,
    lifecycle: AdapterLifecycleSinkSlot,
    early_media_calls: AtomicUsize,
}

impl TestAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<OrchestratorAdapterEvent>) {
        let (events, receiver) = mpsc::channel(32);
        (
            Arc::new(Self {
                receiver: Mutex::new(Some(receiver)),
                live: Mutex::new(HashSet::new()),
                streams: Mutex::new(HashMap::new()),
                lifecycle: AdapterLifecycleSinkSlot::default(),
                early_media_calls: AtomicUsize::new(0),
            }),
            events,
        )
    }

    fn add(&self, connection_id: ConnectionId, stream: Arc<TestMediaStream>) {
        self.live.lock().unwrap().insert(connection_id.clone());
        self.streams.lock().unwrap().insert(connection_id, stream);
    }

    fn retire(&self, connection_id: &ConnectionId) {
        self.live.lock().unwrap().remove(connection_id);
    }
}

#[async_trait]
impl ConnectionAdapter for TestAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities::FAIL_CLOSED_INBOUND
    }

    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> Result<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("lifecycle sink already installed"))
    }

    fn is_connection_live(&self, connection_id: &ConnectionId) -> bool {
        self.live.lock().unwrap().contains(connection_id)
    }

    async fn originate(&self, _request: OriginateRequest) -> Result<ConnectionHandle> {
        Err(RvoipError::NotImplemented(
            "test adapter does not originate",
        ))
    }

    async fn start_inbound_early_media(&self, connection_id: ConnectionId) -> Result<()> {
        let stream = self
            .streams
            .lock()
            .unwrap()
            .get(&connection_id)
            .cloned()
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))?;
        if !self.is_connection_live(&connection_id) {
            return Err(RvoipError::ConnectionNotFound(connection_id));
        }
        self.early_media_calls.fetch_add(1, Ordering::AcqRel);
        stream.set_writable();
        Ok(())
    }

    async fn accept(&self, _connection_id: ConnectionId) -> Result<()> {
        Ok(())
    }

    async fn reject(&self, connection_id: ConnectionId, _reason: RejectReason) -> Result<()> {
        self.retire(&connection_id);
        Ok(())
    }

    async fn end(&self, connection_id: ConnectionId, _reason: EndReason) -> Result<()> {
        self.retire(&connection_id);
        Ok(())
    }

    async fn hold(&self, _connection_id: ConnectionId) -> Result<()> {
        Ok(())
    }

    async fn resume(&self, _connection_id: ConnectionId) -> Result<()> {
        Ok(())
    }

    async fn transfer(&self, _connection_id: ConnectionId, _target: TransferTarget) -> Result<()> {
        Ok(())
    }

    async fn streams(&self, connection_id: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>> {
        self.streams
            .lock()
            .unwrap()
            .get(&connection_id)
            .cloned()
            .map(|stream| vec![stream as Arc<dyn MediaStream>])
            .ok_or(RvoipError::ConnectionNotFound(connection_id))
    }

    async fn send_message(&self, _connection_id: ConnectionId, _message: Message) -> Result<()> {
        Ok(())
    }

    async fn send_data_message(
        &self,
        _connection_id: ConnectionId,
        _message: DataMessage,
    ) -> Result<()> {
        Ok(())
    }

    async fn send_dtmf(
        &self,
        _connection_id: ConnectionId,
        _digits: &str,
        _duration_ms: u32,
    ) -> Result<()> {
        Ok(())
    }

    async fn renegotiate_media(
        &self,
        _connection_id: ConnectionId,
        _capabilities: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        mpsc::channel(1).1
    }

    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        self.receiver
            .lock()
            .unwrap()
            .take()
            .expect("adapter event stream is single-consumer")
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }

    async fn verify_request_signature(
        &self,
        _connection_id: ConnectionId,
        _signature: SignatureHeaders,
    ) -> Result<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

fn principal() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: "provisional-media-peer".into(),
        tenant: Some("tenant-a".into()),
        scopes: vec!["call:attach".into()],
        issuer: Some("provisional-media-test".into()),
        expires_at: None,
        method: AuthenticationMethod::MutualTls,
        assurance: IdentityAssurance::Anonymous,
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

async fn publish_inbound(
    adapter: &TestAdapter,
    events: &mpsc::Sender<OrchestratorAdapterEvent>,
    connection_id: ConnectionId,
    stream: Arc<TestMediaStream>,
) {
    adapter.add(connection_id.clone(), stream);
    events
        .send(OrchestratorAdapterEvent::AuthenticatedInboundConnection {
            connection: connection(connection_id),
            participant_id: "peer".into(),
            principal: principal(),
        })
        .await
        .expect("publish authenticated inbound connection");
}

fn frame(sequence: u8) -> MediaFrame {
    MediaFrame {
        stream_id: StreamId::new(),
        kind: StreamKind::Audio,
        payload: Bytes::from(vec![0xff; 160]),
        timestamp_rtp: u32::from(sequence) * 160,
        captured_at: Utc::now(),
        payload_type: Some(0),
    }
}

async fn next_progress(operational: &mut mpsc::Receiver<OperationalEvent>) -> OperationalEvent {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = operational.recv().await.expect("operational stream live");
            if matches!(event.kind, OperationalEventKind::Progress { .. }) {
                return event;
            }
        }
    })
    .await
    .expect("progress operational deadline")
}

#[tokio::test]
async fn pending_admission_routes_early_media_without_answer_or_target_source_consumption() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut operational = orchestrator
        .install_operational_event_stream(32)
        .expect("operational stream");
    let mut admissions = orchestrator
        .install_inbound_admission_gate(4, Duration::from_secs(2))
        .expect("admission gate");
    let mut public = orchestrator.subscribe_events();
    let (adapter, events) = TestAdapter::new();
    orchestrator
        .register(Arc::clone(&adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");

    let source_id = ConnectionId::new();
    let source_stream = TestMediaStream::new(true);
    source_stream.set_source_ready(false);
    publish_inbound(
        &adapter,
        &events,
        source_id.clone(),
        Arc::clone(&source_stream),
    )
    .await;
    let source_admission = admissions.recv().await.expect("source admission");
    source_admission.accept().await.expect("publish source");

    events
        .send(
            AdapterEvent::Progress {
                connection_id: source_id.clone(),
                status_code: 183,
                reason: "Session Progress".into(),
                early_media: true,
            }
            .into(),
        )
        .await
        .expect("publish scoped progress");
    let progress = next_progress(&mut operational).await;
    assert_eq!(progress.connection_id, source_id);
    assert_eq!(progress.transport, Transport::Sip);
    assert!(matches!(
        progress.kind,
        OperationalEventKind::Progress {
            status_code: 183,
            early_media: true,
        }
    ));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                public.recv().await,
                Ok(Event::ConnectionProgress {
                    ref connection_id,
                    kind: ConnectionProgressKind::EarlyMedia,
                    ..
                }) if connection_id == &source_id
            ) {
                break;
            }
        }
    })
    .await
    .expect("normalized early-media progress deadline");

    let target_id = ConnectionId::new();
    let target_stream = TestMediaStream::new(false);
    let mut target_output = target_stream.take_output();
    publish_inbound(
        &adapter,
        &events,
        target_id.clone(),
        Arc::clone(&target_stream),
    )
    .await;
    let mut target_admission = admissions.recv().await.expect("target admission");
    assert!(target_admission.authenticated_principal().is_ok());

    let source_id_for_setup = source_id.clone();
    let route_setup = tokio::spawn(async move {
        let route = target_admission
            .bridge_early_media_from(source_id_for_setup)
            .await;
        (target_admission, route)
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while adapter.early_media_calls.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("target provisional media setup started");
    assert_eq!(adapter.early_media_calls.load(Ordering::Acquire), 1);
    assert_eq!(source_stream.source_acquisitions(), 0);
    assert_eq!(
        target_stream.source_acquisitions(),
        0,
        "provisional setup must not consume either source before the remote codec is final"
    );
    source_stream.set_source_ready(true);
    let (mut target_admission, route) = route_setup.await.expect("provisional route setup task");
    let route = route.expect("provisional media route");
    assert_eq!(route.source_connection_id(), &source_id);
    assert_eq!(route.target_connection_id(), &target_id);
    assert_eq!(adapter.early_media_calls.load(Ordering::Acquire), 1);
    assert_eq!(source_stream.source_acquisitions(), 1);
    assert_eq!(
        target_stream.source_acquisitions(),
        0,
        "one-way early media must not consume the staged caller's source receiver"
    );
    assert!(target_admission.authenticated_principal().is_ok());

    source_stream.inject(frame(1)).await;
    let delivered = tokio::time::timeout(Duration::from_secs(2), target_output.recv())
        .await
        .expect("early-media frame deadline")
        .expect("early-media target remains live");
    assert_eq!(delivered.payload, Bytes::from(vec![0xff; 160]));
    assert_eq!(delivered.payload_type, Some(0));

    assert!(matches!(
        target_admission
            .bridge_early_media_from(source_id.clone())
            .await,
        Err(RvoipError::InvalidState(
            "inbound admission already started provisional media"
        ))
    ));
    route.stop().await.expect("acknowledged route removal");
    source_stream.inject(frame(2)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), target_output.recv())
            .await
            .is_err(),
        "removed provisional sink must not receive later source frames"
    );
    assert_eq!(source_stream.source_acquisitions(), 1);
    let staged_source = target_stream
        .try_frames_in()
        .expect("target source receiver remains available for final full duplex");
    drop(staged_source);
    assert_eq!(target_stream.source_acquisitions(), 1);
    target_admission
        .accept()
        .await
        .expect("final publication remains possible");
}

#[tokio::test]
async fn stale_pending_generation_cannot_start_adapter_or_graph_side_effects() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(4, Duration::from_secs(2))
        .expect("admission gate");
    let (adapter, events) = TestAdapter::new();
    orchestrator
        .register(Arc::clone(&adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");

    let source_id = ConnectionId::new();
    let source_stream = TestMediaStream::new(true);
    publish_inbound(&adapter, &events, source_id.clone(), source_stream).await;
    admissions
        .recv()
        .await
        .expect("source admission")
        .accept()
        .await
        .expect("publish source");

    let target_id = ConnectionId::new();
    let target_stream = TestMediaStream::new(false);
    publish_inbound(
        &adapter,
        &events,
        target_id.clone(),
        Arc::clone(&target_stream),
    )
    .await;
    let mut stale = admissions.recv().await.expect("target admission");
    adapter.retire(&target_id);
    events
        .send(
            AdapterEvent::Ended {
                connection_id: target_id.clone(),
                reason: EndReason::Cancelled,
            }
            .into(),
        )
        .await
        .expect("publish target terminal");
    tokio::time::timeout(Duration::from_secs(2), async {
        while stale.authenticated_principal().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stale generation retirement deadline");

    assert!(stale.bridge_early_media_from(source_id).await.is_err());
    assert_eq!(adapter.early_media_calls.load(Ordering::Acquire), 0);
    assert_eq!(target_stream.source_acquisitions(), 0);
    assert!(stale.accept().await.is_err());
}
