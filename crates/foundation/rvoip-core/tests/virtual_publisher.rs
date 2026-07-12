use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_core::subscriptions::PublisherEntry;
use rvoip_core::{Config, Orchestrator, Result, RvoipError, VirtualPublisherDescriptor};
use tokio::sync::mpsc;

fn opus() -> CodecInfo {
    CodecInfo {
        name: "opus".to_string(),
        clock_rate_hz: 48_000,
        channels: 1,
        fmtp: None,
    }
}

struct TestMediaStream {
    id: StreamId,
    inbound: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    outbound: mpsc::Sender<MediaFrame>,
}

impl TestMediaStream {
    fn source() -> (Arc<Self>, mpsc::Sender<MediaFrame>) {
        let (inbound_tx, inbound_rx) = mpsc::channel(32);
        let (outbound, _outbound_rx) = mpsc::channel(1);
        (
            Arc::new(Self {
                id: StreamId::new(),
                inbound: Mutex::new(Some(inbound_rx)),
                outbound,
            }),
            inbound_tx,
        )
    }

    fn sink() -> (Arc<Self>, mpsc::Receiver<MediaFrame>) {
        let (_inbound_tx, inbound_rx) = mpsc::channel(1);
        let (outbound, outbound_rx) = mpsc::channel(32);
        (
            Arc::new(Self {
                id: StreamId::new(),
                inbound: Mutex::new(Some(inbound_rx)),
                outbound,
            }),
            outbound_rx,
        )
    }
}

#[async_trait::async_trait]
impl MediaStream for TestMediaStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }

    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }

    fn codec(&self) -> CodecInfo {
        opus()
    }

    fn direction(&self) -> Direction {
        Direction::Inbound
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.inbound
            .lock()
            .expect("inbound lock")
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> Result<mpsc::Receiver<MediaFrame>> {
        self.inbound
            .lock()
            .expect("inbound lock")
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
    streams: dashmap::DashMap<ConnectionId, Vec<Arc<dyn MediaStream>>>,
    events: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl TestAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        let (sender, receiver) = mpsc::channel(32);
        (
            Arc::new(Self {
                streams: dashmap::DashMap::new(),
                events: Mutex::new(Some(receiver)),
            }),
            sender,
        )
    }

    fn add_stream(&self, connection_id: ConnectionId, stream: Arc<dyn MediaStream>) {
        self.streams.insert(connection_id, vec![stream]);
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for TestAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Substrate
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
            .get(&connection_id)
            .map(|streams| streams.value().clone())
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
        self.events
            .lock()
            .expect("event lock")
            .take()
            .expect("adapter events subscribed once")
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

fn connection(id: ConnectionId) -> Connection {
    Connection {
        id,
        session_id: SessionId::new(),
        participant_id: ParticipantId::new(),
        transport: Transport::Sip,
        direction: Direction::Inbound,
        state: ConnectionState::Connecting,
        capabilities: CapabilityDescriptor::default(),
        negotiated_codecs: NegotiatedCodecs::default(),
        streams: vec![],
        messaging_enabled: false,
        transport_handle: TransportHandle(Arc::new(())),
        opened_at: Utc::now(),
        closed_at: None,
    }
}

async fn register_connection(events: &mpsc::Sender<AdapterEvent>, id: ConnectionId) {
    events
        .send(AdapterEvent::InboundConnection {
            connection: connection(id),
        })
        .await
        .expect("send inbound connection");
    tokio::time::sleep(Duration::from_millis(20)).await;
}

fn frame(stream_id: StreamId, payload: &'static [u8]) -> MediaFrame {
    MediaFrame {
        stream_id,
        kind: StreamKind::Audio,
        payload: Bytes::from_static(payload),
        timestamp_rtp: 960,
        captured_at: Utc::now(),
        payload_type: Some(111),
    }
}

#[tokio::test]
async fn virtual_publisher_shares_one_media_graph_and_fans_canonical_stream() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, events) = TestAdapter::new();
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");

    let source_id = ConnectionId::new();
    let subscriber_id = ConnectionId::new();
    let (source, source_tx) = TestMediaStream::source();
    let (subscriber, mut subscriber_rx) = TestMediaStream::sink();
    adapter.add_stream(source_id.clone(), source);
    adapter.add_stream(subscriber_id.clone(), subscriber);
    register_connection(&events, source_id.clone()).await;
    register_connection(&events, subscriber_id.clone()).await;

    let descriptor = VirtualPublisherDescriptor::new(
        SessionId::new(),
        StreamId::from_string("audio/main"),
        "bridgefu-origin",
    );
    orchestrator.add_subscription(
        descriptor.session_id.clone(),
        subscriber_id,
        source_id.clone(),
        descriptor.stream_id.clone(),
    );
    let publisher = orchestrator
        .register_virtual_publisher(source_id.clone(), descriptor.clone())
        .await
        .expect("register virtual publisher");

    let registry = orchestrator.publisher_registry();
    let entry = registry
        .entry(&descriptor.session_id, descriptor.stream_id.as_str())
        .expect("publisher registry row");
    assert_eq!(entry.connection, source_id);
    assert_eq!(entry.participant, "bridgefu-origin");
    assert_eq!(entry.codec, Some(opus()));

    // A second observer attaches after the virtual publisher. Both routes use
    // the reusable graph instead of competing for the source receiver.
    let (observer_tx, mut observer_rx) = mpsc::channel(4);
    let observer_route = orchestrator
        .attach_media_sink(source_id.clone(), opus(), observer_tx)
        .await
        .expect("attach observer");

    source_tx
        .send(frame(StreamId::new(), b"shared-audio"))
        .await
        .expect("send source frame");
    let subscriber_frame = tokio::time::timeout(Duration::from_secs(1), subscriber_rx.recv())
        .await
        .expect("subscriber deadline")
        .expect("subscriber frame");
    let observer_frame = tokio::time::timeout(Duration::from_secs(1), observer_rx.recv())
        .await
        .expect("observer deadline")
        .expect("observer frame");
    assert_eq!(
        subscriber_frame.payload,
        Bytes::from_static(b"shared-audio")
    );
    assert_eq!(subscriber_frame.stream_id, descriptor.stream_id);
    assert_eq!(observer_frame.payload, Bytes::from_static(b"shared-audio"));

    let route_status = publisher.route_status();
    publisher.close().await.expect("close publisher");
    assert!(registry
        .entry(&descriptor.session_id, descriptor.stream_id.as_str())
        .is_none());
    tokio::time::timeout(Duration::from_secs(1), route_status.wait_terminal())
        .await
        .expect("publisher route terminates");
    assert!(orchestrator.detach_media_sink(&source_id, observer_route));
}

#[tokio::test]
async fn virtual_publisher_rejects_duplicate_and_drop_is_exact() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, events) = TestAdapter::new();
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");

    let source_id = ConnectionId::new();
    let (source, _source_tx) = TestMediaStream::source();
    adapter.add_stream(source_id.clone(), source);
    register_connection(&events, source_id.clone()).await;

    let descriptor = VirtualPublisherDescriptor::new(
        SessionId::new(),
        StreamId::from_string("audio/main"),
        "managed",
    );
    let publisher = orchestrator
        .register_virtual_publisher(source_id.clone(), descriptor.clone())
        .await
        .expect("first publisher");
    let duplicate = match orchestrator
        .register_virtual_publisher(source_id.clone(), descriptor.clone())
        .await
    {
        Ok(_) => panic!("duplicate canonical stream must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(duplicate, RvoipError::AdmissionRejected(_)));

    // The compatible legacy API may replace a managed registration. The old
    // handle's generation-scoped Drop must not delete that replacement.
    let replacement_connection = ConnectionId::new();
    let registry = orchestrator.publisher_registry();
    registry.register(
        descriptor.session_id.clone(),
        descriptor.stream_id.to_string(),
        PublisherEntry {
            connection: replacement_connection.clone(),
            participant: "replacement".to_string(),
            kind: "audio".to_string(),
            codec: Some(opus()),
        },
    );
    drop(publisher);

    let replacement = registry
        .entry(&descriptor.session_id, descriptor.stream_id.as_str())
        .expect("replacement survives stale managed drop");
    assert_eq!(replacement.connection, replacement_connection);
    assert!(registry
        .streams_for_participant(&descriptor.session_id, "managed")
        .is_empty());
    assert_eq!(
        registry.streams_for_participant(&descriptor.session_id, "replacement"),
        vec![descriptor.stream_id.to_string()]
    );
}
