//! v0.x MP3a — `Orchestrator::fanout_frame` primitive.
//!
//! Exercises the multi-party media routing primitive in isolation. A
//! StubAdapter exposes a `StubMediaStream` per ConnectionId so the test
//! can register multiple "subscribers" and observe frame arrival
//! per-subscriber. Adapter datagram-receive integration (the publisher
//! side calling `fanout_frame`) is MP3b — out of scope here.

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::sync::mpsc;

// ---------------------------------------------------------------- StubMediaStream

struct StubMediaStream {
    id: StreamId,
    kind: StreamKind,
    out_tx: mpsc::Sender<MediaFrame>,
    // The matching receiver is held by the test harness, not the stream.
}

impl StubMediaStream {
    fn new(kind: StreamKind) -> (Arc<Self>, mpsc::Receiver<MediaFrame>) {
        let (out_tx, out_rx) = mpsc::channel(16);
        let stream = Arc::new(Self {
            id: StreamId::new(),
            kind,
            out_tx,
        });
        (stream, out_rx)
    }
}

#[async_trait::async_trait]
impl MediaStream for StubMediaStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }
    fn kind(&self) -> StreamKind {
        self.kind
    }
    fn codec(&self) -> CodecInfo {
        CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
        }
    }
    fn direction(&self) -> Direction {
        Direction::Outbound
    }
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        // Not exercised by fanout_frame; return a dead channel.
        let (_, rx) = mpsc::channel(1);
        rx
    }
    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.out_tx.clone()
    }
    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }
    async fn close(self: Arc<Self>) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- StubAdapter

struct StubAdapter {
    streams: Arc<dashmap::DashMap<ConnectionId, Vec<Arc<dyn MediaStream>>>>,
    inbound: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl StubAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        let (tx, rx) = mpsc::channel(64);
        let adapter = Arc::new(Self {
            streams: Arc::new(dashmap::DashMap::new()),
            inbound: StdMutex::new(Some(rx)),
        });
        (adapter, tx)
    }

    fn add_streams(&self, conn: ConnectionId, streams: Vec<Arc<dyn MediaStream>>) {
        self.streams.insert(conn, streams);
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for StubAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Substrate
    }
    async fn originate(&self, _request: OriginateRequest) -> Result<ConnectionHandle> {
        Err(RvoipError::NotImplemented("stub originate"))
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
    async fn streams(&self, conn: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>> {
        Ok(self
            .streams
            .get(&conn)
            .map(|e| e.value().clone())
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
        self.inbound
            .lock()
            .unwrap()
            .take()
            .expect("subscribe_events already consumed")
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

// ---------------------------------------------------------------- helpers

fn fake_inbound(connid: ConnectionId) -> Connection {
    Connection {
        id: connid,
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

fn frame(kind: StreamKind, body: &[u8]) -> MediaFrame {
    MediaFrame {
        stream_id: StreamId::new(),
        kind,
        payload: Bytes::copy_from_slice(body),
        timestamp_rtp: 0,
        captured_at: Utc::now(),
        payload_type: None,
    }
}

/// Register a Connection with the orchestrator by emitting an
/// `InboundConnection` AdapterEvent and waiting for the normalization
/// loop to track it. Returns once the orchestrator can resolve the
/// ConnectionId to its adapter (i.e. `subscribers_for`-adjacent
/// machinery works).
async fn register_connection(events_tx: &mpsc::Sender<AdapterEvent>, connection: Connection) {
    events_tx
        .send(AdapterEvent::InboundConnection { connection })
        .await
        .unwrap();
    // Brief yield so the orchestrator's per-adapter event normalizer
    // processes the event and tracks the connection.
    tokio::time::sleep(Duration::from_millis(20)).await;
}

// ---------------------------------------------------------------- tests

#[tokio::test]
async fn fanout_frame_delivers_to_all_subscribers() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, events_tx) = StubAdapter::new();
    orch.register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .unwrap();

    let sid = SessionId::new();
    let publisher_connid = ConnectionId::new();
    let strm_id = StreamId::new();
    let sub_a = ConnectionId::new();
    let sub_b = ConnectionId::new();
    let sub_c = ConnectionId::new();

    // Build stub MediaStreams for the three subscribers.
    let (stream_a, mut rx_a) = StubMediaStream::new(StreamKind::Audio);
    let (stream_b, mut rx_b) = StubMediaStream::new(StreamKind::Audio);
    let (stream_c, mut rx_c) = StubMediaStream::new(StreamKind::Audio);
    adapter.add_streams(sub_a.clone(), vec![stream_a]);
    adapter.add_streams(sub_b.clone(), vec![stream_b]);
    adapter.add_streams(sub_c.clone(), vec![stream_c]);

    // Register all three Connections via the adapter event bus so the
    // orchestrator's connections map can resolve them.
    register_connection(&events_tx, fake_inbound(sub_a.clone())).await;
    register_connection(&events_tx, fake_inbound(sub_b.clone())).await;
    register_connection(&events_tx, fake_inbound(sub_c.clone())).await;

    // Wire all three as subscribers of (sid, publisher, strm_id).
    orch.add_subscription(
        sid.clone(),
        sub_a,
        publisher_connid.clone(),
        strm_id.clone(),
    );
    orch.add_subscription(
        sid.clone(),
        sub_b,
        publisher_connid.clone(),
        strm_id.clone(),
    );
    orch.add_subscription(
        sid.clone(),
        sub_c,
        publisher_connid.clone(),
        strm_id.clone(),
    );

    // Fanout.
    let f = frame(StreamKind::Audio, b"hello-multi-party");
    let delivered = orch
        .fanout_frame(&sid, &publisher_connid, &strm_id, f.clone())
        .await;
    assert_eq!(delivered, 3, "expected all 3 subscribers to receive");

    // Every subscriber's frames_out side should have received the frame.
    for rx in [&mut rx_a, &mut rx_b, &mut rx_c] {
        let got = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("frame arrives within deadline")
            .expect("channel still open");
        assert_eq!(&got.payload[..], b"hello-multi-party");
        assert_eq!(got.kind, StreamKind::Audio);
    }
}

#[tokio::test]
async fn fanout_frame_is_cross_session_isolated() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, events_tx) = StubAdapter::new();
    orch.register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .unwrap();

    let sid_alpha = SessionId::new();
    let sid_beta = SessionId::new();
    let publisher_connid = ConnectionId::new();
    let strm_id = StreamId::new();
    let sub_alpha = ConnectionId::new();
    let sub_beta = ConnectionId::new();

    let (stream_alpha, mut rx_alpha) = StubMediaStream::new(StreamKind::Audio);
    let (stream_beta, mut rx_beta) = StubMediaStream::new(StreamKind::Audio);
    adapter.add_streams(sub_alpha.clone(), vec![stream_alpha]);
    adapter.add_streams(sub_beta.clone(), vec![stream_beta]);
    register_connection(&events_tx, fake_inbound(sub_alpha.clone())).await;
    register_connection(&events_tx, fake_inbound(sub_beta.clone())).await;

    // sub_alpha subscribes in sid_alpha only; sub_beta in sid_beta only.
    orch.add_subscription(
        sid_alpha.clone(),
        sub_alpha,
        publisher_connid.clone(),
        strm_id.clone(),
    );
    orch.add_subscription(
        sid_beta.clone(),
        sub_beta,
        publisher_connid.clone(),
        strm_id.clone(),
    );

    let delivered = orch
        .fanout_frame(
            &sid_alpha,
            &publisher_connid,
            &strm_id,
            frame(StreamKind::Audio, b"alpha-only"),
        )
        .await;
    assert_eq!(delivered, 1, "only sid_alpha subscriber should receive");

    // sub_alpha got it.
    let _ = tokio::time::timeout(Duration::from_millis(200), rx_alpha.recv())
        .await
        .expect("alpha subscriber receives")
        .unwrap();
    // sub_beta did not.
    assert!(
        tokio::time::timeout(Duration::from_millis(60), rx_beta.recv())
            .await
            .is_err(),
        "beta subscriber must NOT receive a frame fanned out in sid_alpha"
    );
}

#[tokio::test]
async fn fanout_frame_with_no_subscribers_returns_zero() {
    let orch = Orchestrator::new(Config::default());
    let delivered = orch
        .fanout_frame(
            &SessionId::new(),
            &ConnectionId::new(),
            &StreamId::new(),
            frame(StreamKind::Audio, b"into-the-void"),
        )
        .await;
    assert_eq!(delivered, 0);
}

#[tokio::test]
async fn fanout_frame_skips_subscribers_without_matching_stream_kind() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, events_tx) = StubAdapter::new();
    orch.register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .unwrap();

    let sid = SessionId::new();
    let publisher_connid = ConnectionId::new();
    let strm_id = StreamId::new();
    let sub_with_audio = ConnectionId::new();
    let sub_video_only = ConnectionId::new();

    let (audio_stream, mut audio_rx) = StubMediaStream::new(StreamKind::Audio);
    let (video_stream, mut video_rx) = StubMediaStream::new(StreamKind::Video);
    adapter.add_streams(sub_with_audio.clone(), vec![audio_stream]);
    adapter.add_streams(sub_video_only.clone(), vec![video_stream]);
    register_connection(&events_tx, fake_inbound(sub_with_audio.clone())).await;
    register_connection(&events_tx, fake_inbound(sub_video_only.clone())).await;

    orch.add_subscription(
        sid.clone(),
        sub_with_audio,
        publisher_connid.clone(),
        strm_id.clone(),
    );
    orch.add_subscription(
        sid.clone(),
        sub_video_only,
        publisher_connid.clone(),
        strm_id.clone(),
    );

    let delivered = orch
        .fanout_frame(
            &sid,
            &publisher_connid,
            &strm_id,
            frame(StreamKind::Audio, b"audio-only-fanout"),
        )
        .await;
    assert_eq!(
        delivered, 1,
        "only the audio-having subscriber should receive an Audio frame"
    );
    let _ = tokio::time::timeout(Duration::from_millis(200), audio_rx.recv())
        .await
        .expect("audio sub got it")
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(60), video_rx.recv())
            .await
            .is_err(),
        "video-only subscriber must not receive an Audio frame"
    );
}

#[tokio::test]
async fn remove_subscription_stops_fanout() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, events_tx) = StubAdapter::new();
    orch.register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .unwrap();

    let sid = SessionId::new();
    let publisher_connid = ConnectionId::new();
    let strm_id = StreamId::new();
    let sub = ConnectionId::new();

    let (stream, mut rx) = StubMediaStream::new(StreamKind::Audio);
    adapter.add_streams(sub.clone(), vec![stream]);
    register_connection(&events_tx, fake_inbound(sub.clone())).await;

    orch.add_subscription(
        sid.clone(),
        sub.clone(),
        publisher_connid.clone(),
        strm_id.clone(),
    );

    // First fanout — subscribed → delivers.
    assert_eq!(
        orch.fanout_frame(
            &sid,
            &publisher_connid,
            &strm_id,
            frame(StreamKind::Audio, b"frame-1"),
        )
        .await,
        1
    );
    let _ = tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .unwrap()
        .unwrap();

    // Unsubscribe.
    orch.remove_subscription(&sid, &sub, &publisher_connid, &strm_id);

    // Second fanout — no subscribers, no delivery.
    assert_eq!(
        orch.fanout_frame(
            &sid,
            &publisher_connid,
            &strm_id,
            frame(StreamKind::Audio, b"frame-2"),
        )
        .await,
        0
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(60), rx.recv())
            .await
            .is_err(),
        "unsubscribed connection must not receive further frames"
    );
}

#[tokio::test]
async fn authenticated_adapter_event_emits_connection_authenticated_event() {
    // A3 regression: the orchestrator must translate
    // `AdapterEvent::Authenticated` into a top-level
    // `Event::ConnectionAuthenticated` on its event bus so external
    // consumers (admission controllers, SIP-bridge routers) can react
    // to auth completion without subscribing to adapter internals.
    use rvoip_core::events::Event;
    use rvoip_core::identity::{AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance};

    let orch = Orchestrator::new(Config::default());
    let (adapter, events_tx) = StubAdapter::new();
    orch.register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .unwrap();
    let mut events = orch.subscribe_events();

    let connid = ConnectionId::new();
    let principal = AuthenticatedPrincipal {
        subject: "id_test_42".into(),
        tenant: Some("tenant-a".into()),
        scopes: vec!["calls:read".into()],
        issuer: Some("https://issuer.example".into()),
        expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(5)),
        method: AuthenticationMethod::Jwt,
        assurance: IdentityAssurance::Anonymous,
    };

    // Adapter emits Authenticated for the new Connection.
    events_tx
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: connid.clone(),
            participant_id: "part_alice".into(),
            principal: principal.clone(),
        })
        .await
        .expect("send");

    // Orchestrator should publish a typed ConnectionAuthenticated event
    // — not a generic Native passthrough.
    let event = loop {
        let ev = tokio::time::timeout(Duration::from_millis(500), events.recv())
            .await
            .expect("event timeout")
            .expect("event bus closed");
        // Skip any unrelated events from the orchestrator's
        // adapter-registration bookkeeping.
        if matches!(&ev, Event::ConnectionAuthenticated { .. }) {
            break ev;
        }
    };

    match event {
        Event::ConnectionAuthenticated {
            connection_id,
            identity_id,
            participant_id,
            assurance,
            ..
        } => {
            assert_eq!(connection_id, connid);
            assert_eq!(identity_id, "id_test_42");
            assert_eq!(participant_id, "part_alice");
            assert!(matches!(assurance, IdentityAssurance::Anonymous));
        }
        other => panic!("expected ConnectionAuthenticated, got {:?}", other),
    }

    let rich_event = loop {
        let ev = tokio::time::timeout(Duration::from_millis(500), events.recv())
            .await
            .expect("event timeout")
            .expect("event bus closed");
        if matches!(&ev, Event::ConnectionPrincipalAuthenticated { .. }) {
            break ev;
        }
    };
    match rich_event {
        Event::ConnectionPrincipalAuthenticated {
            connection_id,
            principal: event_principal,
            ..
        } => {
            assert_eq!(connection_id, connid);
            assert_eq!(event_principal.ownership_key(), principal.ownership_key());
            assert_eq!(
                orch.connection_principal(&connid)
                    .expect("principal retained on route")
                    .ownership_key(),
                principal.ownership_key()
            );
        }
        other => panic!("expected ConnectionPrincipalAuthenticated, got {other:?}"),
    }
}

#[tokio::test]
async fn ended_connection_clears_publisher_registry_rows() {
    // A2 regression: `Orchestrator::forget_connection` must mirror the
    // cleanup into `PublisherRegistry`. Without this, a publisher that
    // hangs up leaves `(sid, strm_id) -> connid` and
    // `(sid, participant) -> [strm_id]` rows pointing at a dead
    // Connection, so a later `from_participant` subscribe resolves to
    // a stale row.
    use rvoip_core::subscriptions::PublisherEntry;

    let orch = Orchestrator::new(Config::default());
    let (adapter, events_tx) = StubAdapter::new();
    orch.register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .unwrap();

    let sid = SessionId::new();
    let publisher_connid = ConnectionId::new();
    let other_publisher = ConnectionId::new();

    // Register the publishing Connection with the orchestrator so
    // `forget_connection` runs against a known entry.
    register_connection(&events_tx, fake_inbound(publisher_connid.clone())).await;

    let registry = orch.publisher_registry();
    registry.register(
        sid.clone(),
        "strm_audio".to_string(),
        PublisherEntry {
            connection: publisher_connid.clone(),
            participant: "alice".to_string(),
            kind: "audio".to_string(),
            codec: None,
        },
    );
    // A second publisher on a different Connection in the same Session
    // — must survive the cleanup, since only the ending connection's
    // rows should go.
    registry.register(
        sid.clone(),
        "strm_alt".to_string(),
        PublisherEntry {
            connection: other_publisher.clone(),
            participant: "bob".to_string(),
            kind: "audio".to_string(),
            codec: None,
        },
    );
    assert!(registry.entry(&sid, "strm_audio").is_some());

    // Trigger forget_connection by ending the publisher's Connection.
    events_tx
        .send(AdapterEvent::Ended {
            connection_id: publisher_connid.clone(),
            reason: EndReason::Normal,
        })
        .await
        .unwrap();
    // Yield so the orchestrator's normalizer processes the event.
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert!(
        registry.entry(&sid, "strm_audio").is_none(),
        "publisher row for the ended Connection must be dropped"
    );
    assert!(
        registry.streams_for_participant(&sid, "alice").is_empty(),
        "the by_participant index must drop alongside the primary row"
    );
    // Bob's row is on a different Connection and must survive.
    assert!(
        registry.entry(&sid, "strm_alt").is_some(),
        "publisher rows for unrelated Connections must not be affected"
    );
}
