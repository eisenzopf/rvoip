//! P5 — recording + AI attach acceptance.
//! Bridge / adapter integration: minimal stub adapter that returns a
//! single MediaStream so the recording pump has something to consume.

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::commands::{InboundAction, RecordingTarget};
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::{Result as RvResult, RvoipError};
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, StreamId, TenantId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_harness::{NoOpAsrProvider, NoOpTtsProvider, ListenOnlyDialog, VecRecordingSink};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

struct OneStreamAdapter {
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    stream: Arc<TestStream>,
}

struct TestStream {
    id: StreamId,
    inbound_tx: mpsc::Sender<MediaFrame>,
    inbound_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    outbound_tx: mpsc::Sender<MediaFrame>,
    outbound_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
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
        CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
        }
    }
    fn direction(&self) -> Direction {
        Direction::Inbound
    }
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.inbound_rx
            .lock()
            .unwrap()
            .take()
            .expect("frames_in called twice")
    }
    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.outbound_tx.clone()
    }
    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot {
            jitter_ms: 0.0,
            packet_loss_pct: 0.0,
            mos: None,
        }
    }
    async fn close(self: Arc<Self>) -> RvResult<()> {
        Ok(())
    }
}

impl OneStreamAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>, Arc<TestStream>) {
        let (tx, rx) = mpsc::channel(16);
        let (in_tx, in_rx) = mpsc::channel::<MediaFrame>(64);
        let (out_tx, out_rx) = mpsc::channel::<MediaFrame>(64);
        let stream = Arc::new(TestStream {
            id: StreamId::new(),
            inbound_tx: in_tx,
            inbound_rx: Mutex::new(Some(in_rx)),
            outbound_tx: out_tx,
            outbound_rx: Mutex::new(Some(out_rx)),
        });
        let a = Arc::new(Self {
            inbound: Mutex::new(Some(rx)),
            stream: stream.clone(),
        });
        (a, tx, stream)
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for OneStreamAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    async fn originate(&self, _: OriginateRequest) -> RvResult<ConnectionHandle> {
        Err(RvoipError::NotImplemented("orig"))
    }
    async fn accept(&self, _: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn reject(&self, _: ConnectionId, _: RejectReason) -> RvResult<()> {
        Ok(())
    }
    async fn end(&self, _: ConnectionId, _: EndReason) -> RvResult<()> {
        Ok(())
    }
    async fn hold(&self, _: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn resume(&self, _: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn transfer(&self, _: ConnectionId, _: TransferTarget) -> RvResult<()> {
        Ok(())
    }
    async fn streams(&self, _: ConnectionId) -> RvResult<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![self.stream.clone() as Arc<dyn MediaStream>])
    }
    async fn send_message(&self, _: ConnectionId, _: Message) -> RvResult<()> {
        Ok(())
    }
    async fn send_dtmf(&self, _: ConnectionId, _: &str, _: u32) -> RvResult<()> {
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _: ConnectionId,
        _: CapabilityDescriptor,
    ) -> RvResult<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.inbound.lock().unwrap().take().unwrap()
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }
    async fn verify_request_signature(
        &self,
        _: ConnectionId,
        _: SignatureHeaders,
    ) -> RvResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

async fn setup() -> (Arc<Orchestrator>, mpsc::Sender<AdapterEvent>, Arc<TestStream>, ConnectionId) {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx, stream) = OneStreamAdapter::new();
    orch.register(adapter).unwrap();
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let sid = orch
        .start_session(cid, SessionMedium::Voice, vec![])
        .await
        .unwrap();
    let connid = ConnectionId::new();
    tx.send(AdapterEvent::InboundConnection {
        connection: Connection {
            id: connid.clone(),
            session_id: sid.clone(),
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
        },
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.route_inbound_connection(
        connid.clone(),
        InboundAction::Accept {
            session_id: sid,
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .unwrap();
    (orch, tx, stream, connid)
}

#[tokio::test]
async fn recording_collects_frames_and_stop_produces_artifact() {
    let (orch, _tx, stream, connid) = setup().await;
    let sink = Arc::new(VecRecordingSink::new("memory:rec/test"));
    orch.register_recording_sink("test", sink.clone());

    let rid = orch
        .start_recording(RecordingTarget::Connection(connid), "test")
        .await
        .unwrap();
    // Push two frames in.
    for i in 0..2 {
        stream
            .inbound_tx
            .send(MediaFrame {
                stream_id: stream.id.clone(),
                kind: StreamKind::Audio,
                payload: Bytes::from(vec![i as u8; 4]),
                timestamp_rtp: 0,
                captured_at: Utc::now(),
                payload_type: Some(111),
            })
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(80)).await;
    let artifact = orch.stop_recording(rid).await.unwrap();
    assert_eq!(artifact.bytes_written, 8);
    assert_eq!(sink.bytes().len(), 8);
}

#[tokio::test]
async fn attach_ai_emits_ai_attached_and_detach_cleanly() {
    let (orch, _tx, _stream, connid) = setup().await;
    orch.register_asr_provider("noop", Arc::new(NoOpAsrProvider));
    orch.register_tts_provider("noop", Arc::new(NoOpTtsProvider));
    orch.register_dialog_manager("noop", Arc::new(ListenOnlyDialog));

    let mut events = orch.subscribe_events();
    let aid = orch
        .attach_ai(connid, "noop", HashMap::new())
        .await
        .unwrap();
    let mut saw = false;
    for _ in 0..5 {
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(Event::AiAttached { attachment_id, .. })) if attachment_id == aid => {
                saw = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    assert!(saw, "AiAttached emitted");

    orch.detach(rvoip_core::commands::AttachmentRef::Ai(aid))
        .await
        .unwrap();
}
