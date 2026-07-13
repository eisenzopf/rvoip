use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::error::{Error, Result};
use bytes::Bytes;
use rvoip_sip_core::{types::uri::Host, HeaderName, Message, Method, Request, TypedHeader};

mod runtime;
pub mod tcp;
pub mod tls;
pub mod udp;
pub mod ws;

pub use runtime::HandshakeAdmissionConfig;
pub use tcp::TcpTransport;
pub use tls::TlsTransport;
pub use udp::{UdpParseConfig, UdpParseDispatch, UdpSocketOptions, UdpTransport};
pub use ws::WebSocketTransport;

/// Enforce credential-header wire safety at a typed transport boundary.
///
/// Raw/verbatim send APIs deliberately do not call this helper: they are the
/// explicit escape hatch for already-serialized proxy traffic. Errors contain
/// no credential value.
pub(crate) fn validate_typed_outbound_message(message: &Message) -> Result<()> {
    rvoip_sip_core::validation::validate_typed_outbound_message(message).map_err(|_| {
        Error::ProtocolError("outbound typed SIP message failed wire-safety validation".to_string())
    })
}

/// Return a log-safe method label without reflecting extension method text.
pub(crate) fn safe_method_label(method: &Method) -> &'static str {
    match method {
        Method::Invite => "INVITE",
        Method::Ack => "ACK",
        Method::Bye => "BYE",
        Method::Cancel => "CANCEL",
        Method::Register => "REGISTER",
        Method::Options => "OPTIONS",
        Method::Subscribe => "SUBSCRIBE",
        Method::Notify => "NOTIFY",
        Method::Update => "UPDATE",
        Method::Refer => "REFER",
        Method::Info => "INFO",
        Method::Message => "MESSAGE",
        Method::Prack => "PRACK",
        Method::Publish => "PUBLISH",
        Method::Extension(_) => "extension",
    }
}

/// Represents the transport type/protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportType {
    Udp,
    Tcp,
    Tls,
    Ws,
    Wss,
}

/// Opaque process-local identity for one established connection-oriented flow.
///
/// Socket addresses are not sufficient identities once a transport can hold
/// multiple authenticated authorities or directions at the same address. The
/// value is deliberately opaque: callers retain and return it to the transport
/// when replying, but must not derive tenant or authorization policy from it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportFlowId(u64);

impl TransportFlowId {
    /// Stable numeric value for low-level diagnostics and packet-capture
    /// correlation. It is process-local and must not be persisted as a route.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[cfg(test)]
    pub(crate) const fn for_test(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Debug for TransportFlowId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TransportFlowId")
            .field(&self.0)
            .finish()
    }
}

static NEXT_TRANSPORT_FLOW_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_transport_flow_id() -> TransportFlowId {
    TransportFlowId(NEXT_TRANSPORT_FLOW_ID.fetch_add(1, Ordering::Relaxed))
}

/// Host identity authenticated for an outbound connection.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TransportAuthority {
    /// Normalized DNS authority (lowercase, without a terminal dot).
    Dns(String),
    /// Literal IP authority.
    Ip(IpAddr),
}

impl TransportAuthority {
    /// Construct and normalize a DNS authority.
    pub fn dns(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let normalized = value.trim_end_matches('.').to_ascii_lowercase();
        if normalized.is_empty() || normalized.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(Error::InvalidAddress(
                "invalid empty or whitespace authority".into(),
            ));
        }
        Ok(Self::Dns(normalized))
    }

    /// Construct an IP-literal authority.
    pub const fn ip(value: IpAddr) -> Self {
        Self::Ip(value)
    }
}

/// Determine the authenticated authority for a typed SIP request: the top
/// Route URI when present, otherwise the Request-URI.
pub fn transport_authority_for_request(request: &Request) -> Result<TransportAuthority> {
    let route_host = request.headers.iter().find_map(|header| {
        if header.name() != HeaderName::Route {
            return None;
        }
        match header {
            TypedHeader::Route(route) => route.first().map(|entry| entry.0.uri.host.clone()),
            _ => None,
        }
    });
    match route_host.as_ref().unwrap_or(&request.uri().host) {
        Host::Domain(domain) => TransportAuthority::dns(domain.clone()),
        Host::Address(address) => Ok(TransportAuthority::ip(*address)),
    }
}

/// Complete routing identity for one transport send.
///
/// Requests carry the selected next-hop authority. Responses and lifecycle
/// traffic additionally carry the exact ingress flow. `transport_type` lets a
/// multiplexer select the same concrete transport without probing by address.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TransportRoute {
    pub destination: SocketAddr,
    pub transport_type: Option<TransportType>,
    pub authority: Option<TransportAuthority>,
    pub flow_id: Option<TransportFlowId>,
}

impl TransportRoute {
    pub const fn new(destination: SocketAddr) -> Self {
        Self {
            destination,
            transport_type: None,
            authority: None,
            flow_id: None,
        }
    }

    pub const fn with_transport_type(mut self, transport_type: TransportType) -> Self {
        self.transport_type = Some(transport_type);
        self
    }

    pub fn with_authority(mut self, authority: TransportAuthority) -> Self {
        self.authority = Some(authority);
        self
    }

    pub const fn with_flow_id(mut self, flow_id: TransportFlowId) -> Self {
        self.flow_id = Some(flow_id);
        self
    }
}

impl fmt::Display for TransportType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportType::Udp => write!(f, "UDP"),
            TransportType::Tcp => write!(f, "TCP"),
            TransportType::Tls => write!(f, "TLS"),
            TransportType::Ws => write!(f, "WS"),
            TransportType::Wss => write!(f, "WSS"),
        }
    }
}

/// Optional receive-side timing stamps carried with an inbound message.
///
/// These timestamps are only populated by transports when diagnostics are
/// enabled. Keeping the field optional lets production paths avoid the extra
/// `Instant::now()` calls and downstream atomic accounting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransportReceiveTiming {
    /// UDP datagram read completed in the socket task.
    pub received_at: Option<Instant>,
    /// Parse worker dequeued the datagram.
    pub parse_worker_dequeued_at: Option<Instant>,
    /// SIP parsing completed successfully.
    pub parse_completed_at: Option<Instant>,
    /// Transport manager forwarded the event to the transaction manager queue.
    pub transport_manager_forwarded_at: Option<Instant>,
    /// Transaction manager received the event from its queue.
    pub transaction_manager_received_at: Option<Instant>,
}

/// Identity of a TLS peer whose certificate chain was accepted by rustls.
///
/// This value is only constructed on inbound TLS/WSS server connections
/// after `WebPkiClientVerifier` succeeds. It intentionally exposes a stable
/// certificate fingerprint rather than retaining the potentially large DER
/// chain or parsing application-specific subject names at the transport
/// boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct TlsPeerIdentity {
    /// Lowercase hexadecimal SHA-256 digest of the presented leaf
    /// certificate's DER encoding.
    pub leaf_certificate_sha256: String,
    /// Number of certificates presented by the peer, including the leaf.
    pub presented_chain_len: usize,
}

impl fmt::Debug for TlsPeerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TlsPeerIdentity")
            .field(
                "leaf_fingerprint_present",
                &!self.leaf_certificate_sha256.is_empty(),
            )
            .field(
                "leaf_fingerprint_bytes",
                &self.leaf_certificate_sha256.len(),
            )
            .field("presented_chain_len", &self.presented_chain_len)
            .finish()
    }
}

/// Connection-scoped metadata attached to every SIP message received on the
/// corresponding transport connection.
#[derive(Clone, PartialEq, Eq)]
pub struct TransportConnectionMetadata {
    /// Verified mutual-TLS client identity. Present only when a TLS or WSS
    /// client supplied a chain accepted by the configured client verifier.
    pub tls_peer_identity: TlsPeerIdentity,
}

impl fmt::Debug for TransportConnectionMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransportConnectionMetadata")
            .field("tls_peer_identity", &self.tls_peer_identity)
            .finish()
    }
}

/// Events emitted by a transport
#[derive(Clone)]
pub enum TransportEvent {
    /// A SIP message was received
    MessageReceived {
        /// The SIP message
        message: Message,
        /// The remote address that sent the message
        source: SocketAddr,
        /// The local address that received the message
        destination: SocketAddr,
        /// Transport flavour that received the message.
        transport_type: TransportType,
        /// Exact connection-oriented flow that carried this message. `None`
        /// for connectionless or legacy transports.
        flow_id: Option<TransportFlowId>,
        /// Original wire bytes the parser consumed, when available.
        ///
        /// Carried through the bus end-to-end so byte-exact consumers
        /// (STIR/SHAKEN Identity verification per RFC 8224,
        /// signature-preserving SBCs, stateless proxies, fuzz harnesses,
        /// replay tools) can recover the upstream form without
        /// re-serializing the parsed `Message`. `None` for synthetic
        /// events that have no real wire bytes (mock transports,
        /// internally-fabricated messages).
        ///
        /// Plain `Bytes` (not `Arc<Bytes>`) — `Bytes` is already
        /// internally Arc-managed; the outer `Arc` was a per-packet
        /// heap allocation with no functional benefit.
        raw_bytes: Option<Bytes>,
        /// Optional receive timing diagnostics for UDP fast-path analysis.
        timing: Option<TransportReceiveTiming>,
        /// Verified connection identity, when the inbound TLS/WSS peer
        /// presented a client certificate. Plain transports and compatible
        /// server-only TLS without client authentication leave this `None`.
        connection_metadata: Option<TransportConnectionMetadata>,
    },

    /// Error occurred in the transport
    Error {
        /// Error description
        error: String,
    },

    /// Transport has been closed
    Closed,

    /// RFC 5626 §3.5.1 keep-alive pong (single CRLF) received from peer.
    /// Emitted by connection-oriented transports (TCP/TLS) when a bare
    /// `\r\n` arrives at the start of a receive buffer. The bytes are
    /// consumed by the transport layer and never handed to the SIP
    /// parser.
    KeepAlivePongReceived {
        /// The remote address that sent the pong
        source: SocketAddr,
        /// The local address that received the pong
        destination: SocketAddr,
        /// Exact flow that carried the pong.
        flow_id: Option<TransportFlowId>,
    },

    /// A connection-oriented transport lost a live connection to `remote_addr`.
    /// Emitted on EOF or error on the read side, before the per-remote
    /// entry is removed from any connection pool / registry, so observers
    /// can correlate the drop with in-flight flow state (e.g., RFC 5626
    /// OutboundFlow).
    ConnectionClosed {
        /// The remote address whose connection was lost
        remote_addr: SocketAddr,
        /// The transport type that owned the dropped connection
        transport_type: TransportType,
        /// Exact flow that closed.
        flow_id: Option<TransportFlowId>,
    },

    // ========== GRACEFUL SHUTDOWN EVENTS ==========
    /// Shutdown request received from transaction layer
    ShutdownRequested,

    /// Transport is ready for shutdown
    ShutdownReady,

    /// Transport should shutdown now
    ShutdownNow,

    /// Transport shutdown complete
    ShutdownComplete,
}

/// Whether an event belongs on the reserved lifecycle/control lane rather
/// than the bounded SIP message lane.
pub const fn is_control_event(event: &TransportEvent) -> bool {
    matches!(
        event,
        TransportEvent::Closed
            | TransportEvent::KeepAlivePongReceived { .. }
            | TransportEvent::ConnectionClosed { .. }
            | TransportEvent::ShutdownRequested
            | TransportEvent::ShutdownReady
            | TransportEvent::ShutdownNow
            | TransportEvent::ShutdownComplete
    )
}

/// Emit lifecycle state without allowing a saturated consumer to pin a socket
/// reader or teardown task indefinitely. Each production transport receives a
/// separately reserved sender; compatibility constructors may point it at the
/// ordinary event sender.
pub(crate) async fn send_control_event(
    sender: &mpsc::Sender<TransportEvent>,
    event: TransportEvent,
) -> bool {
    match sender.try_send(event) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(event)) => {
            matches!(
                tokio::time::timeout(Duration::from_millis(100), sender.send(event)).await,
                Ok(Ok(()))
            )
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

impl fmt::Debug for TransportEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MessageReceived {
                message,
                source,
                destination,
                transport_type,
                flow_id,
                raw_bytes,
                timing,
                connection_metadata,
            } => {
                let mut debug = formatter.debug_struct("MessageReceived");
                match message {
                    Message::Request(request) => {
                        debug
                            .field("message_kind", &"request")
                            .field("method", &safe_method_label(&request.method))
                            .field("header_count", &request.headers.len())
                            .field("body_len", &request.body.len());
                    }
                    Message::Response(response) => {
                        debug
                            .field("message_kind", &"response")
                            .field("status_code", &response.status_code())
                            .field("header_count", &response.headers.len())
                            .field("body_len", &response.body.len());
                    }
                }
                debug
                    .field("source", source)
                    .field("destination", destination)
                    .field("transport_type", transport_type)
                    .field("flow_id", flow_id)
                    .field("raw_bytes_present", &raw_bytes.is_some())
                    .field("raw_bytes_len", &raw_bytes.as_ref().map(Bytes::len))
                    .field("timing", timing)
                    .field("connection_metadata", connection_metadata)
                    .finish()
            }
            Self::Error { error } => formatter
                .debug_struct("Error")
                .field("error_present", &!error.is_empty())
                .field("error_len", &error.len())
                .field("error_class", &"transport-reported")
                .finish(),
            Self::Closed => formatter.write_str("Closed"),
            Self::KeepAlivePongReceived {
                source,
                destination,
                flow_id,
            } => formatter
                .debug_struct("KeepAlivePongReceived")
                .field("source", source)
                .field("destination", destination)
                .field("flow_id", flow_id)
                .finish(),
            Self::ConnectionClosed {
                remote_addr,
                transport_type,
                flow_id,
            } => formatter
                .debug_struct("ConnectionClosed")
                .field("remote_addr", remote_addr)
                .field("transport_type", transport_type)
                .field("flow_id", flow_id)
                .finish(),
            Self::ShutdownRequested => formatter.write_str("ShutdownRequested"),
            Self::ShutdownReady => formatter.write_str("ShutdownReady"),
            Self::ShutdownNow => formatter.write_str("ShutdownNow"),
            Self::ShutdownComplete => formatter.write_str("ShutdownComplete"),
        }
    }
}

#[cfg(test)]
mod diagnostic_safety_tests {
    use super::*;

    #[test]
    fn transport_error_debug_reports_only_fixed_metadata() {
        const SECRET: &str = "resolver failed for secret.example; Authorization: Bearer token";
        let event = TransportEvent::Error {
            error: SECRET.to_string(),
        };

        let rendered = format!("{event:?}");
        assert!(!rendered.contains(SECRET));
        assert!(!rendered.contains("secret.example"));
        assert!(!rendered.contains("Bearer token"));
        assert!(rendered.contains("error_present: true"));
        assert!(rendered.contains(&format!("error_len: {}", SECRET.len())));

        let TransportEvent::Error { error } = event else {
            panic!("expected error event");
        };
        assert_eq!(error, SECRET, "functional error payload must be retained");
    }

    #[test]
    fn diagnostic_errors_do_not_consume_reserved_lifecycle_capacity() {
        assert!(!is_control_event(&TransportEvent::Error {
            error: "malformed datagram".into(),
        }));
        assert!(is_control_event(&TransportEvent::Closed));
        assert!(is_control_event(&TransportEvent::ConnectionClosed {
            remote_addr: "127.0.0.1:5060".parse().unwrap(),
            transport_type: TransportType::Tcp,
            flow_id: None,
        }));
        assert!(is_control_event(&TransportEvent::ShutdownNow));
    }
}

/// Represents a transport layer for SIP messages.
///
/// This trait defines the common interface for all transport types (UDP, TCP, TLS, WebSocket).
#[async_trait::async_trait]
pub trait Transport: Send + Sync + fmt::Debug {
    /// Returns the local address this transport is bound to
    fn local_addr(&self) -> Result<SocketAddr>;

    /// Sends a SIP message to the specified destination
    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()>;

    /// Send a structured SIP message using an authority- and flow-aware route.
    ///
    /// Connection-oriented secure transports override this method. The default
    /// preserves compatibility for connectionless transports while refusing to
    /// silently discard an exact flow requirement.
    async fn send_message_via(&self, message: Message, route: TransportRoute) -> Result<()> {
        if route.flow_id.is_some() {
            return Err(Error::InvalidState(
                "transport does not support exact flow routing".into(),
            ));
        }
        self.send_message(message, route.destination).await
    }

    /// Send a message and return the concrete route that carried it.
    /// Connection-oriented implementations bind the returned route to the
    /// selected opaque flow before exposing the send as successful. Client
    /// transactions retain that route so a queued response remains verifiable
    /// even when the peer closes before the event consumer runs.
    async fn send_message_on_route(
        &self,
        message: Message,
        mut route: TransportRoute,
    ) -> Result<TransportRoute> {
        self.send_message_via(message, route.clone()).await?;
        if route.flow_id.is_none() {
            route.flow_id = self.resolve_flow_id_for_route(&route).await;
        }
        Ok(route)
    }

    /// Prepare and bind a route before any SIP bytes are written.
    ///
    /// Client transactions use this two-phase hook to retain an exact stream
    /// identity before a peer can respond and immediately close. The default
    /// is sufficient for connectionless transports.
    async fn prepare_message_route(
        &self,
        _message: &Message,
        route: TransportRoute,
    ) -> Result<TransportRoute> {
        Ok(route)
    }

    /// Resolve the established flow currently represented by an outbound
    /// route. Used to bind later lifecycle operations to the same connection.
    fn flow_id_for_route(&self, _route: &TransportRoute) -> Option<TransportFlowId> {
        None
    }

    /// Reliably resolve an established flow for security-sensitive routing.
    ///
    /// The synchronous compatibility probe may conservatively return `None`
    /// while an internal registry lock is busy. Transaction validation must
    /// use this awaited variant so transient contention cannot reject a valid
    /// authenticated response.
    async fn resolve_flow_id_for_route(&self, route: &TransportRoute) -> Option<TransportFlowId> {
        self.flow_id_for_route(route)
    }

    /// Closes the transport
    async fn close(&self) -> Result<()>;

    /// Checks if the transport is closed
    fn is_closed(&self) -> bool;

    /// Check if UDP transport is supported
    fn supports_udp(&self) -> bool {
        // Default implementation - UDP is commonly supported
        true
    }

    /// Check if TCP transport is supported
    fn supports_tcp(&self) -> bool {
        // Default implementation
        false
    }

    /// Check if TLS transport is supported
    fn supports_tls(&self) -> bool {
        // Default implementation
        false
    }

    /// Check if WebSocket transport is supported
    fn supports_ws(&self) -> bool {
        // Default implementation
        false
    }

    /// Check if Secure WebSocket transport is supported
    fn supports_wss(&self) -> bool {
        // Default implementation
        false
    }

    /// Check if a specific transport type is supported
    fn supports_transport(&self, transport_type: TransportType) -> bool {
        match transport_type {
            TransportType::Udp => self.supports_udp(),
            TransportType::Tcp => self.supports_tcp(),
            TransportType::Tls => self.supports_tls(),
            TransportType::Ws => self.supports_ws(),
            TransportType::Wss => self.supports_wss(),
        }
    }

    /// Get the default transport type
    fn default_transport_type(&self) -> TransportType {
        // Most implementations default to UDP
        TransportType::Udp
    }

    /// Largest serialized SIP message size this transport can safely
    /// ship in a single send.
    ///
    /// Per RFC 3261 §18.1.1, datagram transports (UDP) MUST switch to
    /// a congestion-controlled transport once a request exceeds 1300
    /// bytes (or comes within 200 bytes of path MTU). The dialog
    /// layer's transport multiplexer consults this method to decide
    /// when to auto-failover UDP → TCP.
    ///
    /// Stream-oriented transports (TCP/TLS/WS/WSS) take the default
    /// `usize::MAX` — they are not byte-bounded at this layer.
    fn max_safe_message_size(&self) -> usize {
        usize::MAX
    }

    /// Check if a specific transport is currently connected
    fn is_transport_connected(&self, transport_type: TransportType) -> bool {
        // For UDP, always considered connected
        // For connection-oriented transports, this would check connection status
        if transport_type == TransportType::Udp {
            true
        } else {
            !self.is_closed()
        }
    }

    /// Get the number of active connections for a transport type
    fn get_connection_count(&self, _transport_type: TransportType) -> usize {
        // Default implementation
        if self.is_closed() {
            0
        } else {
            1
        }
    }

    /// Whether this transport currently has a live connection to the
    /// given remote address. Used by URI-aware multiplexers to route
    /// outbound *responses* back through the connection-oriented
    /// transport that originally received the request (RFC 3261
    /// §17.2 / §18.2.2).
    ///
    /// The default `false` is correct for connectionless transports
    /// (UDP) and conservative for connection-oriented transports that
    /// haven't yet implemented the lookup — they'll just be skipped
    /// and the multiplexer will try the next candidate.
    fn has_connection_to(&self, _remote_addr: SocketAddr) -> bool {
        false
    }

    /// Sends raw bytes over an existing connection to `destination`.
    /// Used for RFC 5626 §3.5.1 CRLFCRLF keep-alive pings — the bytes
    /// are written verbatim without any SIP framing. Connection-oriented
    /// transports (TCP, TLS) must override. UDP has no connection to
    /// keep alive this way (RFC 5626 UDP path uses STUN — out of scope
    /// here) and returns `NotImplemented`.
    async fn send_raw(&self, _destination: SocketAddr, _data: Bytes) -> Result<()> {
        Err(crate::error::Error::NotImplemented(
            "send_raw is not supported on this transport".to_string(),
        ))
    }

    /// Send non-SIP flow bytes on an exact route.
    async fn send_raw_via(&self, route: TransportRoute, data: Bytes) -> Result<()> {
        if route.flow_id.is_some() {
            return Err(Error::InvalidState(
                "transport does not support exact raw flow routing".into(),
            ));
        }
        self.send_raw(route.destination, data).await
    }

    /// Send pre-built SIP-formatted bytes verbatim to `destination`.
    ///
    /// Unlike [`Transport::send_raw`] (RFC 5626 §3.5.1 keep-alive
    /// pings on already-open connection-oriented transports), this
    /// method works for any transport and may open new connections as
    /// needed. The bytes MUST form a valid SIP message per RFC 3261
    /// §25 — no validation is performed at this layer. Use cases:
    /// signature-preserving SBC pass-through (RFC 8224 STIR/SHAKEN
    /// Identity header is canonicalised by the upstream signer; any
    /// re-serialisation would invalidate the signature), stateless
    /// proxy forwarding, fuzz harnesses, and replay tooling.
    ///
    /// The default returns `NotImplemented` so each transport opts in
    /// explicitly.
    async fn send_message_raw(&self, _bytes: Bytes, _destination: SocketAddr) -> Result<()> {
        Err(crate::error::Error::NotImplemented(
            "send_message_raw is not supported on this transport".to_string(),
        ))
    }

    /// Send pre-built SIP bytes using an authority- and flow-aware route.
    async fn send_message_raw_via(&self, bytes: Bytes, route: TransportRoute) -> Result<()> {
        if route.flow_id.is_some() {
            return Err(Error::InvalidState(
                "transport does not support exact raw SIP flow routing".into(),
            ));
        }
        self.send_message_raw(bytes, route.destination).await
    }

    /// Forward a serialized SIP message verbatim while pushing or
    /// popping the top `Via` header in-place at the byte level.
    ///
    /// Designed for stateless-proxy forwarders that need byte-exact
    /// preservation of the RFC 8224 `Identity` header (re-serializing
    /// the structured message would canonicalise the JWT-bearing
    /// header and invalidate the JWS signature). Only the top Via
    /// line is rewritten; every other byte — Identity, body, all
    /// remaining headers — flows through untouched.
    ///
    /// Direction conventions:
    /// - `ViaRewrite::Push(line)` — request forwarding: insert the
    ///   caller-supplied Via line (must include trailing `\r\n`) at
    ///   the top of the Via stack, in front of the existing top
    ///   entry (RFC 3261 §16.6 step 8).
    /// - `ViaRewrite::Pop` — response forwarding: remove the existing
    ///   top Via line (RFC 3261 §16.7 step 3).
    ///
    /// Errors:
    /// - `Error::ProtocolError` when the message has no Via header to
    ///   rewrite (push needs an anchor; pop needs something to remove).
    /// - Whatever `send_message_raw` returns on transport failure.
    ///
    /// The default implementation rewrites the bytes here and then
    /// delegates to `send_message_raw`. Transports do not normally
    /// need to override.
    async fn forward_raw_with_via_rewrite(
        &self,
        bytes: Bytes,
        rewrite: ViaRewrite,
        destination: SocketAddr,
    ) -> Result<()> {
        let rewritten = apply_via_rewrite(bytes, rewrite)?;
        self.send_message_raw(rewritten, destination).await
    }

    /// Flow-aware variant of [`Transport::forward_raw_with_via_rewrite`].
    async fn forward_raw_with_via_rewrite_via(
        &self,
        bytes: Bytes,
        rewrite: ViaRewrite,
        route: TransportRoute,
    ) -> Result<()> {
        let rewritten = apply_via_rewrite(bytes, rewrite)?;
        self.send_message_raw_via(rewritten, route).await
    }
}

/// Direction-specific Via-stack edit for
/// [`Transport::forward_raw_with_via_rewrite`].
#[derive(Clone)]
pub enum ViaRewrite {
    /// Request forwarding — insert the supplied Via line (caller is
    /// responsible for the trailing `\r\n`) above the existing top.
    Push(Bytes),
    /// Response forwarding — remove the existing top Via line.
    Pop,
}

impl fmt::Debug for ViaRewrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Push(bytes) => formatter
                .debug_struct("Push")
                .field("bytes", &bytes.len())
                .finish(),
            Self::Pop => formatter.write_str("Pop"),
        }
    }
}

/// Apply a `ViaRewrite` to a serialized SIP message, returning the
/// edited byte buffer. Public so callers that want to inspect or
/// further-process the rewritten bytes can do so without owning a
/// `Transport`.
pub fn apply_via_rewrite(bytes: Bytes, rewrite: ViaRewrite) -> Result<Bytes> {
    use bytes::BytesMut;
    use rvoip_sip_core::parser::via_locator::find_top_via_line;

    let top_range = find_top_via_line(&bytes).ok_or_else(|| {
        crate::error::Error::ProtocolError(
            "forward_raw_with_via_rewrite: message has no top Via header".to_string(),
        )
    })?;

    let mut buf = BytesMut::with_capacity(bytes.len() + 256);
    buf.extend_from_slice(&bytes[..top_range.start]);
    match rewrite {
        ViaRewrite::Push(new_line) => {
            buf.extend_from_slice(&new_line);
            // Existing top Via stays — slide it down to position 2.
            buf.extend_from_slice(&bytes[top_range.start..]);
        }
        ViaRewrite::Pop => {
            // Drop the top Via line entirely (range includes its CRLF).
            buf.extend_from_slice(&bytes[top_range.end..]);
        }
    }
    Ok(buf.freeze())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::{
        types::headers::{HeaderName, HeaderValue, TypedHeader},
        Method, Request, Response, StatusCode,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // Mock transport for testing the trait
    #[derive(Debug)]
    struct MockTransport {
        closed: Arc<AtomicBool>,
        local_addr: SocketAddr,
    }

    impl MockTransport {
        fn new(addr: &str) -> Self {
            Self {
                closed: Arc::new(AtomicBool::new(false)),
                local_addr: addr.parse().unwrap(),
            }
        }
    }

    #[async_trait::async_trait]
    impl Transport for MockTransport {
        fn local_addr(&self) -> Result<SocketAddr> {
            Ok(self.local_addr)
        }

        async fn send_message(&self, _message: Message, _destination: SocketAddr) -> Result<()> {
            if self.closed.load(Ordering::Relaxed) {
                return Err(crate::error::Error::TransportClosed);
            }
            Ok(())
        }

        async fn close(&self) -> Result<()> {
            self.closed.store(true, Ordering::Relaxed);
            Ok(())
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::Relaxed)
        }
    }

    #[tokio::test]
    async fn test_mock_transport() {
        let transport = MockTransport::new("127.0.0.1:5060");
        assert_eq!(
            transport.local_addr().unwrap().to_string(),
            "127.0.0.1:5060"
        );
        assert!(!transport.is_closed());

        transport.close().await.unwrap();
        assert!(transport.is_closed());
    }

    #[test]
    fn message_received_debug_exposes_only_safe_message_metadata() {
        const URI_SECRET: &str = "uri-secret.example";
        const AUTH_SECRET: &str = "Bearer transport-event-auth-secret";
        const SDP_SECRET: &str = "v=0 s=transport-event-sdp-secret";
        const RAW_SECRET: &str = "raw-wire-secret";
        const REASON_SECRET: &str = "transport-event-reason-secret";
        const METHOD_SECRET: &str = "transport-event-extension-secret";
        let source = "127.0.0.1:5060".parse().unwrap();
        let destination = "127.0.0.1:5061".parse().unwrap();

        let mut request = Request::new(
            Method::Invite,
            format!("sip:bob@{URI_SECRET}").parse().unwrap(),
        )
        .with_body(SDP_SECRET);
        request.headers.push(TypedHeader::Other(
            HeaderName::Authorization,
            HeaderValue::Raw(AUTH_SECRET.as_bytes().to_vec()),
        ));
        let request_debug = format!(
            "{:?}",
            TransportEvent::MessageReceived {
                message: Message::Request(request),
                source,
                destination,
                transport_type: TransportType::Tcp,
                flow_id: Some(TransportFlowId::for_test(1)),
                raw_bytes: Some(Bytes::from_static(RAW_SECRET.as_bytes())),
                timing: None,
                connection_metadata: None,
            }
        );

        let response_debug = format!(
            "{:?}",
            TransportEvent::MessageReceived {
                message: Message::Response(
                    Response::new(StatusCode::Ok).with_reason(REASON_SECRET),
                ),
                source,
                destination,
                transport_type: TransportType::Tls,
                flow_id: Some(TransportFlowId::for_test(2)),
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            }
        );
        let extension_debug = format!(
            "{:?}",
            TransportEvent::MessageReceived {
                message: Message::Request(Request::new(
                    Method::Extension(METHOD_SECRET.into()),
                    "sip:example.test".parse().unwrap(),
                )),
                source,
                destination,
                transport_type: TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            }
        );

        for (debug, secrets) in [
            (
                request_debug,
                [URI_SECRET, AUTH_SECRET, SDP_SECRET, RAW_SECRET],
            ),
            (
                response_debug,
                [REASON_SECRET, AUTH_SECRET, SDP_SECRET, RAW_SECRET],
            ),
            (
                extension_debug,
                [METHOD_SECRET, AUTH_SECRET, SDP_SECRET, RAW_SECRET],
            ),
        ] {
            for secret in secrets {
                assert!(!debug.contains(secret));
            }
            assert!(debug.contains("header_count"));
            assert!(debug.contains("body_len"));
            assert!(debug.contains("raw_bytes_present"));
            assert!(debug.contains("raw_bytes_len"));
        }
    }

    #[test]
    fn via_rewrite_debug_reports_only_wire_extent() {
        const SECRET: &str = "via-rewrite-secret-canary";
        let rewrite = ViaRewrite::Push(Bytes::copy_from_slice(SECRET.as_bytes()));
        let rendered = format!("{rewrite:?}");
        assert!(!rendered.contains(SECRET));
        assert!(rendered.contains(&format!("bytes: {}", SECRET.len())));
    }

    #[test]
    fn tls_peer_metadata_debug_never_exposes_certificate_fingerprint() {
        const CANARY: &str = "tls-certificate-fingerprint-secret-canary";
        let identity = TlsPeerIdentity {
            leaf_certificate_sha256: CANARY.into(),
            presented_chain_len: 2,
        };
        let metadata = TransportConnectionMetadata {
            tls_peer_identity: identity.clone(),
        };
        for rendered in [format!("{identity:?}"), format!("{metadata:?}")] {
            assert!(!rendered.contains(CANARY));
            assert!(rendered.contains("presented_chain_len: 2"));
        }
    }
}
