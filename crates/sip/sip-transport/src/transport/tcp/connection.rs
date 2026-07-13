use crate::error::{Error, Result};
use crate::transport::validate_typed_outbound_message;
use bytes::{Buf, BufMut, BytesMut};
use rvoip_sip_core::framing::{inspect_sip_frame, SipFrameStatus};
use rvoip_sip_core::{parse_message, Message};
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{debug, trace, warn};

// Buffer sizes
const INITIAL_BUFFER_SIZE: usize = 8192;

/// A frame pulled off a stream-oriented SIP connection. RFC 5626 §3.5.1
/// introduces two non-SIP frames the wire may carry — a single CRLF
/// pong and a CRLFCRLF ping — both only legal at the start of the
/// receive buffer (never embedded between messages). Anything else is a
/// regular SIP message.
#[derive(Debug)]
pub enum ReceivedFrame {
    /// A parsed SIP message paired with the original wire bytes that
    /// the parser consumed. The bytes are preserved end-to-end so
    /// byte-exact consumers (STIR/SHAKEN Identity per RFC 8224,
    /// signature-preserving SBCs, replay tools) can recover the
    /// upstream form without re-serializing. See `SIP_API_DESIGN_2.md`
    /// §7.5.
    Message(Message, bytes::Bytes),
    /// RFC 5626 §3.5.1 keep-alive pong — a single `\r\n` received.
    KeepAlivePong,
    /// RFC 5626 §3.5.1 server-initiated ping — `\r\n\r\n` at offset 0
    /// of the buffer. The receiver should answer with a single CRLF.
    KeepAlivePing,
}

/// TCP connection for SIP messages.
///
/// The underlying `TcpStream` is split into owned read and write halves
/// (`tokio::net::TcpStream::into_split`) so concurrent reads (the
/// per-connection reader task) and writes (outbound `send_message` /
/// `send_raw_bytes`) never contend on a single mutex. Required for
/// bidirectional SIP-over-TCP and for RFC 5626 §3.5.1 keep-alive,
/// where a ping task writes while the reader simultaneously awaits a
/// pong.
pub struct TcpConnection {
    /// Owned write half. Held under a mutex so concurrent senders
    /// serialise their writes (RFC 3261 §7.5 requires atomic message
    /// delivery over stream transports).
    write_half: Mutex<OwnedWriteHalf>,
    /// Owned read half. Expected to be consumed by a single reader
    /// task; concurrent `receive_frame` callers serialise via the
    /// mutex but that usage pattern is not recommended.
    read_half: Mutex<OwnedReadHalf>,
    /// Cached local address (captured at construction; doesn't change
    /// once the socket is bound).
    local_addr: SocketAddr,
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
        Self::from_stream(stream, addr)
    }

    /// Creates a TCP connection from an existing stream
    pub fn from_stream(stream: TcpStream, peer_addr: SocketAddr) -> Result<Self> {
        let local_addr = stream.local_addr().map_err(Error::LocalAddrFailed)?;
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            write_half: Mutex::new(write_half),
            read_half: Mutex::new(read_half),
            local_addr,
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
        Ok(self.local_addr)
    }

    /// Sends a SIP message over the connection
    pub async fn send_message(&self, message: &Message) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        validate_typed_outbound_message(message)?;

        let message_bytes = message.to_bytes();
        let mut writer = self.write_half.lock().await;

        writer.write_all(&message_bytes).await.map_err(|e| {
            if e.kind() == io::ErrorKind::BrokenPipe || e.kind() == io::ErrorKind::ConnectionReset {
                self.closed.store(true, Ordering::Relaxed);
                Error::ConnectionReset
            } else {
                Error::SendFailed(self.peer_addr, e)
            }
        })?;

        writer
            .flush()
            .await
            .map_err(|e| Error::SendFailed(self.peer_addr, e))?;

        trace!("Sent {} bytes to {}", message_bytes.len(), self.peer_addr);
        Ok(())
    }

    /// Writes raw bytes over the connection without any SIP framing.
    /// Used for RFC 5626 §3.5.1 CRLFCRLF keep-alive pings / CRLF pongs.
    /// Mirrors `send_message` for error handling — a broken pipe marks
    /// the connection closed so the next send fails fast.
    pub async fn send_raw_bytes(&self, data: &[u8]) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        let mut writer = self.write_half.lock().await;

        writer.write_all(data).await.map_err(|e| {
            if e.kind() == io::ErrorKind::BrokenPipe || e.kind() == io::ErrorKind::ConnectionReset {
                self.closed.store(true, Ordering::Relaxed);
                Error::ConnectionReset
            } else {
                Error::SendFailed(self.peer_addr, e)
            }
        })?;

        writer
            .flush()
            .await
            .map_err(|e| Error::SendFailed(self.peer_addr, e))?;

        trace!("Sent {} raw bytes to {}", data.len(), self.peer_addr);
        Ok(())
    }

    /// Receives a SIP message from the connection.
    ///
    /// Legacy accessor retained for callers that only care about SIP
    /// messages (e.g. existing unit tests). RFC 5626 keep-alive frames
    /// (CRLF pong, CRLFCRLF server ping) are silently consumed and
    /// skipped — use [`receive_frame`](Self::receive_frame) to observe
    /// them.
    pub async fn receive_message(&self) -> Result<Option<Message>> {
        loop {
            match self.receive_frame().await? {
                Some(ReceivedFrame::Message(m, _bytes)) => return Ok(Some(m)),
                Some(ReceivedFrame::KeepAlivePong) | Some(ReceivedFrame::KeepAlivePing) => {
                    // Silently skip — legacy callers aren't aware of
                    // RFC 5626 frames. New code should call
                    // `receive_frame` directly.
                    continue;
                }
                None => return Ok(None),
            }
        }
    }

    /// Receives a frame from the connection. A frame is either a
    /// parsed SIP message or one of the RFC 5626 §3.5.1 keep-alive
    /// frames (CRLF pong, CRLFCRLF server-initiated ping).
    ///
    /// The CRLF / CRLFCRLF frames are only recognised at the *start* of
    /// the receive buffer — embedded CRLF sequences between stacked SIP
    /// messages are treated as ordinary message framing (existing RFC
    /// 3261 §18.3 Content-Length behaviour, unchanged).
    pub async fn receive_frame(&self) -> Result<Option<ReceivedFrame>> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        // Acquire locks for the buffer and read half
        let mut recv_buffer = self.recv_buffer.lock().await;
        let mut reader = self.read_half.lock().await;

        loop {
            // RFC 5626 §3.5.1: keep-alive frames only legal at buffer
            // offset 0 (a SIP message must start with a request- or
            // status-line, never CRLF). `\r\n\r\n` is a server ping,
            // bare `\r\n` is a pong. TCP doesn't split these atomic
            // writes in practice, so when the buffer begins with CRLF
            // we treat it as a complete frame right away — the only
            // genuine ambiguity (buffer is exactly 2 bytes of CRLF)
            // resolves itself correctly: we emit a pong, and if the
            // peer actually sent a ping the next 2 bytes arrive, get
            // classified as a second pong, and no caller cares because
            // we don't act on server-initiated pings anyway.
            if recv_buffer.len() >= 4 && &recv_buffer[0..4] == b"\r\n\r\n" {
                recv_buffer.advance(4);
                return Ok(Some(ReceivedFrame::KeepAlivePing));
            }
            if recv_buffer.len() >= 2 && &recv_buffer[0..2] == b"\r\n" {
                recv_buffer.advance(2);
                return Ok(Some(ReceivedFrame::KeepAlivePong));
            }
            if let Some((frame, raw_bytes)) = self.try_parse_message(&mut recv_buffer)? {
                return Ok(Some(ReceivedFrame::Message(frame, raw_bytes)));
            }

            // No complete frame, read more data
            let mut temp_buffer = vec![0; 8192];

            match reader.read(&mut temp_buffer).await {
                Ok(0) => {
                    // End of stream
                    if recv_buffer.is_empty() {
                        self.closed.store(true, Ordering::Relaxed);
                        return Ok(None);
                    } else {
                        self.closed.store(true, Ordering::Relaxed);
                        return Err(Error::StreamClosed);
                    }
                }
                Ok(n) => {
                    trace!("Read {} bytes from {}", n, self.peer_addr);

                    recv_buffer.put_slice(&temp_buffer[0..n]);
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        continue;
                    } else {
                        self.closed.store(true, Ordering::Relaxed);

                        if e.kind() == io::ErrorKind::BrokenPipe
                            || e.kind() == io::ErrorKind::ConnectionReset
                        {
                            return Err(Error::ConnectionReset);
                        } else {
                            return Err(Error::ReceiveFailed(e));
                        }
                    }
                }
            }
        }
    }

    /// Tries to parse a SIP message from the buffer, returning both the
    /// parsed [`Message`] and a frozen [`Bytes`] snapshot of the wire
    /// bytes that the parser consumed. The Bytes copy lets downstream
    /// code preserve the upstream byte form (STIR/SHAKEN, SBC
    /// pass-through) without re-serializing.
    fn try_parse_message(&self, buffer: &mut BytesMut) -> Result<Option<(Message, bytes::Bytes)>> {
        if buffer.is_empty() {
            return Ok(None);
        }

        let frame = match inspect_sip_frame(buffer) {
            Ok(SipFrameStatus::Incomplete { .. }) => return Ok(None),
            Ok(SipFrameStatus::Complete(frame)) => frame,
            Err(error) => {
                self.closed.store(true, Ordering::Relaxed);
                warn!(
                    error_class = error.class(),
                    "Rejecting malformed SIP TCP frame"
                );
                return Err(Error::ParseError(error.to_string()));
            }
        };
        let message_slice = &buffer[..frame.total_bytes];
        let raw_bytes = bytes::Bytes::copy_from_slice(message_slice);
        let message = parse_message(message_slice).map_err(|error| {
            self.closed.store(true, Ordering::Relaxed);
            warn!(
                error_class = "sip-syntax",
                "Rejecting malformed SIP TCP message"
            );
            Error::ParseError(error.to_string())
        })?;

        trace!("Parsed complete SIP message ({} bytes)", frame.total_bytes);
        buffer.advance(frame.total_bytes);
        Ok(Some((message, raw_bytes)))
    }

    /// Closes the TCP connection
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::Relaxed) {
            // Already closed
            return Ok(());
        }

        // Shutting down the write half closes the socket from both
        // directions (read half will return EOF on its next poll).
        let mut writer = self.write_half.lock().await;
        if let Err(e) = writer.shutdown().await {
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

impl Drop for TcpConnection {
    fn drop(&mut self) {
        if !self.is_closed() {
            // The connection is being dropped without being closed
            debug!(
                "TCP connection to {} dropped without being closed",
                self.peer_addr
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::{ContentLengthBuilderExt, SimpleRequestBuilder};
    use rvoip_sip_core::framing::{MAX_SIP_BODY_BYTES, MAX_SIP_HEADER_BYTES};
    use rvoip_sip_core::{CallId, Method, Request, Response, StatusCode, TypedHeader, Uri};
    use tokio::net::TcpListener;

    async fn buffered_test_connection() -> (TcpConnection, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).await.unwrap();
        let (server, peer) = listener.accept().await.unwrap();
        (TcpConnection::from_stream(server, peer).unwrap(), client)
    }

    fn minimal_request(content_length_headers: &[u8], body: &[u8]) -> Vec<u8> {
        let mut message = b"OPTIONS sip:service.example SIP/2.0\r\n".to_vec();
        message.extend_from_slice(content_length_headers);
        message.extend_from_slice(b"\r\n\r\n");
        message.extend_from_slice(body);
        message
    }

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
    async fn typed_direct_connection_rejects_unsafe_serialized_fields_before_socket_io() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move { listener.accept().await.unwrap().0 });
        let client = TcpStream::connect(server_addr).await.unwrap();
        let mut server = accept.await.unwrap();
        let connection = TcpConnection::from_stream(client, server_addr).unwrap();
        let mut unsafe_header = Request::new(Method::Options, Uri::sip("example.test"));
        unsafe_header.headers.push(TypedHeader::CallId(CallId::new(
            "safe\r\nX-Injected: direct-header-secret",
        )));
        let unsafe_method = Request::new(
            Method::Extension("SAFE\r\nX-Injected: direct-method-secret".into()),
            Uri::sip("example.test"),
        );
        for message in [
            Message::Response(
                Response::new(StatusCode::Ok).with_reason("OK\r\nX-Injected: direct-reason-secret"),
            ),
            Message::Request(unsafe_header),
            Message::Request(unsafe_method),
        ] {
            let error = connection
                .send_message(&message)
                .await
                .expect_err("typed direct send must fail closed");
            assert!(matches!(error, Error::ProtocolError(_)));
            for secret in [
                "direct-reason-secret",
                "direct-header-secret",
                "direct-method-secret",
            ] {
                assert!(!error.to_string().contains(secret));
            }
        }
        let mut buffer = [0u8; 64];
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                server.read(&mut buffer)
            )
            .await
            .is_err(),
            "rejected typed direct send must write no bytes",
        );

        let raw = b"X-Verbatim: direct-raw-retained\r\n";
        connection.send_raw_bytes(raw).await.unwrap();
        server.read_exact(&mut buffer[..raw.len()]).await.unwrap();
        assert_eq!(&buffer[..raw.len()], raw);
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

    #[tokio::test]
    async fn tcp_framing_accepts_compact_content_length() {
        let (connection, _peer) = buffered_test_connection().await;
        let message = minimal_request(b"L: 4", b"body");
        let mut buffer = BytesMut::from(message.as_slice());

        let (parsed, raw) = connection
            .try_parse_message(&mut buffer)
            .unwrap()
            .expect("compact Content-Length frame");
        assert!(buffer.is_empty());
        assert_eq!(raw.as_ref(), message);
        let Message::Request(request) = parsed else {
            panic!("request expected");
        };
        assert_eq!(request.body(), b"body");
        assert!(!connection.is_closed());
    }

    #[tokio::test]
    async fn tcp_rejects_all_ambiguous_content_lengths_without_consuming_following_request() {
        let valid_second = minimal_request(b"Content-Length: 0", b"");
        for (headers, class) in [
            (b"Via: missing-length".as_slice(), "missing-content-length"),
            (
                b"Content-Length: 0\r\nContent-Length: 0".as_slice(),
                "duplicate-content-length",
            ),
            (
                b"Content-Length: 0\r\nl: 1".as_slice(),
                "duplicate-content-length",
            ),
            (
                b"L: 1\r\nCONTENT-LENGTH: 0".as_slice(),
                "duplicate-content-length",
            ),
            (b"Content-Length: nope".as_slice(), "invalid-content-length"),
            (b"l: \xff".as_slice(), "invalid-content-length"),
            (
                b"Content-Length: 184467440737095516160".as_slice(),
                "content-length-overflow",
            ),
        ] {
            let (connection, _peer) = buffered_test_connection().await;
            let mut bytes = minimal_request(headers, b"");
            bytes.extend_from_slice(&valid_second);
            let original = bytes.clone();
            let mut buffer = BytesMut::from(bytes.as_slice());

            let error = connection
                .try_parse_message(&mut buffer)
                .expect_err("ambiguous frame must be rejected");
            assert!(
                matches!(&error, Error::ParseError(detail) if detail.contains(class)),
                "unexpected framing class: {error}"
            );
            assert_eq!(buffer.as_ref(), original);
            assert!(connection.is_closed());
            assert!(matches!(
                connection.try_parse_message(&mut buffer),
                Err(Error::ParseError(_))
            ));
        }
    }

    #[tokio::test]
    async fn tcp_rejects_slow_endless_headers_and_huge_bodies_at_shared_bounds() {
        let (connection, _peer) = buffered_test_connection().await;
        let mut slow =
            BytesMut::from(&b"OPTIONS sip:service.example SIP/2.0\r\nX-Fold: value\r\n"[..]);
        let mut continuation = vec![b'a'; 1_022];
        continuation[0] = b' ';
        continuation.extend_from_slice(b"\r\n");
        while slow.len() <= MAX_SIP_HEADER_BYTES {
            slow.extend_from_slice(&continuation);
            if slow.len() <= MAX_SIP_HEADER_BYTES {
                assert!(connection.try_parse_message(&mut slow).unwrap().is_none());
            }
        }
        let error = connection.try_parse_message(&mut slow).unwrap_err();
        assert!(matches!(
            &error,
            Error::ParseError(detail) if detail.contains("header-too-large")
        ));
        assert!(connection.is_closed());

        let (connection, _peer) = buffered_test_connection().await;
        let huge = minimal_request(
            format!("Content-Length: {}", MAX_SIP_BODY_BYTES + 1).as_bytes(),
            b"",
        );
        let mut buffer = BytesMut::from(huge.as_slice());
        let error = connection.try_parse_message(&mut buffer).unwrap_err();
        assert!(matches!(
            &error,
            Error::ParseError(detail) if detail.contains("body-too-large")
        ));
        assert_eq!(buffer.as_ref(), huge);
        assert!(connection.is_closed());
    }

    /// RFC 5626 §3.5.1: a bare `\r\n` at buffer offset 0 is a keep-alive
    /// pong. It must be consumed as a `KeepAlivePong` frame, not handed
    /// to the SIP parser (which would reject it).
    #[tokio::test]
    async fn keepalive_pong_at_offset_0_is_recognised_as_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();

        // Server: accept then write a single CRLF (pong) and nothing else.
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.write_all(b"\r\n").await.unwrap();
            socket.flush().await.unwrap();
            // Hold the socket open briefly so the client reads the bytes
            // before seeing EOF.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        });

        let connection = TcpConnection::connect(server_addr).await.unwrap();
        let frame = connection.receive_frame().await.unwrap();
        assert!(matches!(frame, Some(ReceivedFrame::KeepAlivePong)));
    }

    /// RFC 5626 §3.5.1: a `\r\n\r\n` at buffer offset 0 is a server-
    /// initiated ping. A SIP message *never* starts with CRLFCRLF (must
    /// start with request- or status-line), so the detection is
    /// unambiguous.
    #[tokio::test]
    async fn keepalive_ping_at_offset_0_is_recognised_as_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.write_all(b"\r\n\r\n").await.unwrap();
            socket.flush().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        });

        let connection = TcpConnection::connect(server_addr).await.unwrap();
        let frame = connection.receive_frame().await.unwrap();
        assert!(matches!(frame, Some(ReceivedFrame::KeepAlivePing)));
    }

    /// RFC 5626 keep-alive frames must not disturb subsequent SIP
    /// message parsing. The pong is stripped; the SIP message that
    /// follows in the same TCP read is parsed cleanly with no spurious
    /// parse errors.
    #[tokio::test]
    async fn keepalive_pong_followed_by_sip_message_parses_both() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let req = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
                .unwrap()
                .from("alice", "sip:alice@example.com", Some("tag1"))
                .to("bob", "sip:bob@example.com", None)
                .call_id("after-pong@example.com")
                .cseq(1)
                .content_length(0)
                .build();

            let mut combined = BytesMut::new();
            combined.extend_from_slice(b"\r\n"); // pong
            combined.extend_from_slice(&Message::Request(req).to_bytes());
            socket.write_all(&combined).await.unwrap();
            socket.flush().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        });

        let connection = TcpConnection::connect(server_addr).await.unwrap();
        // First frame is the pong.
        let first = connection.receive_frame().await.unwrap();
        assert!(matches!(first, Some(ReceivedFrame::KeepAlivePong)));
        // Second frame is the SIP message — must parse cleanly.
        let second = connection.receive_frame().await.unwrap();
        match second {
            Some(ReceivedFrame::Message(Message::Request(req), _raw)) => {
                assert_eq!(req.method(), Method::Register);
                assert_eq!(req.call_id().unwrap().to_string(), "after-pong@example.com");
            }
            other => panic!("Expected SIP request after pong, got {:?}", other),
        }
    }

    /// `send_raw_bytes` writes bytes verbatim with no framing.
    #[tokio::test]
    async fn send_raw_bytes_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16];
            let n = socket.read(&mut buf).await.unwrap();
            buf.truncate(n);
            tx.send(buf).await.unwrap();
        });

        let connection = TcpConnection::connect(server_addr).await.unwrap();
        connection.send_raw_bytes(b"\r\n\r\n").await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received, b"\r\n\r\n");

        connection.close().await.unwrap();
    }
}
