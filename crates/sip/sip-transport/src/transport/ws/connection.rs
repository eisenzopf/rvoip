use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::{AbortHandle, JoinHandle};
#[cfg(feature = "ws")]
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
#[cfg(feature = "ws")]
use tokio_tungstenite::{tungstenite, WebSocketStream};
use tracing::{debug, trace, warn};

#[cfg(feature = "ws")]
use super::SipWsStream;

use crate::error::{Error, Result};
use crate::transport::{validate_typed_outbound_message, TransportConnectionMetadata};
use rvoip_sip_core::{parse_message_with_mode, Message, ParseMode};

// SIP WebSocket subprotocol name as registered by RFC 7118. WS and WSS both
// negotiate `sip`; TLS is represented by the URI scheme.
#[cfg(test)]
const SIP_WS_SUBPROTOCOL: &str = "sip";

// RFC 7118 carries exactly one complete SIP message in each WebSocket
// message. Bound both the WebSocket codec and this application boundary so a
// peer cannot make every connection reserve tungstenite's 64 MiB default.
pub(super) const MAX_MESSAGE_SIZE: usize = 65_535;
const DEFAULT_WRITER_QUEUE_CAPACITY: usize = 64;
const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const WRITER_OPEN: u8 = 0;
const WRITER_CLOSING: u8 = 1;
const WRITER_CLOSED: u8 = 2;

#[cfg(feature = "ws")]
pub(super) fn sip_websocket_config() -> tungstenite::protocol::WebSocketConfig {
    tungstenite::protocol::WebSocketConfig::default()
        .read_buffer_size(16 * 1024)
        .write_buffer_size(16 * 1024)
        .max_write_buffer_size(MAX_MESSAGE_SIZE * 2)
        .max_message_size(Some(MAX_MESSAGE_SIZE))
        .max_frame_size(Some(MAX_MESSAGE_SIZE))
}

/// WebSocket connection for SIP messages
pub struct WebSocketConnection {
    #[cfg(feature = "ws")]
    writer_tx: mpsc::Sender<WriterCommand>,
    #[cfg(feature = "ws")]
    writer_task: Mutex<Option<JoinHandle<()>>>,
    #[cfg(feature = "ws")]
    writer_abort: AbortHandle,
    #[cfg(feature = "ws")]
    writer_state: Arc<AtomicU8>,
    #[cfg(feature = "ws")]
    writer_closed: watch::Sender<bool>,
    #[cfg(feature = "ws")]
    activity: watch::Sender<tokio::time::Instant>,
    #[cfg(feature = "ws")]
    write_timeout: Duration,
    /// The peer's address
    peer_addr: SocketAddr,
    /// Whether the connection is closed
    #[cfg(not(feature = "ws"))]
    closed: std::sync::atomic::AtomicBool,
    /// Whether this is a secure WebSocket connection
    secure: bool,
    /// The selected RFC 7118 subprotocol (`sip` for both WS and WSS)
    subprotocol: String,
    /// Verified TLS client identity retained for every message on an inbound
    /// WSS connection.
    connection_metadata: Option<TransportConnectionMetadata>,
}

#[cfg(feature = "ws")]
enum WriterCommand {
    Send {
        message: WsMessage,
        deadline: tokio::time::Instant,
        reply: oneshot::Sender<Result<()>>,
    },
    Close(oneshot::Sender<Result<()>>),
    PeerClose(oneshot::Sender<Result<()>>),
}

impl WebSocketConnection {
    /// Creates a WebSocket connection from an existing WebSocket stream
    #[cfg(feature = "ws")]
    pub fn from_writer(
        ws_writer: SplitSink<WebSocketStream<SipWsStream>, WsMessage>,
        peer_addr: SocketAddr,
        secure: bool,
        subprotocol: String,
    ) -> Self {
        Self::from_writer_with_runtime(
            ws_writer,
            peer_addr,
            secure,
            subprotocol,
            None,
            DEFAULT_WRITER_QUEUE_CAPACITY,
            DEFAULT_WRITE_TIMEOUT,
        )
    }

    #[cfg(feature = "ws")]
    pub(crate) fn from_writer_with_runtime(
        ws_writer: SplitSink<WebSocketStream<SipWsStream>, WsMessage>,
        peer_addr: SocketAddr,
        secure: bool,
        subprotocol: String,
        connection_metadata: Option<TransportConnectionMetadata>,
        writer_queue_capacity: usize,
        write_timeout: Duration,
    ) -> Self {
        let (writer_tx, writer_rx) = mpsc::channel(writer_queue_capacity.max(1));
        let writer_state = Arc::new(AtomicU8::new(WRITER_OPEN));
        let (writer_closed, _) = watch::channel(false);
        let (activity, _) = watch::channel(tokio::time::Instant::now());
        let task = tokio::spawn(writer_loop(
            ws_writer,
            writer_rx,
            peer_addr,
            write_timeout,
            writer_state.clone(),
            writer_closed.clone(),
            activity.clone(),
        ));
        let writer_abort = task.abort_handle();
        Self {
            writer_tx,
            writer_task: Mutex::new(Some(task)),
            writer_abort,
            writer_state,
            writer_closed,
            activity,
            write_timeout,
            peer_addr,
            secure,
            subprotocol,
            connection_metadata,
        }
    }

    /// Returns the peer address of the connection
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    /// Returns the local address of the connection
    pub fn local_addr(&self) -> Result<SocketAddr> {
        // WebSocket connections don't directly expose the local address
        // Would need to be tracked separately when the connection is created
        Err(Error::NotImplemented(
            "Getting local address from WebSocket connection".into(),
        ))
    }

    /// Returns whether this is a secure connection
    pub fn is_secure(&self) -> bool {
        self.secure
    }

    /// Returns the selected subprotocol
    pub fn subprotocol(&self) -> &str {
        &self.subprotocol
    }

    /// Verified connection identity, if this is an inbound mutually
    /// authenticated WSS connection.
    pub fn connection_metadata(&self) -> Option<&TransportConnectionMetadata> {
        self.connection_metadata.as_ref()
    }

    /// Sends a SIP message over the WebSocket connection
    #[cfg(feature = "ws")]
    pub async fn send_message(&self, message: &Message) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        validate_typed_outbound_message(message)?;

        // Convert SIP message to bytes
        let message_bytes = message.to_bytes();
        if message_bytes.len() > MAX_MESSAGE_SIZE {
            return Err(Error::WebSocketProtocolError(
                "SIP WebSocket message exceeds the configured size limit".to_string(),
            ));
        }

        // RFC 7118 permits Text and Binary messages. Never use lossy UTF-8
        // conversion: a SIP body can contain arbitrary octets and changing
        // one byte can invalidate signatures or corrupt media metadata.
        let ws_message = match String::from_utf8(message_bytes) {
            Ok(text) => WsMessage::Text(text.into()),
            Err(error) => WsMessage::Binary(bytes::Bytes::from(error.into_bytes())),
        };

        self.send_writer_message(ws_message).await?;

        trace!("Sent SIP message over WebSocket to {}", self.peer_addr);
        Ok(())
    }

    /// Send pre-built SIP-formatted bytes verbatim over the WebSocket
    /// connection as a Binary frame. The caller has already produced
    /// wire-format bytes (e.g., copied from an inbound `raw_bytes` for
    /// SBC pass-through); we don't re-canonicalise or re-frame.
    #[cfg(feature = "ws")]
    pub async fn send_raw_bytes(&self, bytes: bytes::Bytes) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        if bytes.len() > MAX_MESSAGE_SIZE {
            return Err(Error::WebSocketProtocolError(
                "SIP WebSocket message exceeds the configured size limit".to_string(),
            ));
        }

        // RFC 7118 §5 allows either Text or Binary; choose Binary so we
        // don't re-validate as UTF-8 (the bytes may be arbitrary SIP
        // payload from a previous capture).
        let ws_message = WsMessage::Binary(bytes);

        self.send_writer_message(ws_message).await
    }

    /// Send a WebSocket-native keepalive ping. RFC 7118 carries SIP as
    /// complete WebSocket messages, so RFC 5626 CRLF bytes must not be framed
    /// as Binary SIP payload. The matching Pong is surfaced by the transport
    /// reader as an exact-flow lifecycle event.
    #[cfg(feature = "ws")]
    pub(crate) async fn send_keepalive_ping(&self) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        self.send_writer_message(WsMessage::Ping(bytes::Bytes::new()))
            .await
    }

    /// Processes a WebSocket message and attempts to parse it as a SIP message.
    /// Returns the parsed [`Message`] paired with a frozen [`bytes::Bytes`]
    /// snapshot of the wire bytes (text or binary frame body) the parser
    /// consumed. The snapshot is preserved end-to-end for byte-exact
    /// consumers (STIR/SHAKEN, signature-preserving SBC).
    #[cfg(feature = "ws")]
    pub fn process_ws_message(
        &self,
        ws_message: WsMessage,
    ) -> Result<Option<(Message, bytes::Bytes)>> {
        self.activity.send_replace(tokio::time::Instant::now());
        match ws_message {
            WsMessage::Text(text) => {
                // RFC 7118 section 7: SIP messages are sent as text frames
                trace!(
                    "Received text message over WebSocket from {}",
                    self.peer_addr
                );

                if text.len() > MAX_MESSAGE_SIZE {
                    return Err(Error::WebSocketProtocolError(
                        "SIP WebSocket message exceeds the configured size limit".to_string(),
                    ));
                }

                // Snapshot the wire bytes before parsing so the upstream
                // form rides through the bus unchanged.
                let raw_bytes = bytes::Bytes::copy_from_slice(text.as_bytes());
                match parse_message_with_mode(&raw_bytes, ParseMode::Strict) {
                    Ok(message) => Ok(Some((message, raw_bytes))),
                    Err(e) => {
                        warn!("Failed to parse SIP message from WebSocket: {}", e);
                        Err(Error::ParseError(e.to_string()))
                    }
                }
            }
            WsMessage::Binary(data) => {
                // Some implementations might send binary data instead of text
                trace!(
                    "Received binary message over WebSocket from {}",
                    self.peer_addr
                );

                if data.len() > MAX_MESSAGE_SIZE {
                    return Err(Error::WebSocketProtocolError(
                        "SIP WebSocket message exceeds the configured size limit".to_string(),
                    ));
                }

                // Tungstenite already owns Binary payloads as Bytes; retain
                // that allocation instead of copying the whole SIP message.
                let raw_bytes = data;
                match parse_message_with_mode(&raw_bytes, ParseMode::Strict) {
                    Ok(message) => Ok(Some((message, raw_bytes))),
                    Err(e) => {
                        warn!("Failed to parse SIP message from WebSocket binary: {}", e);
                        Err(Error::ParseError(e.to_string()))
                    }
                }
            }
            WsMessage::Ping(_) => {
                // Ping messages should be automatically handled by the WebSocket library
                trace!("Received ping from {}", self.peer_addr);
                Ok(None)
            }
            WsMessage::Pong(_) => {
                // Pong messages are responses to our pings
                trace!("Received pong from {}", self.peer_addr);
                Ok(None)
            }
            WsMessage::Close(_) => {
                debug!("Received close frame from {}", self.peer_addr);
                // A peer Close is also a writer-lifecycle transition. Merely
                // setting `WRITER_CLOSING` leaves the writer task blocked on
                // its command queue, so the reader supervisor cannot release
                // established-connection capacity until the full write
                // timeout expires. Queue the close handshake now (or abort a
                // saturated writer) exactly as a local close does.
                self.request_writer_peer_close();
                Ok(None)
            }
            WsMessage::Frame(_) => {
                // Raw frames should not be received with tungstenite
                warn!("Received unexpected raw frame from {}", self.peer_addr);
                Ok(None)
            }
        }
    }

    /// Closes the WebSocket connection
    #[cfg(feature = "ws")]
    pub async fn close(&self) -> Result<()> {
        self.request_writer_close();

        let mut task_slot = self.writer_task.lock().await;
        if let Some(mut task) = task_slot.take() {
            if tokio::time::timeout(self.write_timeout, &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        } else if !*self.writer_closed.borrow() {
            let mut closed = self.writer_closed.subscribe();
            if tokio::time::timeout(self.write_timeout, closed.changed())
                .await
                .is_err()
            {
                self.writer_abort.abort();
            }
        }
        self.writer_state.store(WRITER_CLOSED, Ordering::Release);
        self.writer_closed.send_replace(true);
        Ok(())
    }

    /// Returns whether the connection is closed
    pub fn is_closed(&self) -> bool {
        self.writer_state.load(Ordering::Acquire) != WRITER_OPEN
    }

    #[cfg(feature = "ws")]
    pub(crate) fn activity_receiver(&self) -> watch::Receiver<tokio::time::Instant> {
        self.activity.subscribe()
    }

    #[cfg(feature = "ws")]
    pub(crate) fn writer_closed_receiver(&self) -> watch::Receiver<bool> {
        self.writer_closed.subscribe()
    }

    #[cfg(feature = "ws")]
    fn request_writer_close(&self) {
        self.request_writer_shutdown(false);
    }

    #[cfg(feature = "ws")]
    fn request_writer_peer_close(&self) {
        self.request_writer_shutdown(true);
    }

    #[cfg(feature = "ws")]
    fn request_writer_shutdown(&self, peer_initiated: bool) {
        if self
            .writer_state
            .compare_exchange(
                WRITER_OPEN,
                WRITER_CLOSING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }

        let (reply, _ignored) = oneshot::channel();
        let command = if peer_initiated {
            WriterCommand::PeerClose(reply)
        } else {
            WriterCommand::Close(reply)
        };
        if self.writer_tx.try_send(command).is_err() {
            // A saturated writer queue must not hold peer-close handling or
            // local drain hostage. Aborting drops the split sink; the reader
            // supervisor then drops its half and releases the socket permit.
            self.writer_abort.abort();
            self.writer_state.store(WRITER_CLOSED, Ordering::Release);
            self.writer_closed.send_replace(true);
        }
    }

    #[cfg(feature = "ws")]
    async fn send_writer_message(&self, message: WsMessage) -> Result<()> {
        if self.writer_state.load(Ordering::Acquire) != WRITER_OPEN {
            return Err(Error::TransportClosed);
        }
        let deadline = tokio::time::Instant::now() + self.write_timeout;
        let (reply, result) = oneshot::channel();
        self.writer_tx
            .try_send(WriterCommand::Send {
                message,
                deadline,
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => Error::BufferCapacityExceeded,
                mpsc::error::TrySendError::Closed(_) => Error::TransportClosed,
            })?;
        tokio::time::timeout_at(deadline, result)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::TransportClosed)?
    }
}

#[cfg(feature = "ws")]
async fn writer_loop(
    mut writer: SplitSink<WebSocketStream<SipWsStream>, WsMessage>,
    mut commands: mpsc::Receiver<WriterCommand>,
    peer_addr: SocketAddr,
    write_timeout: Duration,
    state: Arc<AtomicU8>,
    closed: watch::Sender<bool>,
    activity: watch::Sender<tokio::time::Instant>,
) {
    while let Some(command) = commands.recv().await {
        match command {
            WriterCommand::Send {
                message,
                deadline,
                reply,
            } => {
                // A caller that timed out or was cancelled must not leave a
                // queued SIP mutation that executes later and is then retried.
                if reply.is_closed() || tokio::time::Instant::now() >= deadline {
                    continue;
                }
                let result = match tokio::time::timeout_at(deadline, writer.send(message)).await {
                    Ok(Ok(())) => {
                        activity.send_replace(tokio::time::Instant::now());
                        Ok(())
                    }
                    Ok(Err(error)) => Err(map_writer_error(peer_addr, error)),
                    Err(_) => Err(Error::Timeout),
                };
                let failed = result.is_err();
                let _ = reply.send(result);
                if failed {
                    break;
                }
            }
            WriterCommand::Close(reply) => {
                let result = match tokio::time::timeout(write_timeout, async {
                    writer.send(WsMessage::Close(None)).await?;
                    writer.close().await
                })
                .await
                {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => Err(map_writer_error(peer_addr, error)),
                    Err(_) => Err(Error::Timeout),
                };
                let _ = reply.send(result);
                break;
            }
            WriterCommand::PeerClose(reply) => {
                // Reading the peer Close makes tungstenite queue the reply.
                // Closing the sink flushes that queued handshake; attempting
                // to send a second Close first is rejected as "already
                // closing" and can reset the socket before the acknowledgement.
                let result = match tokio::time::timeout(write_timeout, writer.close()).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => Err(map_writer_error(peer_addr, error)),
                    Err(_) => Err(Error::Timeout),
                };
                let _ = reply.send(result);
                break;
            }
        }
    }
    state.store(WRITER_CLOSED, Ordering::Release);
    closed.send_replace(true);
}

#[cfg(feature = "ws")]
fn map_writer_error(peer_addr: SocketAddr, error: tungstenite::Error) -> Error {
    match error {
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
            Error::ConnectionClosedByPeer(peer_addr)
        }
        tungstenite::Error::Protocol(message) => Error::WebSocketProtocolError(message.to_string()),
        tungstenite::Error::Io(error)
            if matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
            ) =>
        {
            Error::ConnectionReset
        }
        tungstenite::Error::Io(error) => Error::SendFailed(peer_addr, error),
        other => Error::SendFailed(
            peer_addr,
            io::Error::new(io::ErrorKind::Other, other.to_string()),
        ),
    }
}

#[cfg(not(feature = "ws"))]
impl WebSocketConnection {
    /// Creates a WebSocket connection from an existing WebSocket stream
    pub fn from_writer(
        _writer: (),
        peer_addr: SocketAddr,
        secure: bool,
        subprotocol: String,
    ) -> Self {
        Self {
            peer_addr,
            closed: std::sync::atomic::AtomicBool::new(false),
            secure,
            subprotocol,
            connection_metadata: None,
        }
    }

    /// Sends a SIP message over the WebSocket connection
    pub async fn send_message(&self, _message: &Message) -> Result<()> {
        Err(Error::NotImplemented(
            "WebSocket support is not enabled".into(),
        ))
    }

    /// Closes the WebSocket connection
    pub async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for WebSocketConnection {
    fn drop(&mut self) {
        #[cfg(feature = "ws")]
        {
            self.writer_abort.abort();
        }
        if !self.is_closed() {
            // The connection is being dropped without being closed
            debug!(
                "WebSocket connection to {} dropped without being closed",
                self.peer_addr
            );
        }
    }
}

// Unit tests will be added later

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "ws")]
    use futures_util::StreamExt;
    use rvoip_sip_core::{
        CallId, Message, Method, Request, Response, StatusCode, TypedHeader, Uri,
    };
    use std::sync::atomic::AtomicBool;
    use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn blocked_writer_cannot_hold_connection_drain() {
        use futures_util::StreamExt as _;
        use tokio_tungstenite::tungstenite::protocol::Role;

        let (transport_side, _blocked_peer) = tokio::io::duplex(1);
        let stream = WebSocketStream::from_raw_socket(
            SipWsStream::Test(transport_side),
            Role::Client,
            Some(sip_websocket_config()),
        )
        .await;
        let (writer, _reader) = stream.split();
        let connection = Arc::new(WebSocketConnection::from_writer_with_runtime(
            writer,
            "127.0.0.1:5060".parse().unwrap(),
            false,
            "sip".into(),
            None,
            1,
            Duration::from_millis(40),
        ));
        let sending = {
            let connection = connection.clone();
            tokio::spawn(async move {
                connection
                    .send_raw_bytes(bytes::Bytes::from(vec![b'x'; 4_096]))
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(5)).await;

        tokio::time::timeout(Duration::from_millis(200), connection.close())
            .await
            .expect("blocked WebSocket writer held drain")
            .unwrap();
        assert!(connection.is_closed());
        assert!(sending.await.unwrap().is_err());
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn writer_failure_notifies_reader_supervision_immediately() {
        use futures_util::StreamExt as _;
        use tokio_tungstenite::tungstenite::protocol::Role;

        let (transport_side, peer) = tokio::io::duplex(64);
        let stream = WebSocketStream::from_raw_socket(
            SipWsStream::Test(transport_side),
            Role::Client,
            Some(sip_websocket_config()),
        )
        .await;
        let (writer, _reader) = stream.split();
        let connection = WebSocketConnection::from_writer_with_runtime(
            writer,
            "127.0.0.1:5060".parse().unwrap(),
            false,
            "sip".into(),
            None,
            1,
            Duration::from_millis(100),
        );
        let mut writer_closed = connection.writer_closed_receiver();
        drop(peer);

        assert!(connection
            .send_raw_bytes(bytes::Bytes::from_static(b"dead peer"))
            .await
            .is_err());
        tokio::time::timeout(Duration::from_millis(100), writer_closed.changed())
            .await
            .expect("writer failure did not wake reader supervision")
            .expect("writer lifecycle sender disappeared without notification");
        assert!(*writer_closed.borrow());
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn peer_close_wakes_writer_and_releases_without_write_timeout() {
        use futures_util::StreamExt as _;
        use tokio_tungstenite::tungstenite::protocol::Role;

        let (transport_side, peer_side) = tokio::io::duplex(4_096);
        let transport_stream = WebSocketStream::from_raw_socket(
            SipWsStream::Test(transport_side),
            Role::Client,
            Some(sip_websocket_config()),
        )
        .await;
        let mut peer_stream = WebSocketStream::from_raw_socket(
            SipWsStream::Test(peer_side),
            Role::Server,
            Some(sip_websocket_config()),
        )
        .await;
        let (writer, _reader) = transport_stream.split();
        let connection = WebSocketConnection::from_writer_with_runtime(
            writer,
            "127.0.0.1:5060".parse().unwrap(),
            false,
            "sip".into(),
            None,
            1,
            Duration::from_secs(5),
        );
        let mut writer_closed = connection.writer_closed_receiver();

        connection
            .process_ws_message(WsMessage::Close(None))
            .unwrap();

        tokio::time::timeout(Duration::from_millis(250), writer_closed.changed())
            .await
            .expect("peer Close left the writer parked until its write timeout")
            .expect("writer lifecycle sender disappeared without notification");
        assert!(*writer_closed.borrow());
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(250), peer_stream.next())
                .await
                .expect("peer did not receive the server Close promptly"),
            Some(Ok(WsMessage::Close(_)))
        ));
        tokio::time::timeout(Duration::from_millis(250), connection.close())
            .await
            .expect("peer Close retained connection capacity until the write timeout")
            .unwrap();
    }

    // For testing only: a simplified WebSocketConnection without real WebSocket dependencies
    #[cfg(feature = "ws")]
    struct TestWebSocketConnection {
        peer_addr: SocketAddr,
        closed: AtomicBool,
        secure: bool,
        subprotocol: String,
    }

    #[cfg(feature = "ws")]
    impl TestWebSocketConnection {
        fn new(addr: SocketAddr, secure: bool, subprotocol: String) -> Self {
            Self {
                peer_addr: addr,
                closed: AtomicBool::new(false),
                secure,
                subprotocol,
            }
        }

        fn peer_addr(&self) -> SocketAddr {
            self.peer_addr
        }

        fn is_secure(&self) -> bool {
            self.secure
        }

        fn subprotocol(&self) -> &str {
            &self.subprotocol
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::Relaxed)
        }

        fn set_closed(&self) {
            self.closed.store(true, Ordering::Relaxed);
        }

        fn local_addr(&self) -> Result<SocketAddr> {
            Err(Error::NotImplemented(
                "Getting local address from WebSocket connection".into(),
            ))
        }

        fn process_ws_message(&self, ws_message: WsMessage) -> Result<Option<Message>> {
            match ws_message {
                WsMessage::Text(text) => {
                    // Parse the text as a SIP message
                    match parse_message_with_mode(text.as_bytes(), ParseMode::Strict) {
                        Ok(message) => Ok(Some(message)),
                        Err(e) => Err(Error::ParseError(e.to_string())),
                    }
                }
                WsMessage::Binary(data) => {
                    // Parse binary data as SIP message
                    match parse_message_with_mode(&data, ParseMode::Strict) {
                        Ok(message) => Ok(Some(message)),
                        Err(e) => Err(Error::ParseError(e.to_string())),
                    }
                }
                WsMessage::Close(_) => {
                    // Mark as closed
                    self.closed.store(true, Ordering::Relaxed);
                    Ok(None)
                }
                _ => Ok(None), // Control frames don't produce messages
            }
        }
    }

    // Test simple parameter validation and getters
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_connection_parameters() {
        let addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let secure = true;
        let subprotocol = SIP_WS_SUBPROTOCOL.to_string();

        let connection = TestWebSocketConnection::new(addr, secure, subprotocol.clone());

        // Test that parameters are correctly stored and retrievable
        assert_eq!(connection.peer_addr(), addr);
        assert_eq!(connection.is_secure(), secure);
        assert_eq!(connection.subprotocol(), subprotocol);
        assert!(!connection.is_closed());

        // Test closing
        connection.set_closed();
        assert!(connection.is_closed());
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn typed_direct_websocket_rejects_unsafe_serialized_fields_before_frame_io() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(SipWsStream::Plain(stream))
                .await
                .unwrap()
        });
        let client = tokio::net::TcpStream::connect(server_addr).await.unwrap();
        let (client_stream, _) = tokio_tungstenite::client_async(
            format!("ws://{server_addr}/sip"),
            SipWsStream::Plain(client),
        )
        .await
        .unwrap();
        let mut server_stream = server_task.await.unwrap();
        let (writer, _reader) = client_stream.split();
        let connection =
            WebSocketConnection::from_writer(writer, server_addr, false, SIP_WS_SUBPROTOCOL.into());
        let mut unsafe_header = Request::new(Method::Options, Uri::sip("example.test"));
        unsafe_header.headers.push(TypedHeader::CallId(CallId::new(
            "safe\r\nX-Injected: direct-websocket-header-secret",
        )));
        let unsafe_uri = Request::new(
            Method::Options,
            Uri::custom("sip:bob@example.test\r\nX-Injected: direct-websocket-uri-secret"),
        );
        for message in [
            Message::Response(
                Response::new(StatusCode::Ok)
                    .with_reason("OK\r\nX-Injected: direct-websocket-reason-secret"),
            ),
            Message::Request(unsafe_header),
            Message::Request(unsafe_uri),
        ] {
            let error = connection
                .send_message(&message)
                .await
                .expect_err("typed direct WebSocket send must fail closed");
            assert!(matches!(error, Error::ProtocolError(_)));
            for secret in [
                "direct-websocket-reason-secret",
                "direct-websocket-header-secret",
                "direct-websocket-uri-secret",
            ] {
                assert!(!error.to_string().contains(secret));
            }
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), server_stream.next())
                .await
                .is_err(),
            "rejected typed direct WebSocket send must emit no frame",
        );

        let raw = bytes::Bytes::from_static(b"X-Verbatim: websocket-raw-retained\r\n");
        connection.send_raw_bytes(raw.clone()).await.unwrap();
        let received = server_stream.next().await.unwrap().unwrap();
        assert_eq!(received, WsMessage::Binary(raw));

        let binary_message = Message::Request(
            Request::new(Method::Message, Uri::sip("example.test"))
                .with_body(bytes::Bytes::from_static(b"valid-utf8-then-\xff")),
        );
        let expected = bytes::Bytes::from(binary_message.to_bytes());
        connection.send_message(&binary_message).await.unwrap();
        let received = server_stream.next().await.unwrap().unwrap();
        assert_eq!(received, WsMessage::Binary(expected));

        let first = "OPTIONS sip:example.test SIP/2.0\r\nContent-Length: 0\r\n\r\n";
        let second = "BYE sip:example.test SIP/2.0\r\nContent-Length: 0\r\n\r\n";
        let error = connection
            .process_ws_message(WsMessage::Text(format!("{first}{second}").into()))
            .expect_err("RFC 7118 permits exactly one SIP message per WS message");
        assert!(matches!(error, Error::ParseError(_)));

        let missing_length =
            "MESSAGE sip:example.test SIP/2.0\r\nVia: SIP/2.0/WS edge.test\r\n\r\nbody";
        let (message, raw_bytes) = connection
            .process_ws_message(WsMessage::Text(missing_length.into()))
            .unwrap()
            .expect("SIP message");
        let Message::Request(request) = message else {
            panic!("request expected");
        };
        assert_eq!(request.body(), b"body");
        assert_eq!(raw_bytes.as_ref(), missing_length.as_bytes());

        let error = connection
            .process_ws_message(WsMessage::Binary(bytes::Bytes::from(vec![
                b'x';
                MAX_MESSAGE_SIZE
                    + 1
            ])))
            .expect_err("oversized SIP WS message must fail closed");
        assert!(matches!(error, Error::WebSocketProtocolError(_)));
        connection.close().await.unwrap();
    }

    #[cfg(feature = "ws")]
    #[test]
    fn websocket_codec_has_sip_sized_resource_limits() {
        let config = sip_websocket_config();
        assert_eq!(config.max_message_size, Some(MAX_MESSAGE_SIZE));
        assert_eq!(config.max_frame_size, Some(MAX_MESSAGE_SIZE));
        assert!(config.max_write_buffer_size < usize::MAX);
    }

    // Test message parsing from WebSocket frames
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_process_ws_message() {
        let addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let connection = TestWebSocketConnection::new(addr, false, SIP_WS_SUBPROTOCOL.to_string());

        // Test processing a text frame with valid SIP content
        let sip_text = "\
REGISTER sip:example.com SIP/2.0\r\n\
Via: SIP/2.0/WS 127.0.0.1:5060;branch=z9hG4bK-524287-1\r\n\
From: <sip:alice@example.com>;tag=1\r\n\
To: <sip:bob@example.com>\r\n\
Call-ID: call1@example.com\r\n\
CSeq: 1 REGISTER\r\n\
Contact: <sip:alice@127.0.0.1>\r\n\
Content-Length: 0\r\n\
\r\n";

        let text_frame = WsMessage::Text(sip_text.to_string().into());
        let result = connection.process_ws_message(text_frame).unwrap();

        assert!(result.is_some());
        if let Some(Message::Request(req)) = result {
            assert_eq!(req.method(), Method::Register);
            assert_eq!(req.call_id().unwrap().to_string(), "call1@example.com");
        } else {
            panic!("Expected SIP request");
        }

        // Test processing a ping frame
        let ping_frame = WsMessage::Ping(vec![1, 2, 3].into());
        let result = connection.process_ws_message(ping_frame).unwrap();
        assert!(
            result.is_none(),
            "Ping frame should not produce a SIP message"
        );

        // Test processing a pong frame
        let pong_frame = WsMessage::Pong(vec![1, 2, 3].into());
        let result = connection.process_ws_message(pong_frame).unwrap();
        assert!(
            result.is_none(),
            "Pong frame should not produce a SIP message"
        );

        // Test processing a close frame
        let close_frame = WsMessage::Close(None);
        let result = connection.process_ws_message(close_frame).unwrap();
        assert!(
            result.is_none(),
            "Close frame should not produce a SIP message"
        );
        assert!(
            connection.is_closed(),
            "Connection should be marked as closed after receiving close frame"
        );
    }

    // Test local_addr returns the expected NotImplemented error
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_local_addr_not_implemented() {
        let addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let connection = TestWebSocketConnection::new(addr, false, SIP_WS_SUBPROTOCOL.to_string());

        let result = connection.local_addr();
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(matches!(e, Error::NotImplemented(_)));
        } else {
            panic!("Expected NotImplemented error");
        }
    }
}
