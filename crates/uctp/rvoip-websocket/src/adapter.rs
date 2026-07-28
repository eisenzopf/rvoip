//! `UctpWsAdapter` — implements `rvoip_core::ConnectionAdapter` over WebSocket.
//!
//! Mirrors `rvoip_quic::adapter::UctpQuicAdapter` line-for-line; only the
//! transport-level send/receive differs (text-frame JSON instead of
//! length-prefixed QUIC streams + datagrams). Media plane is **not** on
//! the WS — it lives in a co-located webrtc-rs PeerConnection that the
//! adapter manages via `crate::media_bridge` (filled in WS-D).

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
use rvoip_core::ids::ConnectionId;
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;
use rvoip_core::{DataMessage, DataReliability};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads;
use rvoip_uctp::types::MessageType;
use rvoip_uctp::CorrelationIdDiagnostic;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use url::Url;

use crate::server::UctpWsServer;

pub const ADAPTER_EVENT_CAP: usize = 256;

/// Per-Connection routing entry; populated by the server's event-pump on
/// `UctpSessionEvent::InboundInvite`. Adapter methods (`accept`, `end`,
/// `send_message`, …) look up the matching `out_tx` to dispatch envelopes
/// back to the peer.
///
/// Under the `media-webrtc` feature, `bridge` slot holds the per-Connection
/// answerer `WebRtcMediaBridge` (constructed asynchronously after
/// InboundInvite). Tests + downstream code can pull this handle via
/// [`UctpWsAdapter::bridge_for`] to drive the SDP exchange directly. Once
/// the bridge connects and exposes a `WebRtcMediaStream`, the server's
/// ready-watcher pushes the stream into `streams`, which makes
/// `Orchestrator::bridge_connections` resolve a real audio path.
///
/// `pending_offer` holds a `WebRtcSubstrateSetup` that arrived before the
/// answerer bridge finished constructing. The bridge-setup task drains
/// it as soon as the bridge is wired (gap plan §2.4 envelope SDP
/// interception).
#[derive(Clone)]
pub(crate) struct Route {
    /// Exact peer-selected Conversation ID from the authenticated invite.
    pub cid: Option<String>,
    pub sid: String,
    pub binding: rvoip_uctp::adapter_helpers::AuthenticatedConnectionBinding,
    pub out_tx: mpsc::Sender<UctpEnvelope>,
    /// Gap plan §4.2 v1 punch list — see rvoip-quic Route doc.
    pub pending: Arc<rvoip_uctp::substrate::Pending>,
    pub streams: Arc<DashMap<rvoip_core::ids::StreamId, Arc<dyn MediaStream>>>,
    pub coordinator: Arc<rvoip_uctp::state::UctpCoordinator>,
    pub route_cancel: CancellationToken,
    #[cfg(feature = "media-webrtc")]
    pub bridge: Arc<parking_lot::Mutex<Option<Arc<crate::media_bridge::WebRtcMediaBridge>>>>,
    #[cfg(feature = "media-webrtc")]
    pub pending_offer:
        Arc<parking_lot::Mutex<Option<rvoip_uctp::payloads::connection::WebRtcSubstrateSetup>>>,
}

pub struct UctpWsConfig {
    pub listener: TcpListener,
    pub bearer_validator: Arc<dyn BearerValidator>,
    pub max_concurrent_connections: usize,
    /// Optional client config for outbound `originate` dials.
    pub client_url: Option<Url>,
    /// Per-peer resource caps (plan D1 / D2). See
    /// `rvoip_quic::UctpQuicConfig::coordinator_caps`.
    pub coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
    /// Optional inline RFC 9421 envelope-signature enforcement. Disabled
    /// by default for compatibility; see
    /// [`rvoip_uctp::state::Sig9421Config`].
    pub sig9421: Option<rvoip_uctp::state::Sig9421Config>,
    /// Optional `rustls::ServerConfig` for TLS-terminating WSS. When
    /// `Some`, the accept loop wraps each `TcpStream` in
    /// `tokio_rustls::TlsAcceptor::accept(...)` before running the
    /// WebSocket handshake. When `None`, the plain `ws://` path runs
    /// unchanged. Only enabled under the `wss` feature.
    #[cfg(feature = "wss")]
    pub tls: Option<Arc<rustls::ServerConfig>>,
}

impl UctpWsConfig {
    pub fn new(listener: TcpListener, bearer_validator: Arc<dyn BearerValidator>) -> Self {
        Self {
            listener,
            bearer_validator,
            max_concurrent_connections: 1024,
            client_url: None,
            coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps::default(),
            sig9421: None,
            #[cfg(feature = "wss")]
            tls: None,
        }
    }

    pub fn with_outbound_url(mut self, url: Url) -> Self {
        self.client_url = Some(url);
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

    /// Enable TLS termination on the WS server socket using the given
    /// `rustls::ServerConfig`. The server's `accept` loop will wrap
    /// each TCP stream via `tokio_rustls::TlsAcceptor` before running
    /// the WebSocket handshake. See `crates/uctp/rvoip-uctp/src/substrate/tls.rs`
    /// for a `self_signed_for_dev` helper that produces a suitable
    /// config for dev/test.
    #[cfg(feature = "wss")]
    pub fn with_tls(mut self, tls: Arc<rustls::ServerConfig>) -> Self {
        self.tls = Some(tls);
        self
    }
}

pub struct UctpWsAdapter {
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
    _server: Arc<UctpWsServer>,
    events_rx: StdMutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    local_addr: SocketAddr,
    client_url: Option<Url>,
}

impl UctpWsAdapter {
    pub async fn new(config: UctpWsConfig) -> Result<Arc<Self>, crate::errors::UctpWsError> {
        let local_addr = config.listener.local_addr()?;
        let (events_tx, events_rx) = mpsc::channel(ADAPTER_EVENT_CAP);

        let by_connection: Arc<DashMap<ConnectionId, String>> = Arc::new(DashMap::new());
        let by_uctp_sid: Arc<DashMap<String, ConnectionId>> = Arc::new(DashMap::new());
        let routes: Arc<DashMap<ConnectionId, Route>> = Arc::new(DashMap::new());
        let lifecycle_sink = AdapterLifecycleSinkSlot::default();

        let server = UctpWsServer::start(
            config.listener,
            config.bearer_validator,
            events_tx.clone(),
            lifecycle_sink.clone(),
            Arc::clone(&by_connection),
            Arc::clone(&by_uctp_sid),
            Arc::clone(&routes),
            config.max_concurrent_connections,
            config.coordinator_caps,
            config.sig9421,
            #[cfg(feature = "wss")]
            config.tls,
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
            client_url: config.client_url,
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
        let streams = route
            .streams
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect::<Vec<_>>();
        route.streams.clear();
        for stream in streams {
            let _ = stream.close().await;
        }
        #[cfg(feature = "media-webrtc")]
        {
            route.pending_offer.lock().take();
            let bridge = route.bridge.lock().take();
            if let Some(bridge) = bridge {
                let _ = bridge.close().await;
            }
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
            warn!(connection_id = ?CorrelationIdDiagnostic::new(conn.as_str()), "UCTP WebSocket terminal notification was not queued");
        }
        route.route_cancel.cancel();
        route
            .coordinator
            .retire_local_session(&rvoip_core::ids::SessionId::from_string(route.sid.clone()));
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
            warn!(connection_id = ?CorrelationIdDiagnostic::new(conn.as_str()), "UCTP WebSocket terminal media cleanup timed out");
        }
    }

    /// Public accessor for the per-Connection `WebRtcMediaBridge` (answerer
    /// side). Returns `None` if the connection isn't known, or if the bridge
    /// is still being constructed (construction is async post-InboundInvite).
    ///
    /// Test + application code uses this handle to drive the
    /// `connection.offer` → `connection.answer` SDP exchange against a
    /// peer-side offerer bridge, then wait on `wait_connected` to confirm
    /// the WebRTC handshake landed.
    #[cfg(feature = "media-webrtc")]
    pub fn bridge_for(
        &self,
        conn: &ConnectionId,
    ) -> Option<Arc<crate::media_bridge::WebRtcMediaBridge>> {
        let route = self.routes.get(conn)?;
        let guard = route.bridge.lock();
        let cloned = guard.clone();
        cloned
    }

    /// Poll for the bridge slot to be populated; returns `None` on timeout.
    /// Useful when the bridge is created asynchronously after InboundInvite
    /// and the caller wants to await its existence.
    #[cfg(feature = "media-webrtc")]
    pub async fn wait_bridge_for(
        &self,
        conn: &ConnectionId,
        timeout: std::time::Duration,
    ) -> Option<Arc<crate::media_bridge::WebRtcMediaBridge>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(bridge) = self.bridge_for(conn) {
                return Some(bridge);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}

#[async_trait]
impl ConnectionAdapter for UctpWsAdapter {
    fn transport(&self) -> Transport {
        Transport::WebSocket
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
            RvoipError::InvalidState("UCTP WebSocket lifecycle sink already installed")
        })
    }

    fn is_connection_live(&self, conn: &ConnectionId) -> bool {
        self.routes.contains_key(conn)
    }

    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        let url = match &self.client_url {
            Some(u) => u.clone(),
            None => {
                return Err(RvoipError::NotImplemented(
                    "rvoip-websocket::originate (no client_url configured; use UctpWsConfig::with_outbound_url)",
                ));
            }
        };

        let client = crate::client::UctpWsClient::connect(&url)
            .await
            .map_err(|e| RvoipError::Adapter(format!("dial failed: {}", e)))?;

        // The transport_handle for a WS Connection wraps the client (the
        // signaling channel). Future hold/transfer code can downcast to
        // get back the live client.
        let connection = Connection {
            id: ConnectionId::new(),
            session_id: request.session_id,
            participant_id: request.participant_id,
            transport: Transport::WebSocket,
            direction: Direction::Outbound,
            state: ConnectionState::Connecting,
            capabilities: request.capabilities,
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: Vec::new(),
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(client)),
            opened_at: Utc::now(),
            closed_at: None,
        };

        Ok(ConnectionHandle::new(connection))
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
        Err(RvoipError::NotImplemented("rvoip-websocket::hold"))
    }

    async fn resume(&self, _conn: ConnectionId) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented("rvoip-websocket::resume"))
    }

    async fn transfer(&self, _conn: ConnectionId, _target: TransferTarget) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented("rvoip-websocket::transfer"))
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
        _subscriber: ConnectionId,
        _kind: rvoip_core::stream::StreamKind,
        _codec: rvoip_core::capability::CodecInfo,
    ) -> RvoipResult<Arc<dyn MediaStream>> {
        // Multi-party MP3c local_id rewriting on WS rides on the
        // `webrtc-rs` co-located PeerConnection (see media_bridge.rs)
        // — that integration is v0.x-deferred pending webrtc-rs 1.0.
        // Until then, fanout to WS subscribers falls back to the
        // legacy first-by-kind path in `Orchestrator::fanout_frame`,
        // which handles single-publisher rooms correctly. Multi-
        // publisher rooms with a WS subscriber will see the symptom
        // documented in the plan (B1 / G4) until webrtc-rs lands.
        Err(RvoipError::NotImplemented(
            "rvoip-websocket::allocate_subscriber_stream (pending webrtc-rs media plane)",
        ))
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
        let wire_conversation_id = route.cid.as_deref().ok_or_else(|| {
            RvoipError::Adapter("UCTP data route has no conversation binding".into())
        })?;
        rvoip_uctp::adapter_helpers::send_data_message_via_envelope(
            &route.out_tx,
            wire_conversation_id,
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
        // Plan C2. WS carries DTMF as a regular `dtmf.send` envelope
        // on the text-frame signaling channel; the co-located WebRTC
        // PeerConnection (once webrtc-rs lands per C3) handles the
        // actual RTP RFC 2833 events for media-side DTMF emission.
        // For now the wire-level signaling works end-to-end.
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
        // `Pending`). The optional `media-webrtc` SDP renegotiation
        // is application-driven for now (caller holds the bridge
        // handle); auto-driving it on a successful reply is a
        // follow-up.
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
                "UctpWsAdapter::subscribe_events called more than once; returning closed channel"
            );
                let (_tx, rx) = mpsc::channel(1);
                rx
            })
    }

    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        let mut guard = self.events_rx.lock().expect("poisoned");
        guard.take().unwrap_or_else(|| {
            warn!("UctpWsAdapter atomic event stream already consumed");
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
            "rvoip-websocket::verify_request_signature",
        ))
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
