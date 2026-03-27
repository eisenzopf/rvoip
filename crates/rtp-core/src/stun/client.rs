//! STUN client for sending Binding Requests and receiving responses.
//!
//! Implements retransmission per RFC 5389 Section 7.2.1:
//! Initial RTO = 500ms, doubled each retransmit, up to configured max attempts.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tracing::{debug, trace, warn};

use crate::Error;
use super::message::{
    StunMessage, BINDING_RESPONSE, BINDING_ERROR_RESPONSE, HEADER_SIZE,
};

/// Result of a successful STUN Binding Request.
#[derive(Debug, Clone)]
pub struct StunBindingResult {
    /// Public IP:port as seen by the STUN server (server-reflexive address).
    pub mapped_address: SocketAddr,
    /// Local IP:port that was used to send the request.
    pub local_address: SocketAddr,
    /// STUN server address that responded.
    pub server_address: SocketAddr,
    /// Round-trip time for the successful exchange.
    pub rtt: Duration,
}

/// Configuration for the STUN client.
#[derive(Debug, Clone)]
pub struct StunClientConfig {
    /// Overall timeout for the binding transaction (default: 7.9s per RFC 5389).
    pub timeout: Duration,
    /// Initial retransmission interval (default: 500ms per RFC 5389).
    pub initial_rto: Duration,
    /// Maximum number of retransmission attempts (default: 7 per RFC 5389).
    pub max_retransmits: u32,
    /// Maximum receive buffer size.
    pub recv_buf_size: usize,
}

impl Default for StunClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(7900),
            initial_rto: Duration::from_millis(500),
            max_retransmits: 7,
            recv_buf_size: 1024,
        }
    }
}

/// A STUN client that performs Binding Requests to discover NAT mappings.
pub struct StunClient {
    /// Address of the STUN server.
    server_addr: SocketAddr,
    /// Client configuration.
    config: StunClientConfig,
}

impl StunClient {
    /// Create a new STUN client targeting the given server address.
    pub fn new(server_addr: SocketAddr) -> Self {
        Self {
            server_addr,
            config: StunClientConfig::default(),
        }
    }

    /// Create a new STUN client with custom configuration.
    pub fn with_config(server_addr: SocketAddr, config: StunClientConfig) -> Self {
        Self {
            server_addr,
            config,
        }
    }

    /// The STUN server address this client targets.
    pub fn server_addr(&self) -> SocketAddr {
        self.server_addr
    }

    /// Perform a STUN Binding Request using the provided UDP socket.
    ///
    /// The socket should already be bound to a local address. This method
    /// sends a Binding Request to the configured STUN server and waits for
    /// a matching Binding Response, implementing RFC 5389 retransmission.
    pub async fn binding_request(&self, socket: &UdpSocket) -> Result<StunBindingResult, Error> {
        let request = StunMessage::binding_request();
        let encoded = request.encode();
        let txn_id = request.transaction_id;

        let local_addr = socket.local_addr().map_err(|e| {
            Error::StunError(format!("failed to get local address: {e}"))
        })?;

        debug!(
            server = %self.server_addr,
            local = %local_addr,
            txn_id = ?txn_id,
            "sending STUN Binding Request"
        );

        let mut recv_buf = vec![0u8; self.config.recv_buf_size];
        let mut rto = self.config.initial_rto;
        let deadline = Instant::now() + self.config.timeout;

        for attempt in 0..=self.config.max_retransmits {
            if Instant::now() >= deadline {
                break;
            }

            // Send the request
            let send_time = Instant::now();
            socket.send_to(&encoded, self.server_addr).await.map_err(|e| {
                Error::StunError(format!("failed to send to {}: {e}", self.server_addr))
            })?;

            trace!(attempt, rto_ms = rto.as_millis(), "sent STUN request");

            // Wait for response with current RTO
            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait_time = rto.min(remaining);

            match tokio::time::timeout(wait_time, self.recv_matching(socket, &mut recv_buf, txn_id)).await {
                Ok(Ok((msg, _src))) => {
                    let rtt = send_time.elapsed();
                    return self.process_response(msg, local_addr, rtt);
                }
                Ok(Err(e)) => {
                    // Received something but it was malformed or non-matching; keep trying
                    trace!(error = %e, "ignoring non-matching STUN response");
                }
                Err(_) => {
                    // Timeout for this attempt
                    if attempt < self.config.max_retransmits {
                        trace!(attempt, "STUN request timed out, retransmitting");
                    }
                }
            }

            // Double RTO for next attempt (RFC 5389 Section 7.2.1)
            rto = rto.saturating_mul(2);
        }

        Err(Error::Timeout(format!(
            "STUN binding request to {} timed out after {:?}",
            self.server_addr, self.config.timeout
        )))
    }

    /// Receive packets until we get one that matches our transaction ID
    /// **and** originates from the configured STUN server address.
    ///
    /// Validating the source address prevents an off-path attacker from
    /// injecting a spoofed STUN response that merely guesses the
    /// transaction ID.
    async fn recv_matching(
        &self,
        socket: &UdpSocket,
        buf: &mut [u8],
        expected_txn: super::message::TransactionId,
    ) -> Result<(StunMessage, SocketAddr), Error> {
        loop {
            let (n, src) = socket.recv_from(buf).await.map_err(|e| {
                Error::StunError(format!("recv error: {e}"))
            })?;

            if n < HEADER_SIZE {
                continue;
            }

            // Verify the response came from the expected STUN server.
            if src != self.server_addr {
                trace!(
                    received_from = %src,
                    expected = %self.server_addr,
                    "ignoring STUN response from unexpected source"
                );
                continue;
            }

            let msg = match StunMessage::decode(&buf[..n]) {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Check transaction ID matches
            if msg.transaction_id != expected_txn {
                trace!("ignoring STUN response with non-matching transaction ID");
                continue;
            }

            // Must be a Binding Response or Error Response
            if msg.msg_type != BINDING_RESPONSE && msg.msg_type != BINDING_ERROR_RESPONSE {
                continue;
            }

            return Ok((msg, src));
        }
    }

    /// Process a decoded STUN response into a binding result.
    fn process_response(
        &self,
        msg: StunMessage,
        local_addr: SocketAddr,
        rtt: Duration,
    ) -> Result<StunBindingResult, Error> {
        if msg.msg_type == BINDING_ERROR_RESPONSE {
            let (code, reason) = msg.error_code().unwrap_or((0, "unknown error"));
            return Err(Error::StunError(format!(
                "STUN server returned error {}: {}",
                code, reason
            )));
        }

        let mapped = msg.mapped_address().ok_or_else(|| {
            Error::StunError("Binding Response has no mapped address attribute".into())
        })?;

        debug!(
            mapped = %mapped,
            rtt_ms = rtt.as_millis(),
            "STUN binding succeeded"
        );

        Ok(StunBindingResult {
            mapped_address: mapped,
            local_address: local_addr,
            server_address: self.server_addr,
            rtt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stun_client_config_defaults() {
        let config = StunClientConfig::default();
        assert_eq!(config.initial_rto, Duration::from_millis(500));
        assert_eq!(config.max_retransmits, 7);
        assert_eq!(config.recv_buf_size, 1024);
    }

    #[test]
    fn test_stun_client_new() {
        let addr: SocketAddr = "74.125.250.129:19302".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let client = StunClient::new(addr);
        assert_eq!(client.server_addr(), addr);
    }

    #[tokio::test]
    async fn test_binding_request_timeout_on_loopback() {
        // Bind to loopback, send to a port that won't respond.
        // This should timeout quickly with a short config.
        let socket = UdpSocket::bind("127.0.0.1:0").await
            .unwrap_or_else(|e| panic!("bind: {e}"));

        let fake_server: SocketAddr = "127.0.0.1:1".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let config = StunClientConfig {
            timeout: Duration::from_millis(200),
            initial_rto: Duration::from_millis(50),
            max_retransmits: 2,
            recv_buf_size: 512,
        };

        let client = StunClient::with_config(fake_server, config);
        let result = client.binding_request(&socket).await;
        assert!(result.is_err());
    }
}
