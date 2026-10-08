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
use rvoip_harness::{ListenOnlyDialog, NoOpAsrProvider, NoOpTtsProvider, VecRecordingSink};
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
            name: "PCMU".into(),
            clock_rate_hz: 8_000,
            channels: 1,
            fmtp: None,
            payload_type: None,
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

async fn setup() -> (
    Arc<Orchestrator>,
    mpsc::Sender<AdapterEvent>,
    Arc<TestStream>,
    ConnectionId,
) {
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

/// TTS provider emitting a fixed number of one-byte frames, so a test can
/// assert exactly what `play_audio` pumped into the stream.
struct CountedTts {
    frames: usize,
    codec_seen: Arc<std::sync::atomic::AtomicBool>,
}

struct CountedPlayback {
    remaining: Mutex<usize>,
}

#[async_trait::async_trait]
impl rvoip_harness::TtsPlayback for CountedPlayback {
    async fn next_frame(&self) -> Option<MediaFrame> {
        let mut remaining = self.remaining.lock().unwrap();
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        Some(MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![0x7f]),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: Some(0),
        })
    }
    async fn cancel(&self) -> RvResult<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl rvoip_harness::TtsProvider for CountedTts {
    async fn synthesize(
        &self,
        _request: rvoip_harness::TtsRequest,
    ) -> RvResult<Box<dyn rvoip_harness::TtsPlayback>> {
        Ok(Box::new(CountedPlayback {
            remaining: Mutex::new(self.frames),
        }))
    }
    async fn synthesize_for_codec(
        &self,
        request: rvoip_harness::TtsRequest,
        codec: CodecInfo,
    ) -> RvResult<Box<dyn rvoip_harness::TtsPlayback>> {
        self.codec_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(codec.name, "PCMU");
        assert_eq!(codec.clock_rate_hz, 8_000);
        assert_eq!(codec.channels, 1);
        assert_eq!(request.sample_rate_hz, Some(codec.clock_rate_hz));
        self.synthesize(request).await
    }
}

/// A spoken prompt must reach the connection's audio stream through the
/// orchestrator's own TTS registry — the adapter contributes only the
/// stream, never a TTS stack. This is the path call-control PlayPrompt
/// rides on transports whose adapters do not implement `play_audio`.
#[tokio::test]
async fn play_audio_tts_pumps_synthesized_frames_into_the_stream() {
    let (orch, _tx, stream, connid) = setup().await;
    let codec_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
    orch.register_tts_provider(
        "counted",
        Arc::new(CountedTts {
            frames: 5,
            codec_seen: codec_seen.clone(),
        }),
    );

    let _handle = orch
        .play_audio(
            connid,
            rvoip_core::commands::AudioSource::TtsRequest {
                provider_ref: "counted".into(),
                text: "prompt under test".into(),
                voice: None,
            },
        )
        .await
        .unwrap();

    let mut outbound = stream.outbound_rx.lock().unwrap().take().unwrap();
    let mut received = 0_usize;
    while received < 5 {
        match tokio::time::timeout(Duration::from_millis(500), outbound.recv()).await {
            Ok(Some(_)) => received += 1,
            _ => break,
        }
    }
    assert_eq!(received, 5, "all synthesized frames reach the stream");
    assert!(
        codec_seen.load(std::sync::atomic::Ordering::SeqCst),
        "destination codec must reach the provider"
    );
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

struct ObservedTts {
    requested: Arc<tokio::sync::Notify>,
    cancelled: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl rvoip_harness::TtsProvider for ObservedTts {
    async fn synthesize(
        &self,
        _: rvoip_harness::TtsRequest,
    ) -> RvResult<Box<dyn rvoip_harness::TtsPlayback>> {
        Ok(Box::new(ObservedPlayback {
            requested: self.requested.clone(),
            cancelled: self.cancelled.clone(),
        }))
    }
}
struct ObservedPlayback {
    requested: Arc<tokio::sync::Notify>,
    cancelled: Arc<tokio::sync::Notify>,
}
#[async_trait::async_trait]
impl rvoip_harness::TtsPlayback for ObservedPlayback {
    async fn next_frame(&self) -> Option<MediaFrame> {
        self.requested.notify_one();
        Some(playback_test_frame())
    }
    async fn cancel(&self) -> RvResult<()> {
        self.cancelled.notify_one();
        Ok(())
    }
}
fn playback_test_frame() -> MediaFrame {
    MediaFrame {
        stream_id: StreamId::new(),
        kind: StreamKind::Audio,
        payload: Bytes::from_static(&[0xff]),
        timestamp_rtp: 0,
        captured_at: Utc::now(),
        payload_type: Some(0),
    }
}
async fn blocked_playback(
    orch: &Orchestrator,
    stream: &TestStream,
    conn: ConnectionId,
) -> (
    rvoip_core::adapter::PlaybackHandle,
    Arc<tokio::sync::Notify>,
) {
    // Fill the bounded queue before starting the provider: its first frame
    // must block on output, rather than relying on a timing-sensitive sleep.
    for _ in 0..stream.outbound_tx.max_capacity() {
        stream.outbound_tx.try_send(playback_test_frame()).unwrap();
    }
    let requested = Arc::new(tokio::sync::Notify::new());
    let cancelled = Arc::new(tokio::sync::Notify::new());
    orch.register_tts_provider(
        "observed",
        Arc::new(ObservedTts {
            requested: requested.clone(),
            cancelled: cancelled.clone(),
        }),
    );
    let handle = orch
        .play_audio(
            conn,
            rvoip_core::commands::AudioSource::TtsRequest {
                provider_ref: "observed".into(),
                text: "test".into(),
                voice: None,
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), requested.notified())
        .await
        .unwrap();
    (handle, cancelled)
}
#[tokio::test]
async fn playback_cancel_interrupts_a_full_output_queue() {
    let (orch, _tx, stream, conn) = setup().await;
    let (handle, cancelled) = blocked_playback(&orch, &stream, conn).await;
    handle.cancel().unwrap();
    tokio::time::timeout(Duration::from_secs(1), cancelled.notified())
        .await
        .unwrap();
    orch.drain_playback_tasks().await;
    assert_eq!(orch.playback_task_count(), 0);
}
#[tokio::test]
async fn playback_terminal_connection_cancels_blocked_output() {
    let (orch, tx, stream, conn) = setup().await;
    let (handle, cancelled) = blocked_playback(&orch, &stream, conn.clone()).await;
    tx.send(AdapterEvent::Ended {
        connection_id: conn,
        reason: EndReason::Normal,
    })
    .await
    .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), handle.wait())
            .await
            .unwrap()
            .unwrap(),
        rvoip_core::adapter::PlaybackOutcome::Cancelled
    );
    tokio::time::timeout(Duration::from_secs(1), cancelled.notified())
        .await
        .unwrap();
    orch.drain_playback_tasks().await;
    assert_eq!(orch.playback_task_count(), 0);
}
#[tokio::test]
async fn playback_drain_joins_workers_and_rejects_restart() {
    let (orch, _tx, stream, conn) = setup().await;
    let (handle, cancelled) = blocked_playback(&orch, &stream, conn.clone()).await;
    orch.drain_playback_tasks().await;
    assert_eq!(
        handle.wait().await.unwrap(),
        rvoip_core::adapter::PlaybackOutcome::Cancelled
    );
    tokio::time::timeout(Duration::from_secs(1), cancelled.notified())
        .await
        .unwrap();
    assert_eq!(orch.playback_task_count(), 0);
    assert!(orch
        .play_audio(
            conn,
            rvoip_core::commands::AudioSource::TtsRequest {
                provider_ref: "observed".into(),
                text: "test".into(),
                voice: None,
            }
        )
        .await
        .is_err());
}
#[tokio::test]
async fn playback_reports_completion_and_delivery_failure() {
    let (orch, _tx, stream, conn) = setup().await;
    orch.register_tts_provider("counted", Arc::new(CountedTts { frames: 5 }));
    let source = || rvoip_core::commands::AudioSource::TtsRequest {
        provider_ref: "counted".into(),
        text: "test".into(),
        voice: None,
    };
    assert_eq!(
        orch.play_audio(conn.clone(), source())
            .await
            .unwrap()
            .wait()
            .await
            .unwrap(),
        rvoip_core::adapter::PlaybackOutcome::Completed
    );
    drop(stream.outbound_rx.lock().unwrap().take());
    assert_eq!(
        orch.play_audio(conn, source())
            .await
            .unwrap()
            .wait()
            .await
            .unwrap(),
        rvoip_core::adapter::PlaybackOutcome::Failed
    );
    orch.drain_playback_tasks().await;
}

#[tokio::test]
async fn pcm_playback_paces_wire_frames_and_completes() {
    let (orch, _tx, stream, conn) = setup().await;
    let (mut input, source) = rvoip_core::playback::PcmPlaybackSource::channel(8_000, 2).unwrap();
    let handle = orch.play_pcm(conn, source).await.unwrap();
    input.send(&[1000; 160]).await.unwrap();
    input.finish(&[1000; 160]).await.unwrap();
    let mut output = stream.outbound_rx.lock().unwrap().take().unwrap();
    let a = tokio::time::timeout(Duration::from_secs(1), output.recv())
        .await
        .unwrap()
        .unwrap();
    let b = tokio::time::timeout(Duration::from_secs(1), output.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.payload.as_ref(), &[0xce; 160]);
    assert_eq!(a.stream_id, stream.id);
    assert_eq!(a.payload_type, Some(0));
    assert_eq!(b.timestamp_rtp.wrapping_sub(a.timestamp_rtp), 160);
    assert!((b.captured_at - a.captured_at).num_milliseconds() >= 20);
    assert_eq!(
        handle.wait().await.unwrap(),
        rvoip_core::adapter::PlaybackOutcome::Completed
    );
    orch.drain_playback_tasks().await;
    assert_eq!(orch.playback_task_count(), 0);
}

#[tokio::test]
async fn pcm_playback_teardown_cancels_an_idle_source_and_closes_its_producer() {
    let (orch, tx, _stream, conn) = setup().await;
    let (mut input, source) = rvoip_core::playback::PcmPlaybackSource::channel(8_000, 1).unwrap();
    let handle = orch.play_pcm(conn.clone(), source).await.unwrap();
    tx.send(AdapterEvent::Ended {
        connection_id: conn,
        reason: EndReason::Normal,
    })
    .await
    .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), handle.wait())
            .await
            .unwrap()
            .unwrap(),
        rvoip_core::adapter::PlaybackOutcome::Cancelled
    );
    assert!(input.send(&[0; 160]).await.is_err());
    orch.drain_playback_tasks().await;
}

#[tokio::test]
async fn pcm_playback_cancel_interrupts_blocked_delivery() {
    let (orch, _tx, stream, conn) = setup().await;
    for _ in 0..stream.outbound_tx.max_capacity() {
        stream.outbound_tx.try_send(playback_test_frame()).unwrap();
    }
    let (mut input, source) = rvoip_core::playback::PcmPlaybackSource::channel(8_000, 1).unwrap();
    input.send(&[0; 160]).await.unwrap();
    let handle = orch.play_pcm(conn, source).await.unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    handle.cancel().unwrap();
    // Admission of a new chunk may race cancellation, so wait for receiver
    // closure rather than assuming the cancellation send joins the task.
    tokio::time::timeout(Duration::from_secs(1), async {
        while input.send(&[0; 160]).await.is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    orch.drain_playback_tasks().await;
    assert_eq!(orch.playback_task_count(), 0);
}
