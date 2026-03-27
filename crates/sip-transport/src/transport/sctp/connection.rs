//! SCTP connection for SIP messages (RFC 4168)
//!
//! Each SCTP connection wraps an SCTP association and provides stream-based
//! multiplexing. Per RFC 4168, different SIP transactions can use different
//! SCTP streams to avoid head-of-line blocking.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use bytes::{BytesMut, BufMut, Buf};
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};
use webrtc_sctp::association::Association;
use webrtc_sctp::stream::Stream as SctpStream;

use rvoip_sip_core::{Message, parse_message};
use crate::error::{Error, Result};

/// Maximum number of SCTP streams to use for round-robin multiplexing
const MAX_STREAMS: u16 = 16;

/// Maximum SIP message size over SCTP
const MAX_MESSAGE_SIZE: usize = 65535;

/// Initial receive buffer capacity
const INITIAL_BUFFER_SIZE: usize = 8192;

/// SCTP connection wrapping an association for SIP message transport
pub struct SctpConnection {
    /// The SCTP association
    association: Arc<Association>,
    /// Remote peer address (the UDP address underlying the SCTP association)
    peer_addr: SocketAddr,
    /// Local address
    local_addr: SocketAddr,
    /// Whether the connection is closed
    closed: AtomicBool,
    /// Round-robin stream identifier counter for outbound messages
    next_stream_id: AtomicU16,
    /// Cached outbound streams for reuse
    outbound_streams: Mutex<Vec<Option<Arc<SctpStream>>>>,
}

impl SctpConnection {
    /// Creates a new SCTP connection from an existing association
    pub fn new(
        association: Arc<Association>,
        peer_addr: SocketAddr,
        local_addr: SocketAddr,
    ) -> Self {
        Self {
            association,
            peer_addr,
            local_addr,
            closed: AtomicBool::new(false),
            next_stream_id: AtomicU16::new(0),
            outbound_streams: Mutex::new(vec![None; MAX_STREAMS as usize]),
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

    /// Gets or opens an SCTP stream for the given stream ID
    async fn get_or_open_stream(&self, stream_id: u16) -> Result<Arc<SctpStream>> {
        let mut streams = self.outbound_streams.lock().await;
        let idx = stream_id as usize % MAX_STREAMS as usize;

        if let Some(ref stream) = streams[idx] {
            return Ok(Arc::clone(stream));
        }

        // Open a new stream on the association
        let stream = self.association
            .open_stream(stream_id, Default::default())
            .await
            .map_err(|e| Error::ProtocolError(format!("Failed to open SCTP stream {}: {}", stream_id, e)))?;

        streams[idx] = Some(Arc::clone(&stream));
        Ok(stream)
    }

    /// Selects the next stream ID using round-robin
    fn next_stream(&self) -> u16 {
        let id = self.next_stream_id.fetch_add(1, Ordering::Relaxed);
        id % MAX_STREAMS
    }

    /// Sends a SIP message over the SCTP connection.
    ///
    /// Per RFC 4168, each transaction can use a different SCTP stream
    /// to avoid head-of-line blocking.
    pub async fn send_message(&self, message: &Message) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        let message_bytes = message.to_bytes();
        let message_len = message_bytes.len();
        if message_len > MAX_MESSAGE_SIZE {
            return Err(Error::MessageTooLarge(message_len));
        }

        let stream_id = self.next_stream();
        let stream = self.get_or_open_stream(stream_id).await?;

        let data = bytes::Bytes::from(message_bytes);
        stream.write(&data).await.map_err(|e| {
            if format!("{}", e).contains("closed") || format!("{}", e).contains("shutdown") {
                self.closed.store(true, Ordering::Relaxed);
                Error::ConnectionReset
            } else {
                Error::ProtocolError(format!("SCTP write error on stream {}: {}", stream_id, e))
            }
        })?;

        trace!(
            stream_id = stream_id,
            bytes = message_len,
            peer = %self.peer_addr,
            "Sent SIP message over SCTP"
        );

        Ok(())
    }

    /// Receives a SIP message from any SCTP stream on this association.
    ///
    /// This accepts a new stream from the association, reads one complete
    /// SIP message from it, and returns it.
    pub async fn receive_message(&self) -> Result<Option<Message>> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        // Accept the next stream that has data
        let stream = match self.association.accept_stream().await {
            Some(stream) => stream,
            None => {
                // Association closed
                self.closed.store(true, Ordering::Relaxed);
                return Ok(None);
            }
        };

        let stream_id = stream.stream_identifier();

        // Read data from the stream
        let mut recv_buffer = BytesMut::with_capacity(INITIAL_BUFFER_SIZE);
        let mut temp_buffer = vec![0u8; INITIAL_BUFFER_SIZE];

        loop {
            match stream.read(&mut temp_buffer).await {
                Ok(0) => {
                    // Stream closed
                    if recv_buffer.is_empty() {
                        return Ok(None);
                    }
                    // Try to parse what we have
                    break;
                }
                Ok(n) => {
                    if recv_buffer.len() + n > MAX_MESSAGE_SIZE {
                        return Err(Error::MessageTooLarge(recv_buffer.len() + n));
                    }
                    recv_buffer.put_slice(&temp_buffer[..n]);

                    // Try to parse a complete SIP message
                    if let Some(message) = try_parse_sip_message(&mut recv_buffer)? {
                        trace!(
                            stream_id = stream_id,
                            peer = %self.peer_addr,
                            "Received SIP message over SCTP"
                        );
                        return Ok(Some(message));
                    }
                }
                Err(e) => {
                    let err_str = format!("{}", e);
                    if err_str.contains("closed") || err_str.contains("shutdown") {
                        self.closed.store(true, Ordering::Relaxed);
                        if recv_buffer.is_empty() {
                            return Ok(None);
                        }
                        return Err(Error::StreamClosed);
                    }
                    return Err(Error::ProtocolError(format!(
                        "SCTP read error on stream {}: {}",
                        stream_id, e
                    )));
                }
            }
        }

        // Try to parse whatever is in the buffer
        if !recv_buffer.is_empty() {
            if let Some(message) = try_parse_sip_message(&mut recv_buffer)? {
                return Ok(Some(message));
            }
        }

        Ok(None)
    }

    /// Closes the SCTP connection
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::Relaxed) {
            // Already closed
            return Ok(());
        }

        // Close all outbound streams
        let mut streams = self.outbound_streams.lock().await;
        for stream_opt in streams.iter_mut() {
            if let Some(stream) = stream_opt.take() {
                if let Err(e) = stream.close().await {
                    debug!("Error closing SCTP stream: {}", e);
                }
            }
        }

        // Close the association
        if let Err(e) = self.association.close().await {
            debug!("Error closing SCTP association: {}", e);
        }

        Ok(())
    }

    /// Returns whether the connection is closed
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

impl Drop for SctpConnection {
    fn drop(&mut self) {
        if !self.is_closed() {
            debug!("SCTP connection to {} dropped without being closed", self.peer_addr);
        }
    }
}

/// Tries to parse a complete SIP message from the buffer.
///
/// This looks for the double CRLF that separates headers from body,
/// then uses Content-Length to determine the total message size.
fn try_parse_sip_message(buffer: &mut BytesMut) -> Result<Option<Message>> {
    if buffer.is_empty() {
        return Ok(None);
    }

    // Find the double CRLF (end of headers)
    let double_crlf_pos = find_double_crlf(buffer);
    let Some(pos) = double_crlf_pos else {
        return Ok(None);
    };

    // Found header/body separator
    let header_slice = &buffer[..pos + 4]; // Include \r\n\r\n
    let content_length = extract_content_length(header_slice);
    let total_length = pos + 4 + content_length;

    if buffer.len() < total_length {
        // Don't have the full message yet
        return Ok(None);
    }

    // Parse the message
    let message_slice = &buffer[..total_length];
    match parse_message(message_slice) {
        Ok(message) => {
            buffer.advance(total_length);
            Ok(Some(message))
        }
        Err(e) => {
            warn!("Failed to parse SIP message from SCTP: {}", e);
            buffer.advance(total_length);
            Err(Error::ParseError(e.to_string()))
        }
    }
}

/// Finds the position of the double CRLF (end of headers)
fn find_double_crlf(buffer: &BytesMut) -> Option<usize> {
    for i in 0..buffer.len().saturating_sub(3) {
        if buffer[i] == b'\r'
            && buffer[i + 1] == b'\n'
            && buffer[i + 2] == b'\r'
            && buffer[i + 3] == b'\n'
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_double_crlf() {
        let mut buf = BytesMut::from("INVITE sip:bob@example.com SIP/2.0\r\nContent-Length: 0\r\n\r\n");
        let pos = find_double_crlf(&buf);
        assert!(pos.is_some());
    }

    #[test]
    fn test_extract_content_length_present() {
        let header = b"INVITE sip:bob@example.com SIP/2.0\r\nContent-Length: 142\r\n\r\n";
        assert_eq!(extract_content_length(header), 142);
    }

    #[test]
    fn test_extract_content_length_missing() {
        let header = b"INVITE sip:bob@example.com SIP/2.0\r\nVia: SIP/2.0/SCTP host\r\n\r\n";
        assert_eq!(extract_content_length(header), 0);
    }

    #[test]
    fn test_try_parse_sip_message_incomplete() {
        let mut buf = BytesMut::from("INVITE sip:bob@example.com SIP/2.0\r\nContent-Length: 100\r\n\r\n");
        // Buffer has 0 body bytes but Content-Length says 100 -- incomplete
        let result = try_parse_sip_message(&mut buf);
        assert!(result.is_ok());
        assert!(result.ok().flatten().is_none());
    }
}
