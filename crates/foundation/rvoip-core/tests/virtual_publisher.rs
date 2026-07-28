use std::sync::atomic::{AtomicUsize, Ordering};
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

fn g711(name: &str) -> CodecInfo {
    CodecInfo {
        name: name.to_owned(),
        clock_rate_hz: 8_000,
        channels: 1,
        fmtp: None,
    }
}

#[test]
fn virtual_publisher_setup_future_is_send() {
    fn assert_send<T: Send>(_: T) {}

    let orchestrator = Orchestrator::new(Config::default());
    assert_send(orchestrator.register_virtual_publisher(
        ConnectionId::new(),
        VirtualPublisherDescriptor::new(
            SessionId::new(),
            StreamId::from_string("audio/main"),
            "send-regression",
        ),
    ));
    assert_send(orchestrator.register_virtual_publisher_with_codec(
        ConnectionId::new(),
        VirtualPublisherDescriptor::new(
            SessionId::new(),
            StreamId::from_string("audio/main"),
            "explicit-codec-send-regression",
        ),
        opus(),
    ));
}

struct TestMediaStream {
    id: StreamId,
    codec: CodecInfo,
    inbound: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    inbound_acquisitions: AtomicUsize,
    outbound: mpsc::Sender<MediaFrame>,
}

impl TestMediaStream {
    fn source() -> (Arc<Self>, mpsc::Sender<MediaFrame>) {
        Self::source_with_codec(opus())
    }

    fn source_with_codec(codec: CodecInfo) -> (Arc<Self>, mpsc::Sender<MediaFrame>) {
        let (inbound_tx, inbound_rx) = mpsc::channel(32);
        let (outbound, _outbound_rx) = mpsc::channel(1);
        (
            Arc::new(Self {
                id: StreamId::new(),
                codec,
                inbound: Mutex::new(Some(inbound_rx)),
                inbound_acquisitions: AtomicUsize::new(0),
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
                codec: opus(),
                inbound: Mutex::new(Some(inbound_rx)),
                inbound_acquisitions: AtomicUsize::new(0),
                outbound,
            }),
            outbound_rx,
        )
    }

    fn inbound_acquisitions(&self) -> usize {
        self.inbound_acquisitions.load(Ordering::Acquire)
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
        self.codec.clone()
    }

    fn direction(&self) -> Direction {
        Direction::Inbound
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        let receiver = self
            .inbound
            .lock()
            .expect("inbound lock")
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1);
        self.inbound_acquisitions.fetch_add(1, Ordering::AcqRel);
        receiver
    }

    fn try_frames_in(&self) -> Result<mpsc::Receiver<MediaFrame>> {
        let receiver =
            self.inbound
                .lock()
                .expect("inbound lock")
                .take()
                .ok_or(RvoipError::InvalidState(
                    "test stream receiver already acquired",
                ))?;
        self.inbound_acquisitions.fetch_add(1, Ordering::AcqRel);
        Ok(receiver)
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

fn encoded_frame(
    stream_id: StreamId,
    payload: Vec<u8>,
    timestamp_rtp: u32,
    payload_type: u8,
) -> MediaFrame {
    MediaFrame {
        stream_id,
        kind: StreamKind::Audio,
        payload: Bytes::from(payload),
        timestamp_rtp,
        captured_at: Utc::now(),
        payload_type: Some(payload_type),
    }
}

async fn assert_g711_source_publishes_canonical_opus(source_codec: CodecInfo, source_pt: u8) {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, events) = TestAdapter::new();
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");

    let source_id = ConnectionId::new();
    let subscriber_id = ConnectionId::new();
    let (source, source_tx) = TestMediaStream::source_with_codec(source_codec);
    let source_stream_id = source.id();
    let (subscriber, mut subscriber_rx) = TestMediaStream::sink();
    adapter.add_stream(source_id.clone(), source.clone());
    adapter.add_stream(subscriber_id.clone(), subscriber);
    register_connection(&events, source_id.clone()).await;
    register_connection(&events, subscriber_id.clone()).await;

    let descriptor = VirtualPublisherDescriptor::new(
        SessionId::new(),
        StreamId::from_string("audio/main"),
        "canonical-opus-origin",
    );
    let publisher = orchestrator
        .register_virtual_publisher_with_codec(source_id.clone(), descriptor.clone(), opus())
        .await
        .expect("register canonical Opus publisher");
    orchestrator
        .try_add_direct_subscriptions(
            &descriptor.session_id,
            &subscriber_id,
            &[(source_id.clone(), descriptor.stream_id.clone())],
        )
        .expect("admit direct subscriber");

    // A second Opus sink must share the publisher's codec group rather than
    // taking or decoding the source receiver again.
    let (observer_tx, mut observer_rx) = mpsc::channel(4);
    let observer_route = orchestrator
        .attach_media_sink(source_id.clone(), opus(), observer_tx)
        .await
        .expect("attach canonical Opus observer");
    let graph = orchestrator
        .media_graph_for_connection(source_id.clone())
        .await
        .expect("source graph");
    let installed = graph.snapshot().await;
    assert_eq!(source.inbound_acquisitions(), 1);
    assert_eq!(installed.codec_groups.len(), 1);
    assert_eq!(installed.codec_groups[0].target_payload_type, 111);
    assert_eq!(installed.codec_groups[0].sink_routes.len(), 2);

    let first_timestamp = u32::MAX - 159;
    for (value, timestamp) in [(0xff, first_timestamp), (0x7f, 0)] {
        source_tx
            .send(encoded_frame(
                source_stream_id.clone(),
                vec![value; 160],
                timestamp,
                source_pt,
            ))
            .await
            .expect("send G.711 frame");
    }

    let subscriber_first = tokio::time::timeout(Duration::from_secs(2), subscriber_rx.recv())
        .await
        .expect("first subscriber frame deadline")
        .expect("first subscriber frame");
    let subscriber_second = tokio::time::timeout(Duration::from_secs(2), subscriber_rx.recv())
        .await
        .expect("second subscriber frame deadline")
        .expect("second subscriber frame");
    let observer_first = tokio::time::timeout(Duration::from_secs(2), observer_rx.recv())
        .await
        .expect("first observer frame deadline")
        .expect("first observer frame");
    let observer_second = tokio::time::timeout(Duration::from_secs(2), observer_rx.recv())
        .await
        .expect("second observer frame deadline")
        .expect("second observer frame");

    for received in [
        &subscriber_first,
        &subscriber_second,
        &observer_first,
        &observer_second,
    ] {
        assert_eq!(received.payload_type, Some(111));
        assert!(!received.payload.is_empty());
    }
    assert_eq!(subscriber_first.stream_id, descriptor.stream_id);
    assert_eq!(subscriber_second.stream_id, descriptor.stream_id);
    assert_eq!(
        subscriber_second
            .timestamp_rtp
            .wrapping_sub(subscriber_first.timestamp_rtp),
        960
    );
    assert_eq!(
        observer_second
            .timestamp_rtp
            .wrapping_sub(observer_first.timestamp_rtp),
        960
    );
    assert_eq!(subscriber_first.payload, observer_first.payload);
    assert_eq!(subscriber_second.payload, observer_second.payload);

    let registry_entry = orchestrator
        .publisher_registry()
        .entry(&descriptor.session_id, descriptor.stream_id.as_str())
        .expect("publisher registry row");
    assert_eq!(registry_entry.codec, Some(opus()));
    let completed = graph.snapshot().await;
    assert_eq!(completed.source_frames, 2);
    assert_eq!(completed.transcode_operations, 2);
    assert_eq!(completed.codec_groups[0].transcode_operations, 2);
    assert_eq!(source.inbound_acquisitions(), 1);

    publisher.close().await.expect("close publisher");
    assert!(orchestrator.detach_media_sink(&source_id, observer_route));
}

#[tokio::test]
async fn pcmu_virtual_publisher_transcodes_once_to_canonical_opus() {
    assert_g711_source_publishes_canonical_opus(g711("pcmu"), 0).await;
}

#[tokio::test]
async fn pcma_virtual_publisher_transcodes_once_to_canonical_opus() {
    assert_g711_source_publishes_canonical_opus(g711("pcma"), 8).await;
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
    let publisher = orchestrator
        .register_virtual_publisher(source_id.clone(), descriptor.clone())
        .await
        .expect("register virtual publisher");
    orchestrator
        .try_add_direct_subscriptions(
            &descriptor.session_id,
            &subscriber_id,
            &[(source_id.clone(), descriptor.stream_id.clone())],
        )
        .expect("admit direct subscriber");
    assert_eq!(orchestrator.active_direct_listener_count(), 1);

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
    assert_eq!(orchestrator.active_direct_listener_count(), 0);
    assert!(orchestrator
        .subscribers_for(&descriptor.session_id, &source_id, &descriptor.stream_id,)
        .is_empty());
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
