use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use bytes::{BytesMut, Buf, BufMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};

use rvoip_sip_core::{Message, parse_message};
use crate::error::{Error, Result};

/// Wrapper enum for TLS streams so we can handle both client and server sides.
/// tokio-rustls has distinct types for each direction.
pub enum TlsStream {
    /// A TLS stream from a client connection
    Client(tokio_rustls::client::TlsStream<tokio::net::TcpStream>),
    /// A TLS stream from an accepted server connection
    Server(tokio_rustls::server::TlsStream<tokio::net::TcpStream>),
}

// Buffer sizes
const INITIAL_BUFFER_SIZE: usize = 8192;
const MAX_MESSAGE_SIZE: usize = 65535;

/// TLS connection for SIP messages
pub struct TlsConnection {
    /// The TLS stream for this connection
    stream: Mutex<TlsStream>,
    /// The peer's address
    peer_addr: SocketAddr,
    /// The local address
    local_addr: SocketAddr,
    /// Whether the connection is closed
    closed: AtomicBool,
    /// Buffer for receiving data
    recv_buffer: Mutex<BytesMut>,
}

impl TlsConnection {
    /// Creates a TLS connection by connecting to a remote address using a TLS client config
    pub async fn connect(
        addr: SocketAddr,
        tls_connector: &tokio_rustls::TlsConnector,
        server_name: rustls::ServerName,
    ) -> Result<Self> {
        let tcp_stream = tokio::net::TcpStream::connect(addr)
            .await
            .map_err(|e| Error::ConnectFailed(addr, e))?;

        let local_addr = tcp_stream.local_addr()
            .map_err(|e| Error::LocalAddrFailed(e))?;

        if let Err(e) = tcp_stream.set_nodelay(true) {
            error!("Failed to set TCP_NODELAY: {}", e);
        }

        let tls_stream = tls_connector.connect(server_name, tcp_stream)
            .await
            .map_err(|e| Error::TlsHandshakeFailed(format!("Client handshake with {}: {}", addr, e)))?;

        Ok(Self {
            stream: Mutex::new(TlsStream::Client(tls_stream)),
            peer_addr: addr,
            local_addr,
            closed: AtomicBool::new(false),
            recv_buffer: Mutex::new(BytesMut::with_capacity(INITIAL_BUFFER_SIZE)),
        })
    }

    /// Creates a TLS connection from an already-accepted server TLS stream
    pub fn from_server_stream(
        stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
        peer_addr: SocketAddr,
        local_addr: SocketAddr,
    ) -> Self {
        Self {
            stream: Mutex::new(TlsStream::Server(stream)),
            peer_addr,
            local_addr,
            closed: AtomicBool::new(false),
            recv_buffer: Mutex::new(BytesMut::with_capacity(INITIAL_BUFFER_SIZE)),
        }
    }

    /// Returns the peer address of the connection
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    /// Returns the local address of the connection
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }

    /// Sends a SIP message over the TLS connection
    pub async fn send_message(&self, message: &Message) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        let message_bytes = message.to_bytes();
        let mut stream = self.stream.lock().await;

        let write_result = match &mut *stream {
            TlsStream::Client(s) => s.write_all(&message_bytes).await,
            TlsStream::Server(s) => s.write_all(&message_bytes).await,
        };

        write_result.map_err(|e| {
            if e.kind() == io::ErrorKind::BrokenPipe || e.kind() == io::ErrorKind::ConnectionReset {
                self.closed.store(true, Ordering::Relaxed);
                Error::ConnectionReset
            } else {
                Error::SendFailed(self.peer_addr, e)
            }
        })?;

        let flush_result = match &mut *stream {
            TlsStream::Client(s) => s.flush().await,
            TlsStream::Server(s) => s.flush().await,
        };

        flush_result.map_err(|e| Error::SendFailed(self.peer_addr, e))?;

        trace!("Sent {} bytes to {} over TLS", message_bytes.len(), self.peer_addr);
        Ok(())
    }

    /// Receives a SIP message from the TLS connection
    pub async fn receive_message(&self) -> Result<Option<Message>> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        let mut recv_buffer = self.recv_buffer.lock().await;
        let mut stream = self.stream.lock().await;

        loop {
            // Try to parse a message from the buffer
            if let Some(message) = try_parse_message(&mut recv_buffer)? {
                return Ok(Some(message));
            }

            // Read more data
            let mut temp_buffer = vec![0u8; 8192];

            let read_result = match &mut *stream {
                TlsStream::Client(s) => s.read(&mut temp_buffer).await,
                TlsStream::Server(s) => s.read(&mut temp_buffer).await,
            };

            match read_result {
                Ok(0) => {
                    if recv_buffer.is_empty() {
                        self.closed.store(true, Ordering::Relaxed);
                        return Ok(None);
                    } else {
                        self.closed.store(true, Ordering::Relaxed);
                        return Err(Error::StreamClosed);
                    }
                }
                Ok(n) => {
                    trace!("Read {} bytes from {} over TLS", n, self.peer_addr);

                    if recv_buffer.len() + n > MAX_MESSAGE_SIZE {
                        return Err(Error::MessageTooLarge(recv_buffer.len() + n));
                    }

                    recv_buffer.put_slice(&temp_buffer[0..n]);
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        continue;
                    }

                    self.closed.store(true, Ordering::Relaxed);

                    if e.kind() == io::ErrorKind::BrokenPipe || e.kind() == io::ErrorKind::ConnectionReset {
                        return Err(Error::ConnectionReset);
                    } else {
                        return Err(Error::ReceiveFailed(e));
                    }
                }
            }
        }
    }

    /// Closes the TLS connection
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::Relaxed) {
            return Ok(());
        }

        let mut stream = self.stream.lock().await;

        let shutdown_result = match &mut *stream {
            TlsStream::Client(s) => s.shutdown().await,
            TlsStream::Server(s) => s.shutdown().await,
        };

        if let Err(e) = shutdown_result {
            if e.kind() != io::ErrorKind::NotConnected {
                return Err(Error::IoError(e));
            }
        }

        Ok(())
    }

    /// Returns whether the connection is closed
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

impl Drop for TlsConnection {
    fn drop(&mut self) {
        if !self.is_closed() {
            debug!("TLS connection to {} dropped without being closed", self.peer_addr);
        }
    }
}

// --- free functions for SIP message framing (same logic as TCP) ---

/// Tries to parse a SIP message from the buffer
fn try_parse_message(buffer: &mut BytesMut) -> Result<Option<Message>> {
    if buffer.is_empty() {
        return Ok(None);
    }

    if let Some(double_crlf_pos) = find_double_crlf(buffer) {
        let header_slice = &buffer[0..double_crlf_pos + 4];
        let content_length = extract_content_length(header_slice);
        let total_length = double_crlf_pos + 4 + content_length;

        if buffer.len() >= total_length {
            let message_slice = &buffer[0..total_length];

            match parse_message(message_slice) {
                Ok(message) => {
                    trace!("Parsed complete SIP message ({} bytes)", total_length);
                    buffer.advance(total_length);
                    return Ok(Some(message));
                }
                Err(e) => {
                    warn!("Failed to parse SIP message: {}", e);
                    buffer.advance(total_length);
                    return Err(Error::ParseError(e.to_string()));
                }
            }
        }
    }

    Ok(None)
}

/// Finds the position of the double CRLF (end of headers)
fn find_double_crlf(buffer: &BytesMut) -> Option<usize> {
    for i in 0..buffer.len().saturating_sub(3) {
        if buffer[i] == b'\r' && buffer[i + 1] == b'\n'
            && buffer[i + 2] == b'\r' && buffer[i + 3] == b'\n'
        {
            return Some(i);
        }
    }
    None
}

/// Extracts the Content-Length value from the header section
fn extract_content_length(header_slice: &[u8]) -> usize {
    let header_str = String::from_utf8_lossy(header_slice);

    for line in header_str.lines() {
        let line = line.trim();
        if line.to_lowercase().starts_with("content-length:") {
            if let Some(value_str) = line.split(':').nth(1) {
                if let Ok(length) = value_str.trim().parse::<usize>() {
                    return length;
                }
            }
        }
    }

    0
}
