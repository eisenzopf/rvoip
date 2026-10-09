//! Deterministic provider implementations for tests and examples.
//!
//! This module is excluded from normal builds. Enable the `test-reference`
//! feature to compose the providers in an example or an external test.

use std::collections::VecDeque;
use std::future::pending;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use rvoip_core_traits::error::{Result, RvoipError};
use rvoip_core_traits::harness::{
    AsrConfig, AsrProvider, AsrResult, AsrStream, DialogAction, DialogManager, TtsAudioFormat,
    TtsPlayback, TtsProvider, TtsRequest,
};
use rvoip_core_traits::ids::{ConnectionId, StreamId};
use rvoip_core_traits::stream::{MediaFrame, StreamKind};
use tokio::sync::Notify;

const PCM_S16LE_PAYLOAD_TYPE: u8 = 120;
const REFERENCE_SAMPLE_RATE_HZ: u32 = 16_000;
const REFERENCE_FRAME_SAMPLES: usize = 320;

/// One cancellable stage in the deterministic cascaded provider set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceStage {
    Asr,
    Dialogue,
    Tts,
}

impl ReferenceStage {
    const fn index(self) -> usize {
        match self {
            Self::Asr => 0,
            Self::Dialogue => 1,
            Self::Tts => 2,
        }
    }

    const fn mask(self) -> u8 {
        1 << self.index()
    }
}

/// Point-in-time work counters for deterministic reference providers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReferenceProviderSnapshot {
    pub active_asr: usize,
    pub active_dialogue: usize,
    pub active_tts: usize,
    pub cancelled_asr: usize,
    pub cancelled_dialogue: usize,
    pub cancelled_tts: usize,
    pub completed_asr: usize,
    pub completed_dialogue: usize,
    pub completed_tts: usize,
}

struct ReferenceProviderControlInner {
    blocked: AtomicU8,
    started: [AtomicUsize; 3],
    active: [AtomicUsize; 3],
    cancelled: [AtomicUsize; 3],
    completed: [AtomicUsize; 3],
    started_notify: [Notify; 3],
    released_notify: [Notify; 3],
}

/// Shared deterministic stage gate and cancellation probe.
///
/// A blocked stage remains pending until [`Self::release`] is called or its
/// future is dropped. Dropping a pending provider future increments that
/// stage's cancellation count, which makes lifecycle cleanup observable
/// without wall-clock sleeps.
#[derive(Clone)]
pub struct ReferenceProviderControl {
    inner: Arc<ReferenceProviderControlInner>,
}

impl Default for ReferenceProviderControl {
    fn default() -> Self {
        Self {
            inner: Arc::new(ReferenceProviderControlInner {
                blocked: AtomicU8::new(0),
                started: std::array::from_fn(|_| AtomicUsize::new(0)),
                active: std::array::from_fn(|_| AtomicUsize::new(0)),
                cancelled: std::array::from_fn(|_| AtomicUsize::new(0)),
                completed: std::array::from_fn(|_| AtomicUsize::new(0)),
                started_notify: std::array::from_fn(|_| Notify::new()),
                released_notify: std::array::from_fn(|_| Notify::new()),
            }),
        }
    }
}

impl ReferenceProviderControl {
    #[must_use]
    pub fn blocking(stage: ReferenceStage) -> Self {
        let control = Self::default();
        control.block(stage);
        control
    }

    pub fn block(&self, stage: ReferenceStage) {
        self.inner.blocked.fetch_or(stage.mask(), Ordering::AcqRel);
    }

    pub fn release(&self, stage: ReferenceStage) {
        self.inner
            .blocked
            .fetch_and(!stage.mask(), Ordering::AcqRel);
        self.inner.released_notify[stage.index()].notify_waiters();
    }

    pub async fn wait_until_started(&self, stage: ReferenceStage) {
        let index = stage.index();
        loop {
            let notified = self.inner.started_notify[index].notified();
            if self.inner.started[index].load(Ordering::Acquire) > 0 {
                return;
            }
            notified.await;
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> ReferenceProviderSnapshot {
        let load = |values: &[AtomicUsize; 3], stage: ReferenceStage| {
            values[stage.index()].load(Ordering::Acquire)
        };
        ReferenceProviderSnapshot {
            active_asr: load(&self.inner.active, ReferenceStage::Asr),
            active_dialogue: load(&self.inner.active, ReferenceStage::Dialogue),
            active_tts: load(&self.inner.active, ReferenceStage::Tts),
            cancelled_asr: load(&self.inner.cancelled, ReferenceStage::Asr),
            cancelled_dialogue: load(&self.inner.cancelled, ReferenceStage::Dialogue),
            cancelled_tts: load(&self.inner.cancelled, ReferenceStage::Tts),
            completed_asr: load(&self.inner.completed, ReferenceStage::Asr),
            completed_dialogue: load(&self.inner.completed, ReferenceStage::Dialogue),
            completed_tts: load(&self.inner.completed, ReferenceStage::Tts),
        }
    }

    async fn run(&self, stage: ReferenceStage) {
        let index = stage.index();
        self.inner.started[index].fetch_add(1, Ordering::AcqRel);
        self.inner.active[index].fetch_add(1, Ordering::AcqRel);
        self.inner.started_notify[index].notify_waiters();
        let mut attempt = ReferenceAttempt {
            control: Arc::clone(&self.inner),
            stage,
            completed: false,
        };
        loop {
            let released = self.inner.released_notify[index].notified();
            if self.inner.blocked.load(Ordering::Acquire) & stage.mask() == 0 {
                break;
            }
            tokio::select! {
                _ = released => {}
                _ = pending::<()>() => unreachable!("pending reference stage completed"),
            }
        }
        attempt.completed = true;
        self.inner.completed[index].fetch_add(1, Ordering::AcqRel);
    }
}

struct ReferenceAttempt {
    control: Arc<ReferenceProviderControlInner>,
    stage: ReferenceStage,
    completed: bool,
}

impl Drop for ReferenceAttempt {
    fn drop(&mut self) {
        let index = self.stage.index();
        self.control.active[index].fetch_sub(1, Ordering::AcqRel);
        if !self.completed {
            self.control.cancelled[index].fetch_add(1, Ordering::AcqRel);
        }
    }
}

/// ASR fixture that emits one final scripted transcript for every audio frame.
#[derive(Clone)]
pub struct ReferenceAsrProvider {
    control: ReferenceProviderControl,
    transcript: Arc<str>,
}

impl ReferenceAsrProvider {
    #[must_use]
    pub fn new(control: ReferenceProviderControl, transcript: impl Into<String>) -> Self {
        Self {
            control,
            transcript: Arc::from(transcript.into()),
        }
    }
}

struct ReferenceAsrStream {
    control: ReferenceProviderControl,
    transcript: Arc<str>,
    results: Mutex<VecDeque<AsrResult>>,
    closed: AtomicBool,
}

#[async_trait]
impl AsrProvider for ReferenceAsrProvider {
    async fn open_stream(
        &self,
        _conn: ConnectionId,
        _config: AsrConfig,
    ) -> Result<Box<dyn AsrStream>> {
        Ok(Box::new(ReferenceAsrStream {
            control: self.control.clone(),
            transcript: Arc::clone(&self.transcript),
            results: Mutex::new(VecDeque::new()),
            closed: AtomicBool::new(false),
        }))
    }
}

#[async_trait]
impl AsrStream for ReferenceAsrStream {
    async fn push(&self, frame: MediaFrame) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(RvoipError::InvalidState("reference ASR stream is closed"));
        }
        if frame.kind != StreamKind::Audio || frame.payload.is_empty() {
            return Err(RvoipError::InvalidState(
                "reference ASR requires a non-empty audio frame",
            ));
        }
        self.control.run(ReferenceStage::Asr).await;
        self.results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(AsrResult {
                stream_id: frame.stream_id,
                speaker: None,
                text: self.transcript.to_string(),
                confidence: 1.0,
                is_final: true,
            });
        Ok(())
    }

    async fn next(&self) -> Option<AsrResult> {
        self.results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
    }

    async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}

/// Dialogue fixture that emits one scripted `Say` action per final transcript.
#[derive(Clone)]
pub struct ReferenceDialogManager {
    control: ReferenceProviderControl,
    response: Arc<str>,
    voice: Option<Arc<str>>,
}

impl ReferenceDialogManager {
    #[must_use]
    pub fn new(control: ReferenceProviderControl, response: impl Into<String>) -> Self {
        Self {
            control,
            response: Arc::from(response.into()),
            voice: None,
        }
    }

    #[must_use]
    pub fn with_voice(mut self, voice: impl Into<String>) -> Self {
        self.voice = Some(Arc::from(voice.into()));
        self
    }
}

#[async_trait]
impl DialogManager for ReferenceDialogManager {
    async fn turn(&self, transcript: &AsrResult) -> Result<DialogAction> {
        if !transcript.is_final || transcript.text.is_empty() {
            return Err(RvoipError::InvalidState(
                "reference dialogue requires a final transcript",
            ));
        }
        self.control.run(ReferenceStage::Dialogue).await;
        Ok(DialogAction::Say {
            text: self.response.to_string(),
            voice: self.voice.as_ref().map(ToString::to_string),
        })
    }
}

/// TTS fixture that emits one 20 ms mono PCM16 frame for every request.
#[derive(Clone)]
pub struct ReferenceTtsProvider {
    control: ReferenceProviderControl,
    pcm: Bytes,
}

impl ReferenceTtsProvider {
    #[must_use]
    pub fn new(control: ReferenceProviderControl) -> Self {
        let mut pcm = Vec::with_capacity(REFERENCE_FRAME_SAMPLES * 2);
        for index in 0..REFERENCE_FRAME_SAMPLES {
            let sample = if index % 32 < 16 {
                1_024_i16
            } else {
                -1_024_i16
            };
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        Self {
            control,
            pcm: Bytes::from(pcm),
        }
    }
}

#[async_trait]
impl TtsProvider for ReferenceTtsProvider {
    async fn synthesize(&self, request: TtsRequest) -> Result<Box<dyn TtsPlayback>> {
        if request.text.is_empty()
            || request
                .sample_rate_hz
                .is_some_and(|rate| rate != REFERENCE_SAMPLE_RATE_HZ)
        {
            return Err(RvoipError::InvalidState(
                "reference TTS requires text and 16 kHz output",
            ));
        }
        Ok(Box::new(ReferenceTtsPlayback {
            control: self.control.clone(),
            pcm: self.pcm.clone(),
            emitted: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }))
    }
}

struct ReferenceTtsPlayback {
    control: ReferenceProviderControl,
    pcm: Bytes,
    emitted: AtomicBool,
    cancelled: AtomicBool,
}

#[async_trait]
impl TtsPlayback for ReferenceTtsPlayback {
    fn audio_format(&self) -> TtsAudioFormat {
        TtsAudioFormat::PcmS16Le {
            sample_rate_hz: REFERENCE_SAMPLE_RATE_HZ,
        }
    }
    async fn next_frame(&self) -> Option<MediaFrame> {
        if self.cancelled.load(Ordering::Acquire) || self.emitted.swap(true, Ordering::AcqRel) {
            return None;
        }
        self.control.run(ReferenceStage::Tts).await;
        Some(MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: self.pcm.clone(),
            timestamp_rtp: 0,
            captured_at: Utc::now(),
            payload_type: Some(PCM_S16LE_PAYLOAD_TYPE),
        })
    }

    async fn cancel(&self) -> Result<()> {
        self.cancelled.store(true, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use rvoip_core::adapter::{ConnectionAdapter, EndReason, OriginateRequest};
    use rvoip_core::capability::CapabilityDescriptor;
    use rvoip_core::connection::{Direction, Transport};
    use rvoip_core::ids::{ParticipantId, SessionId};
    use tokio_util::sync::CancellationToken;

    use crate::{
        InProcessAiAdapter, InProcessAiConfig, InProcessAiMedia, InProcessAiResourceSnapshot,
        InProcessAiSession, InProcessAiSessionFactory, InProcessAiSessionLifecycle,
        InProcessAiSessionRequest,
    };

    use super::*;

    struct ReferencePipelineFactory {
        asr: Arc<ReferenceAsrProvider>,
        dialogue: Arc<ReferenceDialogManager>,
        tts: Arc<ReferenceTtsProvider>,
    }

    struct ReferencePipelineSession {
        connection_id: ConnectionId,
        asr: Arc<ReferenceAsrProvider>,
        dialogue: Arc<ReferenceDialogManager>,
        tts: Arc<ReferenceTtsProvider>,
    }

    #[async_trait]
    impl InProcessAiSessionFactory for ReferencePipelineFactory {
        async fn create(
            &self,
            request: InProcessAiSessionRequest,
        ) -> Result<Box<dyn InProcessAiSession>> {
            Ok(Box::new(ReferencePipelineSession {
                connection_id: ConnectionId::from_string(request.ai_session_id.to_string()),
                asr: Arc::clone(&self.asr),
                dialogue: Arc::clone(&self.dialogue),
                tts: Arc::clone(&self.tts),
            }))
        }
    }

    #[async_trait]
    impl InProcessAiSession for ReferencePipelineSession {
        async fn run(
            self: Box<Self>,
            mut media: InProcessAiMedia,
            mut lifecycle: InProcessAiSessionLifecycle,
            cancellation: CancellationToken,
        ) -> Result<()> {
            let asr = self
                .asr
                .open_stream(self.connection_id, AsrConfig::default())
                .await?;
            loop {
                let frame = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Ok(()),
                    request = lifecycle.recv() => {
                        let Some(request) = request else { return Ok(()) };
                        request.acknowledge();
                        continue;
                    }
                    frame = media.recv() => {
                        let Some(frame) = frame else { return Ok(()) };
                        frame
                    }
                };
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Ok(()),
                    result = asr.push(frame.clone()) => result?,
                }
                let transcript = asr.next().await.ok_or(RvoipError::InvalidState(
                    "reference ASR omitted its final transcript",
                ))?;
                let action = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Ok(()),
                    result = self.dialogue.turn(&transcript) => result?,
                };
                let DialogAction::Say { text, voice } = action else {
                    return Err(RvoipError::InvalidState(
                        "reference dialogue did not produce speech",
                    ));
                };
                let playback = self
                    .tts
                    .synthesize(TtsRequest {
                        voice,
                        text,
                        sample_rate_hz: Some(REFERENCE_SAMPLE_RATE_HZ),
                        destination_codec: None,
                    })
                    .await?;
                let output = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Ok(()),
                    frame = playback.next_frame() => frame,
                };
                let Some(mut output) = output else {
                    return Err(RvoipError::InvalidState(
                        "reference TTS omitted its audio frame",
                    ));
                };
                output.stream_id = frame.stream_id;
                output.timestamp_rtp = frame.timestamp_rtp;
                media.send(output).await?;
            }
        }
    }

    fn adapter_for(
        control: &ReferenceProviderControl,
    ) -> (Arc<InProcessAiAdapter>, InProcessAiConfig) {
        let config = InProcessAiConfig::default();
        let adapter = InProcessAiAdapter::new(
            config.clone(),
            Arc::new(ReferencePipelineFactory {
                asr: Arc::new(ReferenceAsrProvider::new(
                    control.clone(),
                    "reference transcript",
                )),
                dialogue: Arc::new(ReferenceDialogManager::new(
                    control.clone(),
                    "reference response",
                )),
                tts: Arc::new(ReferenceTtsProvider::new(control.clone())),
            }),
        )
        .expect("valid reference adapter");
        (adapter, config)
    }

    async fn start_turn(
        adapter: &Arc<InProcessAiAdapter>,
        config: &InProcessAiConfig,
    ) -> (ConnectionId, tokio::sync::mpsc::Receiver<MediaFrame>) {
        let _events = adapter.subscribe_events();
        let connection_id = adapter
            .originate(
                OriginateRequest::new(
                    SessionId::new(),
                    ParticipantId::new(),
                    "reference",
                    Direction::Outbound,
                    CapabilityDescriptor::default(),
                )
                .with_transport(Transport::InProcessAi),
            )
            .await
            .expect("prepare reference AI")
            .connection
            .id;
        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("activate reference AI");
        let stream = adapter
            .streams(connection_id.clone())
            .await
            .expect("query reference stream")
            .into_iter()
            .next()
            .expect("one reference stream");
        let output = stream
            .reserve_frames_in()
            .expect("reserve reference output")
            .commit();
        stream
            .try_frames_out()
            .expect("reference input")
            .send(MediaFrame {
                stream_id: stream.id(),
                kind: StreamKind::Audio,
                payload: Bytes::from_static(b"caller-pcm"),
                timestamp_rtp: 320,
                captured_at: Utc::now(),
                payload_type: config.codec.payload_type,
            })
            .await
            .expect("send reference input");
        (connection_id, output)
    }

    async fn assert_end_during_stage(stage: ReferenceStage) {
        let control = ReferenceProviderControl::blocking(stage);
        let (adapter, config) = adapter_for(&control);
        let (connection_id, _output) = start_turn(&adapter, &config).await;
        tokio::time::timeout(Duration::from_secs(1), control.wait_until_started(stage))
            .await
            .expect("selected provider stage started");
        adapter
            .end(connection_id, EndReason::Cancelled)
            .await
            .expect("end during provider stage");
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = control.snapshot();
                let (active, cancelled) = match stage {
                    ReferenceStage::Asr => (snapshot.active_asr, snapshot.cancelled_asr),
                    ReferenceStage::Dialogue => {
                        (snapshot.active_dialogue, snapshot.cancelled_dialogue)
                    }
                    ReferenceStage::Tts => (snapshot.active_tts, snapshot.cancelled_tts),
                };
                if active == 0 && cancelled == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("provider future dropped after connection end");
        assert_eq!(
            adapter.resource_snapshot(),
            InProcessAiResourceSnapshot::default()
        );
    }

    #[tokio::test]
    async fn deterministic_reference_pipeline_emits_valid_pcm_audio() {
        let control = ReferenceProviderControl::default();
        let (adapter, config) = adapter_for(&control);
        let (connection_id, mut output) = start_turn(&adapter, &config).await;
        let frame = tokio::time::timeout(Duration::from_secs(1), output.recv())
            .await
            .expect("reference audio deadline")
            .expect("reference audio frame");
        assert_eq!(frame.kind, StreamKind::Audio);
        assert_eq!(frame.payload_type, Some(PCM_S16LE_PAYLOAD_TYPE));
        assert_eq!(frame.payload.len(), REFERENCE_FRAME_SAMPLES * 2);
        assert!(frame.payload.chunks_exact(2).any(|sample| sample != [0, 0]));
        assert_eq!(
            control.snapshot(),
            ReferenceProviderSnapshot {
                completed_asr: 1,
                completed_dialogue: 1,
                completed_tts: 1,
                ..ReferenceProviderSnapshot::default()
            }
        );
        adapter
            .end(connection_id, EndReason::Normal)
            .await
            .expect("end reference pipeline");
        assert_eq!(
            adapter.resource_snapshot(),
            InProcessAiResourceSnapshot::default()
        );
    }

    #[tokio::test]
    async fn end_during_asr_drops_work_and_releases_every_resource() {
        assert_end_during_stage(ReferenceStage::Asr).await;
    }

    #[tokio::test]
    async fn end_during_model_drops_work_and_releases_every_resource() {
        assert_end_during_stage(ReferenceStage::Dialogue).await;
    }

    #[tokio::test]
    async fn end_during_tts_drops_work_and_releases_every_resource() {
        assert_end_during_stage(ReferenceStage::Tts).await;
    }
}
