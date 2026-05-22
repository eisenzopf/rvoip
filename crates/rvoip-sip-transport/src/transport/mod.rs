use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::error::Result;
use bytes::Bytes;
use rvoip_sip_core::Message;

pub mod tcp;
pub mod tls;
pub mod udp;
pub mod ws;

pub use tcp::TcpTransport;
pub use tls::TlsTransport;
pub use udp::UdpTransport;
pub use ws::WebSocketTransport;

/// Represents the transport type/protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportType {
    Udp,
    Tcp,
    Tls,
    Ws,
    Wss,
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

/// Events emitted by a transport
#[derive(Debug, Clone)]
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

/// Represents a transport layer for SIP messages.
///
/// This trait defines the common interface for all transport types (UDP, TCP, TLS, WebSocket).
#[async_trait::async_trait]
pub trait Transport: Send + Sync + fmt::Debug {
    /// Returns the local address this transport is bound to
    fn local_addr(&self) -> Result<SocketAddr>;

    /// Sends a SIP message to the specified destination
    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()>;

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
    fn get_connection_count(&self, transport_type: TransportType) -> usize {
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
}

/// Direction-specific Via-stack edit for
/// [`Transport::forward_raw_with_via_rewrite`].
#[derive(Debug, Clone)]
pub enum ViaRewrite {
    /// Request forwarding — insert the supplied Via line (caller is
    /// responsible for the trailing `\r\n`) above the existing top.
    Push(Bytes),
    /// Response forwarding — remove the existing top Via line.
    Pop,
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::mpsc;

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
}
