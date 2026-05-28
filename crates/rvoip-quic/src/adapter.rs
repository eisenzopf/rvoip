//! `UctpQuicAdapter` — implements `rvoip_core::ConnectionAdapter` for raw QUIC.
//!
//! Mirrors the rvoip-sip `SipAdapter` shape: bidirectional id maps,
//! per-instance mpsc event channel taken once via `subscribe_events`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{
    Connection, ConnectionState, Direction, Transport, TransportHandle,
};
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::ConnectionId;
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;
use rvoip_auth_core::BearerValidator;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads;
use rvoip_uctp::types::MessageType;
use tokio::sync::mpsc;
use tracing::warn;

use crate::server::UctpQuicServer;

/// Default channel depth for `AdapterEvent`s consumed via
/// `subscribe_events`. Matches rvoip-sip's 256.
pub const ADAPTER_EVENT_CAP: usize = 256;

/// Routing entry shared between the server's event-pump and the
/// adapter's `ConnectionAdapter` method handlers. The server populates
/// `out_tx`, `sid`, `conn`, and the allocators at `InboundInvite` time;
/// adapter methods look up the matching entry to dispatch envelopes
/// back to the peer or to allocate per-subscriber MediaStreams.
#[derive(Clone)]
pub(crate) struct Route {
    pub sid: String,
    pub out_tx: mpsc::Sender<UctpEnvelope>,
    /// Gap plan §4.2 v1 punch list — coordinator's `Pending` map,
    /// so adapter methods that need a correlated reply (e.g.
    /// `renegotiate_media`) can `wait_for(env.id, timeout)` after
    /// sending. The dispatch gate in
    /// `UctpCoordinator::dispatch_inner` matches inbound envelopes'
    /// `in_reply_to` against this map.
    pub pending: Arc<rvoip_uctp::substrate::Pending>,
    pub streams: Arc<DashMap<rvoip_core::ids::StreamId, Arc<dyn MediaStream>>>,
    /// The underlying `quinn::Connection` for this peer. Plumbed in so
    /// the adapter's `allocate_subscriber_stream` (plan B1 / MP3c) can
    /// construct a fresh `QuicDatagramMediaStream` without re-asking
    /// the server task.
    pub conn: quinn::Connection,
    /// Next free `stream_local_id` on this connection. The default
    /// audio stream created at `InboundInvite` claims `1`; the
    /// allocator starts at `2`. Wraps to 1 on overflow — bounded by
    /// `u16::MAX` so practical sessions never approach the limit, but
    /// guarding against wrap is cheap.
    pub next_local_id: Arc<std::sync::atomic::AtomicU16>,
    /// Inbound routing table — the `spawn_datagram_reader` task on
    /// this connection consults it on every inbound datagram. Shared
    /// so the adapter can register newly-allocated subscriber-side
    /// streams here for round-trip support.
    pub streams_router:
        Arc<parking_lot::RwLock<Vec<Arc<crate::media_stream::QuicDatagramMediaStream>>>>,
}

pub struct UctpQuicConfig {
    pub endpoint: Arc<quinn::Endpoint>,
    /// ALPN-filtered stream of established connections from the dispatcher.
    pub accept_rx: mpsc::Receiver<quinn::Connection>,
    pub bearer_validator: Arc<dyn BearerValidator>,
    pub max_concurrent_connections: usize,
    /// Per-Connection `quinn::Connection::stats()` sampling cadence
    /// (5s default per design doc §3.9). Zero disables.
    pub quinn_stats_interval: std::time::Duration,
    /// Optional client-side endpoint used by `originate` for outbound dials.
    /// When `None`, `originate` returns `NotImplemented`.
    pub client_endpoint: Option<Arc<quinn::Endpoint>>,
    /// Optional `rustls::ClientConfig` paired with `client_endpoint`. Must
    /// include ALPN `b"uctp/1"` (the client wrapper adds it if empty).
    pub client_tls: Option<Arc<rustls::ClientConfig>>,
    /// Multi-party `SubscriptionHandler` (v0.x MP2/MP2.6). When `Some`,
    /// the coordinator is constructed via
    /// `UctpCoordinator::start_full(...)` so `stream.subscribe` /
    /// `stream.unsubscribe` envelopes route through the handler, and
    /// `stream.opened` emissions auto-register the publisher. When
    /// `None`, falls back to `start(...)` which uses
    /// [`rvoip_uctp::state::RejectingHandler`] (legacy 503 behavior).
    pub subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
    /// Orchestrator reference for multi-party media fanout (v0.x MP3b).
    /// When `Some`, the per-Connection datagram reader builds a
    /// `FanoutContext` and forwards every inbound frame to
    /// `Orchestrator::fanout_frame(...)` after the local route, so
    /// subscribers in this Session receive the publisher's media.
    /// **Cycle note**: the orchestrator holds the adapter Arc and the
    /// adapter holds this Arc back. Both share process lifetime in
    /// practice (no explicit shutdown); a future cleanup may swap to
    /// `Weak<Orchestrator>`.
    pub orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
    /// Per-peer resource caps (plan D1 / D2). Threaded into every
    /// `UctpCoordinator` constructed on this adapter so deployments
    /// can tune the session cap and signaling timeout without
    /// recompiling. Defaults are the safe values from
    /// `UctpCoordinatorCaps::default()`.
    pub coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
}

impl UctpQuicConfig {
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
            quinn_stats_interval: std::time::Duration::from_secs(5),
            client_endpoint: None,
            client_tls: None,
            subscription_handler: None,
            orchestrator: None,
            coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps::default(),
        }
    }

    /// Override the per-peer Session/timeout caps for this adapter's
    /// coordinator instances (plan D1 / D2). Most deployments should
    /// leave the defaults alone; embedded/mobile hosts or extreme N-party
    /// rooms can tune up.
    pub fn with_coordinator_caps(
        mut self,
        caps: rvoip_uctp::state::UctpCoordinatorCaps,
    ) -> Self {
        self.coordinator_caps = caps;
        self
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

    /// Opt in to multi-party routing by attaching a
    /// [`SubscriptionHandler`](rvoip_uctp::state::SubscriptionHandler).
    /// Typically constructed via
    /// `rvoip_uctp::state::OrchestratorSubscriptionHandler::new(orch,
    /// orch.publisher_registry())`.
    pub fn with_subscription_handler(
        mut self,
        handler: Arc<dyn rvoip_uctp::state::SubscriptionHandler>,
    ) -> Self {
        self.subscription_handler = Some(handler);
        self
    }

    /// Opt in to multi-party media fanout by attaching the Orchestrator
    /// reference (MP3b). Typically used together with
    /// [`with_subscription_handler`](Self::with_subscription_handler):
    /// the subscription handler accepts wire-level subscribes, and the
    /// orchestrator reference lets the datagram reader fan published
    /// frames out to each subscriber's MediaStream.
    pub fn with_orchestrator(mut self, orch: Arc<rvoip_core::Orchestrator>) -> Self {
        self.orchestrator = Some(orch);
        self
    }
}

pub struct UctpQuicAdapter {
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
    _server: Arc<UctpQuicServer>,
    events_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
    local_addr: SocketAddr,
    client_endpoint: Option<Arc<quinn::Endpoint>>,
    client_tls: Option<Arc<rustls::ClientConfig>>,
}

impl UctpQuicAdapter {
    /// Construct and spawn the server's accept loop.
    pub async fn new(config: UctpQuicConfig) -> Result<Arc<Self>, crate::errors::UctpQuicError> {
        let local_addr = config
            .endpoint
            .local_addr()
            .map_err(rvoip_uctp::errors::SubstrateError::Io)?;
        let (events_tx, events_rx) = mpsc::channel(ADAPTER_EVENT_CAP);

        let by_connection: Arc<DashMap<ConnectionId, String>> = Arc::new(DashMap::new());
        let by_uctp_sid: Arc<DashMap<String, ConnectionId>> = Arc::new(DashMap::new());
        let routes: Arc<DashMap<ConnectionId, Route>> = Arc::new(DashMap::new());

        let server = UctpQuicServer::start(
            config.accept_rx,
            config.bearer_validator,
            events_tx,
            Arc::clone(&by_connection),
            Arc::clone(&by_uctp_sid),
            Arc::clone(&routes),
            config.max_concurrent_connections,
            config.quinn_stats_interval,
            config.subscription_handler,
            config.orchestrator,
            config.coordinator_caps,
        );

        Ok(Arc::new(Self {
            by_connection,
            by_uctp_sid,
            routes,
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
}

#[async_trait]
impl ConnectionAdapter for UctpQuicAdapter {
    fn transport(&self) -> Transport {
        Transport::Quic
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Substrate
    }

    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        let (endpoint, tls) = match (&self.client_endpoint, &self.client_tls) {
            (Some(e), Some(t)) => (Arc::clone(e), Arc::clone(t)),
            _ => return Err(RvoipError::NotImplemented(
                "rvoip-quic::originate (no client_endpoint configured; use UctpQuicConfig::with_outbound)",
            )),
        };

        // Parse target as a SocketAddr. v0 demo dials by IP:port; URL
        // parsing for "uctp://" / "https://" + SAN-based server_name is a
        // straightforward follow-up.
        let server_addr: SocketAddr = request.target.parse().map_err(|_| {
            RvoipError::Adapter(format!("invalid originate target (expected ip:port): {}", request.target))
        })?;

        let client = crate::client::UctpQuicClient::connect(&endpoint, server_addr, "localhost", tls)
            .await
            .map_err(|e| RvoipError::Adapter(format!("dial failed: {}", e)))?;

        let connection = Connection {
            id: ConnectionId::new(),
            session_id: request.session_id,
            participant_id: request.participant_id,
            transport: Transport::Quic,
            direction: Direction::Outbound,
            state: ConnectionState::Connecting,
            capabilities: request.capabilities,
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: Vec::new(),
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(client.connection.clone())),
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
        let env = UctpEnvelope::new(MessageType::SessionAccept, serde_json::to_value(payload).unwrap())
            .with_sid(route.sid);
        route
            .out_tx
            .send(env)
            .await
            .map_err(|_| RvoipError::Adapter("peer channel closed".into()))
    }

    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let (code, reason_str) = reject_codes(&reason);
        let payload = payloads::session::SessionReject {
            by: "part_local".into(),
            reason_code: code,
            reason: reason_str.into(),
        };
        let env = UctpEnvelope::new(MessageType::SessionReject, serde_json::to_value(payload).unwrap())
            .with_sid(route.sid);
        route
            .out_tx
            .send(env)
            .await
            .map_err(|_| RvoipError::Adapter("peer channel closed".into()))
    }

    async fn end(&self, conn: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let (code, reason_str) = end_codes(&reason);
        let payload = payloads::session::SessionEnd {
            by: "part_local".into(),
            reason_code: code,
            reason: reason_str.into(),
        };
        let env = UctpEnvelope::new(MessageType::SessionEnd, serde_json::to_value(payload).unwrap())
            .with_sid(route.sid);
        route
            .out_tx
            .send(env)
            .await
            .map_err(|_| RvoipError::Adapter("peer channel closed".into()))
    }

    async fn hold(&self, _conn: ConnectionId) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented("rvoip-quic::hold"))
    }

    async fn resume(&self, _conn: ConnectionId) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented("rvoip-quic::resume"))
    }

    async fn transfer(&self, _conn: ConnectionId, _target: TransferTarget) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented("rvoip-quic::transfer"))
    }

    async fn streams(&self, conn: ConnectionId) -> RvoipResult<Vec<Arc<dyn MediaStream>>> {
        // Real but currently always empty: streams aren't created until
        // H3's datagram pump wires `stream.opened` envelopes through to
        // here. The map is populated by the server's event translator
        // once that lands; for now this returns the empty Vec for any
        // connection we know about, or ConnectionNotFound otherwise.
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

        // Allocate a fresh stream_local_id. Skip 1 (the default audio
        // stream's slot) and 0 (reserved). Wrap-around to 2 keeps the
        // u16 namespace fully usable but practical sessions never come
        // close.
        let local_id = loop {
            let next = route
                .next_local_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if next == 0 || next == 1 {
                continue;
            }
            break next;
        };

        // The stream is outbound from the server's perspective: the
        // server packs MediaFrames from the publisher and sends them on
        // this stream to the subscriber. Direction::Outbound makes the
        // outbound pump the active path.
        let stream = crate::media_stream::QuicDatagramMediaStream::start(
            rvoip_core::ids::StreamId::new(),
            kind,
            codec.clone(),
            rvoip_core::connection::Direction::Outbound,
            local_id,
            route.conn.clone(),
        );

        // Register in the per-Connection streams_router so inbound
        // datagrams the subscriber might send back on this local_id
        // route correctly. Without this, the subscriber could not
        // publish on the new stream (one-way fanout otherwise).
        route.streams_router.write().push(Arc::clone(&stream));

        // Also expose via `adapter.streams(connid)` for consumers that
        // enumerate streams (bridge_connections, ad-hoc routing).
        let stream_dyn: Arc<dyn MediaStream> = Arc::clone(&stream) as Arc<dyn MediaStream>;
        route.streams.insert(stream.id(), Arc::clone(&stream_dyn));

        // Announce the new stream to the peer via `stream.opened` per
        // CONVERSATION_PROTOCOL.md §7.4 / §10.1 multi-party note.
        // strm_id is a fresh wire-level identifier — the peer treats it
        // as opaque; what matters is the stream_local_id pairing.
        let strm_id = format!("strm_sub_{}", rvoip_core::ids::StreamId::new().to_string());
        let stream_info = payloads::stream::StreamInfo {
            strm_id,
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
            direction: "recvonly".into(), // server → subscriber
            stream_local_id: local_id,
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
        .with_connid(subscriber.to_string());
        route
            .out_tx
            .send(opened_env)
            .await
            .map_err(|_| RvoipError::Adapter("peer signaling channel closed".into()))?;

        Ok(stream_dyn)
    }

    async fn send_message(&self, conn: ConnectionId, message: Message) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let content_type_str = content_type_to_wire(&message.content_type);
        let payload = payloads::message::MessageSend {
            msg_id: message.id.to_string(),
            from: message.from_participant.to_string(),
            to: serde_json::json!(["all"]),
            content_type: content_type_str.into(),
            body: String::from_utf8_lossy(&message.body).to_string(),
            attachments: Vec::new(),
            in_reply_to_msg: message.in_reply_to.map(|m| m.to_string()),
        };
        let env = UctpEnvelope::new(MessageType::MessageSend, serde_json::to_value(payload).unwrap())
            .with_cid(message.conversation_id.to_string())
            .with_sid(route.sid);
        route
            .out_tx
            .send(env)
            .await
            .map_err(|_| RvoipError::Adapter("peer channel closed".into()))
    }

    async fn send_dtmf(
        &self,
        conn: ConnectionId,
        digits: &str,
        duration_ms: u32,
    ) -> RvoipResult<()> {
        // Plan C2: build a `dtmf.send` envelope and push it on the
        // peer's signaling channel. Default `method` is `"rfc4733"`
        // (RTP events) — the SIP/UCTP gateway transcodes if the peer's
        // negotiated mode is `"info"`.
        let route = self
            .route(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let payload = payloads::control::DtmfSend {
            digits: digits.into(),
            duration_ms,
            method: "rfc4733".into(),
        };
        let env = UctpEnvelope::new(MessageType::DtmfSend, serde_json::to_value(payload).unwrap())
            .with_sid(route.sid.clone())
            .with_connid(conn.to_string());
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
        // Gap plan §4.2 v1 punch list — drive a mid-call codec
        // renegotiation through the shared envelope helper. The
        // helper sends `connection.update` with
        // `action=renegotiate-media`, awaits the peer's correlated
        // reply via `Pending`, parses the chosen codec (or maps
        // `error 488` to AdmissionRejected).
        let route = self
            .routes
            .get(&conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?
            .clone();
        rvoip_uctp::adapter_helpers::renegotiate_via_envelope(
            &route.out_tx,
            &route.pending,
            &route.sid,
            &conn,
            &capabilities,
            rvoip_uctp::adapter_helpers::DEFAULT_RENEGOTIATE_TIMEOUT,
        )
        .await
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        let mut guard = self.events_rx.lock().expect("poisoned");
        guard.take().unwrap_or_else(|| {
            warn!("UctpQuicAdapter::subscribe_events called more than once; returning closed channel");
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
            "rvoip-quic::verify_request_signature",
        ))
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
        RejectReason::Custom { code: _, phrase: _ } => (500, "custom"),
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

fn end_codes(r: &EndReason) -> (u16, &'static str) {
    match r {
        EndReason::Normal => (200, "normal-clearing"),
        EndReason::Cancelled => (487, "request-cancelled"),
        EndReason::Failed { .. } => (500, "session-failed"),
        EndReason::Timeout => (408, "timeout"),
        EndReason::BridgeTorn => (500, "bridge-torn"),
    }
}
