//! Staged outbound [`ConnectionAdapter`] implementation for Vapi.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use parking_lot::Mutex;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, AdapterLifecycleCapabilities, AdapterLifecycleSink,
    AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    ExternalConnectionReference, OriginateRequest, OutboundActivation, RejectReason,
    SignatureHeaders, TransferTarget,
};
use rvoip_core::commands::MuteDirection;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::ConnectionId;
use rvoip_core::message::Message;
use rvoip_core::stream::{MediaStream, MediaStreamHandle};
use rvoip_core::{CapabilityDescriptor, NegotiatedCodecs};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message as WebSocketMessage;
use tracing::warn;

use crate::client::{connect_websocket, send_with_timeout, VapiHttpClient, VapiSocket};
use crate::config::VapiConfig;
use crate::error::{Result, VapiError};
use crate::events::{VapiEvent, VapiEventEnvelope};
use crate::media::{append_bounded_frames, AudioFramer, VapiMediaStream};
use crate::types::{AddedMessage, VapiCallOptions, VapiCommand};

pub const ADAPTER_EVENT_CAPACITY: usize = 256;
pub const VAPI_CALL_REFERENCE_KIND: &str = "vapi-call-id";

/// Opaque handle placed on the rvoip connection.
pub struct VapiTransportHandle {
    connection_id: ConnectionId,
}

impl VapiTransportHandle {
    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }
}

impl fmt::Debug for VapiTransportHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VapiTransportHandle([redacted])")
    }
}

struct Route {
    connection_id: ConnectionId,
    options: VapiCallOptions,
    stream: Arc<VapiMediaStream>,
    cancel: tokio_util::sync::CancellationToken,
    live: AtomicBool,
    active: AtomicBool,
    published: AtomicBool,
    terminal: AtomicBool,
    publication: Mutex<()>,
    activation_started: AtomicBool,
    activation_result:
        watch::Sender<Option<std::result::Result<OutboundActivation, ActivationFailure>>>,
    command_tx: Mutex<Option<mpsc::Sender<String>>>,
    call_id: Mutex<Option<String>>,
    requested_end_reason: Mutex<Option<EndReason>>,
    events: broadcast::Sender<VapiEvent>,
    initial_events: Mutex<Option<broadcast::Receiver<VapiEvent>>>,
    finished: tokio_util::sync::CancellationToken,
}

impl Route {
    fn new(
        connection_id: ConnectionId,
        options: VapiCallOptions,
        config: &VapiConfig,
    ) -> Arc<Self> {
        let cancel = tokio_util::sync::CancellationToken::new();
        let stream = VapiMediaStream::new(
            options.audio_format,
            1,
            config.media_queue_capacity,
            cancel.clone(),
        );
        let (events, initial_events) = broadcast::channel(config.event_queue_capacity);
        let (activation_result, _) = watch::channel(None);
        Arc::new(Self {
            connection_id,
            options,
            stream,
            cancel,
            live: AtomicBool::new(true),
            active: AtomicBool::new(false),
            published: AtomicBool::new(false),
            terminal: AtomicBool::new(false),
            publication: Mutex::new(()),
            activation_started: AtomicBool::new(false),
            activation_result,
            command_tx: Mutex::new(None),
            call_id: Mutex::new(None),
            requested_end_reason: Mutex::new(None),
            events,
            initial_events: Mutex::new(Some(initial_events)),
            finished: tokio_util::sync::CancellationToken::new(),
        })
    }

    fn activation_receipt(&self) -> RvoipResult<OutboundActivation> {
        let call_id = self.call_id.lock().clone().ok_or(RvoipError::InvalidState(
            "Vapi call activation has no call identifier",
        ))?;
        let reference = ExternalConnectionReference::new(VAPI_CALL_REFERENCE_KIND, call_id)
            .map_err(|_| RvoipError::Adapter("Vapi returned an invalid call identifier".into()))?;
        Ok(OutboundActivation::with_external_reference(reference))
    }

    fn take_event_receiver(&self) -> broadcast::Receiver<VapiEvent> {
        self.initial_events
            .lock()
            .take()
            .unwrap_or_else(|| self.events.subscribe())
    }
}

struct RuntimeEnvironment {
    config: VapiConfig,
    routes: Arc<DashMap<ConnectionId, Arc<Route>>>,
    adapter_events: mpsc::Sender<AdapterEvent>,
    vapi_events: broadcast::Sender<VapiEventEnvelope>,
    lifecycle: AdapterLifecycleSinkSlot,
}

#[derive(Clone, Copy)]
enum ActivationFailure {
    Vapi(VapiError),
    Cancelled,
    EventQueueFull,
}

impl ActivationFailure {
    fn into_rvoip(self, connection_id: &ConnectionId) -> RvoipError {
        match self {
            Self::Vapi(error) => error.into(),
            Self::Cancelled => RvoipError::ConnectionNotFound(connection_id.clone()),
            Self::EventQueueFull => {
                RvoipError::AdmissionRejected("Vapi adapter event queue is full")
            }
        }
    }
}

/// An outbound-only Vapi interop adapter.
pub struct VapiAdapter {
    config: VapiConfig,
    http: VapiHttpClient,
    routes: Arc<DashMap<ConnectionId, Arc<Route>>>,
    adapter_events_tx: mpsc::Sender<AdapterEvent>,
    adapter_events_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
    vapi_events: broadcast::Sender<VapiEventEnvelope>,
    lifecycle: AdapterLifecycleSinkSlot,
}

impl VapiAdapter {
    pub fn new(config: VapiConfig) -> Result<Arc<Self>> {
        config.validate()?;
        let http = VapiHttpClient::new(&config)?;
        let (adapter_events_tx, adapter_events_rx) = mpsc::channel(ADAPTER_EVENT_CAPACITY);
        let (vapi_events, _) = broadcast::channel(config.event_queue_capacity);
        Ok(Arc::new(Self {
            config,
            http,
            routes: Arc::new(DashMap::new()),
            adapter_events_tx,
            adapter_events_rx: StdMutex::new(Some(adapter_events_rx)),
            vapi_events,
            lifecycle: AdapterLifecycleSinkSlot::default(),
        }))
    }

    pub fn subscribe_vapi_events(&self) -> broadcast::Receiver<VapiEventEnvelope> {
        self.vapi_events.subscribe()
    }

    pub fn subscribe_call_events(
        &self,
        connection_id: &ConnectionId,
    ) -> RvoipResult<broadcast::Receiver<VapiEvent>> {
        self.routes
            .get(connection_id)
            .map(|route| route.take_event_receiver())
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))
    }

    pub(crate) fn call_event_sender(
        &self,
        connection_id: &ConnectionId,
    ) -> RvoipResult<broadcast::Sender<VapiEvent>> {
        self.routes
            .get(connection_id)
            .map(|route| route.events.clone())
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))
    }

    pub async fn say(
        &self,
        connection_id: &ConnectionId,
        content: impl Into<String>,
        end_call_after_spoken: bool,
        interrupt_assistant: bool,
    ) -> Result<()> {
        let content = content.into();
        if content.trim().is_empty() {
            return Err(VapiError::InvalidCallOptions(
                "say content must not be empty",
            ));
        }
        self.send_command(
            connection_id,
            VapiCommand::Say {
                content,
                end_call_after_spoken,
                interrupt_assistant_enabled: interrupt_assistant,
            },
        )
        .await
    }

    pub async fn add_message(
        &self,
        connection_id: &ConnectionId,
        role: impl Into<String>,
        content: impl Into<String>,
        trigger_response: bool,
    ) -> Result<()> {
        let role = role.into();
        let content = content.into();
        if !matches!(role.as_str(), "user" | "assistant" | "system") || content.trim().is_empty() {
            return Err(VapiError::InvalidCallOptions(
                "the added message role or content is invalid",
            ));
        }
        self.send_command(
            connection_id,
            VapiCommand::AddMessage {
                message: AddedMessage { role, content },
                trigger_response_enabled: trigger_response,
            },
        )
        .await
    }

    pub async fn mute_assistant(&self, connection_id: &ConnectionId) -> Result<()> {
        self.send_control(connection_id, "mute-assistant").await
    }

    pub async fn unmute_assistant(&self, connection_id: &ConnectionId) -> Result<()> {
        self.send_control(connection_id, "unmute-assistant").await
    }

    async fn send_control(
        &self,
        connection_id: &ConnectionId,
        control: &'static str,
    ) -> Result<()> {
        self.send_command(connection_id, VapiCommand::Control { control })
            .await
    }

    async fn send_command(&self, connection_id: &ConnectionId, command: VapiCommand) -> Result<()> {
        let route = self
            .routes
            .get(connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or(VapiError::NotActive)?;
        if !route.active.load(Ordering::Acquire) || !route.live.load(Ordering::Acquire) {
            return Err(VapiError::NotActive);
        }
        let serialized =
            serde_json::to_string(&command).map_err(|_| VapiError::ControlQueueUnavailable)?;
        if serialized.len() > self.config.max_message_bytes {
            return Err(VapiError::ControlMessageTooLarge);
        }
        let sender = route
            .command_tx
            .lock()
            .clone()
            .ok_or(VapiError::NotActive)?;
        sender
            .try_send(serialized)
            .map_err(|_| VapiError::ControlQueueUnavailable)
    }

    fn runtime_environment(&self) -> RuntimeEnvironment {
        RuntimeEnvironment {
            config: self.config.clone(),
            routes: Arc::clone(&self.routes),
            adapter_events: self.adapter_events_tx.clone(),
            vapi_events: self.vapi_events.clone(),
            lifecycle: self.lifecycle.clone(),
        }
    }

    async fn activate_route(&self, route: Arc<Route>) -> RvoipResult<OutboundActivation> {
        let mut activation_result = route.activation_result.subscribe();
        if route
            .activation_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let environment = self.runtime_environment();
            let http = self.http.clone();
            let worker_route = Arc::clone(&route);
            tokio::spawn(async move {
                let result =
                    activate_route_worker(&environment, &http, Arc::clone(&worker_route)).await;
                worker_route.activation_result.send_replace(Some(result));
            });
        }
        loop {
            if let Some(result) = activation_result.borrow().clone() {
                return result.map_err(|failure| failure.into_rvoip(&route.connection_id));
            }
            activation_result.changed().await.map_err(|_| {
                RvoipError::InvalidState("Vapi activation owner stopped unexpectedly")
            })?;
        }
    }

    async fn terminate(&self, connection_id: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        let route = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))?;
        *route.requested_end_reason.lock() = Some(reason.clone());
        route.cancel.cancel();

        if !route.active.load(Ordering::Acquire) {
            finish_route(
                &self.runtime_environment(),
                Arc::clone(&route),
                SessionOutcome::Local(reason),
            )
            .await;
            return Ok(());
        }
        if tokio::time::timeout(
            self.config.graceful_shutdown_timeout,
            route.finished.cancelled(),
        )
        .await
        .is_err()
        {
            let route = self
                .routes
                .get(&connection_id)
                .map(|entry| Arc::clone(entry.value()));
            if let Some(route) = route {
                finish_route(
                    &self.runtime_environment(),
                    route,
                    SessionOutcome::Local(reason),
                )
                .await;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ConnectionAdapter for VapiAdapter {
    fn transport(&self) -> Transport {
        Transport::Vapi
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities {
            authoritative_liveness: true,
            atomic_inbound_handoff: true,
            terminal_fallback: true,
            staged_outbound_activation: true,
        }
    }

    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> RvoipResult<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("Vapi lifecycle sink already installed"))
    }

    fn is_connection_live(&self, connection_id: &ConnectionId) -> bool {
        self.routes
            .get(connection_id)
            .is_some_and(|route| route.live.load(Ordering::Acquire))
    }

    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        if request.direction != Direction::Outbound {
            return Err(RvoipError::AdmissionRejected(
                "Vapi originate requires outbound direction",
            ));
        }
        let options = request.context.downcast_arc::<VapiCallOptions>().ok_or(
            RvoipError::AdmissionRejected("Vapi originate requires VapiCallOptions context"),
        )?;
        options.validate().map_err(RvoipError::from)?;

        let connection_id = ConnectionId::new();
        let route = Route::new(
            connection_id.clone(),
            options.as_ref().clone(),
            &self.config,
        );
        let codec = options.audio_format.codec();
        let connection = Connection {
            id: connection_id.clone(),
            session_id: request.session_id,
            participant_id: request.participant_id,
            transport: Transport::Vapi,
            direction: Direction::Outbound,
            state: ConnectionState::Connecting,
            capabilities: options.audio_format.capabilities(),
            negotiated_codecs: NegotiatedCodecs {
                audio: Some(codec),
                video: None,
            },
            streams: vec![MediaStreamHandle::new(
                Arc::clone(&route.stream) as Arc<dyn MediaStream>
            )],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(VapiTransportHandle {
                connection_id: connection_id.clone(),
            })),
            opened_at: Utc::now(),
            closed_at: None,
        };
        if self.routes.insert(connection_id, route).is_some() {
            return Err(RvoipError::AdmissionRejected(
                "Vapi provisional connection ID collided",
            ));
        }
        Ok(ConnectionHandle::new(connection))
    }

    async fn activate_outbound(&self, connection_id: ConnectionId) -> RvoipResult<()> {
        self.activate_outbound_with_receipt(connection_id)
            .await
            .map(|_| ())
    }

    async fn activate_outbound_with_receipt(
        &self,
        connection_id: ConnectionId,
    ) -> RvoipResult<OutboundActivation> {
        let route = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id))?;
        self.activate_route(route).await
    }

    async fn accept(&self, connection_id: ConnectionId) -> RvoipResult<()> {
        if self.is_connection_live(&connection_id) {
            Ok(())
        } else {
            Err(RvoipError::ConnectionNotFound(connection_id))
        }
    }

    async fn reject(&self, connection_id: ConnectionId, _reason: RejectReason) -> RvoipResult<()> {
        self.terminate(connection_id, EndReason::Cancelled).await
    }

    async fn end(&self, connection_id: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        self.terminate(connection_id, reason).await
    }

    async fn hold(&self, _connection_id: ConnectionId) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented(
            "Vapi transport hold is not supported",
        ))
    }

    async fn resume(&self, _connection_id: ConnectionId) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented(
            "Vapi transport resume is not supported",
        ))
    }

    async fn transfer(
        &self,
        _connection_id: ConnectionId,
        _target: TransferTarget,
    ) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented(
            "Vapi transport transfer remains owned by rvoip",
        ))
    }

    async fn streams(&self, connection_id: ConnectionId) -> RvoipResult<Vec<Arc<dyn MediaStream>>> {
        let route = self
            .routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))?;
        if !route.active.load(Ordering::Acquire) || !route.live.load(Ordering::Acquire) {
            return Ok(Vec::new());
        }
        Ok(vec![Arc::clone(&route.stream) as Arc<dyn MediaStream>])
    }

    async fn send_message(
        &self,
        _connection_id: ConnectionId,
        _message: Message,
    ) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented(
            "use VapiAdapter::add_message for Vapi context injection",
        ))
    }

    async fn send_dtmf(
        &self,
        _connection_id: ConnectionId,
        _digits: &str,
        _duration_ms: u32,
    ) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented(
            "Vapi WebSocket transport has no DTMF input message",
        ))
    }

    async fn renegotiate_media(
        &self,
        _connection_id: ConnectionId,
        _capabilities: CapabilityDescriptor,
    ) -> RvoipResult<NegotiatedCodecs> {
        Err(RvoipError::NotImplemented(
            "Vapi raw-audio format is fixed for the call lifetime",
        ))
    }

    async fn mute(&self, connection_id: ConnectionId, direction: MuteDirection) -> RvoipResult<()> {
        match direction {
            MuteDirection::Send | MuteDirection::Both => {
                return Err(RvoipError::NotImplemented(
                    "Vapi customer-input mute is not a documented WebSocket control",
                ));
            }
            MuteDirection::Receive => self.send_control(&connection_id, "mute-assistant").await?,
        }
        Ok(())
    }

    async fn unmute(
        &self,
        connection_id: ConnectionId,
        direction: MuteDirection,
    ) -> RvoipResult<()> {
        match direction {
            MuteDirection::Send | MuteDirection::Both => {
                return Err(RvoipError::NotImplemented(
                    "Vapi customer-input unmute is not a documented WebSocket control",
                ));
            }
            MuteDirection::Receive => {
                self.send_control(&connection_id, "unmute-assistant")
                    .await?
            }
        }
        Ok(())
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        match self
            .adapter_events_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            Some(receiver) => receiver,
            None => {
                warn!("VapiAdapter::subscribe_events called more than once");
                mpsc::channel(1).1
            }
        }
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            audio_codecs: vec![
                crate::types::VapiAudioFormat::MuLaw8Khz.codec(),
                crate::types::VapiAudioFormat::PcmS16Le16Khz.codec(),
            ],
            max_streams_per_connection: 1,
            ..CapabilityDescriptor::default()
        }
    }

    async fn verify_request_signature(
        &self,
        _connection_id: ConnectionId,
        _signature: SignatureHeaders,
    ) -> RvoipResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

enum SessionOutcome {
    Local(EndReason),
    RemoteEnded,
    RemoteClosed,
    Failed(&'static str),
}

async fn activate_route_worker(
    environment: &RuntimeEnvironment,
    http: &VapiHttpClient,
    route: Arc<Route>,
) -> std::result::Result<OutboundActivation, ActivationFailure> {
    if route.cancel.is_cancelled() || !route.live.load(Ordering::Acquire) {
        return Err(ActivationFailure::Cancelled);
    }

    // This request deliberately lives in a detached worker. Cancellation of
    // the caller awaiting `activate_outbound` cannot abandon an ambiguous
    // POST that may already have created a billable remote call.
    let created = match http.create_call(&environment.config, &route.options).await {
        Ok(created) => created,
        Err(error) => {
            metrics::counter!("rvoip_vapi_setup_failures_total", "stage" => "call_creation")
                .increment(1);
            return Err(ActivationFailure::Vapi(error));
        }
    };
    *route.call_id.lock() = Some(created.id);
    let receipt = match route.activation_receipt() {
        Ok(receipt) => receipt,
        Err(_) => {
            metrics::counter!("rvoip_vapi_setup_failures_total", "stage" => "call_response")
                .increment(1);
            cleanup_created_call(environment.config.clone(), created.websocket_url).await;
            return Err(ActivationFailure::Vapi(VapiError::InvalidResponse(
                "the call ID was invalid",
            )));
        }
    };

    if route.cancel.is_cancelled() || !route.live.load(Ordering::Acquire) {
        cleanup_created_call(environment.config.clone(), created.websocket_url).await;
        return Err(ActivationFailure::Cancelled);
    }

    let mut socket = match connect_websocket(&environment.config, &created.websocket_url).await {
        Ok(socket) => socket,
        Err(error) => {
            metrics::counter!("rvoip_vapi_setup_failures_total", "stage" => "websocket")
                .increment(1);
            let cleanup_config = environment.config.clone();
            let cleanup_url = created.websocket_url;
            tokio::spawn(async move {
                cleanup_created_call(cleanup_config, cleanup_url).await;
            });
            return Err(ActivationFailure::Vapi(error));
        }
    };
    if route.cancel.is_cancelled() || !route.live.load(Ordering::Acquire) {
        close_vapi_socket(&environment.config, &mut socket).await;
        return Err(ActivationFailure::Cancelled);
    }

    let outgoing = match route.stream.take_outgoing_receiver() {
        Ok(outgoing) => outgoing,
        Err(error) => {
            metrics::counter!("rvoip_vapi_setup_failures_total", "stage" => "media").increment(1);
            close_vapi_socket(&environment.config, &mut socket).await;
            return Err(ActivationFailure::Vapi(error));
        }
    };
    let (command_tx, command_rx) = mpsc::channel(environment.config.control_queue_capacity);
    *route.command_tx.lock() = Some(command_tx);
    route.active.store(true, Ordering::Release);
    route.stream.activate();

    if route.cancel.is_cancelled() || !route.live.load(Ordering::Acquire) {
        route.active.store(false, Ordering::Release);
        route.stream.deactivate();
        route.command_tx.lock().take();
        close_vapi_socket(&environment.config, &mut socket).await;
        let reason = route
            .requested_end_reason
            .lock()
            .clone()
            .unwrap_or(EndReason::Cancelled);
        finish_route(
            environment,
            Arc::clone(&route),
            SessionOutcome::Local(reason),
        )
        .await;
        return Err(ActivationFailure::Cancelled);
    }
    // Serialize publication against terminal teardown. This guarantees that a
    // terminal event can neither overtake `Connected` nor disappear in the
    // interval between queue admission and the published flag becoming visible.
    let publication = {
        let _publication = route.publication.lock();
        if route.cancel.is_cancelled()
            || !route.live.load(Ordering::Acquire)
            || route.terminal.load(Ordering::Acquire)
        {
            Err(ActivationFailure::Cancelled)
        } else {
            route.published.store(true, Ordering::Release);
            if environment
                .adapter_events
                .try_send(AdapterEvent::Connected {
                    connection_id: route.connection_id.clone(),
                })
                .is_err()
            {
                route.published.store(false, Ordering::Release);
                Err(ActivationFailure::EventQueueFull)
            } else {
                Ok(())
            }
        }
    };
    if let Err(failure) = publication {
        if matches!(failure, ActivationFailure::EventQueueFull) {
            metrics::counter!("rvoip_vapi_setup_failures_total", "stage" => "publication")
                .increment(1);
        }
        route.active.store(false, Ordering::Release);
        route.stream.deactivate();
        route.command_tx.lock().take();
        close_vapi_socket(&environment.config, &mut socket).await;
        let reason = route
            .requested_end_reason
            .lock()
            .clone()
            .unwrap_or(EndReason::Cancelled);
        finish_route(
            environment,
            Arc::clone(&route),
            SessionOutcome::Local(reason),
        )
        .await;
        return Err(failure);
    }
    if route.cancel.is_cancelled()
        || !route.live.load(Ordering::Acquire)
        || route.terminal.load(Ordering::Acquire)
    {
        close_vapi_socket(&environment.config, &mut socket).await;
        let reason = route
            .requested_end_reason
            .lock()
            .clone()
            .unwrap_or(EndReason::Cancelled);
        finish_route(
            environment,
            Arc::clone(&route),
            SessionOutcome::Local(reason),
        )
        .await;
        return Err(ActivationFailure::Cancelled);
    }

    let runtime_environment = RuntimeEnvironment {
        config: environment.config.clone(),
        routes: Arc::clone(&environment.routes),
        adapter_events: environment.adapter_events.clone(),
        vapi_events: environment.vapi_events.clone(),
        lifecycle: environment.lifecycle.clone(),
    };
    let task_route = Arc::clone(&route);
    tokio::spawn(async move {
        let outcome = run_websocket_session(
            &runtime_environment.config,
            &runtime_environment.vapi_events,
            &task_route,
            socket,
            outgoing,
            command_rx,
        )
        .await;
        finish_route(&runtime_environment, task_route, outcome).await;
    });
    Ok(receipt)
}

async fn cleanup_created_call(config: VapiConfig, websocket_url: url::Url) {
    if let Ok(mut socket) = connect_websocket(&config, &websocket_url).await {
        close_vapi_socket(&config, &mut socket).await;
    }
}

async fn close_vapi_socket(config: &VapiConfig, socket: &mut VapiSocket) {
    let shutdown = async {
        let _ = socket
            .send(WebSocketMessage::Text(r#"{"type":"end-call"}"#.into()))
            .await;
        let _ = socket.close(None).await;
    };
    let _ = tokio::time::timeout(config.graceful_shutdown_timeout, shutdown).await;
}

enum SessionSendFailure {
    Cancelled,
    Failed,
}

async fn cancellable_session_send(
    config: &VapiConfig,
    route: &Route,
    socket: &mut VapiSocket,
    message: WebSocketMessage,
) -> std::result::Result<(), SessionSendFailure> {
    tokio::select! {
        _ = route.cancel.cancelled() => Err(SessionSendFailure::Cancelled),
        result = send_with_timeout(config.websocket_io_timeout, socket, message) => {
            result.map_err(|_| SessionSendFailure::Failed)
        }
    }
}

fn requested_local_outcome(route: &Route) -> SessionOutcome {
    SessionOutcome::Local(
        route
            .requested_end_reason
            .lock()
            .clone()
            .unwrap_or(EndReason::Normal),
    )
}

async fn run_websocket_session(
    config: &VapiConfig,
    global_events: &broadcast::Sender<VapiEventEnvelope>,
    route: &Arc<Route>,
    mut socket: VapiSocket,
    mut outgoing: mpsc::Receiver<rvoip_core::MediaFrame>,
    mut commands: mpsc::Receiver<String>,
) -> SessionOutcome {
    let mut incoming_framer = AudioFramer::new(route.options.audio_format);
    let mut outgoing_framer = AudioFramer::new(route.options.audio_format);
    let mut incoming_frames = VecDeque::<Bytes>::new();
    let mut outgoing_frames = VecDeque::<Bytes>::new();
    let mut timestamp_rtp = 0u32;
    let mut incoming_tick = tokio::time::interval(std::time::Duration::from_millis(20));
    incoming_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    incoming_tick.tick().await;
    let mut outgoing_tick = tokio::time::interval(std::time::Duration::from_millis(20));
    outgoing_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    outgoing_tick.tick().await;
    let mut heartbeat = tokio::time::interval(config.heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let heartbeat_deadline = tokio::time::sleep(config.websocket_io_timeout);
    tokio::pin!(heartbeat_deadline);
    let mut awaiting_heartbeat_response = false;

    let outcome = loop {
        tokio::select! {
            _ = route.cancel.cancelled() => {
                break requested_local_outcome(route);
            }
            _ = &mut heartbeat_deadline, if awaiting_heartbeat_response => {
                break SessionOutcome::Failed("Vapi WebSocket heartbeat response timed out");
            }
            _ = incoming_tick.tick(), if !incoming_frames.is_empty() => {
                if route.stream.incoming_is_closed() {
                    break SessionOutcome::Local(EndReason::BridgeTorn);
                }
                if !route.stream.incoming_has_capacity() {
                    continue;
                }
                let Some(frame) = incoming_frames.pop_front() else {
                    continue;
                };
                if route.stream.try_push_incoming(frame, timestamp_rtp).is_err() {
                    metrics::counter!("rvoip_vapi_queue_overflow_total", "direction" => "in").increment(1);
                    break SessionOutcome::Failed("Vapi inbound media queue overflow");
                }
                timestamp_rtp = timestamp_rtp
                    .wrapping_add(route.options.audio_format.timestamp_increment());
                metrics::counter!("rvoip_vapi_audio_frames_total", "direction" => "in").increment(1);
            }
            _ = outgoing_tick.tick(), if !outgoing_frames.is_empty() => {
                let Some(frame) = outgoing_frames.pop_front() else {
                    continue;
                };
                match cancellable_session_send(
                    config,
                    route,
                    &mut socket,
                    WebSocketMessage::Binary(frame),
                ).await {
                    Ok(()) => {}
                    Err(SessionSendFailure::Cancelled) => break requested_local_outcome(route),
                    Err(SessionSendFailure::Failed) => {
                        break SessionOutcome::Failed("Vapi WebSocket audio write failed");
                    }
                }
                metrics::counter!("rvoip_vapi_audio_frames_total", "direction" => "out").increment(1);
            }
            _ = heartbeat.tick() => {
                if awaiting_heartbeat_response {
                    continue;
                }
                match cancellable_session_send(
                    config,
                    route,
                    &mut socket,
                    WebSocketMessage::Ping(Bytes::new()),
                ).await {
                    Ok(()) => {}
                    Err(SessionSendFailure::Cancelled) => break requested_local_outcome(route),
                    Err(SessionSendFailure::Failed) => {
                        break SessionOutcome::Failed("Vapi WebSocket heartbeat failed");
                    }
                }
                heartbeat_deadline
                    .as_mut()
                    .reset(tokio::time::Instant::now() + config.websocket_io_timeout);
                awaiting_heartbeat_response = true;
            }
            media = outgoing.recv() => {
                let Some(media) = media else {
                    break SessionOutcome::Local(EndReason::BridgeTorn);
                };
                if media.payload_type == Some(101) {
                    metrics::counter!("rvoip_vapi_dtmf_frames_dropped_total").increment(1);
                    continue;
                }
                if append_bounded_frames(
                    &mut outgoing_framer,
                    &media.payload,
                    &mut outgoing_frames,
                    config.media_queue_capacity,
                    config.max_message_bytes,
                ).is_err() {
                    metrics::counter!("rvoip_vapi_queue_overflow_total", "direction" => "out").increment(1);
                    break SessionOutcome::Failed("Vapi outbound media queue overflow");
                }
            }
            command = commands.recv() => {
                let Some(json) = command else {
                    break SessionOutcome::Local(EndReason::Normal);
                };
                match cancellable_session_send(
                    config,
                    route,
                    &mut socket,
                    WebSocketMessage::Text(json.into()),
                ).await {
                    Ok(()) => {}
                    Err(SessionSendFailure::Cancelled) => break requested_local_outcome(route),
                    Err(SessionSendFailure::Failed) => {
                        break SessionOutcome::Failed("Vapi WebSocket control write failed");
                    }
                }
            }
            message = socket.next() => {
                if matches!(message.as_ref(), Some(Ok(_))) {
                    awaiting_heartbeat_response = false;
                }
                match message {
                    Some(Ok(WebSocketMessage::Binary(payload))) => {
                        let max_queued = config
                            .startup_audio_frames
                            .saturating_sub(route.stream.incoming_pending_frames());
                        if append_bounded_frames(
                            &mut incoming_framer,
                            &payload,
                            &mut incoming_frames,
                            max_queued,
                            config.max_message_bytes,
                        ).is_err() {
                            metrics::counter!("rvoip_vapi_queue_overflow_total", "direction" => "in").increment(1);
                            break SessionOutcome::Failed("Vapi inbound media buffer overflow");
                        }
                    }
                    Some(Ok(WebSocketMessage::Text(text))) => {
                        if text.len() > config.max_message_bytes {
                            break SessionOutcome::Failed("Vapi event exceeded size limit");
                        }
                        let event = VapiEvent::parse(text.as_str());
                        let terminal = event.is_terminal_status();
                        publish_vapi_event(global_events, route, event);
                        if terminal {
                            break SessionOutcome::RemoteEnded;
                        }
                    }
                    Some(Ok(WebSocketMessage::Ping(payload))) => {
                        match cancellable_session_send(
                            config,
                            route,
                            &mut socket,
                            WebSocketMessage::Pong(payload),
                        ).await {
                            Ok(()) => {}
                            Err(SessionSendFailure::Cancelled) => break requested_local_outcome(route),
                            Err(SessionSendFailure::Failed) => {
                                break SessionOutcome::Failed("Vapi WebSocket pong failed");
                            }
                        }
                    }
                    Some(Ok(WebSocketMessage::Pong(_))) => {}
                    Some(Ok(WebSocketMessage::Close(frame))) => {
                        break match frame.map(|frame| frame.code) {
                            Some(CloseCode::Normal | CloseCode::Away) => {
                                SessionOutcome::RemoteEnded
                            }
                            _ => SessionOutcome::RemoteClosed,
                        };
                    }
                    None => break SessionOutcome::RemoteClosed,
                    Some(Ok(WebSocketMessage::Frame(_))) => {}
                    Some(Err(_)) => {
                        break SessionOutcome::Failed("Vapi WebSocket read failed");
                    }
                }
            }
        }
    };

    match &outcome {
        SessionOutcome::Local(_) => {
            wait_for_vapi_shutdown(config, global_events, route, &mut socket).await;
        }
        _ => {
            let _ =
                tokio::time::timeout(config.graceful_shutdown_timeout, socket.close(None)).await;
        }
    }
    outcome
}

async fn wait_for_vapi_shutdown(
    config: &VapiConfig,
    global_events: &broadcast::Sender<VapiEventEnvelope>,
    route: &Route,
    socket: &mut VapiSocket,
) {
    let deadline = tokio::time::Instant::now() + config.graceful_shutdown_timeout;
    if !matches!(
        tokio::time::timeout_at(
            deadline,
            socket.send(WebSocketMessage::Text(r#"{"type":"end-call"}"#.into())),
        )
        .await,
        Ok(Ok(()))
    ) {
        return;
    }

    loop {
        match tokio::time::timeout_at(deadline, socket.next()).await {
            Ok(Some(Ok(WebSocketMessage::Text(text)))) => {
                if text.len() > config.max_message_bytes {
                    break;
                }
                let event = VapiEvent::parse(text.as_str());
                let terminal = event.is_terminal_status();
                publish_vapi_event(global_events, route, event);
                if terminal {
                    break;
                }
            }
            Ok(Some(Ok(WebSocketMessage::Ping(payload)))) => {
                if !matches!(
                    tokio::time::timeout_at(
                        deadline,
                        socket.send(WebSocketMessage::Pong(payload)),
                    )
                    .await,
                    Ok(Ok(()))
                ) {
                    break;
                }
            }
            Ok(Some(Ok(
                WebSocketMessage::Binary(_)
                | WebSocketMessage::Pong(_)
                | WebSocketMessage::Frame(_),
            ))) => {}
            Ok(Some(Ok(WebSocketMessage::Close(_))) | Some(Err(_)) | None) | Err(_) => break,
        }
    }

    if tokio::time::Instant::now() < deadline {
        let _ = tokio::time::timeout_at(deadline, socket.close(None)).await;
    }
}

fn publish_vapi_event(
    global_events: &broadcast::Sender<VapiEventEnvelope>,
    route: &Route,
    event: VapiEvent,
) {
    if matches!(event, VapiEvent::Unknown(_)) {
        metrics::counter!("rvoip_vapi_unknown_events_total").increment(1);
    }
    if matches!(event, VapiEvent::Malformed { .. }) {
        metrics::counter!("rvoip_vapi_malformed_events_total").increment(1);
    }
    let _ = global_events.send(VapiEventEnvelope {
        connection_id: route.connection_id.clone(),
        event: event.clone(),
    });
    let _ = route.events.send(event);
}

async fn finish_route(
    environment: &RuntimeEnvironment,
    route: Arc<Route>,
    outcome: SessionOutcome,
) {
    if route.terminal.swap(true, Ordering::AcqRel) {
        return;
    }
    let published = {
        let _publication = route.publication.lock();
        route.published.load(Ordering::Acquire)
    };
    route.live.store(false, Ordering::Release);
    route.active.store(false, Ordering::Release);
    route.stream.deactivate();
    route.cancel.cancel();
    route.command_tx.lock().take();
    route.call_id.lock().take();

    let remove = environment
        .routes
        .get(&route.connection_id)
        .is_some_and(|current| Arc::ptr_eq(current.value(), &route));
    if remove {
        environment.routes.remove(&route.connection_id);
    }

    match &outcome {
        SessionOutcome::RemoteEnded => {
            metrics::counter!("rvoip_vapi_disconnects_total", "outcome" => "normal").increment(1);
        }
        SessionOutcome::RemoteClosed => {
            metrics::counter!("rvoip_vapi_disconnects_total", "outcome" => "abnormal").increment(1);
        }
        SessionOutcome::Failed(_) => {
            metrics::counter!("rvoip_vapi_disconnects_total", "outcome" => "failed").increment(1);
        }
        SessionOutcome::Local(_) => {}
    }

    if published {
        let terminal_event = match outcome {
            SessionOutcome::Local(reason) => AdapterEvent::Ended {
                connection_id: route.connection_id.clone(),
                reason,
            },
            SessionOutcome::RemoteEnded => AdapterEvent::Ended {
                connection_id: route.connection_id.clone(),
                reason: EndReason::Normal,
            },
            SessionOutcome::RemoteClosed => AdapterEvent::Failed {
                connection_id: route.connection_id.clone(),
                detail: "Vapi WebSocket closed without a terminal status".into(),
            },
            SessionOutcome::Failed(detail) => AdapterEvent::Failed {
                connection_id: route.connection_id.clone(),
                detail: detail.into(),
            },
        };
        let _ = environment
            .lifecycle
            .queue_or_deliver_terminal(&environment.adapter_events, terminal_event)
            .await;
    }
    route.finished.cancel();
    metrics::counter!("rvoip_vapi_sessions_terminated_total").increment(1);
}
