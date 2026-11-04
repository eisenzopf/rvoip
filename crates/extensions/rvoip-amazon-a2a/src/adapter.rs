use crate::{
    media::{self, AudioStream},
    A2aError, AmazonA2aConfig, FinishType, InitSession, InterruptionType, Result, TurnTrace,
};
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use parking_lot::Mutex;
use rvoip_core::{
    adapter::{
        AdapterEvent, AdapterKind, AdapterLifecycleCapabilities, AdapterLifecycleSink,
        AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
        RejectReason, SignatureHeaders, TransferTarget,
    },
    connection::Transport,
    error::{Result as CoreResult, RvoipError},
    identity::IdentityAssurance,
    ids::{ConnectionId, ParticipantId, SessionId},
    message::Message,
    stream::MediaStream,
    CapabilityDescriptor, NegotiatedCodecs,
};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio_util::sync::CancellationToken;

/// Host-selected rvoip binding. Never derive session/tenant IDs from an A2A
/// payload. Allocate these through the host's authenticated admission policy.
#[derive(Clone)]
pub struct CallBinding {
    pub session_id: SessionId,
    pub participant_id: ParticipantId,
    /// Trusted instance configured by the host, checked against INIT_SESSION.
    pub instance_arn: String,
}
/// Content-bearing inputs use an owned, bounded single-consumer channel.
/// Hosts must consume it; losing events fails the call instead of losing tools.
pub enum InputEvent {
    Ready,
    Text(String),
    Dtmf {
        digits: String,
        termination_reason: String,
    },
    Interrupted(InterruptionType),
    Diagnostic(serde_json::Value),
}
impl fmt::Debug for InputEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ready => "Ready",
            Self::Text(_) => "Text([redacted])",
            Self::Dtmf { .. } => "Dtmf([redacted])",
            Self::Interrupted(_) => "Interrupted",
            Self::Diagnostic(_) => "Diagnostic([redacted])",
        })
    }
}
pub(crate) enum Command {
    Audio(Bytes),
    Text(String),
    Complete(TurnTrace),
    Interrupt(InterruptionType),
    Finish(FinishType, String),
}
pub(crate) struct CommandRequest {
    pub command: Command,
    pub reply: oneshot::Sender<Result<()>>,
}
pub(crate) struct Route {
    pub stream: Arc<AudioStream>,
    pub cancel: CancellationToken,
    pub accepted: AtomicBool,
    pub acceptance: tokio::sync::Notify,
    pub finished: CancellationToken,
    pub commands: mpsc::Sender<CommandRequest>,
    pub context: String,
    pub audio_enabled: bool,
}
/// One authenticated Connect collaboration. Dropping this handle does not
/// abandon the socket task; `close()` explicitly cancels it and awaits cleanup.
pub struct AmazonA2aCall {
    pub(crate) connection_id: ConnectionId,
    pub(crate) route: Arc<Route>,
    pub(crate) init: InitSession,
    pub(crate) inputs: mpsc::Receiver<InputEvent>,
    pub(crate) command_timeout: std::time::Duration,
}
impl AmazonA2aCall {
    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }
    pub fn context_id(&self) -> &str {
        &self.route.context
    }
    pub fn initialization(&self) -> &InitSession {
        &self.init
    }
    pub async fn next_input(&mut self) -> Option<InputEvent> {
        self.inputs.recv().await
    }
    pub async fn closed(&self) {
        self.route.finished.cancelled().await
    }
    pub async fn close(&self) {
        self.route.cancel.cancel();
        self.closed().await
    }
    /// Ordered application-managed output. PCM16LE at the negotiated rate.
    pub async fn send_audio(&self, pcm: Bytes) -> Result<()> {
        self.command(Command::Audio(pcm)).await
    }
    pub async fn send_text(&self, text: impl Into<String>) -> Result<()> {
        self.command(Command::Text(text.into())).await
    }
    /// The producer must stop and drain its upstream media before this call.
    /// Media already queued at this adapter is drained before the terminal
    /// chunk, trace, and task status; silence is never guessed to be a boundary.
    pub async fn complete_turn(&self, trace: TurnTrace) -> Result<()> {
        self.command(Command::Complete(trace)).await
    }
    pub async fn interrupt(&self, kind: InterruptionType) -> Result<()> {
        self.command(Command::Interrupt(kind)).await
    }
    pub async fn finish(&self, kind: FinishType, reason: impl Into<String>) -> Result<()> {
        self.command(Command::Finish(kind, reason.into())).await
    }
    async fn command(&self, command: Command) -> Result<()> {
        if self.route.cancel.is_cancelled() {
            return Err(A2aError::Closed);
        }
        let (reply, rx) = oneshot::channel();
        let operation = async {
            self.route
                .commands
                .send(CommandRequest { command, reply })
                .await
                .map_err(|_| A2aError::Closed)?;
            rx.await.map_err(|_| A2aError::Closed)?
        };
        match tokio::time::timeout(self.command_timeout, operation).await {
            Ok(result) => result,
            Err(_) => {
                self.route.cancel.cancel();
                Err(A2aError::Timeout)
            }
        }
    }
}
impl fmt::Debug for AmazonA2aCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AmazonA2aCall([redacted])")
    }
}

/// Inbound-only Connect A2A transport. Register once with an Orchestrator.
pub struct AmazonA2aAdapter {
    pub(crate) config: Arc<AmazonA2aConfig>,
    pub(crate) admission: Mutex<()>,
    pub(crate) routes: DashMap<ConnectionId, Arc<Route>>,
    pub(crate) contexts: DashMap<String, ConnectionId>,
    pub(crate) slots: Arc<Semaphore>,
    pub(crate) events: mpsc::Sender<AdapterEvent>,
    receiver: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    pub(crate) lifecycle: AdapterLifecycleSinkSlot,
}
impl AmazonA2aAdapter {
    pub fn new(config: AmazonA2aConfig) -> Result<Arc<Self>> {
        config.validate()?;
        let (events, rx) = mpsc::channel(config.queue_capacity);
        Ok(Arc::new(Self {
            slots: Arc::new(Semaphore::new(config.max_connections)),
            config: Arc::new(config),
            admission: Mutex::new(()),
            routes: DashMap::new(),
            contexts: DashMap::new(),
            events,
            receiver: Mutex::new(Some(rx)),
            lifecycle: AdapterLifecycleSinkSlot::default(),
        }))
    }
    fn route(&self, id: &ConnectionId) -> CoreResult<Arc<Route>> {
        self.routes
            .get(id)
            .map(|r| r.clone())
            .ok_or_else(|| RvoipError::ConnectionNotFound(id.clone()))
    }
    pub async fn shutdown(&self) {
        let routes: Vec<_> = {
            let _admission = self.admission.lock();
            self.slots.close();
            self.routes.iter().map(|r| r.value().clone()).collect()
        };
        for route in &routes {
            route.cancel.cancel();
        }
        for route in routes {
            route.finished.cancelled().await;
        }
    }
}
#[async_trait]
impl ConnectionAdapter for AmazonA2aAdapter {
    fn transport(&self) -> Transport {
        Transport::AmazonA2a
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities {
            authoritative_liveness: true,
            terminal_fallback: true,
            ..Default::default()
        }
    }
    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> CoreResult<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("A2A lifecycle sink already installed"))
    }
    fn is_connection_live(&self, id: &ConnectionId) -> bool {
        self.routes
            .get(id)
            .is_some_and(|r| !r.cancel.is_cancelled())
    }
    async fn originate(&self, _: OriginateRequest) -> CoreResult<ConnectionHandle> {
        Err(RvoipError::NotImplemented(
            "Connect initiates A2A connections",
        ))
    }
    async fn accept(&self, id: ConnectionId) -> CoreResult<()> {
        let route = self.route(&id)?;
        if route.cancel.is_cancelled() {
            return Err(RvoipError::InvalidState("A2A connection closed"));
        }
        route.accepted.store(true, Ordering::Release);
        route.acceptance.notify_one();
        Ok(())
    }
    async fn reject(&self, id: ConnectionId, _: RejectReason) -> CoreResult<()> {
        self.end(id, EndReason::Cancelled).await
    }
    async fn end(&self, id: ConnectionId, _: EndReason) -> CoreResult<()> {
        if let Some(route) = self.routes.get(&id).map(|r| r.clone()) {
            route.cancel.cancel();
        }
        Ok(())
    }
    async fn hold(&self, _: ConnectionId) -> CoreResult<()> {
        Err(RvoipError::NotImplemented("A2A hold"))
    }
    async fn resume(&self, _: ConnectionId) -> CoreResult<()> {
        Err(RvoipError::NotImplemented("A2A resume"))
    }
    async fn transfer(&self, _: ConnectionId, _: TransferTarget) -> CoreResult<()> {
        Err(RvoipError::NotImplemented("use A2A finish ESCALATE"))
    }
    async fn streams(&self, id: ConnectionId) -> CoreResult<Vec<Arc<dyn MediaStream>>> {
        let route = self.route(&id)?;
        if route.audio_enabled {
            Ok(vec![route.stream.clone()])
        } else {
            Ok(vec![])
        }
    }
    async fn send_message(&self, _: ConnectionId, _: Message) -> CoreResult<()> {
        Err(RvoipError::NotImplemented(
            "use AmazonA2aCall::send_text and complete_turn",
        ))
    }
    async fn send_dtmf(&self, _: ConnectionId, _: &str, _: u32) -> CoreResult<()> {
        Err(RvoipError::NotImplemented("A2A DTMF is inbound only"))
    }
    async fn renegotiate_media(
        &self,
        _: ConnectionId,
        _: CapabilityDescriptor,
    ) -> CoreResult<NegotiatedCodecs> {
        Err(RvoipError::NotImplemented(
            "A2A format is fixed for the session",
        ))
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.receiver
            .lock()
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            audio_codecs: vec![media::codec(self.config.sample_rate_hz)],
            max_streams_per_connection: 1,
            ..Default::default()
        }
    }
    async fn verify_request_signature(
        &self,
        _: ConnectionId,
        _: SignatureHeaders,
    ) -> CoreResult<IdentityAssurance> {
        Err(RvoipError::NotImplemented(
            "A2A authenticates the WebSocket upgrade",
        ))
    }
}
