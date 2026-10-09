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
    transport: Transport,
    snapshot_mode: std::sync::atomic::AtomicUsize,
    snapshot_calls: std::sync::atomic::AtomicUsize,
    snapshot_inflight: std::sync::atomic::AtomicUsize,
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
            transport: Transport::Sip,
            snapshot_mode: std::sync::atomic::AtomicUsize::new(0),
            snapshot_calls: std::sync::atomic::AtomicUsize::new(0),
            snapshot_inflight: std::sync::atomic::AtomicUsize::new(0),
        });
        (a, tx, stream)
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for OneStreamAdapter {
    fn transport(&self) -> Transport {
        self.transport
    }
    async fn resource_snapshot(
        &self,
    ) -> RvResult<Option<rvoip_core::resources::AdapterResourceCounts>> {
        use std::sync::atomic::Ordering;
        self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
        match self.snapshot_mode.load(Ordering::SeqCst) {
            1 => {
                let mut counts = rvoip_core::resources::AdapterResourceCounts::default();
                counts.registered_connections = Some(0);
                Ok(Some(counts))
            }
            2 => {
                struct Inflight<'a>(&'a std::sync::atomic::AtomicUsize);
                impl Drop for Inflight<'_> {
                    fn drop(&mut self) {
                        self.0.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                self.snapshot_inflight.fetch_add(1, Ordering::SeqCst);
                let _guard = Inflight(&self.snapshot_inflight);
                std::future::pending().await
            }
            3 => Err(RvoipError::InvalidState("private adapter failure detail")),
            _ => Ok(None),
        }
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

/// TTS provider emitting a fixed number of destination-encoded frames, so a
/// test can assert exactly what `play_audio` pumped into the stream. It
/// records the request so tests can check what the provider was told.
struct CountedTts {
    frames: usize,
    request_seen: Arc<Mutex<Option<rvoip_harness::TtsRequest>>>,
}

impl CountedTts {
    fn new(frames: usize) -> Self {
        Self {
            frames,
            request_seen: Arc::default(),
        }
    }
}

struct CountedPlayback {
    remaining: Mutex<usize>,
    codec: CodecInfo,
}

#[async_trait::async_trait]
impl rvoip_harness::TtsPlayback for CountedPlayback {
    fn audio_format(&self) -> rvoip_harness::TtsAudioFormat {
        rvoip_harness::TtsAudioFormat::Encoded {
            codec: self.codec.clone(),
        }
    }
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
        request: rvoip_harness::TtsRequest,
    ) -> RvResult<Box<dyn rvoip_harness::TtsPlayback>> {
        let codec = request
            .destination_codec
            .clone()
            .expect("play_audio names the destination codec");
        *self.request_seen.lock().unwrap() = Some(request);
        Ok(Box::new(CountedPlayback {
            remaining: Mutex::new(self.frames),
            codec,
        }))
    }
}

/// A spoken prompt must reach the connection's audio stream through the
/// orchestrator's own TTS registry — the adapter contributes only the
/// stream, never a TTS stack. This is the path call-control PlayPrompt
/// rides on transports whose adapters do not implement `play_audio`.
#[tokio::test]
async fn play_audio_tts_pumps_synthesized_frames_into_the_stream() {
    let (orch, _tx, stream, connid) = setup().await;
    let tts = Arc::new(CountedTts::new(5));
    let request_seen = Arc::clone(&tts.request_seen);
    orch.register_tts_provider("counted", tts);

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
    let request = request_seen.lock().unwrap().take().expect("synthesized");
    let codec = request.destination_codec.expect("destination codec");
    assert_eq!(codec.name, "PCMU");
    assert_eq!(codec.clock_rate_hz, 8_000);
    assert_eq!(codec.channels, 1);
    assert_eq!(
        request.sample_rate_hz, None,
        "an RTP clock rate is never passed off as a PCM rate"
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
    fn audio_format(&self) -> rvoip_harness::TtsAudioFormat {
        rvoip_harness::TtsAudioFormat::Encoded {
            codec: CodecInfo {
                name: "PCMU".into(),
                clock_rate_hz: 8_000,
                channels: 1,
                fmtp: None,
                payload_type: None,
            },
        }
    }
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
    orch.register_tts_provider("counted", Arc::new(CountedTts::new(5)));
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

#[tokio::test(start_paused = true)]
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
    let a_at = tokio::time::Instant::now();
    let b = tokio::time::timeout(Duration::from_secs(1), output.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.payload.as_ref(), &[0xce; 160]);
    assert_eq!(a.stream_id, stream.id);
    assert_eq!(a.payload_type, Some(0));
    assert_eq!(b.timestamp_rtp.wrapping_sub(a.timestamp_rtp), 160);
    assert_eq!(a_at.elapsed(), Duration::from_millis(20), "paced delivery");
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

#[tokio::test]
async fn resource_snapshot_separates_live_objects_from_retained_terminal_state() {
    use rvoip_core::resources::AdapterResourceObservation;
    let (orch, tx, _stream, conn) = setup().await;
    let session = orch.session_of(&conn).unwrap();
    let conversation = orch
        .session(&session)
        .unwrap()
        .read()
        .unwrap()
        .conversation_id
        .clone();
    let before = orch
        .resource_snapshot(Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(before.core.conversations.live, 1);
    assert_eq!(before.core.sessions.live, 1);
    assert_eq!(before.core.connection_routes, 1);
    assert_eq!(before.core.retained_connection_ids, 1);
    assert_eq!(before.core.retired_connection_ids, 0);
    assert_eq!(
        before.adapters[0].observation,
        AdapterResourceObservation::Unsupported
    );
    tx.send(AdapterEvent::Ended {
        connection_id: conn.clone(),
        reason: EndReason::Normal,
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while orch
            .resource_snapshot(Duration::ZERO)
            .await
            .unwrap()
            .core
            .connection_routes
            != 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    orch.end_session(session.clone(), EndReason::Normal)
        .await
        .unwrap();
    orch.close_conversation(conversation.clone(), false)
        .await
        .unwrap();
    orch.spawn_media_quality_sampler(Duration::from_secs(60));
    let after = orch
        .resource_snapshot(Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(after.core.conversations.live, 0);
    assert_eq!(after.core.conversations.retained_terminal, 1);
    assert_eq!(after.core.sessions.live, 0);
    assert_eq!(after.core.sessions.retained_terminal, 1);
    assert_eq!(after.core.retained_connection_ids, 1);
    assert_eq!(after.core.retired_connection_ids, 1);
    assert_eq!(after.core.periodic_workers, 1);
    let json = serde_json::to_string(&after).unwrap();
    for id in [
        conn.to_string(),
        session.to_string(),
        conversation.to_string(),
    ] {
        assert!(!json.contains(&id));
    }
    orch.drain_connection_lifecycle_tasks().await;
    assert_eq!(
        orch.resource_snapshot(Duration::ZERO)
            .await
            .unwrap()
            .core
            .periodic_workers,
        0
    );
}

#[tokio::test]
async fn resource_snapshot_collects_concurrently_and_drops_timed_out_hooks() {
    use rvoip_core::resources::AdapterResourceObservation;
    use std::sync::atomic::Ordering;
    let orch = Orchestrator::new(Config::default());
    let mut adapters = Vec::new();
    for (transport, mode) in [
        (Transport::Sip, 1),
        (Transport::WebRtc, 2),
        (Transport::Vapi, 3),
    ] {
        let (mut adapter, _events, _stream) = OneStreamAdapter::new();
        Arc::get_mut(&mut adapter).unwrap().transport = transport;
        adapter.snapshot_mode.store(mode, Ordering::SeqCst);
        orch.register(adapter.clone()).unwrap();
        adapters.push(adapter);
    }
    let snapshot = tokio::time::timeout(
        Duration::from_secs(1),
        orch.resource_snapshot(Duration::from_millis(30)),
    )
    .await
    .unwrap()
    .unwrap();
    let observation = |transport| {
        &snapshot
            .adapters
            .iter()
            .find(|item| item.transport == transport)
            .unwrap()
            .observation
    };
    let AdapterResourceObservation::Reported(counts) = observation(Transport::Sip) else {
        panic!("fast hook must complete despite another pending hook");
    };
    assert_eq!(counts.registered_connections, Some(0));
    assert_eq!(counts.allocated_media_ports, None);
    assert_eq!(
        observation(Transport::WebRtc),
        &AdapterResourceObservation::TimedOut
    );
    assert_eq!(
        observation(Transport::Vapi),
        &AdapterResourceObservation::Failed
    );
    assert_eq!(adapters[1].snapshot_inflight.load(Ordering::SeqCst), 0);
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("private adapter failure detail"));
    orch.drain_connection_lifecycle_tasks().await;
}

#[tokio::test]
async fn resource_snapshot_zero_budget_skips_hooks() {
    use std::sync::atomic::Ordering;
    let orch = Orchestrator::new(Config::default());
    let (adapter, _events, _stream) = OneStreamAdapter::new();
    adapter.snapshot_mode.store(2, Ordering::SeqCst);
    orch.register(adapter.clone()).unwrap();
    let snapshot = orch.resource_snapshot(Duration::ZERO).await.unwrap();
    assert_eq!(
        snapshot.adapters[0].observation,
        rvoip_core::resources::AdapterResourceObservation::TimedOut
    );
    assert_eq!(adapter.snapshot_calls.load(Ordering::SeqCst), 0);
    orch.drain_connection_lifecycle_tasks().await;
}

#[tokio::test]
async fn resource_snapshot_zero_admission_capacity_has_zero_reservations() {
    let orch = Orchestrator::new(Config {
        max_concurrent_setups: 0,
        ..Config::default()
    });
    let snapshot = orch.resource_snapshot(Duration::ZERO).await.unwrap();
    assert_eq!(snapshot.core.prepared_outbound_reservations, 0);
    assert_eq!(snapshot.core.connection_routes, 0);
    assert_eq!(snapshot.core.retired_connection_ids, 0);
    assert!(snapshot.adapters.is_empty());
}
