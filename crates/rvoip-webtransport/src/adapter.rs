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
use url::Url;

use crate::server::UctpWtServer;

pub const ADAPTER_EVENT_CAP: usize = 256;

#[derive(Clone)]
pub(crate) struct Route {
    pub sid: String,
    pub out_tx: mpsc::Sender<UctpEnvelope>,
    pub streams: Arc<DashMap<rvoip_core::ids::StreamId, Arc<dyn MediaStream>>>,
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
    /// [`UctpQuicConfig`](rvoip_quic::UctpQuicConfig::subscription_handler)
    /// for semantics — identical wiring.
    pub subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
    /// Orchestrator reference for multi-party media fanout (v0.x MP3b).
    /// See [`UctpQuicConfig`](rvoip_quic::UctpQuicConfig::orchestrator).
    pub orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
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
            orchestrator: None,
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

    pub fn with_orchestrator(mut self, orch: Arc<rvoip_core::Orchestrator>) -> Self {
        self.orchestrator = Some(orch);
        self
    }
}

pub struct UctpWtAdapter {
    by_connection: Arc<DashMap<ConnectionId, String>>,
    by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
    routes: Arc<DashMap<ConnectionId, Route>>,
    _server: Arc<UctpWtServer>,
    events_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
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

        let server = UctpWtServer::start(
            config.accept_rx,
            config.bearer_validator,
            events_tx,
            Arc::clone(&by_connection),
            Arc::clone(&by_uctp_sid),
            Arc::clone(&routes),
            config.max_concurrent_connections,
            config.mount_path,
            config.quinn_stats_interval,
            config.subscription_handler,
            config.orchestrator,
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
impl ConnectionAdapter for UctpWtAdapter {
    fn transport(&self) -> Transport {
        Transport::WebTransport
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Substrate
    }

    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        let (endpoint, tls) = match (&self.client_endpoint, &self.client_tls) {
            (Some(e), Some(t)) => (Arc::clone(e), Arc::clone(t)),
            _ => return Err(RvoipError::NotImplemented(
                "rvoip-webtransport::originate (no client_endpoint configured; use UctpWtConfig::with_outbound)",
            )),
        };

        // For WT, `target` is expected to be a full https:// URL.
        let url = Url::parse(&request.target).map_err(|e| {
            RvoipError::Adapter(format!("invalid originate target URL: {}", e))
        })?;
        let host = url
            .host_str()
            .ok_or_else(|| RvoipError::Adapter("URL has no host".into()))?;
        let port = url.port().unwrap_or(443);
        let addr: SocketAddr = format!("{}:{}", host, port)
            .parse()
            .map_err(|_| RvoipError::Adapter(format!("can't resolve {}:{} to SocketAddr (use ip address for v0)", host, port)))?;

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

    async fn send_dtmf(&self, _conn: ConnectionId, _digits: &str, _dur: u32) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented("rvoip-webtransport::send_dtmf"))
    }

    async fn renegotiate_media(
        &self,
        _conn: ConnectionId,
        _capabilities: CapabilityDescriptor,
    ) -> RvoipResult<NegotiatedCodecs> {
        Err(RvoipError::NotImplemented(
            "rvoip-webtransport::renegotiate_media",
        ))
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        let mut guard = self.events_rx.lock().expect("poisoned");
        guard.take().unwrap_or_else(|| {
            warn!("UctpWtAdapter::subscribe_events called more than once; returning closed channel");
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
