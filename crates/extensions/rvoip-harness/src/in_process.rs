use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
use tokio::sync::mpsc;
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
            .finish()
    }
}

impl InProcessAiConfig {
    fn validate(&self) -> Result<()> {
        if self.media_queue_capacity == 0 || self.event_queue_capacity == 0 {
            return Err(RvoipError::InvalidState(
                "in-process AI queue capacities must be non-zero",
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
    caller_audio: mpsc::Receiver<MediaFrame>,
    agent_audio: mpsc::Sender<MediaFrame>,
}

impl fmt::Debug for InProcessAiMedia {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InProcessAiMedia")
            .field("caller_audio_closed", &self.caller_audio.is_closed())
            .field("agent_audio_closed", &self.agent_audio.is_closed())
            .finish()
    }
}

impl InProcessAiMedia {
    pub async fn recv(&mut self) -> Option<MediaFrame> {
        self.caller_audio.recv().await
    }

    pub async fn send(&self, frame: MediaFrame) -> Result<()> {
        self.agent_audio
            .send(frame)
            .await
            .map_err(|_| RvoipError::InvalidState("in-process AI media route is closed"))
    }
}

/// One provider-owned AI session. `run` begins only after core has durably
/// bound and activated the outbound Connection.
#[async_trait]
pub trait InProcessAiSession: Send + 'static {
    async fn run(
        self: Box<Self>,
        media: InProcessAiMedia,
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
        cancellation: CancellationToken,
    ) -> Result<()> {
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                frame = media.recv() => {
                    let Some(frame) = frame else { return Ok(()); };
                    media.send(frame).await?;
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
    fn new(codec: CodecInfo, capacity: usize) -> (Arc<Self>, InProcessAiMedia) {
        let (agent_audio, agent_audio_rx) = mpsc::channel(capacity);
        let (caller_audio, caller_audio_rx) = mpsc::channel(capacity);
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
                caller_audio: caller_audio_rx,
                agent_audio,
            },
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

struct Route {
    connection_id: ConnectionId,
    stream: Arc<InProcessAiMediaStream>,
    session: Mutex<Option<Box<dyn InProcessAiSession>>>,
    media: Mutex<Option<InProcessAiMedia>>,
    cancellation: CancellationToken,
    task: Mutex<Option<tokio::task::AbortHandle>>,
    requested_end: Mutex<Option<EndReason>>,
    live: AtomicBool,
    active: AtomicBool,
    terminal: AtomicBool,
}

impl Route {
    fn new(
        connection_id: ConnectionId,
        codec: CodecInfo,
        capacity: usize,
        session: Box<dyn InProcessAiSession>,
    ) -> Arc<Self> {
        let (stream, media) = InProcessAiMediaStream::new(codec, capacity);
        Arc::new(Self {
            connection_id,
            stream,
            session: Mutex::new(Some(session)),
            media: Mutex::new(Some(media)),
            cancellation: CancellationToken::new(),
            task: Mutex::new(None),
            requested_end: Mutex::new(None),
            live: AtomicBool::new(true),
            active: AtomicBool::new(false),
            terminal: AtomicBool::new(false),
        })
    }
}

/// First-party outbound adapter for transport-neutral in-process AI workers.
pub struct InProcessAiAdapter {
    config: InProcessAiConfig,
    factory: Arc<dyn InProcessAiSessionFactory>,
    routes: Arc<dashmap::DashMap<ConnectionId, Arc<Route>>>,
    events: mpsc::Sender<AdapterEvent>,
    event_receiver: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    lifecycle: AdapterLifecycleSinkSlot,
}

impl Drop for InProcessAiAdapter {
    fn drop(&mut self) {
        for route in self.routes.iter() {
            route.live.store(false, Ordering::Release);
            route.active.store(false, Ordering::Release);
            route.cancellation.cancel();
            route.stream.deactivate();
            if let Some(task) = route
                .task
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

    async fn finish_route(&self, route: Arc<Route>, event: AdapterEvent) {
        finish_route(&self.routes, &self.events, &self.lifecycle, route, event).await;
    }
}

async fn finish_route(
    routes: &dashmap::DashMap<ConnectionId, Arc<Route>>,
    events: &mpsc::Sender<AdapterEvent>,
    lifecycle: &AdapterLifecycleSinkSlot,
    route: Arc<Route>,
    event: AdapterEvent,
) {
    if route.terminal.swap(true, Ordering::AcqRel) {
        return;
    }
    route.live.store(false, Ordering::Release);
    route.active.store(false, Ordering::Release);
    route.cancellation.cancel();
    route.stream.deactivate();
    routes.remove_if(&route.connection_id, |_, current| {
        Arc::ptr_eq(current, &route)
    });
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
            atomic_inbound_handoff: false,
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
        let route = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))?;
        if route
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return if route.live.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(RvoipError::ConnectionNotFound(connection_id))
            };
        }
        route.stream.activate();
        if self
            .events
            .send(AdapterEvent::Connected {
                connection_id: connection_id.clone(),
            })
            .await
            .is_err()
        {
            self.finish_route(
                route,
                AdapterEvent::Failed {
                    connection_id,
                    detail: "in-process AI event receiver is unavailable".into(),
                },
            )
            .await;
            return Err(RvoipError::InvalidState(
                "in-process AI event receiver is unavailable",
            ));
        }
        let session = route
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or(RvoipError::InvalidState(
                "in-process AI session was already activated",
            ))?;
        let media = route
            .media
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or(RvoipError::InvalidState(
                "in-process AI media was already activated",
            ))?;
        let routes = Arc::clone(&self.routes);
        let events = self.events.clone();
        let lifecycle = self.lifecycle.clone();
        let task_route = Arc::clone(&route);
        let cancellation = route.cancellation.clone();
        let task = tokio::spawn(async move {
            let result = session.run(media, cancellation).await;
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
            finish_route(&routes, &events, &lifecycle, task_route, event).await;
        });
        *route
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(task.abort_handle());
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
        let route = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))?;
        *route
            .requested_end
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason.clone());
        route.cancellation.cancel();
        if let Some(task) = route
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            task.abort();
        }
        self.finish_route(
            route,
            AdapterEvent::Ended {
                connection_id,
                reason,
            },
        )
        .await;
        Ok(())
    }

    async fn hold(&self, _connection_id: ConnectionId) -> Result<()> {
        Err(RvoipError::NotImplemented("in-process AI hold"))
    }

    async fn resume(&self, _connection_id: ConnectionId) -> Result<()> {
        Err(RvoipError::NotImplemented("in-process AI resume"))
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
