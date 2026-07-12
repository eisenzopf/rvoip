//! `UctpWtAdapter` — implements `rvoip_core::ConnectionAdapter` over
//! WebTransport. Mirrors `rvoip_quic::adapter::UctpQuicAdapter`
//! line-for-line; only `transport()` and the WT-upgrade in the accept
//! path differ.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_auth_core::BearerValidator;
use rvoip_core::adapter::{
    legacy_normalized_event_receiver, AdapterEvent, AdapterKind, AdapterLifecycleCapabilities,
    AdapterLifecycleSink, AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    OrchestratorAdapterEvent, OriginateRequest, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;
use rvoip_core::{DataMessage, DataReliability};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads;
use rvoip_uctp::types::MessageType;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use url::Url;

use crate::server::UctpWtServer;

pub const ADAPTER_EVENT_CAP: usize = 256;

#[derive(Clone)]
pub(crate) struct Route {
    pub sid: String,
    pub core_session_id: SessionId,
    pub core_connection_id: ConnectionId,
    pub binding: rvoip_uctp::adapter_helpers::AuthenticatedConnectionBinding,
    pub out_tx: mpsc::Sender<UctpEnvelope>,
    /// Gap plan §4.2 v1 punch list — see rvoip-quic Route doc.
    pub pending: Arc<rvoip_uctp::substrate::Pending>,
    pub streams: Arc<DashMap<rvoip_core::ids::StreamId, Arc<dyn MediaStream>>>,
    /// The WT session; cloned into per-Stream pumps allocated by
    /// `allocate_subscriber_stream` (plan B1 / MP3c).
    pub session: web_transport_quinn::Session,
    /// One peer-global local-ID namespace shared by every logical Session and
    /// Connection carried over this physical WebTransport peer.
    pub media_router: Arc<rvoip_uctp::substrate::PeerMediaRouter>,
    pub route_cancel: CancellationToken,
    pub coordinator: Arc<rvoip_uctp::state::UctpCoordinator>,
}

pub struct UctpWtConfig {
    pub endpoint: Arc<quinn::Endpoint>,
    pub accept_rx: mpsc::Receiver<quinn::Connection>,
    pub bearer_validator: Arc<dyn BearerValidator>,
    pub max_concurrent_connections: usize,
    /// HTTP/3 `:path` to accept the WebTransport upgrade on.
    pub mount_path: String,
    pub quinn_stats_interval: std::time::Duration,
    pub client_endpoint: Option<Arc<quinn::Endpoint>>,
    pub client_tls: Option<Arc<rustls::ClientConfig>>,
    /// Multi-party `SubscriptionHandler` (v0.x MP2/MP2.6). See
    /// `UctpQuicConfig`
    /// for semantics — identical wiring.
    pub subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
    /// Authorization boundary between a peer-supplied Session ID and the
    /// canonical core Session used by shared publisher/subscriber registries.
    /// When omitted, every physical peer receives an isolated namespace.
    pub session_binding_resolver: Option<Arc<dyn rvoip_uctp::state::SessionBindingResolver>>,
    /// Orchestrator reference for multi-party media fanout (v0.x MP3b).
    /// See `UctpQuicConfig`.
    pub orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
    /// Per-peer resource caps (plan D1 / D2). See
    /// `rvoip_quic::UctpQuicConfig::coordinator_caps` for semantics —
    /// identical wiring.
    pub coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
    /// Optional inline RFC 9421 envelope-signature enforcement. Disabled
    /// by default for compatibility; see
    /// [`rvoip_uctp::state::Sig9421Config`].
    pub sig9421: Option<rvoip_uctp::state::Sig9421Config>,
}

impl UctpWtConfig {
    pub fn new(
        endpoint: Arc<quinn::Endpoint>,
        accept_rx: mpsc::Receiver<quinn::Connection>,
        bearer_validator: Arc<dyn BearerValidator>,
    ) -> Self {
        Self {
            endpoint,
            accept_rx,
            bearer_validator,
            max_concurrent_connections: 1024,
            mount_path: "/uctp".into(),
            quinn_stats_interval: std::time::Duration::from_secs(5),
            client_endpoint: None,
            client_tls: None,
            subscription_handler: None,
            session_binding_resolver: None,
            orchestrator: None,
            coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps::default(),
            sig9421: None,
        }
    }

    pub fn with_outbound(
        mut self,
        endpoint: Arc<quinn::Endpoint>,
        tls: Arc<rustls::ClientConfig>,
    ) -> Self {
        self.client_endpoint = Some(endpoint);
        self.client_tls = Some(tls);
        self
    }

    pub fn with_subscription_handler(
        mut self,
        handler: Arc<dyn rvoip_uctp::state::SubscriptionHandler>,
    ) -> Self {
        self.subscription_handler = Some(handler);
        self
    }

    pub fn with_session_binding_resolver(
        mut self,
        resolver: Arc<dyn rvoip_uctp::state::SessionBindingResolver>,
    ) -> Self {
        self.session_binding_resolver = Some(resolver);
        self
    }

    pub fn with_orchestrator(mut self, orch: Arc<rvoip_core::Orchestrator>) -> Self {
        self.orchestrator = Some(orch);
        self
    }

    /// Override the per-peer Session/timeout caps (plan D1 / D2). See
    /// [`rvoip_uctp::state::UctpCoordinatorCaps`].
    pub fn with_coordinator_caps(mut self, caps: rvoip_uctp::state::UctpCoordinatorCaps) -> Self {
        self.coordinator_caps = caps;
        self
    }

    /// Opt in to inline RFC 9421 envelope-signature verification at
    /// the adapter ingress boundary.
    pub fn with_sig9421(mut self, config: rvoip_uctp::state::Sig9421Config) -> Self {
        self.sig9421 = Some(config);
        self
    }
}

pub struct UctpWtAdapter {
    // These two maps are populated and consulted by the spawned server
    // accept loop; the adapter only retains its clones to keep the
    // backing Arcs alive if `_server` is ever dropped first. Marked
    // dead_code because the adapter API doesn't expose lookups on
    // them — the lookups happen inside the server task.
    #[allow(dead_code)]
    by_connection: Arc<DashMap<ConnectionId, String>>,
    #[allow(dead_code)]
    by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
    routes: Arc<DashMap<ConnectionId, Route>>,
    lifecycle_sink: AdapterLifecycleSinkSlot,
    events_tx: mpsc::Sender<OrchestratorAdapterEvent>,
    _server: Arc<UctpWtServer>,
    events_rx: StdMutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    local_addr: SocketAddr,
    client_endpoint: Option<Arc<quinn::Endpoint>>,
    client_tls: Option<Arc<rustls::ClientConfig>>,
}

impl UctpWtAdapter {
    pub async fn new(config: UctpWtConfig) -> Result<Arc<Self>, crate::errors::UctpWtError> {
        let local_addr = config
            .endpoint
            .local_addr()
            .map_err(rvoip_uctp::errors::SubstrateError::Io)?;
        let (events_tx, events_rx) = mpsc::channel(ADAPTER_EVENT_CAP);

        let by_connection: Arc<DashMap<ConnectionId, String>> = Arc::new(DashMap::new());
        let by_uctp_sid: Arc<DashMap<String, ConnectionId>> = Arc::new(DashMap::new());
        let routes: Arc<DashMap<ConnectionId, Route>> = Arc::new(DashMap::new());
        let lifecycle_sink = AdapterLifecycleSinkSlot::default();

        let server = UctpWtServer::start(
            config.accept_rx,
            config.bearer_validator,
            events_tx.clone(),
            lifecycle_sink.clone(),
            Arc::clone(&by_connection),
            Arc::clone(&by_uctp_sid),
            Arc::clone(&routes),
            config.max_concurrent_connections,
            config.mount_path,
            config.quinn_stats_interval,
            config.subscription_handler,
            config.session_binding_resolver,
            config.orchestrator,
            config.coordinator_caps,
            config.sig9421,
        );

        Ok(Arc::new(Self {
            by_connection,
            by_uctp_sid,
            routes,
            lifecycle_sink,
            events_tx,
            _server: server,
            events_rx: StdMutex::new(Some(events_rx)),
            local_addr,
            client_endpoint: config.client_endpoint,
            client_tls: config.client_tls,
        }))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    fn route(&self, conn: &ConnectionId) -> Option<Route> {
        self.routes.get(conn).map(|r| r.clone())
    }

    fn take_terminal_route(&self, conn: &ConnectionId) -> Option<Route> {
        let (_, route) = self.routes.remove(conn)?;
        self.by_connection.remove(conn);
        if self
            .by_uctp_sid
            .get(&route.sid)
            .is_some_and(|mapped| mapped.value() == conn)
        {
            self.by_uctp_sid.remove(&route.sid);
        }
        Some(route)
    }

    async fn close_terminal_media(route: &Route) {
        let removed = route.media_router.remove_connection(
            &rvoip_uctp::substrate::PeerMediaConnectionKey::new(
                route.core_session_id.clone(),
                route.core_connection_id.clone(),
            ),
        );
        route.streams.clear();
        for binding in removed {
            let _ = binding.stream().clone().close().await;
        }
    }

    async fn terminate_route(
        &self,
        conn: &ConnectionId,
        envelope: impl FnOnce(&Route) -> UctpEnvelope,
        terminal_event: AdapterEvent,
    ) {
        let Some(route) = self.take_terminal_route(conn) else {
            return;
        };
        let terminal_envelope = envelope(&route);
        if route.out_tx.try_send(terminal_envelope).is_err() {
            warn!(connection_id = %conn, "UCTP WebTransport terminal notification was not queued");
        }
        route.route_cancel.cancel();
        route
            .coordinator
            .retire_local_session(&SessionId::from_string(route.sid.clone()));
        let _ = self
            .lifecycle_sink
            .queue_or_deliver_orchestrator_terminal(&self.events_tx, terminal_event)
            .await;
        if tokio::time::timeout(
            std::time::Duration::from_secs(2),
            Self::close_terminal_media(&route),
        )
        .await
        .is_err()
        {
            warn!(connection_id = %conn, "UCTP WebTransport terminal media cleanup timed out");
        }
    }
}

#[async_trait]
impl ConnectionAdapter for UctpWtAdapter {
    fn transport(&self) -> Transport {
        Transport::WebTransport
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Substrate
    }

    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities {
            authoritative_liveness: true,
            atomic_inbound_handoff: true,
            terminal_fallback: true,
            staged_outbound_activation: false,
        }
    }

    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> RvoipResult<()> {
        self.lifecycle_sink.install(sink).map_err(|_| {
            RvoipError::InvalidState("UCTP WebTransport lifecycle sink already installed")
        })
    }

    fn is_connection_live(&self, conn: &ConnectionId) -> bool {
        self.routes.contains_key(conn)
    }

    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        let (endpoint, tls) = match (&self.client_endpoint, &self.client_tls) {
            (Some(e), Some(t)) => (Arc::clone(e), Arc::clone(t)),
            _ => return Err(RvoipError::NotImplemented(
                "rvoip-webtransport::originate (no client_endpoint configured; use UctpWtConfig::with_outbound)",
            )),
        };

        // For WT, `target` is expected to be a full https:// URL.
        let url = Url::parse(&request.target)
            .map_err(|e| RvoipError::Adapter(format!("invalid originate target URL: {}", e)))?;
        let host = url
            .host_str()
            .ok_or_else(|| RvoipError::Adapter("URL has no host".into()))?;
        let port = url.port().unwrap_or(443);
        let addr: SocketAddr = format!("{}:{}", host, port).parse().map_err(|_| {
            RvoipError::Adapter(format!(
                "can't resolve {}:{} to SocketAddr (use ip address for v0)",
                host, port
            ))
        })?;

        let client = crate::client::UctpWtClient::connect(&endpoint, addr, &url, tls)
            .await
            .map_err(|e| RvoipError::Adapter(format!("dial failed: {}", e)))?;

        let quinn_conn = (*client.session).clone();

        let connection = Connection {
            id: ConnectionId::new(),
            session_id: request.session_id,
            participant_id: request.participant_id,
            transport: Transport::WebTransport,
            direction: Direction::Outbound,
            state: ConnectionState::Connecting,
            capabilities: request.capabilities,
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: Vec::new(),
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(quinn_conn)),
            opened_at: Utc::now(),
            closed_at: None,
        };

        Ok(ConnectionHandle { connection })
    }

    async fn accept(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let payload = payloads::session::SessionAccept {
            by: "part_local".into(),
            capabilities_answer: serde_json::Value::Object(Default::default()),
        };
        let env = UctpEnvelope::new(
            MessageType::SessionAccept,
            serde_json::to_value(payload).unwrap(),
        )
        .with_sid(route.sid);
        route
            .out_tx
            .send(env)
            .await
            .map_err(|_| RvoipError::Adapter("peer channel closed".into()))
    }

    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> RvoipResult<()> {
        let (code, reason_str) = reject_codes(&reason);
        self.terminate_route(
            &conn,
            |route| {
                let payload = payloads::session::SessionReject {
                    by: "part_local".into(),
                    reason_code: code,
                    reason: reason_str.into(),
                };
                UctpEnvelope::new(
                    MessageType::SessionReject,
                    serde_json::to_value(payload).expect("SessionReject is serializable"),
                )
                .with_sid(route.sid.clone())
            },
            AdapterEvent::Failed {
                connection_id: conn.clone(),
                detail: "session rejected locally".into(),
            },
        )
        .await;
        Ok(())
    }

    async fn end(&self, conn: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        let (code, reason_str) = end_codes(&reason);
        let terminal_reason = reason.clone();
        self.terminate_route(
            &conn,
            |route| {
                let payload = payloads::session::SessionEnd {
                    by: "part_local".into(),
                    reason_code: code,
                    reason: reason_str.into(),
                };
                UctpEnvelope::new(
                    MessageType::SessionEnd,
                    serde_json::to_value(payload).expect("SessionEnd is serializable"),
                )
                .with_sid(route.sid.clone())
            },
            AdapterEvent::Ended {
                connection_id: conn.clone(),
                reason: terminal_reason,
            },
        )
        .await;
        Ok(())
    }

    async fn hold(&self, _conn: ConnectionId) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented("rvoip-webtransport::hold"))
    }

    async fn resume(&self, _conn: ConnectionId) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented("rvoip-webtransport::resume"))
    }

    async fn transfer(&self, _conn: ConnectionId, _target: TransferTarget) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented("rvoip-webtransport::transfer"))
    }

    async fn streams(&self, conn: ConnectionId) -> RvoipResult<Vec<Arc<dyn MediaStream>>> {
        let route = self
            .route(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        Ok(route
            .streams
            .iter()
            .map(|entry| entry.value().clone())
            .collect())
    }

    async fn allocate_subscriber_stream(
        &self,
        subscriber: ConnectionId,
        kind: rvoip_core::stream::StreamKind,
        codec: rvoip_core::capability::CodecInfo,
    ) -> RvoipResult<Arc<dyn MediaStream>> {
        let route = self
            .route(&subscriber)
            .ok_or_else(|| RvoipError::ConnectionNotFound(subscriber.clone()))?;
        if route.route_cancel.is_cancelled() {
            return Err(RvoipError::InvalidState("UCTP route is ending"));
        }
        let wire_connection_id =
            rvoip_uctp::adapter_helpers::require_bound_wire_connection(&route.binding)?;

        let reservation = route
            .media_router
            .reserve()
            .map_err(peer_media_router_error)?;
        let local_id = reservation.local_id();
        let stream_id = StreamId::new();

        let stream = crate::media_stream::WebTransportDatagramMediaStream::start_with_cancel(
            stream_id.clone(),
            kind,
            codec.clone(),
            rvoip_core::connection::Direction::Outbound,
            local_id.get(),
            route.session.clone(),
            reservation.cancellation_token(),
        );
        if route.route_cancel.is_cancelled() {
            drop(reservation);
            let _ = stream.close().await;
            return Err(RvoipError::InvalidState(
                "UCTP route ended during allocation",
            ));
        }

        let stream_info = payloads::stream::StreamInfo {
            strm_id: stream_id.to_string(),
            kind: match kind {
                rvoip_core::stream::StreamKind::Audio => "audio".into(),
                rvoip_core::stream::StreamKind::Video => "video".into(),
                rvoip_core::stream::StreamKind::Data => "data".into(),
            },
            codec: serde_json::json!({
                "name": codec.name,
                "params": {
                    "sample_rate": codec.clock_rate_hz,
                    "channels": codec.channels,
                }
            }),
            direction: "recvonly".into(),
            stream_local_id: local_id.get(),
            opened_at: chrono::Utc::now(),
        };
        let opened_env = UctpEnvelope::new(
            MessageType::StreamOpened,
            serde_json::to_value(payloads::stream::StreamOpened {
                stream: stream_info,
            })
            .map_err(|e| RvoipError::Adapter(format!("encode stream.opened: {e}")))?,
        )
        .with_sid(route.sid.clone())
        .with_connid(wire_connection_id.to_string());

        let stream_dyn: Arc<dyn MediaStream> = Arc::clone(&stream) as Arc<dyn MediaStream>;
        let registration = rvoip_uctp::substrate::PeerMediaRegistration::new(
            route.binding.owner().clone(),
            rvoip_uctp::substrate::PeerMediaRouteKey::new(
                route.core_session_id.clone(),
                subscriber.clone(),
                stream_id.clone(),
            ),
            Arc::clone(&stream_dyn),
            stream.inbound_tx(),
        );
        let binding = match reservation.commit(registration) {
            Ok(binding) => binding,
            Err(error) => {
                let _ = stream.close().await;
                return Err(peer_media_router_error(error));
            }
        };
        route
            .streams
            .insert(stream_id.clone(), Arc::clone(&stream_dyn));
        if route.route_cancel.is_cancelled() {
            route.streams.remove(&stream_id);
            route.media_router.remove_local_id(binding.local_id());
            let _ = stream.close().await;
            return Err(RvoipError::InvalidState(
                "UCTP route ended during allocation",
            ));
        }

        if route.out_tx.try_send(opened_env).is_err() {
            route.streams.remove(&stream_id);
            route.media_router.remove_local_id(binding.local_id());
            let _ = stream.close().await;
            return Err(RvoipError::Adapter(
                "peer signaling channel closed or backpressured".into(),
            ));
        }

        Ok(stream_dyn)
    }

    async fn send_message(&self, conn: ConnectionId, message: Message) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let content_type_str = content_type_to_wire(&message.content_type);
        let data = DataMessage {
            label: "rvoip-messages".into(),
            content_type: content_type_str.into(),
            bytes: message.body.clone(),
            reliability: DataReliability::ReliableOrdered,
            message_id: message.id.clone(),
        };
        let mut payload = payloads::message::MessageSend::from_data_message(
            &data,
            message.from_participant.to_string(),
            serde_json::json!(["all"]),
        )
        .map_err(|error| RvoipError::Adapter(format!("invalid message: {error}")))?;
        payload.in_reply_to_msg = message.in_reply_to.map(|m| m.to_string());
        let env = UctpEnvelope::new(
            MessageType::MessageSend,
            serde_json::to_value(payload).unwrap(),
        )
        .with_cid(message.conversation_id.to_string())
        .with_sid(route.sid.clone())
        .with_connid(
            rvoip_uctp::adapter_helpers::require_bound_wire_connection(&route.binding)?.to_string(),
        );
        route
            .out_tx
            .send(env)
            .await
            .map_err(|_| RvoipError::Adapter("peer channel closed".into()))
    }

    async fn send_data_message(&self, conn: ConnectionId, message: DataMessage) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let wire_connection_id =
            rvoip_uctp::adapter_helpers::require_bound_wire_connection(&route.binding)?;
        rvoip_uctp::adapter_helpers::send_data_message_via_envelope(
            &route.out_tx,
            &route.sid,
            &wire_connection_id,
            &message,
        )
        .await
    }

    async fn send_dtmf(
        &self,
        conn: ConnectionId,
        digits: &str,
        duration_ms: u32,
    ) -> RvoipResult<()> {
        // Plan C2 — see `rvoip_quic::UctpQuicAdapter::send_dtmf` for
        // the wire semantics. Identical implementation; the substrate
        // difference doesn't show up at this layer.
        let route = self
            .route(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let payload = payloads::control::DtmfSend {
            digits: digits.into(),
            duration_ms,
            method: "rfc4733".into(),
        };
        let env = UctpEnvelope::new(
            MessageType::DtmfSend,
            serde_json::to_value(payload).unwrap(),
        )
        .with_sid(route.sid.clone())
        .with_connid(
            rvoip_uctp::adapter_helpers::require_bound_wire_connection(&route.binding)?.to_string(),
        );
        route
            .out_tx
            .send(env)
            .await
            .map_err(|_| RvoipError::Adapter("peer signaling channel closed".into()))
    }

    async fn renegotiate_media(
        &self,
        conn: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> RvoipResult<NegotiatedCodecs> {
        // Gap plan §4.2 v1 punch list — see rvoip-quic adapter for
        // the design (shared envelope helper, awaits peer reply via
        // `Pending`).
        let route = self
            .routes
            .get(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?
            .clone();
        let wire_connection_id =
            rvoip_uctp::adapter_helpers::require_bound_wire_connection(&route.binding)?;
        rvoip_uctp::adapter_helpers::renegotiate_via_envelope(
            &route.out_tx,
            &route.pending,
            &route.sid,
            &wire_connection_id,
            &capabilities,
            rvoip_uctp::adapter_helpers::DEFAULT_RENEGOTIATE_TIMEOUT,
        )
        .await
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        let mut guard = self.events_rx.lock().expect("poisoned");
        guard
            .take()
            .map(|events| legacy_normalized_event_receiver(events, ADAPTER_EVENT_CAP * 2))
            .unwrap_or_else(|| {
                warn!(
                "UctpWtAdapter::subscribe_events called more than once; returning closed channel"
            );
                let (_tx, rx) = mpsc::channel(1);
                rx
            })
    }

    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        let mut guard = self.events_rx.lock().expect("poisoned");
        guard.take().unwrap_or_else(|| {
            warn!("UctpWtAdapter atomic event stream already consumed");
            let (_tx, rx) = mpsc::channel(1);
            rx
        })
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }

    async fn verify_request_signature(
        &self,
        _conn: ConnectionId,
        _signature: SignatureHeaders,
    ) -> RvoipResult<IdentityAssurance> {
        Err(RvoipError::NotImplemented(
            "rvoip-webtransport::verify_request_signature",
        ))
    }
}

fn peer_media_router_error(error: rvoip_uctp::substrate::PeerMediaRouterError) -> RvoipError {
    match error {
        rvoip_uctp::substrate::PeerMediaRouterError::LocalIdExhausted => {
            RvoipError::AdmissionRejected("UCTP peer media stream-local-id namespace exhausted")
        }
        other => RvoipError::Adapter(format!("UCTP peer media route rejected: {other}")),
    }
}

fn content_type_to_wire(c: &rvoip_core::message::ContentType) -> &'static str {
    use rvoip_core::message::ContentType::*;
    match c {
        Text => "text/plain",
        Json => "application/json",
        Binary => "application/octet-stream",
        Image => "image/*",
        Audio => "audio/*",
        Attachment(_) => "application/octet-stream",
    }
}

fn reject_codes(r: &RejectReason) -> (u16, &'static str) {
    match r {
        RejectReason::Busy => (486, "busy"),
        RejectReason::Decline => (603, "decline"),
        RejectReason::NotFound => (404, "not-found"),
        RejectReason::Forbidden => (403, "forbidden"),
        RejectReason::NotAcceptable => (488, "not-acceptable"),
        RejectReason::ServerError => (500, "internal"),
        RejectReason::Custom { .. } => (500, "custom"),
    }
}

fn end_codes(r: &EndReason) -> (u16, &'static str) {
    match r {
        EndReason::Normal => (200, "normal-clearing"),
        EndReason::Cancelled => (487, "request-cancelled"),
        EndReason::Failed { .. } => (500, "session-failed"),
        EndReason::Timeout => (408, "timeout"),
        EndReason::BridgeTorn => (500, "bridge-torn"),
    }
}
