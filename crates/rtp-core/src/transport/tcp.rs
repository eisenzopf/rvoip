//! TCP transport for RTP/RTCP
//!
//! This module provides a TCP transport for RTP and RTCP using RFC 4571 framing.
//! Each RTP/RTCP packet is prefixed with a 2-byte length field (network byte order).

use std::any::Any;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, broadcast};

use crate::error::Error;
use crate::Result;
use crate::traits::RtpEvent;
use super::{RtpTransport, RtpTransportConfig};

/// TCP transport for RTP using RFC 4571 framing.
///
/// Each packet on the wire is prefixed with a 2-byte big-endian length field
/// followed by the RTP or RTCP payload bytes.
pub struct TcpRtpTransport {
    /// Configuration
    config: RtpTransportConfig,

    /// Event channel
    event_tx: broadcast::Sender<RtpEvent>,

    /// Connected TCP stream (client side or accepted connection)
    stream: Arc<Mutex<Option<TcpStream>>>,

    /// TCP listener for server mode
    listener: Arc<Mutex<Option<TcpListener>>>,

    /// Local address once bound/connected
    local_addr: Arc<Mutex<Option<SocketAddr>>>,

    /// Whether the transport has been closed
    closed: Arc<Mutex<bool>>,
}

impl TcpRtpTransport {
    /// Create a new TCP transport.
    ///
    /// The transport starts unconnected. Call the trait methods to connect or
    /// bind as needed. Alternatively, use `connect` or `bind` helpers.
    pub async fn new(config: RtpTransportConfig) -> Result<Self> {
        let (event_tx, _) = broadcast::channel(100);

        Ok(Self {
            config,
            event_tx,
            stream: Arc::new(Mutex::new(None)),
            listener: Arc::new(Mutex::new(None)),
            local_addr: Arc::new(Mutex::new(None)),
            closed: Arc::new(Mutex::new(false)),
        })
    }

    /// Establish a TCP connection to the given remote address.
    pub async fn connect(&self, remote: SocketAddr) -> Result<()> {
        let tcp_stream = TcpStream::connect(remote).await
            .map_err(|e| Error::Transport(format!("TCP connect to {} failed: {}", remote, e)))?;

        let local = tcp_stream.local_addr()
            .map_err(|e| Error::Transport(format!("Failed to get local addr: {}", e)))?;

        {
            let mut addr_guard = self.local_addr.lock().await;
            *addr_guard = Some(local);
        }
        {
            let mut stream_guard = self.stream.lock().await;
            *stream_guard = Some(tcp_stream);
        }

        Ok(())
    }

    /// Bind a TCP listener on the configured local address.
    pub async fn bind(&self) -> Result<()> {
        let tcp_listener = TcpListener::bind(self.config.local_rtp_addr).await
            .map_err(|e| Error::Transport(format!("TCP bind on {} failed: {}", self.config.local_rtp_addr, e)))?;

        let local = tcp_listener.local_addr()
            .map_err(|e| Error::Transport(format!("Failed to get listener local addr: {}", e)))?;

        {
            let mut addr_guard = self.local_addr.lock().await;
            *addr_guard = Some(local);
        }
        {
            let mut listener_guard = self.listener.lock().await;
            *listener_guard = Some(tcp_listener);
        }

        Ok(())
    }

    /// Accept one incoming TCP connection on the bound listener.
    ///
    /// After accepting, the transport can send and receive packets on the
    /// accepted connection.
    pub async fn accept(&self) -> Result<SocketAddr> {
        let listener_guard = self.listener.lock().await;
        let listener = listener_guard.as_ref().ok_or_else(|| {
            Error::InvalidState("TCP listener not bound; call bind() first".to_string())
        })?;

        let (tcp_stream, peer_addr) = listener.accept().await
            .map_err(|e| Error::Transport(format!("TCP accept failed: {}", e)))?;

        drop(listener_guard);

        {
            let mut stream_guard = self.stream.lock().await;
            *stream_guard = Some(tcp_stream);
        }

        Ok(peer_addr)
    }

    /// Send a framed packet: 2-byte big-endian length prefix + payload (RFC 4571).
    async fn send_framed(&self, data: &[u8], _dest: SocketAddr) -> Result<()> {
        let mut stream_guard = self.stream.lock().await;
        let stream = stream_guard.as_mut().ok_or_else(|| {
            Error::InvalidState("TCP stream not connected".to_string())
        })?;

        let len = data.len();
        if len > u16::MAX as usize {
            return Err(Error::InvalidParameter(
                format!("Packet too large for RFC 4571 framing: {} bytes (max {})", len, u16::MAX),
            ));
        }

        // Write 2-byte length prefix
        stream.write_all(&(len as u16).to_be_bytes()).await
            .map_err(|e| Error::Transport(format!("TCP write length failed: {}", e)))?;

        // Write payload
        stream.write_all(data).await
            .map_err(|e| Error::Transport(format!("TCP write payload failed: {}", e)))?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl RtpTransport for TcpRtpTransport {
    /// Get the local RTP address
    fn local_rtp_addr(&self) -> Result<SocketAddr> {
        // We cannot .await in a sync fn, but we stored the address during
        // connect/bind which are the only paths that set it. Use try_lock as a
        // best-effort read; callers are expected to have connected/bound first.
        let guard = self.local_addr.try_lock()
            .map_err(|_| Error::InvalidState("Local address lock contended".to_string()))?;
        guard.ok_or_else(|| Error::InvalidState("TCP transport not yet connected or bound".to_string()))
    }

    /// Get the local RTCP address (TCP uses the same connection for both)
    fn local_rtcp_addr(&self) -> Result<Option<SocketAddr>> {
        // In TCP mode, RTCP is multiplexed on the same connection
        Ok(Some(self.local_rtp_addr()?))
    }

    /// Send an RTP packet with RFC 4571 framing
    async fn send_rtp(&self, packet: &crate::packet::RtpPacket, dest: SocketAddr) -> Result<()> {
        let bytes = packet.serialize()?;
        self.send_framed(&bytes, dest).await
    }

    /// Send raw RTP bytes with RFC 4571 framing
    async fn send_rtp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        self.send_framed(bytes, dest).await
    }

    /// Send raw RTCP bytes with RFC 4571 framing
    async fn send_rtcp_bytes(&self, data: &[u8], dest: SocketAddr) -> Result<()> {
        self.send_framed(data, dest).await
    }

    /// Send an RTCP packet with RFC 4571 framing
    async fn send_rtcp(&self, packet: &crate::packet::rtcp::RtcpPacket, dest: SocketAddr) -> Result<()> {
        let bytes = packet.serialize()?;
        self.send_framed(&bytes, dest).await
    }

    /// Receive a framed packet: read 2-byte length prefix then payload
    async fn receive_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        let mut stream_guard = self.stream.lock().await;
        let stream = stream_guard.as_mut().ok_or_else(|| {
            Error::InvalidState("TCP stream not connected".to_string())
        })?;

        let peer_addr = stream.peer_addr()
            .map_err(|e| Error::Transport(format!("Failed to get peer addr: {}", e)))?;

        // Read 2-byte length prefix
        let mut len_buf = [0u8; 2];
        stream.read_exact(&mut len_buf).await
            .map_err(|e| Error::Transport(format!("TCP read length failed: {}", e)))?;

        let payload_len = u16::from_be_bytes(len_buf) as usize;

        if payload_len > buffer.len() {
            return Err(Error::BufferTooSmall {
                required: payload_len,
                available: buffer.len(),
            });
        }

        // Read the payload
        stream.read_exact(&mut buffer[..payload_len]).await
            .map_err(|e| Error::Transport(format!("TCP read payload failed: {}", e)))?;

        Ok((payload_len, peer_addr))
    }

    /// Close the transport
    async fn close(&self) -> Result<()> {
        {
            let mut closed_guard = self.closed.lock().await;
            *closed_guard = true;
        }

        // Shut down the TCP stream if present
        let mut stream_guard = self.stream.lock().await;
        if let Some(ref mut stream) = *stream_guard {
            if let Err(e) = stream.shutdown().await {
                tracing::debug!("Failed to shutdown TCP stream during close: {e}");
            }
        }
        *stream_guard = None;

        // Drop the listener
        let mut listener_guard = self.listener.lock().await;
        *listener_guard = None;

        Ok(())
    }

    /// Subscribe to transport events
    fn subscribe(&self) -> broadcast::Receiver<RtpEvent> {
        self.event_tx.subscribe()
    }

    /// Get the transport as Any (for downcasting)
    fn as_any(&self) -> &dyn Any {
        self
    }
}
