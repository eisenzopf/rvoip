use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, AdapterLifecycleCapabilities, AdapterLifecycleSink,
    AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::commands::MuteDirection;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::{
    MediaFrame, MediaReceiverReservation, MediaStream, MediaStreamHandle, QualitySnapshot,
    StreamKind,
};
use rvoip_core::DataMessage;
use tokio::sync::{mpsc, oneshot, watch, Mutex as TokioMutex, Notify, RwLock};
use tokio_util::sync::CancellationToken;

const PCM_S16LE_PAYLOAD_TYPE: u8 = 120;

/// Bounded runtime settings for [`InProcessAiAdapter`].
#[derive(Clone)]
pub struct InProcessAiConfig {
    /// Canonical codec exposed at the AI boundary. MediaGraph transcodes SIP
    /// PCMU/PCMA and WebRTC Opus to this format.
    pub codec: CodecInfo,
    /// Frames buffered in each direction for one AI session.
    pub media_queue_capacity: usize,
    /// Transport-neutral lifecycle events retained for core.
    pub event_queue_capacity: usize,
    /// Maximum time an adapter lifecycle call waits for all in-flight media
    /// operations to cross the session gate. Media operations never hold the
    /// gate across channel I/O, so this is an operational fail-safe rather
    /// than a normal source of latency.
    pub lifecycle_ack_timeout: Duration,
    /// Recently completed connection IDs retained to make repeated stop
    /// requests idempotent without retaining session resources indefinitely.
    pub terminal_history_capacity: usize,
}

impl Default for InProcessAiConfig {
    fn default() -> Self {
        Self {
            codec: CodecInfo {
                name: "pcm_s16le".into(),
                clock_rate_hz: 16_000,
                channels: 1,
                fmtp: None,
                payload_type: Some(PCM_S16LE_PAYLOAD_TYPE),
            },
            media_queue_capacity: 64,
            event_queue_capacity: 256,
            lifecycle_ack_timeout: Duration::from_secs(1),
            terminal_history_capacity: 1_024,
        }
    }
}

impl fmt::Debug for InProcessAiConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InProcessAiConfig")
            .field("codec", &self.codec)
            .field("media_queue_capacity", &self.media_queue_capacity)
            .field("event_queue_capacity", &self.event_queue_capacity)
            .field("lifecycle_ack_timeout", &self.lifecycle_ack_timeout)
            .field("terminal_history_capacity", &self.terminal_history_capacity)
            .finish()
    }
}

impl InProcessAiConfig {
    fn validate(&self) -> Result<()> {
        if self.media_queue_capacity == 0
            || self.event_queue_capacity == 0
            || self.lifecycle_ack_timeout.is_zero()
            || self.terminal_history_capacity == 0
        {
            return Err(RvoipError::InvalidState(
                "in-process AI capacities and lifecycle timeout must be non-zero",
            ));
        }
        if !self.codec.name.eq_ignore_ascii_case("pcm_s16le")
            || self.codec.clock_rate_hz != 16_000
            || self.codec.channels != 1
            || self.codec.payload_type != Some(PCM_S16LE_PAYLOAD_TYPE)
        {
            return Err(RvoipError::InvalidState(
                "in-process AI requires pcm_s16le/16000/mono payload type 120",
            ));
        }
        Ok(())
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            audio_codecs: vec![self.codec.clone()],
            max_streams_per_connection: 1,
            ..CapabilityDescriptor::default()
        }
    }
}

/// Cooperative lifecycle visible to a provider-owned AI session.
///
/// Providers should stop or suspend in-flight ASR/LLM/TTS work when this
/// changes to [`Self::Paused`] or [`Self::Stopped`]. The adapter also fences
/// both media directions independently, so correctness does not depend on a
/// provider polling this signal promptly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum InProcessAiLifecycleState {
    Running = 0,
    Paused = 1,
    Stopped = 2,
}

/// One adapter-requested provider lifecycle transition.
///
/// A provider must acknowledge [`Self::state`] only after work from the prior
/// state is quiescent. In particular, a `Paused` acknowledgement proves that
/// no ASR/model/tool result can still commit dialogue state and no TTS
/// playback can advance until a later `Running` request is acknowledged.
/// Dropping a request rejects the transition.
pub struct InProcessAiLifecycleRequest {
    state: InProcessAiLifecycleState,
    acknowledgement: Option<oneshot::Sender<()>>,
}

impl fmt::Debug for InProcessAiLifecycleRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InProcessAiLifecycleRequest")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl InProcessAiLifecycleRequest {
    #[must_use]
    pub fn state(&self) -> InProcessAiLifecycleState {
        self.state
    }

    /// Acknowledge that the requested lifecycle state is fully in effect.
    pub fn acknowledge(mut self) {
        let _ = self
            .acknowledgement
            .take()
            .expect("lifecycle acknowledgement is consumed once")
            .send(());
    }
}

/// Provider-owned receiver for bounded pause and resume requests.
///
/// Session implementations should include [`Self::recv`] in every select that
/// waits on ASR, model, tool, TTS, or playback work. When a pause wins, dropping
/// the competing future cancels that attempt; providers acknowledge only after
/// preserving enough state to retry or continue it after resume.
pub struct InProcessAiSessionLifecycle {
    requests: mpsc::Receiver<InProcessAiLifecycleRequest>,
}

impl fmt::Debug for InProcessAiSessionLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InProcessAiSessionLifecycle")
            .finish_non_exhaustive()
    }
}

impl InProcessAiSessionLifecycle {
    pub async fn recv(&mut self) -> Option<InProcessAiLifecycleRequest> {
        self.requests.recv().await
    }
}

#[derive(Clone)]
struct InProcessAiLifecycleControl {
    requests: mpsc::Sender<InProcessAiLifecycleRequest>,
}

impl InProcessAiLifecycleControl {
    async fn transition(&self, state: InProcessAiLifecycleState) -> Result<()> {
        let (acknowledgement, completed) = oneshot::channel();
        self.requests
            .send(InProcessAiLifecycleRequest {
                state,
                acknowledgement: Some(acknowledgement),
            })
            .await
            .map_err(|_| RvoipError::InvalidState("in-process AI lifecycle receiver is closed"))?;
        completed.await.map_err(|_| {
            RvoipError::InvalidState("in-process AI lifecycle request was not acknowledged")
        })
    }
}

impl InProcessAiLifecycleState {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Paused,
            2 => Self::Stopped,
            _ => Self::Running,
        }
    }
}

/// Per-session fence shared by the adapter and provider media boundary.
///
/// A lifecycle transition takes the write side of `transition`; a frame may
/// cross only while holding its read side and only after rechecking state.
/// No media operation holds the lock across channel I/O. Therefore a
/// successful pause acknowledgement proves that subsequent media cannot
/// cross until resume, while the provider-owned session task remains alive.
struct InProcessAiMediaGate {
    state: AtomicU8,
    transition: RwLock<()>,
    updates: watch::Sender<InProcessAiLifecycleState>,
}

impl InProcessAiMediaGate {
    fn new() -> Arc<Self> {
        let (updates, _) = watch::channel(InProcessAiLifecycleState::Running);
        Arc::new(Self {
            state: AtomicU8::new(InProcessAiLifecycleState::Running as u8),
            transition: RwLock::new(()),
            updates,
        })
    }

    fn state(&self) -> InProcessAiLifecycleState {
        InProcessAiLifecycleState::from_u8(self.state.load(Ordering::Acquire))
    }

    async fn pause(&self) -> Result<()> {
        let _transition = self.transition.write().await;
        match self.state() {
            InProcessAiLifecycleState::Stopped => Err(RvoipError::InvalidState(
                "in-process AI media gate is stopped",
            )),
            InProcessAiLifecycleState::Paused => Ok(()),
            InProcessAiLifecycleState::Running => {
                self.state
                    .store(InProcessAiLifecycleState::Paused as u8, Ordering::Release);
                self.updates.send_replace(InProcessAiLifecycleState::Paused);
                Ok(())
            }
        }
    }

    async fn resume(&self) -> Result<()> {
        let _transition = self.transition.write().await;
        match self.state() {
            InProcessAiLifecycleState::Stopped => Err(RvoipError::InvalidState(
                "in-process AI media gate is stopped",
            )),
            InProcessAiLifecycleState::Running => Ok(()),
            InProcessAiLifecycleState::Paused => {
                self.state
                    .store(InProcessAiLifecycleState::Running as u8, Ordering::Release);
                self.updates
                    .send_replace(InProcessAiLifecycleState::Running);
                Ok(())
            }
        }
    }

    async fn stop(&self) {
        let _transition = self.transition.write().await;
        if self.state() != InProcessAiLifecycleState::Stopped {
            self.state
                .store(InProcessAiLifecycleState::Stopped as u8, Ordering::Release);
            self.updates
                .send_replace(InProcessAiLifecycleState::Stopped);
        }
    }

    fn stop_now(&self) {
        self.state
            .store(InProcessAiLifecycleState::Stopped as u8, Ordering::Release);
        self.updates
            .send_replace(InProcessAiLifecycleState::Stopped);
    }

    fn subscribe(&self) -> watch::Receiver<InProcessAiLifecycleState> {
        self.updates.subscribe()
    }
}

/// Stable, transport-neutral identity supplied while an AI runtime is still
/// dormant. `target` selects a provider-owned assistant/profile and is always
/// redacted from diagnostics.
#[derive(Clone)]
pub struct InProcessAiSessionRequest {
    pub session_id: SessionId,
    pub participant_id: ParticipantId,
    pub target: String,
    pub codec: CodecInfo,
}

impl fmt::Debug for InProcessAiSessionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InProcessAiSessionRequest")
            .field("session_id", &self.session_id)
            .field("participant_id", &self.participant_id)
            .field("target", &"[redacted]")
            .field("target_bytes", &self.target.len())
            .field("codec", &self.codec)
            .finish()
    }
}

/// The AI side of one bidirectional MediaGraph endpoint.
///
/// `recv` yields caller audio after core's transport-to-PCM conversion.
/// `send` injects agent PCM for conversion toward the active caller transport.
pub struct InProcessAiMedia {
    caller_audio: Arc<TokioMutex<mpsc::Receiver<MediaFrame>>>,
    agent_audio: mpsc::Sender<MediaFrame>,
    gate: Arc<InProcessAiMediaGate>,
}

impl fmt::Debug for InProcessAiMedia {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InProcessAiMedia")
            .field("agent_audio_closed", &self.agent_audio.is_closed())
            .finish()
    }
}

impl InProcessAiMedia {
    /// Observe media-gate pause/resume/stop transitions. This watch is useful
    /// for diagnostics, but it does not acknowledge provider quiescence; session
    /// implementations must service the lifecycle receiver passed to `run`.
    pub fn subscribe_lifecycle(&self) -> watch::Receiver<InProcessAiLifecycleState> {
        self.gate.subscribe()
    }

    pub async fn recv(&mut self) -> Option<MediaFrame> {
        let mut updates = self.gate.subscribe();
        loop {
            match self.gate.state() {
                InProcessAiLifecycleState::Stopped => return None,
                InProcessAiLifecycleState::Paused => {
                    if updates.changed().await.is_err() {
                        return None;
                    }
                }
                InProcessAiLifecycleState::Running => {
                    let mut caller_audio = self.caller_audio.lock().await;
                    let frame = tokio::select! {
                        biased;
                        changed = updates.changed() => {
                            if changed.is_err() {
                                return None;
                            }
                            continue;
                        }
                        frame = caller_audio.recv() => frame?,
                    };
                    drop(caller_audio);
                    let _transition = self.gate.transition.read().await;
                    if self.gate.state() == InProcessAiLifecycleState::Running {
                        return Some(frame);
                    }
                    // A pause won the fence after channel receipt. Drop this
                    // frame rather than delivering it after acknowledgement.
                }
            }
        }
    }

    pub async fn send(&self, frame: MediaFrame) -> Result<()> {
        let mut updates = self.gate.subscribe();
        loop {
            match self.gate.state() {
                InProcessAiLifecycleState::Stopped => {
                    return Err(RvoipError::InvalidState(
                        "in-process AI media route is closed",
                    ));
                }
                InProcessAiLifecycleState::Paused => {
                    if updates.changed().await.is_err() {
                        return Err(RvoipError::InvalidState(
                            "in-process AI media route is closed",
                        ));
                    }
                    continue;
                }
                InProcessAiLifecycleState::Running => {}
            }
            let permit = tokio::select! {
                biased;
                changed = updates.changed() => {
                    if changed.is_err() {
                        return Err(RvoipError::InvalidState(
                            "in-process AI media route is closed",
                        ));
                    }
                    continue;
                }
                permit = self.agent_audio.reserve() => permit.map_err(|_| {
                    RvoipError::InvalidState("in-process AI media route is closed")
                })?,
            };
            let _transition = self.gate.transition.read().await;
            match self.gate.state() {
                InProcessAiLifecycleState::Running => {
                    permit.send(frame);
                    return Ok(());
                }
                InProcessAiLifecycleState::Paused => continue,
                InProcessAiLifecycleState::Stopped => {
                    return Err(RvoipError::InvalidState(
                        "in-process AI media route is closed",
                    ));
                }
            }
        }
    }
}

/// One provider-owned AI session. `run` begins only after core has durably
/// bound and activated the outbound Connection. Implementations must service
/// the lifecycle receiver while provider or playback futures are in flight and
/// acknowledge pause only after those futures can no longer commit state.
#[async_trait]
pub trait InProcessAiSession: Send + 'static {
    async fn run(
        self: Box<Self>,
        media: InProcessAiMedia,
        lifecycle: InProcessAiSessionLifecycle,
        cancellation: CancellationToken,
    ) -> Result<()>;
}

/// Creates dormant, per-call AI sessions. Implementations may validate and
/// reserve local resources in `create`, but must defer provider-visible work
/// and media processing until [`InProcessAiSession::run`].
#[async_trait]
pub trait InProcessAiSessionFactory: Send + Sync + 'static {
    async fn create(
        &self,
        request: InProcessAiSessionRequest,
    ) -> Result<Box<dyn InProcessAiSession>>;
}

/// Deterministic test/experiment provider that returns caller PCM unchanged.
pub struct EchoAiSessionFactory;

struct EchoAiSession;

#[async_trait]
impl InProcessAiSessionFactory for EchoAiSessionFactory {
    async fn create(
        &self,
        _request: InProcessAiSessionRequest,
    ) -> Result<Box<dyn InProcessAiSession>> {
        Ok(Box::new(EchoAiSession))
    }
}

#[async_trait]
impl InProcessAiSession for EchoAiSession {
    async fn run(
        self: Box<Self>,
        mut media: InProcessAiMedia,
        mut lifecycle: InProcessAiSessionLifecycle,
        cancellation: CancellationToken,
    ) -> Result<()> {
        let mut pending: Option<MediaFrame> = None;
        loop {
            if let Some(frame) = pending.take() {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Ok(()),
                    request = lifecycle.recv() => {
                        let Some(request) = request else {
                            return Err(RvoipError::InvalidState(
                                "in-process AI lifecycle receiver is closed",
                            ));
                        };
                        request.acknowledge();
                        pending = Some(frame);
                    }
                    result = media.send(frame.clone()) => result?,
                }
            } else {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Ok(()),
                    request = lifecycle.recv() => {
                        let Some(request) = request else {
                            return Err(RvoipError::InvalidState(
                                "in-process AI lifecycle receiver is closed",
                            ));
                        };
                        request.acknowledge();
                    }
                    frame = media.recv() => {
                        let Some(frame) = frame else { return Ok(()); };
                        pending = Some(frame);
                    },
                }
            }
        }
    }
}

enum InputGateCommand {
    Flush { complete: oneshot::Sender<()> },
}

#[derive(Clone)]
struct InputGateControl {
    commands: mpsc::Sender<InputGateCommand>,
}

impl InputGateControl {
    async fn flush(&self) -> Result<()> {
        let (complete, completion) = oneshot::channel();
        self.commands
            .send(InputGateCommand::Flush { complete })
            .await
            .map_err(|_| RvoipError::InvalidState("in-process AI input gate is closed"))?;
        completion
            .await
            .map_err(|_| RvoipError::InvalidState("in-process AI input gate is closed"))
    }
}

/// Activation-time pump that owns the raw MediaGraph input receiver.
///
/// This task continues consuming while a provider is busy in ASR/LLM/TTS, so
/// caller audio captured during hold cannot remain queued for replay after
/// resume. `Flush` is an ordered lifecycle barrier used before acknowledging
/// hold and before making resume visible.
struct PendingInputGate {
    caller_audio: mpsc::Receiver<MediaFrame>,
    provider_audio: mpsc::Sender<MediaFrame>,
    commands: mpsc::Receiver<InputGateCommand>,
}

impl PendingInputGate {
    async fn run(mut self, gate: Arc<InProcessAiMediaGate>, cancellation: CancellationToken) {
        let mut updates = gate.subscribe();
        loop {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return,
                command = self.commands.recv() => {
                    let Some(InputGateCommand::Flush { complete }) = command else {
                        return;
                    };
                    while self.caller_audio.try_recv().is_ok() {}
                    let _ = complete.send(());
                }
                changed = updates.changed() => {
                    if changed.is_err()
                        || gate.state() == InProcessAiLifecycleState::Stopped
                    {
                        return;
                    }
                }
                frame = self.caller_audio.recv() => {
                    let Some(frame) = frame else { return; };
                    if gate.state() != InProcessAiLifecycleState::Running {
                        continue;
                    }
                    let permit = tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return,
                        changed = updates.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            continue;
                        }
                        permit = self.provider_audio.reserve() => {
                            let Ok(permit) = permit else { return; };
                            permit
                        }
                    };
                    let _transition = gate.transition.read().await;
                    if gate.state() == InProcessAiLifecycleState::Running {
                        permit.send(frame);
                    }
                }
            }
        }
    }
}

struct InProcessAiMediaStream {
    id: StreamId,
    codec: CodecInfo,
    agent_audio: Arc<Mutex<Option<mpsc::Receiver<MediaFrame>>>>,
    caller_audio: Mutex<Option<mpsc::Sender<MediaFrame>>>,
    active: AtomicBool,
    closed: AtomicBool,
}

impl InProcessAiMediaStream {
    fn new(
        codec: CodecInfo,
        capacity: usize,
        gate: Arc<InProcessAiMediaGate>,
    ) -> (
        Arc<Self>,
        InProcessAiMedia,
        PendingInputGate,
        InputGateControl,
    ) {
        let (agent_audio, agent_audio_rx) = mpsc::channel(capacity);
        let (caller_audio, caller_audio_rx) = mpsc::channel(capacity);
        let (provider_audio, provider_audio_rx) = mpsc::channel(capacity);
        let provider_audio_rx = Arc::new(TokioMutex::new(provider_audio_rx));
        let (commands, command_receiver) = mpsc::channel(1);
        (
            Arc::new(Self {
                id: StreamId::new(),
                codec,
                agent_audio: Arc::new(Mutex::new(Some(agent_audio_rx))),
                caller_audio: Mutex::new(Some(caller_audio)),
                active: AtomicBool::new(false),
                closed: AtomicBool::new(false),
            }),
            InProcessAiMedia {
                caller_audio: Arc::clone(&provider_audio_rx),
                agent_audio,
                gate,
            },
            PendingInputGate {
                caller_audio: caller_audio_rx,
                provider_audio,
                commands: command_receiver,
            },
            InputGateControl { commands },
        )
    }

    fn activate(&self) {
        self.active.store(true, Ordering::Release);
    }

    fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
        self.closed.store(true, Ordering::Release);
        self.caller_audio
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
}

#[async_trait]
impl MediaStream for InProcessAiMediaStream {
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
        Direction::Outbound
    }

    fn source_ready(&self) -> bool {
        self.active.load(Ordering::Acquire) && !self.closed.load(Ordering::Acquire)
    }

    #[allow(deprecated)]
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> Result<mpsc::Receiver<MediaFrame>> {
        Ok(self.reserve_frames_in()?.commit())
    }

    fn reserve_frames_in(&self) -> Result<MediaReceiverReservation> {
        if !self.source_ready() {
            return Err(RvoipError::InvalidState(
                "in-process AI media stream is not active",
            ));
        }
        let receiver = self
            .agent_audio
            .lock()
            .map_err(|_| RvoipError::InvalidState("AI media receiver lock is poisoned"))?
            .take()
            .ok_or(RvoipError::InvalidState(
                "in-process AI media receiver has already been acquired",
            ))?;
        let slot = Arc::clone(&self.agent_audio);
        Ok(MediaReceiverReservation::new(receiver, move |receiver| {
            *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(receiver);
        }))
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.try_frames_out().unwrap_or_else(|_| mpsc::channel(1).0)
    }

    fn try_frames_out(&self) -> Result<mpsc::Sender<MediaFrame>> {
        if !self.source_ready() {
            return Err(RvoipError::InvalidState(
                "in-process AI media stream is not active",
            ));
        }
        self.caller_audio
            .lock()
            .map_err(|_| RvoipError::InvalidState("AI media sender lock is poisoned"))?
            .clone()
            .ok_or(RvoipError::InvalidState(
                "in-process AI media stream is closed",
            ))
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }

    async fn close(self: Arc<Self>) -> Result<()> {
        self.deactivate();
        Ok(())
    }
}

struct InProcessAiTransportHandle {
    connection_id: ConnectionId,
}

impl fmt::Debug for InProcessAiTransportHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InProcessAiTransportHandle")
            .field("connection_id", &self.connection_id)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivationState {
    Dormant,
    Active,
    Failed(&'static str),
}

struct Route {
    connection_id: ConnectionId,
    stream: Arc<InProcessAiMediaStream>,
    session: Mutex<Option<Box<dyn InProcessAiSession>>>,
    media: Mutex<Option<InProcessAiMedia>>,
    session_lifecycle: Mutex<Option<InProcessAiSessionLifecycle>>,
    lifecycle_control: InProcessAiLifecycleControl,
    lifecycle_operation: TokioMutex<()>,
    activation: TokioMutex<ActivationState>,
    pending_input_gate: Mutex<Option<PendingInputGate>>,
    input_gate_control: InputGateControl,
    provider_input: Arc<TokioMutex<mpsc::Receiver<MediaFrame>>>,
    cancellation: CancellationToken,
    task: Mutex<Option<tokio::task::AbortHandle>>,
    input_task: Mutex<Option<tokio::task::AbortHandle>>,
    requested_end: Mutex<Option<EndReason>>,
    live: AtomicBool,
    active: AtomicBool,
    terminal: AtomicBool,
    cleanup_complete: AtomicBool,
    cleanup_notify: Notify,
    gate: Arc<InProcessAiMediaGate>,
}

impl Route {
    fn new(
        connection_id: ConnectionId,
        codec: CodecInfo,
        capacity: usize,
        session: Box<dyn InProcessAiSession>,
    ) -> Arc<Self> {
        let gate = InProcessAiMediaGate::new();
        let (stream, media, pending_input_gate, input_gate_control) =
            InProcessAiMediaStream::new(codec, capacity, Arc::clone(&gate));
        let (lifecycle_requests, session_lifecycle) = mpsc::channel(2);
        let provider_input = Arc::clone(&media.caller_audio);
        Arc::new(Self {
            connection_id,
            stream,
            session: Mutex::new(Some(session)),
            media: Mutex::new(Some(media)),
            session_lifecycle: Mutex::new(Some(InProcessAiSessionLifecycle {
                requests: session_lifecycle,
            })),
            lifecycle_control: InProcessAiLifecycleControl {
                requests: lifecycle_requests,
            },
            lifecycle_operation: TokioMutex::new(()),
            activation: TokioMutex::new(ActivationState::Dormant),
            pending_input_gate: Mutex::new(Some(pending_input_gate)),
            input_gate_control,
            provider_input,
            cancellation: CancellationToken::new(),
            task: Mutex::new(None),
            input_task: Mutex::new(None),
            requested_end: Mutex::new(None),
            live: AtomicBool::new(true),
            active: AtomicBool::new(false),
            terminal: AtomicBool::new(false),
            cleanup_complete: AtomicBool::new(false),
            cleanup_notify: Notify::new(),
            gate,
        })
    }

    async fn flush_caller_input(&self) -> Result<()> {
        self.input_gate_control.flush().await?;
        let mut provider_input = self.provider_input.lock().await;
        while provider_input.try_recv().is_ok() {}
        Ok(())
    }

    async fn wait_for_cleanup(&self, timeout: Duration) -> Result<()> {
        if self.cleanup_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        let notified = self.cleanup_notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.cleanup_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        tokio::time::timeout(timeout, notified).await.map_err(|_| {
            RvoipError::Adapter("in-process AI cleanup acknowledgement timed out".into())
        })
    }
}

struct TerminalHistory {
    capacity: usize,
    order: VecDeque<ConnectionId>,
    ids: HashSet<ConnectionId>,
}

impl TerminalHistory {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::with_capacity(capacity),
            ids: HashSet::with_capacity(capacity),
        }
    }

    fn insert(&mut self, id: ConnectionId) {
        if !self.ids.insert(id.clone()) {
            return;
        }
        self.order.push_back(id);
        if self.order.len() > self.capacity {
            if let Some(expired) = self.order.pop_front() {
                self.ids.remove(&expired);
            }
        }
    }

    fn contains(&self, id: &ConnectionId) -> bool {
        self.ids.contains(id)
    }
}

/// Sanitized adapter-owned resource counts for readiness and leak checks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InProcessAiResourceSnapshot {
    pub live_routes: usize,
    pub active_sessions: usize,
    pub held_sessions: usize,
    pub running_session_tasks: usize,
    pub running_media_tasks: usize,
}

/// First-party outbound adapter for transport-neutral in-process AI workers.
pub struct InProcessAiAdapter {
    config: InProcessAiConfig,
    factory: Arc<dyn InProcessAiSessionFactory>,
    routes: Arc<dashmap::DashMap<ConnectionId, Arc<Route>>>,
    events: mpsc::Sender<AdapterEvent>,
    event_receiver: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    lifecycle: AdapterLifecycleSinkSlot,
    terminal_history: Arc<Mutex<TerminalHistory>>,
}

impl Drop for InProcessAiAdapter {
    fn drop(&mut self) {
        for route in self.routes.iter() {
            route.live.store(false, Ordering::Release);
            route.active.store(false, Ordering::Release);
            route.cancellation.cancel();
            route.gate.stop_now();
            route.stream.deactivate();
            if let Some(task) = route
                .task
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                task.abort();
            }
            if let Some(task) = route
                .input_task
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                task.abort();
            }
        }
        self.routes.clear();
    }
}

impl InProcessAiAdapter {
    pub fn new(
        config: InProcessAiConfig,
        factory: Arc<dyn InProcessAiSessionFactory>,
    ) -> Result<Arc<Self>> {
        config.validate()?;
        let (events, event_receiver) = mpsc::channel(config.event_queue_capacity);
        Ok(Arc::new(Self {
            terminal_history: Arc::new(Mutex::new(TerminalHistory::new(
                config.terminal_history_capacity,
            ))),
            config,
            factory,
            routes: Arc::new(dashmap::DashMap::new()),
            events,
            event_receiver: Mutex::new(Some(event_receiver)),
            lifecycle: AdapterLifecycleSinkSlot::default(),
        }))
    }

    pub fn echo(config: InProcessAiConfig) -> Result<Arc<Self>> {
        Self::new(config, Arc::new(EchoAiSessionFactory))
    }

    pub fn resource_snapshot(&self) -> InProcessAiResourceSnapshot {
        let mut snapshot = InProcessAiResourceSnapshot::default();
        for route in self.routes.iter() {
            if !route.live.load(Ordering::Acquire) {
                continue;
            }
            snapshot.live_routes += 1;
            if route.active.load(Ordering::Acquire) {
                snapshot.active_sessions += 1;
            }
            if route.gate.state() == InProcessAiLifecycleState::Paused {
                snapshot.held_sessions += 1;
            }
            if route
                .task
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .is_some_and(|task| !task.is_finished())
            {
                snapshot.running_session_tasks += 1;
            }
            if route
                .input_task
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .is_some_and(|task| !task.is_finished())
            {
                snapshot.running_media_tasks += 1;
            }
        }
        snapshot
    }

    async fn finish_route(&self, route: Arc<Route>, event: AdapterEvent) -> Result<()> {
        finish_route(
            &self.routes,
            &self.events,
            &self.lifecycle,
            &self.terminal_history,
            self.config.lifecycle_ack_timeout,
            route,
            event,
        )
        .await
    }
}

async fn finish_route(
    routes: &dashmap::DashMap<ConnectionId, Arc<Route>>,
    events: &mpsc::Sender<AdapterEvent>,
    lifecycle: &AdapterLifecycleSinkSlot,
    terminal_history: &Mutex<TerminalHistory>,
    lifecycle_ack_timeout: Duration,
    route: Arc<Route>,
    event: AdapterEvent,
) -> Result<()> {
    if route
        .terminal
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return route.wait_for_cleanup(lifecycle_ack_timeout).await;
    }
    finish_claimed_route(
        routes,
        events,
        lifecycle,
        terminal_history,
        lifecycle_ack_timeout,
        route,
        event,
    )
    .await;
    Ok(())
}

async fn finish_claimed_route(
    routes: &dashmap::DashMap<ConnectionId, Arc<Route>>,
    events: &mpsc::Sender<AdapterEvent>,
    lifecycle: &AdapterLifecycleSinkSlot,
    terminal_history: &Mutex<TerminalHistory>,
    lifecycle_ack_timeout: Duration,
    route: Arc<Route>,
    event: AdapterEvent,
) {
    route.live.store(false, Ordering::Release);
    route.active.store(false, Ordering::Release);
    route.cancellation.cancel();
    if tokio::time::timeout(lifecycle_ack_timeout, route.gate.stop())
        .await
        .is_err()
    {
        route.gate.stop_now();
    }
    route.stream.deactivate();
    if let Some(input_task) = route
        .input_task
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        input_task.abort();
    }
    routes.remove_if(&route.connection_id, |_, current| {
        Arc::ptr_eq(current, &route)
    });
    terminal_history
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(route.connection_id.clone());
    route.cleanup_complete.store(true, Ordering::Release);
    route.cleanup_notify.notify_waiters();
    let _ = lifecycle.queue_or_deliver_terminal(events, event).await;
}

#[async_trait]
impl ConnectionAdapter for InProcessAiAdapter {
    fn transport(&self) -> Transport {
        Transport::InProcessAi
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities {
            authoritative_liveness: true,
            // This adapter is outbound-only. It can never publish an inbound
            // connection, so the atomic inbound contract is satisfied
            // vacuously and it is safe to compose with a fail-closed gate.
            atomic_inbound_handoff: true,
            terminal_fallback: true,
            staged_outbound_activation: true,
        }
    }

    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> Result<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("in-process AI lifecycle sink already installed"))
    }

    fn is_connection_live(&self, connection_id: &ConnectionId) -> bool {
        self.routes
            .get(connection_id)
            .is_some_and(|route| route.live.load(Ordering::Acquire))
    }

    async fn originate(&self, request: OriginateRequest) -> Result<ConnectionHandle> {
        if request.direction != Direction::Outbound {
            return Err(RvoipError::AdmissionRejected(
                "in-process AI originate requires outbound direction",
            ));
        }
        if request.transport != Some(Transport::InProcessAi) {
            return Err(RvoipError::AdmissionRejected(
                "in-process AI originate requires InProcessAi transport",
            ));
        }
        let session_request = InProcessAiSessionRequest {
            session_id: request.session_id.clone(),
            participant_id: request.participant_id.clone(),
            target: request.target,
            codec: self.config.codec.clone(),
        };
        let session = self.factory.create(session_request).await?;
        let connection_id = ConnectionId::new();
        let route = Route::new(
            connection_id.clone(),
            self.config.codec.clone(),
            self.config.media_queue_capacity,
            session,
        );
        let connection = Connection {
            id: connection_id.clone(),
            session_id: request.session_id,
            participant_id: request.participant_id,
            transport: Transport::InProcessAi,
            direction: Direction::Outbound,
            state: ConnectionState::Connecting,
            capabilities: self.config.capabilities(),
            negotiated_codecs: NegotiatedCodecs {
                audio: Some(self.config.codec.clone()),
                video: None,
            },
            streams: vec![MediaStreamHandle::new(
                Arc::clone(&route.stream) as Arc<dyn MediaStream>
            )],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(InProcessAiTransportHandle {
                connection_id: connection_id.clone(),
            })),
            opened_at: chrono::Utc::now(),
            closed_at: None,
        };
        if self.routes.insert(connection_id, route).is_some() {
            return Err(RvoipError::AdmissionRejected(
                "in-process AI connection ID collided",
            ));
        }
        Ok(ConnectionHandle::new(connection))
    }

    async fn activate_outbound(&self, connection_id: ConnectionId) -> Result<()> {
        const ACTIVATION_STATE_ERROR: &str = "in-process AI activation state is incomplete";
        const EVENT_RECEIVER_ERROR: &str = "in-process AI event receiver is unavailable";

        let route = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))?;
        // Hold this lock through activation so exact concurrent retries await
        // one authoritative result instead of observing the early `active`
        // publication while setup can still fail.
        let mut activation = route.activation.lock().await;
        match *activation {
            ActivationState::Active => {
                return if route.live.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err(RvoipError::ConnectionNotFound(connection_id))
                };
            }
            ActivationState::Failed(detail) => {
                return Err(RvoipError::InvalidState(detail));
            }
            ActivationState::Dormant => {}
        }

        let pending_input_gate = route
            .pending_input_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let session = route
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let media = route
            .media
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let session_lifecycle = route
            .session_lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let (Some(pending_input_gate), Some(session), Some(media), Some(session_lifecycle)) =
            (pending_input_gate, session, media, session_lifecycle)
        else {
            *activation = ActivationState::Failed(ACTIVATION_STATE_ERROR);
            drop(activation);
            let _ = self
                .finish_route(
                    route,
                    AdapterEvent::Failed {
                        connection_id,
                        detail: ACTIVATION_STATE_ERROR.into(),
                    },
                )
                .await;
            return Err(RvoipError::InvalidState(ACTIVATION_STATE_ERROR));
        };

        route.stream.activate();
        let input_gate = Arc::clone(&route.gate);
        let input_cancellation = route.cancellation.clone();
        let input_task = tokio::spawn(async move {
            pending_input_gate.run(input_gate, input_cancellation).await;
        });
        *route
            .input_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(input_task.abort_handle());
        route.active.store(true, Ordering::Release);
        if self
            .events
            .send(AdapterEvent::Connected {
                connection_id: connection_id.clone(),
            })
            .await
            .is_err()
        {
            *activation = ActivationState::Failed(EVENT_RECEIVER_ERROR);
            drop(activation);
            let _ = self
                .finish_route(
                    route,
                    AdapterEvent::Failed {
                        connection_id,
                        detail: EVENT_RECEIVER_ERROR.into(),
                    },
                )
                .await;
            return Err(RvoipError::InvalidState(EVENT_RECEIVER_ERROR));
        }
        let routes = Arc::clone(&self.routes);
        let events = self.events.clone();
        let lifecycle = self.lifecycle.clone();
        let terminal_history = Arc::clone(&self.terminal_history);
        let lifecycle_ack_timeout = self.config.lifecycle_ack_timeout;
        let task_route = Arc::clone(&route);
        let cancellation = route.cancellation.clone();
        let task = tokio::spawn(async move {
            let result = session.run(media, session_lifecycle, cancellation).await;
            let event = if let Some(reason) = task_route
                .requested_end
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
            {
                AdapterEvent::Ended {
                    connection_id: task_route.connection_id.clone(),
                    reason,
                }
            } else if result.is_ok() {
                AdapterEvent::Ended {
                    connection_id: task_route.connection_id.clone(),
                    reason: EndReason::Normal,
                }
            } else {
                AdapterEvent::Failed {
                    connection_id: task_route.connection_id.clone(),
                    detail: "in-process AI session failed".into(),
                }
            };
            let _ = finish_route(
                &routes,
                &events,
                &lifecycle,
                &terminal_history,
                lifecycle_ack_timeout,
                task_route,
                event,
            )
            .await;
        });
        *route
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(task.abort_handle());
        *activation = ActivationState::Active;
        Ok(())
    }

    async fn accept(&self, connection_id: ConnectionId) -> Result<()> {
        if self.is_connection_live(&connection_id) {
            Ok(())
        } else {
            Err(RvoipError::ConnectionNotFound(connection_id))
        }
    }

    async fn reject(&self, connection_id: ConnectionId, _reason: RejectReason) -> Result<()> {
        self.end(connection_id, EndReason::Cancelled).await
    }

    async fn end(&self, connection_id: ConnectionId, reason: EndReason) -> Result<()> {
        let Some(route) = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
        else {
            return if self
                .terminal_history
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(&connection_id)
            {
                Ok(())
            } else {
                Err(RvoipError::ConnectionNotFound(connection_id))
            };
        };
        *route
            .requested_end
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason.clone());
        route.cancellation.cancel();
        let owns_cleanup = route
            .terminal
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if owns_cleanup {
            // Only the task that wins terminal ownership may abort the
            // provider task. If natural completion already owns cleanup,
            // aborting here could strand the route after its terminal claim.
            if let Some(task) = route
                .task
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                task.abort();
            }
            let routes = Arc::clone(&self.routes);
            let events = self.events.clone();
            let lifecycle = self.lifecycle.clone();
            let terminal_history = Arc::clone(&self.terminal_history);
            let lifecycle_ack_timeout = self.config.lifecycle_ack_timeout;
            let cleanup_route = Arc::clone(&route);
            tokio::spawn(async move {
                finish_claimed_route(
                    &routes,
                    &events,
                    &lifecycle,
                    &terminal_history,
                    lifecycle_ack_timeout,
                    cleanup_route,
                    AdapterEvent::Ended {
                        connection_id,
                        reason,
                    },
                )
                .await;
            });
        }
        route
            .wait_for_cleanup(self.config.lifecycle_ack_timeout)
            .await
    }

    async fn hold(&self, connection_id: ConnectionId) -> Result<()> {
        let route = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))?;
        if !route.live.load(Ordering::Acquire) || !route.active.load(Ordering::Acquire) {
            return Err(RvoipError::InvalidState(
                "in-process AI session is not active",
            ));
        }
        let operation = route.lifecycle_operation.lock().await;
        if route.gate.state() == InProcessAiLifecycleState::Paused {
            return Ok(());
        }
        let hold_result = tokio::time::timeout(self.config.lifecycle_ack_timeout, async {
            route.gate.pause().await?;
            route.flush_caller_input().await?;
            route
                .lifecycle_control
                .transition(InProcessAiLifecycleState::Paused)
                .await
        })
        .await
        .map_err(|_| RvoipError::Adapter("in-process AI hold acknowledgement timed out".into()))
        .and_then(|result| result);
        let Err(hold_error) = hold_result else {
            return Ok(());
        };

        // A failed hold may have crossed the media fence or reached the
        // provider. Restore both sides before returning an error. If the
        // provider cannot prove the rollback, end the route rather than leave
        // an apparently active but permanently silent AI service.
        if route.gate.state() == InProcessAiLifecycleState::Paused {
            let rollback_result = tokio::time::timeout(self.config.lifecycle_ack_timeout, async {
                route.flush_caller_input().await?;
                route.gate.resume().await?;
                route
                    .lifecycle_control
                    .transition(InProcessAiLifecycleState::Running)
                    .await
            })
            .await
            .map_err(|_| RvoipError::Adapter("in-process AI hold rollback timed out".into()))
            .and_then(|result| result);
            if rollback_result.is_err() {
                drop(operation);
                let _ = self
                    .end(
                        connection_id,
                        EndReason::Failed {
                            detail: "in-process AI hold rollback failed".into(),
                        },
                    )
                    .await;
                return Err(RvoipError::Adapter(
                    "in-process AI hold failed and rollback could not be acknowledged".into(),
                ));
            }
        }
        Err(hold_error)
    }

    async fn resume(&self, connection_id: ConnectionId) -> Result<()> {
        let route = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))?;
        if !route.live.load(Ordering::Acquire) || !route.active.load(Ordering::Acquire) {
            return Err(RvoipError::InvalidState(
                "in-process AI session is not active",
            ));
        }
        let operation = route.lifecycle_operation.lock().await;
        if route.gate.state() == InProcessAiLifecycleState::Running {
            return Ok(());
        }
        let resume_result = tokio::time::timeout(self.config.lifecycle_ack_timeout, async {
            route.flush_caller_input().await?;
            route.gate.resume().await?;
            route
                .lifecycle_control
                .transition(InProcessAiLifecycleState::Running)
                .await
        })
        .await
        .map_err(|_| RvoipError::Adapter("in-process AI resume acknowledgement timed out".into()))
        .and_then(|result| result);
        if let Err(resume_error) = resume_result {
            drop(operation);
            let _ = self
                .end(
                    connection_id,
                    EndReason::Failed {
                        detail: "in-process AI resume failed".into(),
                    },
                )
                .await;
            return Err(resume_error);
        }
        Ok(())
    }

    async fn transfer(&self, _connection_id: ConnectionId, _target: TransferTarget) -> Result<()> {
        Err(RvoipError::NotImplemented("in-process AI transfer"))
    }

    async fn streams(&self, connection_id: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>> {
        let route = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id))?;
        if !route.active.load(Ordering::Acquire) || !route.live.load(Ordering::Acquire) {
            return Ok(Vec::new());
        }
        Ok(vec![Arc::clone(&route.stream) as Arc<dyn MediaStream>])
    }

    async fn send_message(&self, _connection_id: ConnectionId, _message: Message) -> Result<()> {
        Err(RvoipError::NotImplemented("in-process AI messaging"))
    }

    async fn send_data_message(
        &self,
        _connection_id: ConnectionId,
        _message: DataMessage,
    ) -> Result<()> {
        Err(RvoipError::NotImplemented("in-process AI data messaging"))
    }

    async fn send_dtmf(
        &self,
        _connection_id: ConnectionId,
        _digits: &str,
        _duration_ms: u32,
    ) -> Result<()> {
        Err(RvoipError::NotImplemented("in-process AI DTMF"))
    }

    async fn renegotiate_media(
        &self,
        _connection_id: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs> {
        if capabilities
            .audio_codecs
            .iter()
            .any(|codec| codec == &self.config.codec)
        {
            Ok(NegotiatedCodecs {
                audio: Some(self.config.codec.clone()),
                video: None,
            })
        } else {
            Err(RvoipError::UnsupportedCodec(
                "in-process AI codec is fixed".into(),
            ))
        }
    }

    async fn mute(&self, _connection_id: ConnectionId, _direction: MuteDirection) -> Result<()> {
        Err(RvoipError::NotImplemented("in-process AI mute"))
    }

    async fn unmute(&self, _connection_id: ConnectionId, _direction: MuteDirection) -> Result<()> {
        Err(RvoipError::NotImplemented("in-process AI unmute"))
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.event_receiver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        self.config.capabilities()
    }

    async fn verify_request_signature(
        &self,
        _connection_id: ConnectionId,
        _signature: SignatureHeaders,
    ) -> Result<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use bytes::Bytes;
    use chrono::Utc;
    use rvoip_core::conversation::ConversationPolicy;
    use rvoip_core::session::SessionMedium;
    use rvoip_core::{Config, Orchestrator, TenantId};

    use super::*;

    struct RecordingFactory {
        created: Arc<AtomicBool>,
        running: Arc<AtomicBool>,
    }

    struct RecordingSession {
        running: Arc<AtomicBool>,
    }

    struct BusyInputFactory {
        started: Arc<Notify>,
        release: Arc<Notify>,
        lifecycle: mpsc::UnboundedSender<watch::Receiver<InProcessAiLifecycleState>>,
        observed: mpsc::UnboundedSender<Bytes>,
    }

    struct BusyInputSession {
        started: Arc<Notify>,
        release: Arc<Notify>,
        lifecycle: mpsc::UnboundedSender<watch::Receiver<InProcessAiLifecycleState>>,
        observed: mpsc::UnboundedSender<Bytes>,
    }

    #[async_trait]
    impl InProcessAiSessionFactory for BusyInputFactory {
        async fn create(
            &self,
            _request: InProcessAiSessionRequest,
        ) -> Result<Box<dyn InProcessAiSession>> {
            Ok(Box::new(BusyInputSession {
                started: Arc::clone(&self.started),
                release: Arc::clone(&self.release),
                lifecycle: self.lifecycle.clone(),
                observed: self.observed.clone(),
            }))
        }
    }

    #[async_trait]
    impl InProcessAiSession for BusyInputSession {
        async fn run(
            self: Box<Self>,
            mut media: InProcessAiMedia,
            mut session_lifecycle: InProcessAiSessionLifecycle,
            cancellation: CancellationToken,
        ) -> Result<()> {
            let _ = self.lifecycle.send(media.subscribe_lifecycle());
            self.started.notify_one();
            loop {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Ok(()),
                    request = session_lifecycle.recv() => {
                        let Some(request) = request else { return Ok(()) };
                        request.acknowledge();
                    }
                    _ = self.release.notified() => break,
                }
            }
            if let Some(frame) = media.recv().await {
                let _ = self.observed.send(frame.payload);
            }
            cancellation.cancelled().await;
            Ok(())
        }
    }

    struct CompletingFactory {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    struct CompletingSession {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    struct RejectPauseFactory;

    struct RejectPauseSession;

    #[async_trait]
    impl InProcessAiSessionFactory for RejectPauseFactory {
        async fn create(
            &self,
            _request: InProcessAiSessionRequest,
        ) -> Result<Box<dyn InProcessAiSession>> {
            Ok(Box::new(RejectPauseSession))
        }
    }

    #[async_trait]
    impl InProcessAiSession for RejectPauseSession {
        async fn run(
            self: Box<Self>,
            mut media: InProcessAiMedia,
            mut lifecycle: InProcessAiSessionLifecycle,
            cancellation: CancellationToken,
        ) -> Result<()> {
            let mut pending: Option<MediaFrame> = None;
            loop {
                if let Some(frame) = pending.take() {
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return Ok(()),
                        request = lifecycle.recv() => {
                            let Some(request) = request else { return Ok(()) };
                            if request.state() == InProcessAiLifecycleState::Paused {
                                drop(request);
                            } else {
                                request.acknowledge();
                            }
                            pending = Some(frame);
                        }
                        result = media.send(frame.clone()) => result?,
                    }
                } else {
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return Ok(()),
                        request = lifecycle.recv() => {
                            let Some(request) = request else { return Ok(()) };
                            if request.state() == InProcessAiLifecycleState::Paused {
                                drop(request);
                            } else {
                                request.acknowledge();
                            }
                        }
                        frame = media.recv() => {
                            let Some(frame) = frame else { return Ok(()) };
                            pending = Some(frame);
                        }
                    }
                }
            }
        }
    }

    struct QuiescingFactory {
        provider_started: Arc<Notify>,
        pause_received: Arc<Notify>,
        allow_quiescence: Arc<Notify>,
        provider_cancelled: Arc<AtomicBool>,
    }

    struct QuiescingSession {
        provider_started: Arc<Notify>,
        pause_received: Arc<Notify>,
        allow_quiescence: Arc<Notify>,
        provider_cancelled: Arc<AtomicBool>,
    }

    struct ProviderAttemptGuard(Arc<AtomicBool>);

    impl Drop for ProviderAttemptGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[async_trait]
    impl InProcessAiSessionFactory for QuiescingFactory {
        async fn create(
            &self,
            _request: InProcessAiSessionRequest,
        ) -> Result<Box<dyn InProcessAiSession>> {
            Ok(Box::new(QuiescingSession {
                provider_started: Arc::clone(&self.provider_started),
                pause_received: Arc::clone(&self.pause_received),
                allow_quiescence: Arc::clone(&self.allow_quiescence),
                provider_cancelled: Arc::clone(&self.provider_cancelled),
            }))
        }
    }

    #[async_trait]
    impl InProcessAiSession for QuiescingSession {
        async fn run(
            self: Box<Self>,
            mut media: InProcessAiMedia,
            mut lifecycle: InProcessAiSessionLifecycle,
            cancellation: CancellationToken,
        ) -> Result<()> {
            let Some(frame) = media.recv().await else {
                return Ok(());
            };
            let provider_started = Arc::clone(&self.provider_started);
            let provider_cancelled = Arc::clone(&self.provider_cancelled);
            let mut provider_attempt = Box::pin(async move {
                let _guard = ProviderAttemptGuard(provider_cancelled);
                provider_started.notify_one();
                std::future::pending::<()>().await;
            });
            let request = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(()),
                request = lifecycle.recv() => request,
                _ = &mut provider_attempt => unreachable!("provider attempt is deliberately pending"),
            };
            drop(provider_attempt);
            let Some(request) = request else {
                return Ok(());
            };
            if request.state() != InProcessAiLifecycleState::Paused {
                return Err(RvoipError::InvalidState(
                    "quiescing session expected a pause",
                ));
            }
            self.pause_received.notify_one();
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                _ = self.allow_quiescence.notified() => {}
            }
            request.acknowledge();
            loop {
                let request = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Ok(()),
                    request = lifecycle.recv() => request,
                };
                let Some(request) = request else {
                    return Ok(());
                };
                let running = request.state() == InProcessAiLifecycleState::Running;
                request.acknowledge();
                if running {
                    break;
                }
            }
            media.send(frame).await?;
            cancellation.cancelled().await;
            Ok(())
        }
    }

    #[async_trait]
    impl InProcessAiSessionFactory for CompletingFactory {
        async fn create(
            &self,
            _request: InProcessAiSessionRequest,
        ) -> Result<Box<dyn InProcessAiSession>> {
            Ok(Box::new(CompletingSession {
                started: Arc::clone(&self.started),
                release: Arc::clone(&self.release),
            }))
        }
    }

    #[async_trait]
    impl InProcessAiSession for CompletingSession {
        async fn run(
            self: Box<Self>,
            _media: InProcessAiMedia,
            _lifecycle: InProcessAiSessionLifecycle,
            cancellation: CancellationToken,
        ) -> Result<()> {
            self.started.notify_one();
            tokio::select! {
                _ = cancellation.cancelled() => {}
                _ = self.release.notified() => {}
            }
            Ok(())
        }
    }

    #[async_trait]
    impl InProcessAiSessionFactory for RecordingFactory {
        async fn create(
            &self,
            _request: InProcessAiSessionRequest,
        ) -> Result<Box<dyn InProcessAiSession>> {
            self.created.store(true, Ordering::Release);
            Ok(Box::new(RecordingSession {
                running: Arc::clone(&self.running),
            }))
        }
    }

    #[async_trait]
    impl InProcessAiSession for RecordingSession {
        async fn run(
            self: Box<Self>,
            _media: InProcessAiMedia,
            _lifecycle: InProcessAiSessionLifecycle,
            cancellation: CancellationToken,
        ) -> Result<()> {
            self.running.store(true, Ordering::Release);
            cancellation.cancelled().await;
            Ok(())
        }
    }

    fn originate_request(config: &InProcessAiConfig) -> OriginateRequest {
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            "assistant-with-sensitive-options",
            Direction::Outbound,
            config.capabilities(),
        )
        .with_transport(Transport::InProcessAi)
    }

    #[tokio::test]
    async fn outbound_only_adapter_composes_with_fail_closed_ingress() {
        let orchestrator = Orchestrator::new(Config::default());
        let _admissions = orchestrator
            .install_inbound_admission_gate(1, Duration::from_secs(1))
            .expect("install gate before adapters");
        let adapter =
            InProcessAiAdapter::echo(InProcessAiConfig::default()).expect("valid AI adapter");
        assert!(adapter
            .lifecycle_capabilities()
            .supports_fail_closed_inbound());
        orchestrator
            .register(adapter as Arc<dyn ConnectionAdapter>)
            .expect("outbound-only AI adapter should coexist with gated ingress");
        orchestrator.drain_connection_lifecycle_tasks().await;
    }

    #[tokio::test]
    async fn session_remains_dormant_until_outbound_activation() {
        let created = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(false));
        let config = InProcessAiConfig::default();
        let adapter = InProcessAiAdapter::new(
            config.clone(),
            Arc::new(RecordingFactory {
                created: Arc::clone(&created),
                running: Arc::clone(&running),
            }),
        )
        .expect("valid adapter");
        let mut events = adapter.subscribe_events();

        let handle = adapter
            .originate(originate_request(&config))
            .await
            .expect("prepare AI connection");
        let connection_id = handle.connection.id;

        assert!(created.load(Ordering::Acquire));
        assert!(!running.load(Ordering::Acquire));
        assert!(adapter
            .streams(connection_id.clone())
            .await
            .expect("query dormant streams")
            .is_empty());

        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("activate AI connection");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { connection_id: observed }) if observed == connection_id
        ));
        tokio::time::timeout(Duration::from_secs(1), async {
            while !running.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("AI session starts after activation");

        adapter
            .end(connection_id.clone(), EndReason::Cancelled)
            .await
            .expect("end AI connection");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Ended { connection_id: observed, reason: EndReason::Cancelled })
                if observed == connection_id
        ));
        assert!(!adapter.is_connection_live(&connection_id));
    }

    #[tokio::test]
    async fn concurrent_activation_retries_await_the_same_failure() {
        let config = InProcessAiConfig::default();
        let adapter = InProcessAiAdapter::echo(config.clone()).expect("valid AI adapter");
        drop(adapter.subscribe_events());
        let handle = adapter
            .originate(originate_request(&config))
            .await
            .expect("prepare AI connection");
        let connection_id = handle.connection.id;
        let route = adapter
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .expect("prepared route");
        let activation_guard = route.activation.lock().await;
        let first_adapter = Arc::clone(&adapter);
        let first_id = connection_id.clone();
        let first = tokio::spawn(async move { first_adapter.activate_outbound(first_id).await });
        let second_adapter = Arc::clone(&adapter);
        let second_id = connection_id.clone();
        let second = tokio::spawn(async move { second_adapter.activate_outbound(second_id).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while Arc::strong_count(&route) < 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both activation retries reached the shared state");
        drop(activation_guard);

        for result in [first.await.unwrap(), second.await.unwrap()] {
            assert!(matches!(
                result,
                Err(RvoipError::InvalidState(
                    "in-process AI event receiver is unavailable"
                ))
            ));
        }
        assert!(!adapter.is_connection_live(&connection_id));
    }

    #[tokio::test]
    async fn hold_waits_for_provider_quiescence_and_resume_reuses_pending_work() {
        let provider_started = Arc::new(Notify::new());
        let pause_received = Arc::new(Notify::new());
        let allow_quiescence = Arc::new(Notify::new());
        let provider_cancelled = Arc::new(AtomicBool::new(false));
        let config = InProcessAiConfig::default();
        let adapter = InProcessAiAdapter::new(
            config.clone(),
            Arc::new(QuiescingFactory {
                provider_started: Arc::clone(&provider_started),
                pause_received: Arc::clone(&pause_received),
                allow_quiescence: Arc::clone(&allow_quiescence),
                provider_cancelled: Arc::clone(&provider_cancelled),
            }),
        )
        .expect("valid adapter");
        let mut events = adapter.subscribe_events();
        let handle = adapter
            .originate(originate_request(&config))
            .await
            .expect("prepare AI connection");
        let connection_id = handle.connection.id;
        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("activate AI connection");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { .. })
        ));
        let stream = adapter
            .streams(connection_id.clone())
            .await
            .expect("query stream")
            .into_iter()
            .next()
            .expect("one AI stream");
        let mut output = stream
            .reserve_frames_in()
            .expect("reserve provider output")
            .commit();
        let input = stream.try_frames_out().expect("caller input");
        let frame = MediaFrame {
            stream_id: stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(b"pending-provider-turn"),
            timestamp_rtp: 320,
            captured_at: Utc::now(),
            payload_type: Some(PCM_S16LE_PAYLOAD_TYPE),
        };
        input
            .send(frame.clone())
            .await
            .expect("send provider input");
        provider_started.notified().await;

        let held_adapter = Arc::clone(&adapter);
        let held_id = connection_id.clone();
        let hold = tokio::spawn(async move { held_adapter.hold(held_id).await });
        pause_received.notified().await;
        assert!(provider_cancelled.load(Ordering::Acquire));
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !hold.is_finished(),
            "hold acknowledged before provider quiescence"
        );
        allow_quiescence.notify_one();
        hold.await.unwrap().expect("provider acknowledged hold");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), output.recv())
                .await
                .is_err(),
            "held provider output crossed the media fence"
        );

        adapter
            .resume(connection_id.clone())
            .await
            .expect("resume quiesced provider");
        let resumed = tokio::time::timeout(Duration::from_secs(1), output.recv())
            .await
            .expect("resumed output deadline")
            .expect("resumed provider output");
        assert_eq!(resumed.payload, frame.payload);
        adapter
            .end(connection_id, EndReason::Normal)
            .await
            .expect("end quiescence test");
    }

    #[tokio::test]
    async fn rejected_hold_rolls_media_and_provider_back_to_running() {
        let config = InProcessAiConfig::default();
        let adapter = InProcessAiAdapter::new(config.clone(), Arc::new(RejectPauseFactory))
            .expect("valid adapter");
        let mut events = adapter.subscribe_events();
        let handle = adapter
            .originate(originate_request(&config))
            .await
            .expect("prepare AI connection");
        let connection_id = handle.connection.id;
        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("activate AI connection");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { .. })
        ));
        let stream = adapter
            .streams(connection_id.clone())
            .await
            .expect("query stream")
            .into_iter()
            .next()
            .expect("one AI stream");
        let mut output = stream
            .reserve_frames_in()
            .expect("reserve provider output")
            .commit();
        let input = stream.try_frames_out().expect("caller input");

        assert!(adapter.hold(connection_id.clone()).await.is_err());
        assert_eq!(
            adapter
                .routes
                .get(&connection_id)
                .expect("live route after rollback")
                .gate
                .state(),
            InProcessAiLifecycleState::Running
        );
        let frame = MediaFrame {
            stream_id: stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(b"after-hold-rollback"),
            timestamp_rtp: 640,
            captured_at: Utc::now(),
            payload_type: Some(PCM_S16LE_PAYLOAD_TYPE),
        };
        input
            .send(frame.clone())
            .await
            .expect("send after rollback");
        let echoed = tokio::time::timeout(Duration::from_secs(1), output.recv())
            .await
            .expect("rollback output deadline")
            .expect("rollback provider output");
        assert_eq!(echoed.payload, frame.payload);
        adapter
            .end(connection_id, EndReason::Normal)
            .await
            .expect("end rollback test");
    }

    #[tokio::test]
    async fn echo_session_is_a_bidirectional_media_stream_with_rollback() {
        let config = InProcessAiConfig::default();
        let adapter = InProcessAiAdapter::echo(config.clone()).expect("valid echo adapter");
        let mut events = adapter.subscribe_events();
        let handle = adapter
            .originate(originate_request(&config))
            .await
            .expect("prepare echo connection");
        let connection_id = handle.connection.id;
        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("activate echo connection");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { .. })
        ));

        let stream = adapter
            .streams(connection_id.clone())
            .await
            .expect("query active stream")
            .into_iter()
            .next()
            .expect("one audio stream");
        let reservation = stream
            .reserve_frames_in()
            .expect("reserve AI output receiver");
        drop(reservation);
        let mut agent_output = stream
            .reserve_frames_in()
            .expect("rolled-back receiver is restored")
            .commit();
        let caller_input = stream.try_frames_out().expect("caller input sender");
        let frame = MediaFrame {
            stream_id: stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(b"pcm"),
            timestamp_rtp: 320,
            captured_at: Utc::now(),
            payload_type: Some(PCM_S16LE_PAYLOAD_TYPE),
        };
        caller_input
            .send(frame.clone())
            .await
            .expect("send caller PCM");
        let echoed = tokio::time::timeout(Duration::from_secs(1), agent_output.recv())
            .await
            .expect("echo deadline")
            .expect("echo frame");
        assert_eq!(echoed.payload, frame.payload);
        assert_eq!(echoed.timestamp_rtp, frame.timestamp_rtp);
        assert_eq!(echoed.payload_type, frame.payload_type);

        adapter
            .end(connection_id, EndReason::Normal)
            .await
            .expect("end echo connection");
    }

    #[tokio::test]
    async fn hold_fences_media_without_restarting_session_and_stop_is_idempotent() {
        let config = InProcessAiConfig {
            lifecycle_ack_timeout: Duration::from_millis(250),
            ..InProcessAiConfig::default()
        };
        let adapter = InProcessAiAdapter::echo(config.clone()).expect("valid echo adapter");
        let mut events = adapter.subscribe_events();
        let handle = adapter
            .originate(originate_request(&config))
            .await
            .expect("prepare echo connection");
        let connection_id = handle.connection.id;
        assert_eq!(
            adapter.resource_snapshot(),
            InProcessAiResourceSnapshot {
                live_routes: 1,
                ..InProcessAiResourceSnapshot::default()
            }
        );
        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("activate echo connection");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { .. })
        ));

        let stream = adapter
            .streams(connection_id.clone())
            .await
            .expect("query active stream")
            .into_iter()
            .next()
            .expect("one audio stream");
        let mut agent_output = stream
            .reserve_frames_in()
            .expect("reserve AI output receiver")
            .commit();
        let caller_input = stream.try_frames_out().expect("caller input sender");
        let frame = |payload: &'static [u8], timestamp_rtp| MediaFrame {
            stream_id: stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(payload),
            timestamp_rtp,
            captured_at: Utc::now(),
            payload_type: Some(PCM_S16LE_PAYLOAD_TYPE),
        };

        caller_input
            .send(frame(b"before-hold", 320))
            .await
            .expect("send pre-hold media");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), agent_output.recv())
                .await
                .expect("pre-hold echo deadline")
                .expect("pre-hold echo")
                .payload,
            Bytes::from_static(b"before-hold")
        );

        tokio::time::timeout(
            config.lifecycle_ack_timeout,
            adapter.hold(connection_id.clone()),
        )
        .await
        .expect("bounded hold acknowledgement")
        .expect("hold");
        adapter
            .hold(connection_id.clone())
            .await
            .expect("repeated hold is idempotent");
        assert_eq!(
            adapter.resource_snapshot(),
            InProcessAiResourceSnapshot {
                live_routes: 1,
                active_sessions: 1,
                held_sessions: 1,
                running_session_tasks: 1,
                running_media_tasks: 1,
            }
        );

        caller_input
            .send(frame(b"during-hold", 640))
            .await
            .expect("held input remains a live bounded route");
        assert!(
            tokio::time::timeout(Duration::from_millis(75), agent_output.recv())
                .await
                .is_err(),
            "no provider media may cross after the hold acknowledgement"
        );

        tokio::time::timeout(
            config.lifecycle_ack_timeout,
            adapter.resume(connection_id.clone()),
        )
        .await
        .expect("bounded resume acknowledgement")
        .expect("resume");
        adapter
            .resume(connection_id.clone())
            .await
            .expect("repeated resume is idempotent");
        caller_input
            .send(frame(b"after-resume", 960))
            .await
            .expect("send resumed media");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), agent_output.recv())
                .await
                .expect("resumed echo deadline")
                .expect("resumed echo")
                .payload,
            Bytes::from_static(b"after-resume")
        );

        adapter
            .end(connection_id.clone(), EndReason::Normal)
            .await
            .expect("stop session");
        adapter
            .end(connection_id.clone(), EndReason::Normal)
            .await
            .expect("repeated stop is idempotent");
        assert_eq!(
            adapter.resource_snapshot(),
            InProcessAiResourceSnapshot::default()
        );
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Ended { connection_id: observed, reason: EndReason::Normal })
                if observed == connection_id
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), events.recv())
                .await
                .is_err(),
            "idempotent stop must not emit a duplicate terminal event"
        );
    }

    #[tokio::test]
    async fn hold_discards_input_while_provider_is_busy_and_publishes_lifecycle() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
        let config = InProcessAiConfig::default();
        let adapter = InProcessAiAdapter::new(
            config.clone(),
            Arc::new(BusyInputFactory {
                started: Arc::clone(&started),
                release: Arc::clone(&release),
                lifecycle: lifecycle_tx,
                observed: observed_tx,
            }),
        )
        .expect("valid adapter");
        let mut events = adapter.subscribe_events();
        let handle = adapter
            .originate(originate_request(&config))
            .await
            .expect("prepare AI connection");
        let connection_id = handle.connection.id;
        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("activate AI connection");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { .. })
        ));
        started.notified().await;
        let mut lifecycle = lifecycle_rx
            .recv()
            .await
            .expect("provider lifecycle receiver");
        assert_eq!(
            *lifecycle.borrow_and_update(),
            InProcessAiLifecycleState::Running
        );

        let stream = adapter
            .streams(connection_id.clone())
            .await
            .expect("query active stream")
            .into_iter()
            .next()
            .expect("one audio stream");
        let caller_input = stream.try_frames_out().expect("caller input sender");
        let frame = |payload: &'static [u8], timestamp_rtp| MediaFrame {
            stream_id: stream.id(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(payload),
            timestamp_rtp,
            captured_at: Utc::now(),
            payload_type: Some(PCM_S16LE_PAYLOAD_TYPE),
        };

        adapter
            .hold(connection_id.clone())
            .await
            .expect("hold busy provider");
        lifecycle.changed().await.expect("paused lifecycle update");
        assert_eq!(
            *lifecycle.borrow_and_update(),
            InProcessAiLifecycleState::Paused
        );
        caller_input
            .send(frame(b"held-input", 320))
            .await
            .expect("send held caller media");
        adapter
            .resume(connection_id.clone())
            .await
            .expect("resume busy provider");
        lifecycle.changed().await.expect("resumed lifecycle update");
        assert_eq!(
            *lifecycle.borrow_and_update(),
            InProcessAiLifecycleState::Running
        );

        release.notify_one();
        caller_input
            .send(frame(b"resumed-input", 640))
            .await
            .expect("send resumed caller media");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), observed_rx.recv())
                .await
                .expect("provider input deadline")
                .expect("provider observed one frame"),
            Bytes::from_static(b"resumed-input"),
            "media captured while held must not replay after resume"
        );

        adapter
            .end(connection_id, EndReason::Normal)
            .await
            .expect("end AI connection");
        lifecycle.changed().await.expect("stopped lifecycle update");
        assert_eq!(
            *lifecycle.borrow_and_update(),
            InProcessAiLifecycleState::Stopped
        );
    }

    #[tokio::test]
    async fn end_waits_for_natural_cleanup_owner_instead_of_aborting_it() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let config = InProcessAiConfig::default();
        let adapter = InProcessAiAdapter::new(
            config.clone(),
            Arc::new(CompletingFactory {
                started: Arc::clone(&started),
                release: Arc::clone(&release),
            }),
        )
        .expect("valid adapter");
        let mut events = adapter.subscribe_events();
        let handle = adapter
            .originate(originate_request(&config))
            .await
            .expect("prepare AI connection");
        let connection_id = handle.connection.id;
        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("activate AI connection");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { .. })
        ));
        started.notified().await;

        let route = adapter
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .expect("live route");
        let transition = route.gate.transition.write().await;
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !route.terminal.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("natural completion claimed cleanup");

        let ending_adapter = Arc::clone(&adapter);
        let ending_connection = connection_id.clone();
        let ending = tokio::spawn(async move {
            ending_adapter
                .end(ending_connection, EndReason::Cancelled)
                .await
        });
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !ending.is_finished(),
            "end must wait while the natural terminal owner is cleaning up"
        );
        drop(transition);
        ending.await.expect("end task").expect("end connection");

        assert!(!adapter.is_connection_live(&connection_id));
        assert_eq!(
            adapter.resource_snapshot(),
            InProcessAiResourceSnapshot::default()
        );
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Ended { connection_id: observed, reason: EndReason::Normal })
                if observed == connection_id
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), events.recv())
                .await
                .is_err(),
            "cleanup race must still emit exactly one terminal event"
        );
    }

    #[tokio::test]
    async fn orchestrator_originates_and_retires_ai_as_a_normal_connection() {
        let config = InProcessAiConfig::default();
        let adapter = InProcessAiAdapter::echo(config.clone()).expect("valid echo adapter");
        let orchestrator = Orchestrator::new(Config::default());
        orchestrator
            .register(Arc::clone(&adapter) as Arc<dyn ConnectionAdapter>)
            .expect("register AI adapter");
        let conversation_id = orchestrator
            .open_conversation(
                TenantId::new(),
                ConversationPolicy::default(),
                HashMap::new(),
            )
            .await
            .expect("open conversation");
        let session_id = orchestrator
            .start_session(conversation_id, SessionMedium::Voice, vec![])
            .await
            .expect("start voice session");
        let participant_id = ParticipantId::new();
        let handle = orchestrator
            .originate_connection(
                OriginateRequest::new(
                    session_id.clone(),
                    participant_id,
                    "assistant-profile",
                    Direction::Outbound,
                    config.capabilities(),
                )
                .with_transport(Transport::InProcessAi),
            )
            .await
            .expect("originate AI through core");
        let connection_id = handle.connection.id;
        assert_eq!(orchestrator.session_of(&connection_id), Some(session_id));
        assert_eq!(
            orchestrator
                .connection_transport(&connection_id)
                .expect("registered connection"),
            Transport::InProcessAi
        );

        orchestrator
            .end_connection(connection_id.clone(), EndReason::Normal)
            .await
            .expect("end AI through core");
        tokio::time::timeout(Duration::from_secs(1), async {
            while orchestrator.session_of(&connection_id).is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("core consumes AI terminal lifecycle");
    }

    #[test]
    fn session_request_debug_redacts_target() {
        let request = InProcessAiSessionRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: "secret-assistant-and-provider-token".into(),
            codec: InProcessAiConfig::default().codec,
        };
        let debug = format!("{request:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("secret-assistant"));
        assert!(!debug.contains("provider-token"));
    }
}
