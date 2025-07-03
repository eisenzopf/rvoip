use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use bytes::{BytesMut, Buf, BufMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};

use rvoip_sip_core::{Message, parse_message};
use crate::error::{Error, Result};
use rvoip_sip_core::builder::ContentLengthBuilderExt;

// Buffer sizes
const INITIAL_BUFFER_SIZE: usize = 8192;
const MAX_MESSAGE_SIZE: usize = 65535;

/// TCP connection for SIP messages
pub struct TcpConnection {
    /// The TCP stream for this connection
    stream: Mutex<TcpStream>,
    /// The peer's address
    peer_addr: SocketAddr,
    /// Whether the connection is closed
    closed: AtomicBool,
    /// Buffer for receiving data
    recv_buffer: Mutex<BytesMut>,
}

impl TcpConnection {
    /// Creates a new TCP connection to the specified address
    pub async fn connect(addr: SocketAddr) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| Error::ConnectFailed(addr, e))?;
        
        Ok(Self {
            stream: Mutex::new(stream),
            peer_addr: addr,
            closed: AtomicBool::new(false),
            recv_buffer: Mutex::new(BytesMut::with_capacity(INITIAL_BUFFER_SIZE)),
        })
    }
    
    /// Creates a TCP connection from an existing stream
    pub fn from_stream(stream: TcpStream, peer_addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            stream: Mutex::new(stream),
            peer_addr,
            closed: AtomicBool::new(false),
            recv_buffer: Mutex::new(BytesMut::with_capacity(INITIAL_BUFFER_SIZE)),
        })
    }
    
    /// Returns the peer address of the connection
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }
    
    /// Returns the local address of the connection
    pub fn local_addr(&self) -> Result<SocketAddr> {
        let stream = self.stream.try_lock()
            .map_err(|_| Error::InvalidState("Failed to acquire stream lock".to_string()))?;
        
        stream.local_addr().map_err(|e| Error::LocalAddrFailed(e))
    }
    
    /// Sends a SIP message over the connection
    pub async fn send_message(&self, message: &Message) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        
        // Convert message to bytes
        let message_bytes = message.to_bytes();
        
        // Acquire lock on the stream
        let mut stream = self.stream.lock().await;
        
        // Send the message
        stream.write_all(&message_bytes).await
            .map_err(|e| {
                if e.kind() == io::ErrorKind::BrokenPipe || e.kind() == io::ErrorKind::ConnectionReset {
                    self.closed.store(true, Ordering::Relaxed);
                    Error::ConnectionReset
                } else {
                    Error::SendFailed(self.peer_addr, e)
                }
            })?;
        
        // Flush the stream to ensure all data is sent
        stream.flush().await
            .map_err(|e| Error::SendFailed(self.peer_addr, e))?;
        
        trace!("Sent {} bytes to {}", message_bytes.len(), self.peer_addr);
        Ok(())
    }
    
    /// Receives a SIP message from the connection
    pub async fn receive_message(&self) -> Result<Option<Message>> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        
        // Acquire locks for the buffer and stream
        let mut recv_buffer = self.recv_buffer.lock().await;
        let mut stream = self.stream.lock().await;
        
        // Loop until we have a complete message or error
        loop {
            // Try to parse a message from the buffer
            if let Some(message) = self.try_parse_message(&mut recv_buffer)? {
                return Ok(Some(message));
            }
            
            // No complete message, read more data
            let mut temp_buffer = vec![0; 8192];
            
            match stream.read(&mut temp_buffer).await {
                Ok(0) => {
                    // End of stream
                    if recv_buffer.is_empty() {
                        // Clean EOF
                        self.closed.store(true, Ordering::Relaxed);
                        return Ok(None);
                    } else {
                        // Partial message left in buffer
                        self.closed.store(true, Ordering::Relaxed);
                        return Err(Error::StreamClosed);
                    }
                }
                Ok(n) => {
                    trace!("Read {} bytes from {}", n, self.peer_addr);
                    
                    // Check buffer capacity
                    if recv_buffer.len() + n > MAX_MESSAGE_SIZE {
                        return Err(Error::MessageTooLarge(recv_buffer.len() + n));
                    }
                    
                    // Append read data to the buffer
                    recv_buffer.put_slice(&temp_buffer[0..n]);
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        // No data available yet, try again
                        continue;
                    } else {
                        // Connection error
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
    }
    
    /// Tries to parse a SIP message from the buffer
    fn try_parse_message(&self, buffer: &mut BytesMut) -> Result<Option<Message>> {
        if buffer.is_empty() {
            return Ok(None);
        }
        
        // First check if we have headers and body
        if let Some(double_crlf_pos) = self.find_double_crlf(buffer) {
            // Found header/body separator, now check Content-Length
            let header_slice = &buffer[0..double_crlf_pos + 4]; // Include the separator
            
            // Try to extract Content-Length
            let content_length = self.extract_content_length(header_slice);
            
            // Calculate total message length
            let total_length = double_crlf_pos + 4 + content_length;
            
            // Check if we have the complete message
            if buffer.len() >= total_length {
                // We have a complete message, extract it
                let message_slice = &buffer[0..total_length];
                
                // Parse the message
                match parse_message(message_slice) {
                    Ok(message) => {
                        // Message parsed successfully
                        trace!("Parsed complete SIP message ({} bytes)", total_length);
                        
                        // Remove the message from the buffer
                        buffer.advance(total_length);
                        
                        return Ok(Some(message));
                    }
                    Err(e) => {
                        // Parsing error
                        warn!("Failed to parse SIP message: {}", e);
                        
                        // Advance past this malformed message to avoid getting stuck
                        buffer.advance(total_length);
                        
                        return Err(Error::ParseError(e.to_string()));
                    }
                }
            }
        }
        
        // No complete message found
        Ok(None)
    }
    
    /// Finds the position of the double CRLF (end of headers)
    fn find_double_crlf(&self, buffer: &BytesMut) -> Option<usize> {
        for i in 0..buffer.len().saturating_sub(3) {
            if buffer[i] == b'\r' && buffer[i + 1] == b'\n' && 
               buffer[i + 2] == b'\r' && buffer[i + 3] == b'\n' {
                return Some(i);
            }
        }
        None
    }
    
    /// Extracts the Content-Length value from the header section
    fn extract_content_length(&self, header_slice: &[u8]) -> usize {
        let header_str = String::from_utf8_lossy(header_slice);
        
        // Simple parsing to extract Content-Length
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
        
        // Default to 0 if not found
        0
    }
    
    /// Closes the TCP connection
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::Relaxed) {
            // Already closed
            return Ok(());
        }
        
        let mut stream = self.stream.lock().await;
        
        // Shutdown the stream
        if let Err(e) = stream.shutdown().await {
            if e.kind() != io::ErrorKind::NotConnected {
                // Only report errors if they're not due to the socket already being closed
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

impl Drop for TcpConnection {
    fn drop(&mut self) {
        if !self.is_closed() {
            // The connection is being dropped without being closed
            debug!("TCP connection to {} dropped without being closed", self.peer_addr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use rvoip_sip_core::{Method, Request};
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    
    #[tokio::test]
    async fn test_tcp_connection_connect() {
        // Start a TCP server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        
        // Start accepting connections in the background
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            // Just accept and drop it
            drop(socket);
        });
        
        // Connect to the server
        let connection = TcpConnection::connect(server_addr).await.unwrap();
        
        assert_eq!(connection.peer_addr(), server_addr);
        assert!(!connection.is_closed());
        
        connection.close().await.unwrap();
        assert!(connection.is_closed());
    }
    
    #[tokio::test]
    async fn test_tcp_connection_send_receive() {
        // Start a TCP server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        
        // Set up a channel to communicate with the server task
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        
        // Start the server task
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let connection = TcpConnection::from_stream(socket, server_addr).unwrap();
            
            // Receive a message and send it back through the channel
            let message = connection.receive_message().await.unwrap().unwrap();
            tx.send(message).await.unwrap();
        });
        
        // Connect to the server
        let connection = TcpConnection::connect(server_addr).await.unwrap();
        
        // Create a test SIP message
        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag1"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("call1@example.com")
            .cseq(1)
            .build();
        
        // Send the message
        connection.send_message(&request.into()).await.unwrap();
        
        // Wait for the server to echo it back
        let received_message = rx.recv().await.unwrap();
        
        // Verify the message was received correctly
        assert!(received_message.is_request());
        if let Message::Request(req) = received_message {
            assert_eq!(req.method(), Method::Register);
            assert_eq!(req.call_id().unwrap().to_string(), "call1@example.com");
        } else {
            panic!("Expected a request");
        }
        
        // Clean up
        connection.close().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_tcp_connection_framing() {
        // Start a TCP server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        
        // Set up a channel to communicate with the server task
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        
        // Start the server task
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            
            // Send two messages in a single TCP packet/operation
            let req1 = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
                .unwrap()
                .from("alice", "sip:alice@example.com", Some("tag1"))
                .to("bob", "sip:bob@example.com", None)
                .call_id("call1@example.com")
                .cseq(1)
                .content_length(0)
                .build();
            
            let req2 = SimpleRequestBuilder::new(Method::Options, "sip:example.com")
                .unwrap()
                .from("alice", "sip:alice@example.com", Some("tag2"))
                .to("bob", "sip:bob@example.com", None)
                .call_id("call2@example.com")
                .cseq(2)
                .content_length(0)
                .build();
            
            // Combine both messages into a single buffer
            let mut combined = BytesMut::new();
            combined.extend_from_slice(&Message::Request(req1).to_bytes());
            combined.extend_from_slice(&Message::Request(req2).to_bytes());
            
            // Send both messages at once
            socket.write_all(&combined).await.unwrap();
            
            // Tell the test we sent the data
            tx.send(2).await.unwrap(); // Sent 2 messages
        });
        
        // Connect to the server
        let connection = TcpConnection::connect(server_addr).await.unwrap();
        
        // Wait for the server to send the messages
        let num_messages = rx.recv().await.unwrap();
        assert_eq!(num_messages, 2);
        
        // Read the first message
        let message1 = connection.receive_message().await.unwrap().unwrap();
        assert!(message1.is_request());
        if let Message::Request(req) = message1 {
            assert_eq!(req.method(), Method::Register);
            assert_eq!(req.call_id().unwrap().to_string(), "call1@example.com");
        } else {
            panic!("Expected a request");
        }
        
        // Read the second message
        let message2 = connection.receive_message().await.unwrap().unwrap();
        assert!(message2.is_request());
        if let Message::Request(req) = message2 {
            assert_eq!(req.method(), Method::Options);
            assert_eq!(req.call_id().unwrap().to_string(), "call2@example.com");
        } else {
            panic!("Expected a request");
        }
        
        // Clean up
        connection.close().await.unwrap();
    }
} 