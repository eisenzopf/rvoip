//! [`ConnectionAdapter`] integration shared by the WebSocket and gRPC
//! transports.

use crate::{
    auth::{WebexPrincipal, WebexTransportKind},
    media::{self, AudioReceivers, AudioStream},
    Result, WebexByovaConfig, WebexError,
};
use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex;
use rvoip_core::{
    adapter::{
        legacy_normalized_event_receiver, AdapterEvent, AdapterKind, AdapterLifecycleCapabilities,
        AdapterLifecycleSink, AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle,
        EndReason, OrchestratorAdapterEvent, OriginateRequest, RejectReason, SignatureHeaders,
        TransferTarget,
    },
    connection::{Connection, ConnectionState, Direction, Transport, TransportHandle},
    error::{Result as CoreResult, RvoipError},
    identity::{AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance},
    ids::{ConnectionId, ParticipantId, SessionId},
    message::Message,
    stream::{MediaStream, MediaStreamHandle},
    CapabilityDescriptor, NegotiatedCodecs,
};
use serde_json::Value;
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

/// Caller-side inputs that are not audio. Caller audio flows through the
/// connection's media stream; these arrive in Webex order on one bounded,
/// single-consumer queue. A host that stops consuming fails the call rather
/// than losing input.
pub enum WebexInput {
    /// Core accepted the connection; replies may begin. Delivered once.
    Started,
    /// Collected digits in received order (`0`-`9`, `A`-`D`, `*`, `#`).
    Dtmf(String),
    /// The caller started entering digits.
    DtmfStarted,
    /// Webex reported that the caller said nothing.
    NoInput,
    /// An application event from the flow. `parameters` is untrusted.
    Custom { name: String, parameters: Value },
    /// Webex ended the session.
    Ended,
}

impl fmt::Debug for WebexInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Started => "Started",
            Self::Dtmf(_) => "Dtmf([redacted])",
            Self::DtmfStarted => "DtmfStarted",
            Self::NoInput => "NoInput",
            Self::Custom { .. } => "Custom([redacted])",
            Self::Ended => "Ended",
        })
    }
}

/// Where in the agent's audio stream a reply boundary falls.
///
/// Positions count bytes of agent audio (PCMU) this connection has received
/// from its media path since it was admitted, including bytes discarded after
/// an interruption. A source that knows where its own events fall in its
/// audio, such as `rvoip-vapi` in ordered mode (`VapiOrderedEvent::audio_offset`),
/// can name the exact boundary as long as the bridge does not transcode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyBoundary {
    /// Everything received so far.
    Received,
    /// An exact position.
    At(u64),
}

/// What happens at a [`ReplyMark`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyAction {
    /// The reply ends here: one FINAL after every earlier byte.
    Complete,
    /// The caller barged in here: START_OF_INPUT, then the interrupted
    /// reply's single FINAL. Later audio starts the next reply.
    Interrupt,
}

/// A reply boundary at an exact agent-audio position, delivered in band.
///
/// See [`WebexByovaCall::follow_reply_marks`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplyMark {
    pub position: u64,
    pub action: ReplyAction,
}

/// A source of [`ReplyMark`]s in non-decreasing position order.
pub type ReplyMarks = std::pin::Pin<Box<dyn futures::Stream<Item = ReplyMark> + Send>>;

/// Output events the host may report between replies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum WebexOutputEvent {
    /// The agent committed the caller's input turn.
    EndOfInput,
    /// The agent could not match the caller's input.
    NoMatch,
    /// The agent decided the caller gave no input.
    NoInput,
}

impl WebexOutputEvent {
    pub(crate) const fn wire_name(self) -> &'static str {
        match self {
            Self::EndOfInput => "END_OF_INPUT",
            Self::NoMatch => "NO_MATCH",
            Self::NoInput => "NO_INPUT",
        }
    }
}

pub(crate) enum Command {
    CompleteReply(ReplyBoundary),
    Interrupt(ReplyBoundary),
    Event(WebexOutputEvent, Option<String>),
    Terminal { handoff: bool },
    CompleteWithMetadata { name: String, metadata: Value },
    FollowMarks(ReplyMarks),
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
    /// Lookup key of the external conversation, erased with the route.
    pub key: (String, String),
    pub received: AtomicU64,
    #[cfg(feature = "grpc")]
    pub rpcs: Option<mpsc::Sender<crate::grpc_server::RpcAttachment>>,
}

impl Route {
    pub(crate) async fn wait_accepted(&self, deadline: Duration) -> Result<()> {
        let accepted = async {
            loop {
                let notified = self.acceptance.notified();
                if self.accepted.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
        };
        tokio::select! {
            _ = self.cancel.cancelled() => Err(WebexError::Rejected),
            result = tokio::time::timeout(deadline, accepted) => {
                result.map_err(|_| WebexError::Timeout)
            }
        }
    }
}

/// One admitted Webex conversation, owned by the host.
///
/// Dropping this handle does not end the conversation; [`Self::close`] ends it
/// and waits for cleanup. Agent audio reaches Webex through the media graph
/// (bridge this connection to the agent's connection); this handle marks reply
/// boundaries and reports events in the same order the session writes them.
pub struct WebexByovaCall {
    pub(crate) connection_id: ConnectionId,
    pub(crate) route: Arc<Route>,
    pub(crate) inputs: mpsc::Receiver<WebexInput>,
    pub(crate) organization_id: String,
    pub(crate) conversation_id: String,
    pub(crate) start_parameters: Value,
    pub(crate) transport: WebexTransportKind,
    pub(crate) command_timeout: Duration,
}

impl WebexByovaCall {
    /// Local rvoip connection minted for this conversation.
    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }
    /// Verified organization from the Webex token.
    pub fn organization_id(&self) -> &str {
        &self.organization_id
    }
    /// Webex's conversation ID, for correlation only. Never use it as an
    /// rvoip identifier.
    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    pub fn transport(&self) -> WebexTransportKind {
        self.transport
    }
    /// Optional, untrusted application data from SESSION_START (an object or
    /// null, at most 16 KiB).
    pub fn start_parameters(&self) -> &Value {
        &self.start_parameters
    }
    pub async fn next_input(&mut self) -> Option<WebexInput> {
        self.inputs.recv().await
    }
    /// Agent audio bytes received from the media path so far.
    pub fn agent_audio_position(&self) -> u64 {
        self.route.received.load(Ordering::Acquire)
    }
    pub async fn closed(&self) {
        self.route.finished.cancelled().await
    }
    /// End the conversation locally (Webex receives SESSION_END when its
    /// transport is still writable) and wait for cleanup.
    pub async fn close(&self) {
        self.route.cancel.cancel();
        self.closed().await
    }

    /// Finish the current reply with exactly one FINAL once agent audio has
    /// reached `boundary`. Audio that arrived before the boundary is written
    /// first; FINAL never overtakes it. A reply with no audio still gets one
    /// FINAL. Fails the call when the audio does not arrive within
    /// [`WebexByovaConfig::reply_boundary_timeout`].
    pub async fn complete_reply(&self, boundary: ReplyBoundary) -> Result<()> {
        self.command(Command::CompleteReply(boundary)).await
    }

    /// Caller barge-in: send START_OF_INPUT, finish the interrupted reply with
    /// its single FINAL, and discard agent audio up to `boundary` so no stale
    /// audio reaches Webex. Audio after the boundary starts the next reply.
    pub async fn interrupt(&self, boundary: ReplyBoundary) -> Result<()> {
        self.command(Command::Interrupt(boundary)).await
    }

    /// Report an input event, with an optional caller transcript (at most
    /// 8 KiB) where the tenant's policy allows publishing it.
    pub async fn report(&self, event: WebexOutputEvent, transcript: Option<String>) -> Result<()> {
        self.command(Command::Event(event, transcript)).await
    }

    /// Request a Contact Center handoff (TRANSFER_TO_AGENT) and end the
    /// virtual-agent leg. Webex owns the transfer; this records the request.
    pub async fn transfer_to_agent(&self) -> Result<()> {
        self.command(Command::Terminal { handoff: true }).await
    }

    /// End the virtual-agent leg with SESSION_END.
    pub async fn end_session(&self) -> Result<()> {
        self.command(Command::Terminal { handoff: false }).await
    }

    /// End the leg with one FINAL carrying a CUSTOM_EVENT result and
    /// SESSION_END, both with `metadata` (a JSON object, at most 16 KiB).
    pub async fn complete_with_metadata(
        &self,
        name: impl Into<String>,
        metadata: Value,
    ) -> Result<()> {
        self.command(Command::CompleteWithMetadata {
            name: name.into(),
            metadata,
        })
        .await
    }

    /// Follow reply boundaries from the agent's own ordered event stream.
    ///
    /// The session task polls `marks` itself and applies each mark exactly at
    /// its position in the agent audio: bytes before it are written first,
    /// then FINAL (or START_OF_INPUT and FINAL), then bytes after it, even when
    /// the audio is already queued. Before writing any agent audio frame the
    /// task first takes every mark that is already available, so a source
    /// that publishes each event before the audio that follows it (such as
    /// `rvoip-vapi` in ordered mode, mapped from `VapiOrderedEvent`) gets
    /// exact boundaries without a race against a host task. A mark whose
    /// audio does not arrive within [`WebexByovaConfig::reply_boundary_timeout`]
    /// fails the call. A completion that would leave a reply shorter than
    /// Webex's 320-byte minimum keeps that reply open rather than padding or
    /// dropping it. One mark source per call.
    pub async fn follow_reply_marks<S>(&self, marks: S) -> Result<()>
    where
        S: futures::Stream<Item = ReplyMark> + Send + 'static,
    {
        self.command(Command::FollowMarks(Box::pin(marks))).await
    }

    async fn command(&self, command: Command) -> Result<()> {
        if self.route.cancel.is_cancelled() {
            return Err(WebexError::Closed);
        }
        let (reply, rx) = oneshot::channel();
        let operation = async {
            self.route
                .commands
                .send(CommandRequest { command, reply })
                .await
                .map_err(|_| WebexError::Closed)?;
            rx.await.map_err(|_| WebexError::Closed)?
        };
        match tokio::time::timeout(self.command_timeout, operation).await {
            Ok(result) => result,
            Err(_) => {
                self.route.cancel.cancel();
                Err(WebexError::Timeout)
            }
        }
    }
}

impl fmt::Debug for WebexByovaCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebexByovaCall")
            .field("connection_id", &self.connection_id)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

/// Inbound-only Webex Contact Center BYOVA adapter (`Transport::WebexByova`).
///
/// Register it once with an `Orchestrator`. Every connection is authenticated
/// before it is published, and is published atomically with its principal.
pub struct WebexByovaAdapter {
    pub(crate) config: Arc<WebexByovaConfig>,
    pub(crate) admission: Mutex<()>,
    pub(crate) routes: DashMap<ConnectionId, Arc<Route>>,
    pub(crate) conversations: DashMap<(String, String), ConnectionId>,
    pub(crate) slots: Arc<Semaphore>,
    pub(crate) events: mpsc::Sender<OrchestratorAdapterEvent>,
    receiver: Mutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    pub(crate) lifecycle: AdapterLifecycleSinkSlot,
}

pub(crate) struct Admitted {
    pub route: Arc<Route>,
    pub call: WebexByovaCall,
    pub audio: AudioReceivers,
    pub commands: mpsc::Receiver<CommandRequest>,
    pub inputs: mpsc::Sender<WebexInput>,
    #[cfg(feature = "grpc")]
    pub rpcs: Option<mpsc::Receiver<crate::grpc_server::RpcAttachment>>,
}

impl WebexByovaAdapter {
    pub fn new(config: WebexByovaConfig) -> Result<Arc<Self>> {
        config.validate()?;
        let (events, receiver) = mpsc::channel(config.queue_capacity);
        Ok(Arc::new(Self {
            slots: Arc::new(Semaphore::new(config.max_connections)),
            config: Arc::new(config),
            admission: Mutex::new(()),
            routes: DashMap::new(),
            conversations: DashMap::new(),
            events,
            receiver: Mutex::new(Some(receiver)),
            lifecycle: AdapterLifecycleSinkSlot::default(),
        }))
    }

    pub fn config(&self) -> &WebexByovaConfig {
        &self.config
    }

    /// Live conversations.
    pub fn connection_count(&self) -> usize {
        self.routes.len()
    }

    pub(crate) fn acquire_slot(&self) -> Result<OwnedSemaphorePermit> {
        self.slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| WebexError::Capacity)
    }

    /// Mint local identifiers for an authenticated conversation and publish
    /// it to core with its principal as one atomic event.
    pub(crate) fn admit(
        &self,
        principal: &WebexPrincipal,
        conversation: &str,
        transport: WebexTransportKind,
        start_parameters: Value,
    ) -> Result<Admitted> {
        let _admission = self.admission.lock();
        if self.slots.is_closed() {
            return Err(WebexError::Closed);
        }
        let key = (principal.organization_id.clone(), conversation.to_owned());
        let connection_id = ConnectionId::new();
        match self.conversations.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(_) => {
                return Err(WebexError::DuplicateConversation)
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(connection_id.clone());
            }
        }
        let cancel = CancellationToken::new();
        let (stream, audio) = AudioStream::new(
            self.config.queue_capacity,
            self.config.media_stall_timeout,
            cancel.clone(),
        );
        let (commands, command_rx) = mpsc::channel(self.config.queue_capacity);
        let (inputs, input_rx) = mpsc::channel(self.config.queue_capacity);
        #[cfg(feature = "grpc")]
        let (rpcs, rpc_rx) = if transport == WebexTransportKind::Grpc {
            let (tx, rx) = mpsc::channel(4);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let route = Arc::new(Route {
            stream: stream.clone(),
            cancel,
            accepted: AtomicBool::new(false),
            acceptance: tokio::sync::Notify::new(),
            finished: CancellationToken::new(),
            commands,
            key: key.clone(),
            received: AtomicU64::new(0),
            #[cfg(feature = "grpc")]
            rpcs,
        });
        let participant_id = ParticipantId::new();
        let connection = Connection {
            id: connection_id.clone(),
            // A placeholder: the host places the connection in its session
            // when it accepts it (`InboundAction::Accept`).
            session_id: SessionId::new(),
            participant_id: participant_id.clone(),
            transport: Transport::WebexByova,
            direction: Direction::Inbound,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor {
                audio_codecs: vec![media::codec()],
                max_streams_per_connection: 1,
                ..Default::default()
            },
            negotiated_codecs: NegotiatedCodecs {
                audio: Some(media::codec()),
                video: None,
            },
            streams: vec![MediaStreamHandle::new(stream)],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: chrono::Utc::now(),
            closed_at: None,
        };
        let core_principal = AuthenticatedPrincipal {
            subject: principal.subject.clone(),
            tenant: Some(principal.organization_id.clone()),
            scopes: Vec::new(),
            issuer: Some(principal.issuer.clone()),
            expires_at: Some(principal.expires_at),
            method: AuthenticationMethod::Jwt,
            // The token identifies the Webex organization's data source,
            // never the human caller.
            assurance: IdentityAssurance::Anonymous,
        };
        self.routes.insert(connection_id.clone(), route.clone());
        if self
            .events
            .try_send(OrchestratorAdapterEvent::AuthenticatedInboundConnection {
                connection,
                participant_id: participant_id.to_string(),
                principal: core_principal,
            })
            .is_err()
        {
            self.routes.remove(&connection_id);
            self.conversations.remove(&key);
            return Err(WebexError::Capacity);
        }
        let call = WebexByovaCall {
            connection_id,
            route: route.clone(),
            inputs: input_rx,
            organization_id: principal.organization_id.clone(),
            conversation_id: conversation.to_owned(),
            start_parameters,
            transport,
            command_timeout: self.config.write_timeout * 4 + self.config.reply_boundary_timeout,
        };
        Ok(Admitted {
            route,
            call,
            audio,
            commands: command_rx,
            inputs,
            #[cfg(feature = "grpc")]
            rpcs: rpc_rx,
        })
    }

    pub(crate) fn publish_connected(&self, id: &ConnectionId) -> Result<()> {
        self.events
            .try_send(OrchestratorAdapterEvent::Public(AdapterEvent::Connected {
                connection_id: id.clone(),
            }))
            .map_err(|_| WebexError::Capacity)
    }

    /// Retire a route after its session task ends and report the outcome.
    pub(crate) async fn retire(&self, id: &ConnectionId, route: &Route, result: Result<()>) {
        route.stream.stop();
        let removed = self.routes.remove(id).is_some();
        self.conversations
            .remove_if(&route.key, |_, current| current == id);
        if removed {
            let event = match result {
                Ok(()) => AdapterEvent::Ended {
                    connection_id: id.clone(),
                    reason: EndReason::Normal,
                },
                Err(error) => AdapterEvent::Failed {
                    connection_id: id.clone(),
                    detail: error.to_string(),
                },
            };
            self.lifecycle
                .queue_or_deliver_orchestrator_terminal(&self.events, event)
                .await;
        }
    }

    fn route(&self, id: &ConnectionId) -> CoreResult<Arc<Route>> {
        self.routes
            .get(id)
            .map(|route| route.clone())
            .ok_or_else(|| RvoipError::ConnectionNotFound(id.clone()))
    }

    /// Stop admitting, end every live conversation, and wait for cleanup.
    pub async fn shutdown(&self) {
        let routes: Vec<_> = {
            let _admission = self.admission.lock();
            self.slots.close();
            self.routes
                .iter()
                .map(|route| route.value().clone())
                .collect()
        };
        for route in &routes {
            route.cancel.cancel();
        }
        for route in routes {
            route.finished.cancelled().await;
        }
    }
}

/// Owns a connection slot and erases the route if its session task stops
/// without reaching normal retirement.
pub(crate) struct RouteGuard {
    pub adapter: Arc<WebexByovaAdapter>,
    pub id: ConnectionId,
    pub route: Arc<Route>,
    pub permit: Option<OwnedSemaphorePermit>,
}

impl Drop for RouteGuard {
    fn drop(&mut self) {
        self.route.stream.stop();
        self.route.cancel.cancel();
        if self.adapter.routes.remove(&self.id).is_some() {
            let id = self.id.clone();
            self.adapter
                .conversations
                .remove_if(&self.route.key, |_, current| *current == id);
            let _ = self
                .adapter
                .events
                .try_send(OrchestratorAdapterEvent::Public(AdapterEvent::Failed {
                    connection_id: self.id.clone(),
                    detail: "Webex BYOVA worker stopped".into(),
                }));
        }
        self.permit.take();
        self.route.finished.cancel();
    }
}

#[async_trait]
impl ConnectionAdapter for WebexByovaAdapter {
    fn transport(&self) -> Transport {
        Transport::WebexByova
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities::FAIL_CLOSED_INBOUND
    }
    fn supports_inbound_admission_confirmation(&self) -> bool {
        true
    }
    /// A rejected admission ends the pending conversation. An acceptance is
    /// only a policy result: the session task still waits for `accept`, and a
    /// late notification for an ended route finds nothing to resurrect.
    fn notify_inbound_admission_outcome(
        &self,
        connection_id: &ConnectionId,
        _lifecycle_generation: u64,
        accepted: bool,
    ) {
        if accepted {
            return;
        }
        if let Some(route) = self.routes.get(connection_id) {
            route.cancel.cancel();
        }
    }
    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> CoreResult<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("Webex lifecycle sink already installed"))
    }
    fn is_connection_live(&self, id: &ConnectionId) -> bool {
        self.routes
            .get(id)
            .is_some_and(|route| !route.cancel.is_cancelled())
    }
    async fn originate(&self, _: OriginateRequest) -> CoreResult<ConnectionHandle> {
        Err(RvoipError::NotImplemented(
            "Webex Contact Center initiates BYOVA connections",
        ))
    }
    async fn accept(&self, id: ConnectionId) -> CoreResult<()> {
        let route = self.route(&id)?;
        if route.cancel.is_cancelled() {
            return Err(RvoipError::InvalidState("Webex connection closed"));
        }
        route.accepted.store(true, Ordering::Release);
        route.acceptance.notify_one();
        Ok(())
    }
    async fn reject(&self, id: ConnectionId, _: RejectReason) -> CoreResult<()> {
        self.end(id, EndReason::Cancelled).await
    }
    async fn end(&self, id: ConnectionId, _: EndReason) -> CoreResult<()> {
        if let Some(route) = self.routes.get(&id).map(|route| route.clone()) {
            route.cancel.cancel();
        }
        Ok(())
    }
    async fn hold(&self, _: ConnectionId) -> CoreResult<()> {
        Err(RvoipError::NotImplemented("Webex BYOVA hold"))
    }
    async fn resume(&self, _: ConnectionId) -> CoreResult<()> {
        Err(RvoipError::NotImplemented("Webex BYOVA resume"))
    }
    async fn transfer(&self, _: ConnectionId, _: TransferTarget) -> CoreResult<()> {
        Err(RvoipError::NotImplemented(
            "use WebexByovaCall::transfer_to_agent",
        ))
    }
    async fn streams(&self, id: ConnectionId) -> CoreResult<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![self.route(&id)?.stream.clone()])
    }
    async fn send_message(&self, _: ConnectionId, _: Message) -> CoreResult<()> {
        Err(RvoipError::NotImplemented(
            "Webex BYOVA carries no messages",
        ))
    }
    async fn send_dtmf(&self, _: ConnectionId, _: &str, _: u32) -> CoreResult<()> {
        Err(RvoipError::NotImplemented(
            "Webex BYOVA DTMF is inbound only",
        ))
    }
    async fn renegotiate_media(
        &self,
        _: ConnectionId,
        _: CapabilityDescriptor,
    ) -> CoreResult<NegotiatedCodecs> {
        Err(RvoipError::NotImplemented(
            "Webex BYOVA media is fixed PCMU/8 kHz",
        ))
    }
    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        self.receiver
            .lock()
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }
    /// Direct subscription for hosts that drive the adapter without an
    /// Orchestrator. The atomic handoff is expanded into `InboundConnection`
    /// then `PrincipalAuthenticated`. Only one of the two subscriptions can be
    /// taken.
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        match self.receiver.lock().take() {
            Some(receiver) => {
                legacy_normalized_event_receiver(receiver, self.config.queue_capacity)
            }
            None => mpsc::channel(1).1,
        }
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            audio_codecs: vec![media::codec()],
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
            "Webex BYOVA authenticates each connection's JWS",
        ))
    }
}

/// Agent audio accounting shared by both transports.
#[derive(Default)]
pub(crate) struct AgentAudio {
    pub received: u64,
    discard_until: u64,
}

impl AgentAudio {
    pub fn position(&self, boundary: ReplyBoundary) -> u64 {
        match boundary {
            ReplyBoundary::Received => self.received,
            ReplyBoundary::At(position) => position,
        }
    }

    /// Account one frame and return the part after the stale-audio fence.
    pub fn accept<'a>(&mut self, payload: &'a [u8]) -> &'a [u8] {
        let start = self.received;
        self.received = self.received.saturating_add(payload.len() as u64);
        if self.discard_until <= start {
            return payload;
        }
        let skip = (self.discard_until - start).min(payload.len() as u64) as usize;
        &payload[skip..]
    }

    pub fn fence(&mut self, boundary: ReplyBoundary) {
        self.discard_until = self.discard_until.max(self.position(boundary));
    }
}

/// In-band reply marks for one session task.
#[derive(Default)]
pub(crate) struct Marks {
    source: Option<ReplyMarks>,
    queued: std::collections::VecDeque<ReplyMark>,
    progress: Option<tokio::time::Instant>,
}

impl Marks {
    pub fn follow(&mut self, source: ReplyMarks) -> Result<()> {
        if self.source.is_some() {
            return Err(WebexError::State);
        }
        self.source = Some(source);
        Ok(())
    }

    pub fn is_following(&self) -> bool {
        self.source.is_some()
    }

    pub async fn next(&mut self) {
        let next = match self.source.as_mut() {
            Some(source) => futures::StreamExt::next(source).await,
            None => std::future::pending().await,
        };
        match next {
            Some(mark) => self.push(mark),
            None => self.source = None,
        }
    }

    /// Take every mark that is already available without waiting.
    pub fn drain_ready(&mut self) {
        use futures::FutureExt;
        while let Some(source) = self.source.as_mut() {
            match futures::StreamExt::next(source).now_or_never() {
                Some(Some(mark)) => self.push(mark),
                Some(None) => self.source = None,
                None => break,
            }
        }
    }

    fn push(&mut self, mark: ReplyMark) {
        if self.queued.is_empty() {
            self.progress = Some(tokio::time::Instant::now());
        }
        self.queued.push_back(mark);
    }

    /// The next mark at or before `received`, removed from the queue.
    pub fn due(&mut self, received: u64) -> Option<ReplyMark> {
        let due = self
            .queued
            .front()
            .is_some_and(|mark| mark.position <= received);
        if due {
            self.progress = Some(tokio::time::Instant::now());
            self.queued.pop_front()
        } else {
            None
        }
    }

    /// Bytes of the next `available` that precede the next queued mark.
    pub fn before_next(&self, received: u64, available: usize) -> usize {
        match self.queued.front() {
            Some(mark) if mark.position > received => {
                (mark.position - received).min(available as u64) as usize
            }
            Some(_) => 0,
            None => available,
        }
    }

    pub fn audio_progressed(&mut self) {
        if !self.queued.is_empty() {
            self.progress = Some(tokio::time::Instant::now());
        }
    }

    /// When the head mark gives up waiting for its audio.
    pub fn deadline(&self, budget: Duration) -> Option<tokio::time::Instant> {
        if self.queued.is_empty() {
            return None;
        }
        self.progress.map(|progress| progress + budget)
    }
}

/// A completion waiting for its audio.
pub(crate) struct PendingCompletion {
    pub position: u64,
    pub reply: oneshot::Sender<Result<()>>,
    pub deadline: tokio::time::Instant,
}
