//! TURN client for relay-based NAT traversal (RFC 5766).
//!
//! Provides an async client that can:
//! - Allocate a relay address on a TURN server
//! - Refresh the allocation to keep it alive
//! - Create permissions for specific peers
//! - Bind channels for efficient data relay
//! - Send data through the relay via Send Indication
//! - Deallocate the relay when done

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tracing::{debug, trace};

use crate::Error;
use crate::stun::message::{
    TransactionId, HEADER_SIZE,
};
use super::credentials::LongTermCredentials;
use super::message::{
    TurnAttribute, TRANSPORT_UDP,
    ALLOCATE_REQUEST, ALLOCATE_RESPONSE, ALLOCATE_ERROR_RESPONSE,
    REFRESH_REQUEST, REFRESH_RESPONSE, REFRESH_ERROR_RESPONSE,
    CREATE_PERMISSION_REQUEST, CREATE_PERMISSION_RESPONSE, CREATE_PERMISSION_ERROR_RESPONSE,
    CHANNEL_BIND_REQUEST, CHANNEL_BIND_RESPONSE, CHANNEL_BIND_ERROR_RESPONSE,
    SEND_INDICATION,
    build_turn_message, parse_turn_response,
    find_relayed_address, find_mapped_address, find_lifetime,
    find_error_code, find_realm, find_nonce,
};

/// Default TURN allocation lifetime in seconds (10 minutes per RFC 5766).
const DEFAULT_LIFETIME_SECS: u32 = 600;

/// Maximum receive buffer size.
const RECV_BUF_SIZE: usize = 4096;

/// Retransmission timeout (initial).
const INITIAL_RTO: Duration = Duration::from_millis(500);

/// Maximum retransmissions for a TURN request.
const MAX_RETRANSMITS: u32 = 7;

/// Overall transaction timeout.
const TRANSACTION_TIMEOUT: Duration = Duration::from_millis(7900);

/// Result of a successful TURN allocation.
#[derive(Debug, Clone)]
pub struct TurnAllocation {
    /// The relay address allocated by the TURN server.
    pub relayed_address: SocketAddr,
    /// Server-reflexive address (XOR-MAPPED-ADDRESS from the response).
    pub mapped_address: SocketAddr,
    /// Allocation lifetime granted by the server.
    pub lifetime: Duration,
}

/// A TURN client for relay-based NAT traversal.
///
/// Manages a single TURN allocation on a server, handling the long-term
/// credential challenge/response flow and providing methods for all TURN
/// operations (allocate, refresh, permission, channel bind, send, deallocate).
pub struct TurnClient {
    /// TURN server address.
    server: SocketAddr,
    /// UDP socket used for communication.
    socket: Arc<UdpSocket>,
    /// Long-term credentials.
    credentials: LongTermCredentials,
    /// The relay address (set after successful allocation).
    relayed_address: Option<SocketAddr>,
    /// The server-reflexive address (set after successful allocation).
    mapped_address: Option<SocketAddr>,
    /// Current allocation lifetime.
    lifetime: Duration,
    /// Packets received that did not match the current TURN transaction.
    /// When using a shared socket, these may belong to ICE/STUN/media and
    /// must not be silently dropped.
    pending_packets: Vec<(Vec<u8>, SocketAddr)>,
}

impl TurnClient {
    /// Create a new TURN client.
    ///
    /// Binds a local UDP socket and prepares to communicate with the TURN server.
    /// Does NOT allocate yet -- call [`allocate`] to request a relay address.
    pub async fn new(
        server: SocketAddr,
        username: String,
        password: String,
    ) -> Result<Self, Error> {
        let bind_addr = if server.is_ipv6() {
            "[::]:0"
        } else {
            "0.0.0.0:0"
        };

        let socket = UdpSocket::bind(bind_addr).await.map_err(|e| {
            Error::TurnError(format!("failed to bind UDP socket: {e}"))
        })?;

        Ok(Self {
            server,
            socket: Arc::new(socket),
            credentials: LongTermCredentials::new(username, password),
            relayed_address: None,
            mapped_address: None,
            lifetime: Duration::from_secs(DEFAULT_LIFETIME_SECS as u64),
            pending_packets: Vec::new(),
        })
    }

    /// Create a TURN client using an existing UDP socket.
    ///
    /// Useful when the socket is shared with STUN or ICE.
    pub fn with_socket(
        server: SocketAddr,
        socket: Arc<UdpSocket>,
        username: String,
        password: String,
    ) -> Self {
        Self {
            server,
            socket,
            credentials: LongTermCredentials::new(username, password),
            relayed_address: None,
            mapped_address: None,
            lifetime: Duration::from_secs(DEFAULT_LIFETIME_SECS as u64),
            pending_packets: Vec::new(),
        }
    }

    /// The TURN server address.
    pub fn server(&self) -> SocketAddr {
        self.server
    }

    /// The allocated relay address, if any.
    pub fn relayed_address(&self) -> Option<SocketAddr> {
        self.relayed_address
    }

    /// The server-reflexive (mapped) address, if any.
    pub fn mapped_address(&self) -> Option<SocketAddr> {
        self.mapped_address
    }

    /// The current allocation lifetime.
    pub fn lifetime(&self) -> Duration {
        self.lifetime
    }

    /// Allocate a relay address on the TURN server.
    ///
    /// This implements the two-step allocation flow:
    /// 1. Send an unauthenticated Allocate request.
    /// 2. Receive a 401 Unauthorized with realm and nonce.
    /// 3. Resend with long-term credentials (USERNAME, REALM, NONCE, MESSAGE-INTEGRITY).
    /// 4. Receive the Allocate Success Response with XOR-RELAYED-ADDRESS.
    pub async fn allocate(&mut self) -> Result<TurnAllocation, Error> {
        // Step 1: Send unauthenticated Allocate request
        let txn_id = TransactionId::random();
        let attrs = vec![
            TurnAttribute::RequestedTransport(TRANSPORT_UDP),
        ];
        let msg = build_turn_message(ALLOCATE_REQUEST, &txn_id, &attrs);

        debug!(server = %self.server, "sending initial Allocate request (unauthenticated)");

        let (resp_type, resp_attrs) = self.send_and_recv(&msg, &txn_id).await?;

        // If we got a success on first try (unlikely but handle it)
        if is_success_response(resp_type) {
            return self.process_allocate_success(&resp_attrs);
        }

        // Step 2: Expect a 401 Unauthorized with REALM and NONCE
        if !is_error_response(resp_type) {
            return Err(Error::TurnError(format!(
                "unexpected response type 0x{:04X} to Allocate",
                resp_type
            )));
        }

        let error = find_error_code(&resp_attrs);
        match error {
            Some((401, _)) => {
                // Extract realm and nonce
                let realm = find_realm(&resp_attrs).ok_or_else(|| {
                    Error::TurnError("401 response missing REALM attribute".into())
                })?.to_owned();
                let nonce = find_nonce(&resp_attrs).ok_or_else(|| {
                    Error::TurnError("401 response missing NONCE attribute".into())
                })?.to_owned();

                debug!(realm = %realm, "received 401, retrying with credentials");
                self.credentials.set_challenge(realm, nonce);
            }
            Some((code, reason)) => {
                return Err(Error::TurnError(format!(
                    "Allocate failed with error {}: {}",
                    code, reason
                )));
            }
            None => {
                return Err(Error::TurnError(
                    "Allocate error response missing ERROR-CODE".into()
                ));
            }
        }

        // Step 3: Retry with credentials
        let txn_id2 = TransactionId::random();
        let realm = self.credentials.realm().unwrap_or_default().to_owned();
        let nonce = self.credentials.nonce().unwrap_or_default().to_owned();
        let username = self.credentials.username().to_owned();

        let attrs2 = vec![
            TurnAttribute::RequestedTransport(TRANSPORT_UDP),
            TurnAttribute::Username(username),
            TurnAttribute::Realm(realm),
            TurnAttribute::Nonce(nonce),
        ];
        let mut msg2 = build_turn_message(ALLOCATE_REQUEST, &txn_id2, &attrs2);

        // Sign with MESSAGE-INTEGRITY
        self.credentials.sign_message(&mut msg2)?;

        debug!("sending authenticated Allocate request");
        let (resp_type2, resp_attrs2) = self.send_and_recv(&msg2, &txn_id2).await?;

        if is_error_response(resp_type2) {
            let (code, reason) = find_error_code(&resp_attrs2)
                .unwrap_or((0, "unknown"));
            return Err(Error::TurnError(format!(
                "Allocate failed after auth: error {}: {}",
                code, reason
            )));
        }

        if !is_success_response(resp_type2) {
            return Err(Error::TurnError(format!(
                "unexpected response 0x{:04X} to authenticated Allocate",
                resp_type2
            )));
        }

        self.process_allocate_success(&resp_attrs2)
    }

    /// Refresh the current allocation.
    ///
    /// `lifetime_secs` is the requested lifetime. Pass 0 to deallocate.
    pub async fn refresh(&mut self, lifetime_secs: u32) -> Result<(), Error> {
        if !self.credentials.has_challenge() {
            return Err(Error::TurnError("no allocation to refresh (not authenticated)".into()));
        }

        let txn_id = TransactionId::random();
        let realm = self.credentials.realm().unwrap_or_default().to_owned();
        let nonce = self.credentials.nonce().unwrap_or_default().to_owned();
        let username = self.credentials.username().to_owned();

        let attrs = vec![
            TurnAttribute::Lifetime(lifetime_secs),
            TurnAttribute::Username(username),
            TurnAttribute::Realm(realm),
            TurnAttribute::Nonce(nonce),
        ];
        let mut msg = build_turn_message(REFRESH_REQUEST, &txn_id, &attrs);
        self.credentials.sign_message(&mut msg)?;

        debug!(lifetime_secs, "sending Refresh request");
        let (resp_type, resp_attrs) = self.send_and_recv(&msg, &txn_id).await?;

        if is_error_response(resp_type) {
            let (code, reason) = find_error_code(&resp_attrs)
                .unwrap_or((0, "unknown"));
            return Err(Error::TurnError(format!(
                "Refresh failed: error {}: {}",
                code, reason
            )));
        }

        // Update lifetime from server response
        if let Some(granted) = find_lifetime(&resp_attrs) {
            self.lifetime = Duration::from_secs(granted as u64);
            debug!(granted_lifetime = granted, "allocation refreshed");
        }

        if lifetime_secs == 0 {
            self.relayed_address = None;
            self.mapped_address = None;
            debug!("allocation deallocated");
        }

        Ok(())
    }

    /// Create a permission for a peer address.
    ///
    /// This allows the peer to send data through the relay to us.
    /// Permissions must be refreshed every 5 minutes (300s).
    pub async fn create_permission(&mut self, peer_addr: SocketAddr) -> Result<(), Error> {
        if !self.credentials.has_challenge() {
            return Err(Error::TurnError("no allocation (not authenticated)".into()));
        }

        let txn_id = TransactionId::random();
        let realm = self.credentials.realm().unwrap_or_default().to_owned();
        let nonce = self.credentials.nonce().unwrap_or_default().to_owned();
        let username = self.credentials.username().to_owned();

        let attrs = vec![
            TurnAttribute::XorPeerAddress(peer_addr),
            TurnAttribute::Username(username),
            TurnAttribute::Realm(realm),
            TurnAttribute::Nonce(nonce),
        ];
        let mut msg = build_turn_message(CREATE_PERMISSION_REQUEST, &txn_id, &attrs);
        self.credentials.sign_message(&mut msg)?;

        debug!(peer = %peer_addr, "sending CreatePermission request");
        let (resp_type, resp_attrs) = self.send_and_recv(&msg, &txn_id).await?;

        if is_error_response(resp_type) {
            let (code, reason) = find_error_code(&resp_attrs)
                .unwrap_or((0, "unknown"));
            return Err(Error::TurnError(format!(
                "CreatePermission failed: error {}: {}",
                code, reason
            )));
        }

        debug!(peer = %peer_addr, "permission created");
        Ok(())
    }

    /// Bind a channel number to a peer address for efficient relay.
    ///
    /// Channel numbers must be in the range 0x4000..=0x7FFE.
    /// Channel bindings must be refreshed every 10 minutes (600s).
    pub async fn channel_bind(
        &mut self,
        peer_addr: SocketAddr,
        channel: u16,
    ) -> Result<(), Error> {
        if !self.credentials.has_challenge() {
            return Err(Error::TurnError("no allocation (not authenticated)".into()));
        }

        if !(0x4000..=0x7FFE).contains(&channel) {
            return Err(Error::TurnError(format!(
                "invalid channel number 0x{:04X}: must be in range 0x4000..0x7FFE",
                channel
            )));
        }

        let txn_id = TransactionId::random();
        let realm = self.credentials.realm().unwrap_or_default().to_owned();
        let nonce = self.credentials.nonce().unwrap_or_default().to_owned();
        let username = self.credentials.username().to_owned();

        let attrs = vec![
            TurnAttribute::ChannelNumber(channel),
            TurnAttribute::XorPeerAddress(peer_addr),
            TurnAttribute::Username(username),
            TurnAttribute::Realm(realm),
            TurnAttribute::Nonce(nonce),
        ];
        let mut msg = build_turn_message(CHANNEL_BIND_REQUEST, &txn_id, &attrs);
        self.credentials.sign_message(&mut msg)?;

        debug!(peer = %peer_addr, channel = channel, "sending ChannelBind request");
        let (resp_type, resp_attrs) = self.send_and_recv(&msg, &txn_id).await?;

        if is_error_response(resp_type) {
            let (code, reason) = find_error_code(&resp_attrs)
                .unwrap_or((0, "unknown"));
            return Err(Error::TurnError(format!(
                "ChannelBind failed: error {}: {}",
                code, reason
            )));
        }

        debug!(peer = %peer_addr, channel = channel, "channel bound");
        Ok(())
    }

    /// Send data to a peer through the TURN relay via a Send Indication.
    ///
    /// The peer must have a permission installed. The data will be relayed
    /// from the server's relay address to the peer.
    pub async fn send_indication(
        &self,
        peer_addr: SocketAddr,
        data: &[u8],
    ) -> Result<(), Error> {
        let txn_id = TransactionId::random();
        let attrs = vec![
            TurnAttribute::XorPeerAddress(peer_addr),
            TurnAttribute::Data(data.to_vec()),
        ];
        let msg = build_turn_message(SEND_INDICATION, &txn_id, &attrs);

        // Indications are fire-and-forget (no response expected)
        self.socket.send_to(&msg, self.server).await.map_err(|e| {
            Error::TurnError(format!("failed to send indication: {e}"))
        })?;

        trace!(peer = %peer_addr, len = data.len(), "sent Send Indication");
        Ok(())
    }

    /// Send data via a ChannelData message (more efficient than Send Indication).
    ///
    /// ChannelData messages use a 4-byte header instead of the full STUN header.
    /// Format: channel_number (2 bytes) + length (2 bytes) + data.
    pub async fn send_channel_data(
        &self,
        channel: u16,
        data: &[u8],
    ) -> Result<(), Error> {
        if !(0x4000..=0x7FFE).contains(&channel) {
            return Err(Error::TurnError(format!(
                "invalid channel number 0x{:04X}",
                channel
            )));
        }

        let mut msg = Vec::with_capacity(4 + data.len());
        msg.extend_from_slice(&channel.to_be_bytes());
        msg.extend_from_slice(&(data.len() as u16).to_be_bytes());
        msg.extend_from_slice(data);

        // Pad to 4-byte boundary
        let padding = (4 - (data.len() % 4)) % 4;
        msg.extend(std::iter::repeat(0u8).take(padding));

        self.socket.send_to(&msg, self.server).await.map_err(|e| {
            Error::TurnError(format!("failed to send channel data: {e}"))
        })?;

        trace!(channel, len = data.len(), "sent ChannelData");
        Ok(())
    }

    /// Deallocate the relay address (Refresh with lifetime=0).
    pub async fn deallocate(&mut self) -> Result<(), Error> {
        self.refresh(0).await
    }

    /// Drain all buffered non-matching packets that were received during
    /// TURN transactions.  When the socket is shared with ICE/STUN/media,
    /// callers should drain these after every TURN operation and deliver
    /// them to the appropriate handler.
    pub fn drain_pending(&mut self) -> Vec<(Vec<u8>, SocketAddr)> {
        std::mem::take(&mut self.pending_packets)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Process a successful Allocate response.
    fn process_allocate_success(
        &mut self,
        attrs: &[TurnAttribute],
    ) -> Result<TurnAllocation, Error> {
        let relayed = find_relayed_address(attrs).ok_or_else(|| {
            Error::TurnError("Allocate success missing XOR-RELAYED-ADDRESS".into())
        })?;

        let mapped = find_mapped_address(attrs).unwrap_or(relayed);

        let lifetime_secs = find_lifetime(attrs).unwrap_or(DEFAULT_LIFETIME_SECS);
        let lifetime = Duration::from_secs(lifetime_secs as u64);

        self.relayed_address = Some(relayed);
        self.mapped_address = Some(mapped);
        self.lifetime = lifetime;

        debug!(
            relay = %relayed,
            mapped = %mapped,
            lifetime_secs,
            "TURN allocation succeeded"
        );

        Ok(TurnAllocation {
            relayed_address: relayed,
            mapped_address: mapped,
            lifetime,
        })
    }

    /// Send a STUN/TURN message and wait for a matching response.
    ///
    /// Implements retransmission per RFC 5389 Section 7.2.1.
    async fn send_and_recv(
        &mut self,
        msg: &[u8],
        expected_txn: &TransactionId,
    ) -> Result<(u16, Vec<TurnAttribute>), Error> {
        let mut recv_buf = vec![0u8; RECV_BUF_SIZE];
        let mut rto = INITIAL_RTO;
        let deadline = Instant::now() + TRANSACTION_TIMEOUT;

        for attempt in 0..=MAX_RETRANSMITS {
            if Instant::now() >= deadline {
                break;
            }

            self.socket.send_to(msg, self.server).await.map_err(|e| {
                Error::TurnError(format!("failed to send to {}: {e}", self.server))
            })?;

            trace!(attempt, rto_ms = rto.as_millis(), "sent TURN request");

            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait_time = rto.min(remaining);

            match tokio::time::timeout(
                wait_time,
                self.recv_matching_response(&mut recv_buf, expected_txn),
            )
            .await
            {
                Ok(Ok(result)) => return Ok(result),
                Ok(Err(e)) => {
                    trace!(error = %e, "ignoring non-matching response");
                }
                Err(_) => {
                    if attempt < MAX_RETRANSMITS {
                        trace!(attempt, "TURN request timed out, retransmitting");
                    }
                }
            }

            rto = rto.saturating_mul(2);
        }

        Err(Error::Timeout(format!(
            "TURN request to {} timed out after {:?}",
            self.server, TRANSACTION_TIMEOUT
        )))
    }

    /// Receive and decode a TURN response matching the expected transaction ID.
    ///
    /// Non-matching packets (ChannelData, other STUN transactions, media) are
    /// buffered in `pending_packets` so the caller can retrieve them via
    /// [`drain_pending`] instead of silently losing them.
    async fn recv_matching_response(
        &mut self,
        buf: &mut [u8],
        expected_txn: &TransactionId,
    ) -> Result<(u16, Vec<TurnAttribute>), Error> {
        loop {
            let (n, src) = self.socket.recv_from(buf).await.map_err(|e| {
                Error::TurnError(format!("recv error: {e}"))
            })?;

            if n < HEADER_SIZE {
                // Too small for STUN but could be meaningful to another
                // consumer — buffer it rather than dropping.
                self.pending_packets.push((buf[..n].to_vec(), src));
                continue;
            }

            // Check if this is a ChannelData message (first two bits != 00)
            if buf[0] & 0xC0 != 0 {
                // Buffer ChannelData for the caller instead of discarding.
                self.pending_packets.push((buf[..n].to_vec(), src));
                continue;
            }

            let (stun_msg, turn_attrs) = match parse_turn_response(&buf[..n]) {
                Ok(r) => r,
                Err(_) => {
                    // Not a valid TURN message — buffer for other consumers.
                    self.pending_packets.push((buf[..n].to_vec(), src));
                    continue;
                }
            };

            if stun_msg.transaction_id != *expected_txn {
                trace!("buffering response with non-matching transaction ID");
                self.pending_packets.push((buf[..n].to_vec(), src));
                continue;
            }

            return Ok((stun_msg.msg_type, turn_attrs));
        }
    }
}

/// Check if a message type is a success response (class = 0b10 in bits 4,8).
fn is_success_response(msg_type: u16) -> bool {
    // Success response has C0=1, C1=0 in the class bits
    // Class bits: bit 4 (C0) and bit 8 (C1) of the message type
    (msg_type & 0x0110) == 0x0100
}

/// Check if a message type is an error response (class = 0b11 in bits 4,8).
fn is_error_response(msg_type: u16) -> bool {
    // Error response has C0=1, C1=1
    (msg_type & 0x0110) == 0x0110
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_success_response() {
        assert!(is_success_response(ALLOCATE_RESPONSE));
        assert!(is_success_response(REFRESH_RESPONSE));
        assert!(is_success_response(CREATE_PERMISSION_RESPONSE));
        assert!(is_success_response(CHANNEL_BIND_RESPONSE));
        assert!(!is_success_response(ALLOCATE_REQUEST));
        assert!(!is_success_response(ALLOCATE_ERROR_RESPONSE));
    }

    #[test]
    fn test_is_error_response() {
        assert!(is_error_response(ALLOCATE_ERROR_RESPONSE));
        assert!(is_error_response(REFRESH_ERROR_RESPONSE));
        assert!(is_error_response(CREATE_PERMISSION_ERROR_RESPONSE));
        assert!(is_error_response(CHANNEL_BIND_ERROR_RESPONSE));
        assert!(!is_error_response(ALLOCATE_REQUEST));
        assert!(!is_error_response(ALLOCATE_RESPONSE));
    }

    #[test]
    fn test_channel_number_validation() {
        // Valid range: 0x4000..=0x7FFE
        assert!((0x4000..=0x7FFE).contains(&0x4000u16));
        assert!((0x4000..=0x7FFE).contains(&0x7FFEu16));
        assert!(!(0x4000..=0x7FFE).contains(&0x3FFFu16));
        assert!(!(0x4000..=0x7FFE).contains(&0x7FFFu16));
        assert!(!(0x4000..=0x7FFE).contains(&0x8000u16));
    }

    #[tokio::test]
    async fn test_turn_client_new() {
        let server: SocketAddr = "127.0.0.1:3478".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let client = TurnClient::new(server, "user".into(), "pass".into())
            .await
            .unwrap_or_else(|e| panic!("new: {e}"));

        assert_eq!(client.server(), server);
        assert!(client.relayed_address().is_none());
        assert!(client.mapped_address().is_none());
        assert_eq!(client.lifetime(), Duration::from_secs(600));
    }

    #[tokio::test]
    async fn test_turn_client_with_socket() {
        let socket = UdpSocket::bind("127.0.0.1:0").await
            .unwrap_or_else(|e| panic!("bind: {e}"));
        let server: SocketAddr = "127.0.0.1:3478".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let client = TurnClient::with_socket(
            server,
            Arc::new(socket),
            "user".into(),
            "pass".into(),
        );

        assert_eq!(client.server(), server);
    }

    #[tokio::test]
    async fn test_send_indication_fire_and_forget() {
        let server: SocketAddr = "127.0.0.1:1".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let socket = UdpSocket::bind("127.0.0.1:0").await
            .unwrap_or_else(|e| panic!("bind: {e}"));

        let client = TurnClient::with_socket(
            server,
            Arc::new(socket),
            "user".into(),
            "pass".into(),
        );

        let peer: SocketAddr = "10.0.0.1:5060".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        // Send indication is fire-and-forget, should not block
        let _ = client.send_indication(peer, b"test data").await;
    }

    #[tokio::test]
    async fn test_channel_data_validation() {
        let server: SocketAddr = "127.0.0.1:1".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let socket = UdpSocket::bind("127.0.0.1:0").await
            .unwrap_or_else(|e| panic!("bind: {e}"));

        let client = TurnClient::with_socket(
            server,
            Arc::new(socket),
            "user".into(),
            "pass".into(),
        );

        // Invalid channel number should be rejected
        let result = client.send_channel_data(0x3FFF, b"data").await;
        assert!(result.is_err());

        let result = client.send_channel_data(0x8000, b"data").await;
        assert!(result.is_err());
    }
}
