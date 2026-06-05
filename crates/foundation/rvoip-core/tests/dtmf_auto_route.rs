//! Gap plan §4.3 (v1 punch list) — cross-bridge DTMF auto-route.
//!
//! When the orchestrator receives `AdapterEvent::Dtmf` from one leg of
//! a cross-transport bridge, it should automatically dispatch
//! `send_dtmf` to the peer adapter so the digits cross the bridge
//! without application code having to plumb them. This test pins the
//! contract: bridge two connections on different transports, push a
//! DTMF event from leg A, observe leg B's adapter record a matching
//! `send_dtmf` call.
//!
//! Companion to `crates/uctp/rvoip-uctp/tests/dtmf_bridge.rs`, which covers
//! the in-band RTP RFC 4733 path through the frame pump.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason as AdapterEndReason,
    OriginateRequest, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_core::{Config, Orchestrator, RvoipError};
use tokio::sync::mpsc;

#[derive(Default)]
struct DtmfRecord {
    calls: AtomicUsize,
    last_digits: StdMutex<Option<String>>,
    last_duration_ms: AtomicUsize,
}

struct StreamHandle {
    id: StreamId,
    codec: CodecInfo,
    in_rx: StdMutex<Option<mpsc::Receiver<MediaFrame>>>,
    out_tx: mpsc::Sender<MediaFrame>,
}

impl StreamHandle {
    fn new(codec_name: &str) -> Arc<Self> {
        let (_in_tx, in_rx) = mpsc::channel::<MediaFrame>(64);
        let (out_tx, _out_rx) = mpsc::channel::<MediaFrame>(64);
        Arc::new(Self {
            id: StreamId::new(),
            codec: CodecInfo {
                name: codec_name.into(),
                clock_rate_hz: 48000,
                channels: 1,
                fmtp: None,
            },
            in_rx: StdMutex::new(Some(in_rx)),
            out_tx,
        })
    }
}

#[async_trait]
impl MediaStream for StreamHandle {
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

struct RecordingAdapter {
    transport: Transport,
    streams: dashmap::DashMap<ConnectionId, Arc<StreamHandle>>,
    dtmf: Arc<DtmfRecord>,
    events_tx: mpsc::Sender<AdapterEvent>,
    events_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl RecordingAdapter {
    fn new(transport: Transport) -> (Arc<Self>, Arc<DtmfRecord>) {
        let (events_tx, events_rx) = mpsc::channel(64);
        let dtmf = Arc::new(DtmfRecord::default());
        let adapter = Arc::new(Self {
            transport,
            streams: dashmap::DashMap::new(),
            dtmf: Arc::clone(&dtmf),
            events_tx,
            events_rx: StdMutex::new(Some(events_rx)),
        });
        (adapter, dtmf)
    }

    async fn announce(&self, id: ConnectionId, stream: Arc<StreamHandle>, session: SessionId) {
        self.streams.insert(id.clone(), stream);
        let conn = Connection {
            id,
            session_id: session,
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

    fn push_event(&self, event: AdapterEvent) {
        let tx = self.events_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(event).await;
        });
    }
}

#[async_trait]
impl ConnectionAdapter for RecordingAdapter {
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
        digits: &str,
        duration_ms: u32,
    ) -> rvoip_core::error::Result<()> {
        self.dtmf.calls.fetch_add(1, Ordering::SeqCst);
        *self.dtmf.last_digits.lock().unwrap() = Some(digits.to_string());
        self.dtmf
            .last_duration_ms
            .store(duration_ms as usize, Ordering::SeqCst);
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _c: ConnectionId,
        _caps: CapabilityDescriptor,
    ) -> rvoip_core::error::Result<NegotiatedCodecs> {
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

#[tokio::test]
async fn dtmf_auto_forwards_across_cross_transport_bridge() {
    let _ = tracing_subscriber::fmt::try_init();

    // Two adapters on different transports — Quic plays the "UCTP
    // peer" role (where the dtmf.send envelope arrives), Sip plays
    // the "PSTN leg" role (where digits need to be emitted as RFC
    // 4733 RTP). The adapter contract is the same on both sides; we
    // only need to observe send_dtmf landed.
    let (uctp_adapter, _uctp_dtmf) = RecordingAdapter::new(Transport::Quic);
    let (sip_adapter, sip_dtmf) = RecordingAdapter::new(Transport::Sip);

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(uctp_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register uctp");
    orchestrator
        .register(sip_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register sip");

    let session = SessionId::new();
    let uctp_conn = ConnectionId::new();
    let sip_conn = ConnectionId::new();
    let uctp_stream = StreamHandle::new("opus");
    let sip_stream = StreamHandle::new("g.711-mu");

    uctp_adapter
        .announce(uctp_conn.clone(), Arc::clone(&uctp_stream), session.clone())
        .await;
    sip_adapter
        .announce(sip_conn.clone(), Arc::clone(&sip_stream), session)
        .await;
    // Let the adapter-event pumps register both connections.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut events = orchestrator.subscribe_events();

    orchestrator
        .bridge_connections(uctp_conn.clone(), sip_conn.clone())
        .await
        .expect("bridge");

    // Drain the bridge-created event so it doesn't masquerade as DTMF.
    let _ = tokio::time::timeout(Duration::from_millis(100), events.recv()).await;

    // Simulate the UCTP coordinator decoding a `dtmf.send` envelope:
    // adapter emits AdapterEvent::Dtmf, orchestrator must (a) emit
    // Event::DtmfReceived and (b) auto-forward to the bridged peer.
    uctp_adapter.push_event(AdapterEvent::Dtmf {
        connection_id: uctp_conn.clone(),
        digits: "5".into(),
        duration_ms: 160,
    });

    // Event::DtmfReceived arrives synchronously through the pump.
    let dtmf_event = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match events.recv().await {
                Ok(Event::DtmfReceived {
                    connection_id,
                    digits,
                    ..
                }) => return (connection_id, digits),
                Ok(_) => continue,
                Err(_) => panic!("event channel closed"),
            }
        }
    })
    .await
    .expect("DtmfReceived emitted");
    assert_eq!(dtmf_event.0, uctp_conn);
    assert_eq!(dtmf_event.1, "5");

    // The auto-forward is spawned async; poll up to ~500ms for the
    // SIP-side adapter to record the send_dtmf invocation.
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    while sip_dtmf.calls.load(Ordering::SeqCst) == 0 {
        if std::time::Instant::now() >= deadline {
            panic!("SIP-side send_dtmf was never invoked after AdapterEvent::Dtmf on the UCTP leg");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(sip_dtmf.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        sip_dtmf.last_digits.lock().unwrap().clone(),
        Some("5".to_string())
    );
    assert_eq!(sip_dtmf.last_duration_ms.load(Ordering::SeqCst), 160);
}

#[tokio::test]
async fn dtmf_does_not_forward_when_connection_is_not_bridged() {
    let _ = tracing_subscriber::fmt::try_init();

    let (uctp_adapter, _uctp_dtmf) = RecordingAdapter::new(Transport::Quic);
    let (sip_adapter, sip_dtmf) = RecordingAdapter::new(Transport::Sip);

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(uctp_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register uctp");
    orchestrator
        .register(sip_adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register sip");

    let session = SessionId::new();
    let uctp_conn = ConnectionId::new();
    let uctp_stream = StreamHandle::new("opus");
    uctp_adapter
        .announce(uctp_conn.clone(), Arc::clone(&uctp_stream), session)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut events = orchestrator.subscribe_events();

    // No bridge created — DTMF should still emit the local event but
    // must NOT forward anywhere.
    uctp_adapter.push_event(AdapterEvent::Dtmf {
        connection_id: uctp_conn.clone(),
        digits: "9".into(),
        duration_ms: 100,
    });

    let _evt = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match events.recv().await {
                Ok(Event::DtmfReceived { .. }) => return (),
                Ok(_) => continue,
                Err(_) => panic!("event channel closed"),
            }
        }
    })
    .await
    .expect("DtmfReceived emitted");

    // Wait a beat to confirm no async forward sneaks in.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        sip_dtmf.calls.load(Ordering::SeqCst),
        0,
        "no bridge → no auto-forward"
    );
}

// Silence the unused-import lint when MediaFrame/Bytes/etc. aren't
// referenced in a future trimmed version of this file.
#[allow(dead_code)]
fn _unused() {
    let _ = Bytes::new();
}
