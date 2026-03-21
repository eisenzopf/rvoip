//! Adapter bridging the production `stun-rs` crate to our internal STUN API.
//!
//! This module provides [`StunClientAdapter`] — a drop-in replacement for the
//! hand-rolled STUN binding client — backed by the well-tested `stun-rs` 0.1
//! library (RFC 5389 / RFC 8489).  It also exposes helpers for building
//! ICE connectivity-check messages with MESSAGE-INTEGRITY and FINGERPRINT
//! computed automatically by `stun-rs`.
//!
//! # Migration
//!
//! The legacy `message.rs` / `client.rs` modules are preserved but marked
//! `#[deprecated]`.  New call-sites should use this adapter instead.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use stun_rs::attributes::stun::{
    Fingerprint, MessageIntegrity, UserName, XorMappedAddress,
};
use stun_rs::methods::BINDING;
use stun_rs::{
    DecoderContextBuilder, HMACKey, MessageClass, MessageDecoderBuilder,
    MessageEncoderBuilder, StunMessageBuilder,
};
use tokio::net::UdpSocket;
use tracing::{debug, trace};

use crate::Error;
#[allow(deprecated)]
use super::client::{StunBindingResult, StunClientConfig};

// ── Constants ────────────────────────────────────────────────────────────

/// Encoder buffer size — large enough for a Binding Request with ICE attrs,
/// MESSAGE-INTEGRITY (24 B) and FINGERPRINT (8 B).
const ENCODE_BUF_SIZE: usize = 512;

// ── StunClientAdapter ────────────────────────────────────────────────────

/// Production STUN client backed by the `stun-rs` crate.
///
/// Implements STUN Binding Requests (RFC 5389) with RFC-compliant
/// retransmission, and can build ICE connectivity-check requests
/// with authentication and fingerprint.
pub struct StunClientAdapter {
    /// STUN server address.
    server_addr: SocketAddr,
    /// Client configuration (timeouts, retransmissions).
    config: StunClientConfig,
}

impl StunClientAdapter {
    /// Create a new adapter targeting `server_addr` with default config.
    pub fn new(server_addr: SocketAddr) -> Self {
        Self {
            server_addr,
            config: StunClientConfig::default(),
        }
    }

    /// Create a new adapter with custom configuration.
    pub fn with_config(server_addr: SocketAddr, config: StunClientConfig) -> Self {
        Self {
            server_addr,
            config,
        }
    }

    /// The STUN server address this adapter targets.
    pub fn server_addr(&self) -> SocketAddr {
        self.server_addr
    }

    // ── Binding Request ──────────────────────────────────────────────

    /// Perform a STUN Binding Request and return the server-reflexive
    /// address (XOR-MAPPED-ADDRESS) from the response.
    ///
    /// Implements RFC 5389 Section 7.2.1 retransmission (initial RTO
    /// doubled each attempt up to `max_retransmits`).
    pub async fn binding_request(
        &self,
        socket: &UdpSocket,
    ) -> Result<StunBindingResult, Error> {
        // Build a simple Binding Request (no authentication).
        let msg = StunMessageBuilder::new(BINDING, MessageClass::Request).build();

        let mut encode_buf = [0u8; ENCODE_BUF_SIZE];
        let encoder = MessageEncoderBuilder::default().build();
        let encoded_size = encoder.encode(&mut encode_buf, &msg).map_err(|e| {
            Error::StunError(format!("stun-rs encode error: {e}"))
        })?;
        let encoded = &encode_buf[..encoded_size];

        // Extract the transaction-ID bytes for matching.
        let txn_id = *msg.transaction_id();

        let local_addr = socket.local_addr().map_err(|e| {
            Error::StunError(format!("failed to get local address: {e}"))
        })?;

        debug!(
            server = %self.server_addr,
            local = %local_addr,
            txn_id = %txn_id,
            "sending STUN Binding Request (stun-rs adapter)"
        );

        let mut recv_buf = vec![0u8; self.config.recv_buf_size];
        let mut rto = self.config.initial_rto;
        let deadline = Instant::now() + self.config.timeout;

        for attempt in 0..=self.config.max_retransmits {
            if Instant::now() >= deadline {
                break;
            }

            let send_time = Instant::now();
            socket.send_to(encoded, self.server_addr).await.map_err(|e| {
                Error::StunError(format!("failed to send to {}: {e}", self.server_addr))
            })?;

            trace!(attempt, rto_ms = rto.as_millis(), "sent STUN request (stun-rs)");

            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait_time = rto.min(remaining);

            match tokio::time::timeout(
                wait_time,
                self.recv_matching(socket, &mut recv_buf, &txn_id),
            )
            .await
            {
                Ok(Ok((mapped_addr, _src))) => {
                    let rtt = send_time.elapsed();
                    debug!(
                        mapped = %mapped_addr,
                        rtt_ms = rtt.as_millis(),
                        "STUN binding succeeded (stun-rs)"
                    );
                    return Ok(StunBindingResult {
                        mapped_address: mapped_addr,
                        local_address: local_addr,
                        server_address: self.server_addr,
                        rtt,
                    });
                }
                Ok(Err(e)) => {
                    trace!(error = %e, "ignoring non-matching STUN response");
                }
                Err(_) => {
                    if attempt < self.config.max_retransmits {
                        trace!(attempt, "STUN request timed out, retransmitting");
                    }
                }
            }

            rto = rto.saturating_mul(2);
        }

        Err(Error::Timeout(format!(
            "STUN binding request to {} timed out after {:?}",
            self.server_addr, self.config.timeout
        )))
    }

    /// Receive datagrams until one matches our transaction-ID and comes
    /// from the expected STUN server.  Returns the mapped address.
    async fn recv_matching(
        &self,
        socket: &UdpSocket,
        buf: &mut [u8],
        expected_txn: &stun_rs::TransactionId,
    ) -> Result<(SocketAddr, SocketAddr), Error> {
        let decoder = MessageDecoderBuilder::default().build();

        loop {
            let (n, src) = socket.recv_from(buf).await.map_err(|e| {
                Error::StunError(format!("recv error: {e}"))
            })?;

            if n < 20 {
                continue;
            }

            // Validate source address to prevent off-path injection.
            if src != self.server_addr {
                trace!(
                    received_from = %src,
                    expected = %self.server_addr,
                    "ignoring STUN response from unexpected source"
                );
                continue;
            }

            let (msg, _size) = match decoder.decode(&buf[..n]) {
                Ok(decoded) => decoded,
                Err(_) => continue,
            };

            if msg.transaction_id() != expected_txn {
                trace!("ignoring STUN response with non-matching transaction ID");
                continue;
            }

            match msg.class() {
                MessageClass::SuccessResponse => {}
                MessageClass::ErrorResponse => {
                    return Err(Error::StunError(
                        "STUN server returned error response".into(),
                    ));
                }
                _ => continue,
            }

            if msg.method() != BINDING {
                continue;
            }

            // Extract XOR-MAPPED-ADDRESS (preferred) or MAPPED-ADDRESS.
            let mapped = msg
                .get::<XorMappedAddress>()
                .and_then(|attr| attr.as_xor_mapped_address().ok())
                .map(|xma| xma.socket_address());

            let mapped_addr = match mapped {
                Some(addr) => *addr,
                None => {
                    // Fall back to MappedAddress if XOR variant is absent.
                    use stun_rs::attributes::stun::MappedAddress;
                    let ma = msg
                        .get::<MappedAddress>()
                        .and_then(|attr| attr.as_mapped_address().ok())
                        .map(|ma| *ma.socket_address())
                        .ok_or_else(|| {
                            Error::StunError(
                                "Binding Response has no mapped address attribute".into(),
                            )
                        })?;
                    ma
                }
            };

            return Ok((mapped_addr, src));
        }
    }

    // ── ICE connectivity-check builder ───────────────────────────────

    /// Build a STUN Binding Request carrying ICE attributes and
    /// MESSAGE-INTEGRITY + FINGERPRINT, ready for transmission.
    ///
    /// The returned `Vec<u8>` is the encoded STUN message.
    ///
    /// # Arguments
    ///
    /// * `username` — ICE short-term credential (`ufrag:ufrag`).
    /// * `password` — ICE short-term credential password.
    /// * `priority` — Candidate priority (RFC 8445 Section 5.1.2).
    /// * `use_candidate` — Set the USE-CANDIDATE flag (controlling agent).
    /// * `controlling` — `true` → ICE-CONTROLLING, `false` → ICE-CONTROLLED.
    /// * `tie_breaker` — Tie-breaker value for role conflict resolution.
    pub fn build_ice_check(
        username: &str,
        password: &str,
        priority: u32,
        use_candidate: bool,
        controlling: bool,
        tie_breaker: u64,
    ) -> Result<Vec<u8>, Error> {
        use stun_rs::attributes::ice::{
            IceControlled, IceControlling, Priority, UseCandidate,
        };

        let user_attr = UserName::new(username).map_err(|e| {
            Error::StunError(format!("invalid ICE username: {e}"))
        })?;

        let key = HMACKey::new_short_term(password).map_err(|e| {
            Error::StunError(format!("invalid ICE password: {e}"))
        })?;

        let integrity = MessageIntegrity::new(key.clone());
        let fingerprint = Fingerprint::default();

        let mut builder = StunMessageBuilder::new(BINDING, MessageClass::Request)
            .with_attribute(user_attr)
            .with_attribute(Priority::from(priority));

        if use_candidate {
            builder = builder.with_attribute(UseCandidate::default());
        }

        if controlling {
            builder = builder.with_attribute(IceControlling::from(tie_breaker));
        } else {
            builder = builder.with_attribute(IceControlled::from(tie_breaker));
        }

        // MESSAGE-INTEGRITY must precede FINGERPRINT.
        builder = builder.with_attribute(integrity);
        builder = builder.with_attribute(fingerprint);

        let msg = builder.build();

        let encoder = MessageEncoderBuilder::default().build();
        let mut buf = [0u8; ENCODE_BUF_SIZE];
        let size = encoder.encode(&mut buf, &msg).map_err(|e| {
            Error::StunError(format!("stun-rs encode error: {e}"))
        })?;

        Ok(buf[..size].to_vec())
    }

    // ── Message-integrity verification ───────────────────────────────

    /// Verify the MESSAGE-INTEGRITY of a received STUN message using
    /// the `stun-rs` decoder with short-term credentials.
    ///
    /// Returns `true` if the HMAC is valid; `false` on mismatch or if
    /// MESSAGE-INTEGRITY is absent.
    pub fn verify_message_integrity(data: &[u8], key: &[u8]) -> Result<bool, Error> {
        // stun-rs requires an HMACKey; build one from raw bytes via the
        // short-term credential path (key == SASLprep(password)).
        let key_str = std::str::from_utf8(key).map_err(|e| {
            Error::StunError(format!("key is not valid UTF-8: {e}"))
        })?;
        let hmac_key = HMACKey::new_short_term(key_str).map_err(|e| {
            Error::StunError(format!("invalid HMAC key: {e}"))
        })?;

        let ctx = DecoderContextBuilder::default()
            .with_key(hmac_key)
            .with_validation()
            .build();

        let decoder = MessageDecoderBuilder::default().with_context(ctx).build();

        match decoder.decode(data) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_ice_check_controlling() {
        let result = StunClientAdapter::build_ice_check(
            "user1:user2",
            "password123",
            1234,
            true,  // use_candidate
            true,  // controlling
            0xDEAD_BEEF_CAFE_BABE,
        );
        assert!(result.is_ok(), "ICE check build failed: {:?}", result.err());

        let encoded = result.unwrap_or_else(|e| panic!("unreachable: {e}"));
        // Must be at least 20 bytes (STUN header)
        assert!(encoded.len() >= 20);
        // First two bits must be 0
        assert_eq!(encoded[0] & 0xC0, 0);
        // Magic cookie at offset 4..8
        let cookie = u32::from_be_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
        assert_eq!(cookie, 0x2112_A442);
    }

    #[test]
    fn test_build_ice_check_controlled() {
        let result = StunClientAdapter::build_ice_check(
            "user1:user2",
            "password123",
            5678,
            false, // no use_candidate
            false, // controlled
            0x1234_5678_9ABC_DEF0,
        );
        assert!(result.is_ok(), "ICE check build failed: {:?}", result.err());

        let encoded = result.unwrap_or_else(|e| panic!("unreachable: {e}"));
        assert!(encoded.len() >= 20);
    }

    #[test]
    fn test_adapter_new() {
        let addr: SocketAddr = "74.125.250.129:19302"
            .parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let adapter = StunClientAdapter::new(addr);
        assert_eq!(adapter.server_addr(), addr);
    }

    /// Integration test for MESSAGE-INTEGRITY verification using the
    /// RFC 5769 test vector (short-term credential).
    #[test]
    fn test_verify_rfc5769_response() {
        let sample_ipv4_response: [u8; 80] = [
            0x01, 0x01, 0x00, 0x3c, // Response type and message length
            0x21, 0x12, 0xa4, 0x42, // Magic cookie
            0xb7, 0xe7, 0xa7, 0x01, // }
            0xbc, 0x34, 0xd6, 0x86, // }  Transaction ID
            0xfa, 0x87, 0xdf, 0xae, // }
            0x80, 0x22, 0x00, 0x0b, // SOFTWARE attribute header
            0x74, 0x65, 0x73, 0x74, // }
            0x20, 0x76, 0x65, 0x63, // }  UTF-8 server name
            0x74, 0x6f, 0x72, 0x20, // }
            0x00, 0x20, 0x00, 0x08, // XOR-MAPPED-ADDRESS attribute header
            0x00, 0x01, 0xa1, 0x47, // Address family and xor'd port
            0xe1, 0x12, 0xa6, 0x43, // Xor'd IPv4 address
            0x00, 0x08, 0x00, 0x14, // MESSAGE-INTEGRITY header
            0x2b, 0x91, 0xf5, 0x99, // }
            0xfd, 0x9e, 0x90, 0xc3, // }
            0x8c, 0x74, 0x89, 0xf9, // } HMAC-SHA1
            0x2a, 0xf9, 0xba, 0x53, // }
            0xf0, 0x6b, 0xe7, 0xd7, // }
            0x80, 0x28, 0x00, 0x04, // FINGERPRINT attribute header
            0xc0, 0x7d, 0x4c, 0x96, // CRC32 fingerprint
        ];

        let result = StunClientAdapter::verify_message_integrity(
            &sample_ipv4_response,
            b"VOkJxbRl1RmTxUk/WvJxBt",
        );
        assert!(result.is_ok());
        assert!(
            result.unwrap_or(false),
            "MESSAGE-INTEGRITY verification should succeed for RFC 5769 test vector"
        );
    }

    /// Verify that an ICE connectivity check message built by `build_ice_check`
    /// contains the required attributes: USERNAME, PRIORITY, ICE-CONTROLLING (or
    /// ICE-CONTROLLED), MESSAGE-INTEGRITY, and FINGERPRINT.
    /// Also validates MESSAGE-INTEGRITY against the password used to build it.
    #[test]
    fn test_ice_check_message_attributes() {
        let username = "remote:local";
        let password = "supersecretpassword42";
        let priority = 2_130_706_431u32;
        let tie_breaker = 0xDEAD_BEEF_CAFE_BABEu64;

        let encoded = StunClientAdapter::build_ice_check(
            username,
            password,
            priority,
            true,   // use_candidate
            true,   // controlling
            tie_breaker,
        )
        .map_err(|e| Error::StunError(format!("build_ice_check: {e}")))?;

        // Basic STUN header validation
        assert!(encoded.len() >= 20, "STUN message too short");
        // First two bits must be 0 (STUN)
        assert_eq!(encoded[0] & 0xC0, 0, "not a STUN message");
        // Magic cookie
        let cookie = u32::from_be_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
        assert_eq!(cookie, 0x2112_A442, "magic cookie mismatch");

        // Decode the message to verify attributes are present.
        // We use the stun-rs decoder with the correct key so that
        // MESSAGE-INTEGRITY verification passes.
        let hmac_key = stun_rs::HMACKey::new_short_term(password)
            .map_err(|e| Error::StunError(format!("hmac key: {e}")))?;
        let ctx = stun_rs::DecoderContextBuilder::default()
            .with_key(hmac_key)
            .with_validation()
            .build();
        let decoder = stun_rs::MessageDecoderBuilder::default()
            .with_context(ctx)
            .build();

        let (msg, _size) = decoder.decode(&encoded)
            .unwrap_or_else(|e| panic!(
                "decode failed (MESSAGE-INTEGRITY validation should pass): {e:?}"
            ));

        // Verify method = BINDING, class = Request
        assert_eq!(msg.method(), stun_rs::methods::BINDING);
        assert_eq!(msg.class(), stun_rs::MessageClass::Request);

        // Check USERNAME attribute is present with expected value
        use stun_rs::attributes::stun::UserName;
        let user_attr = msg.get::<UserName>();
        assert!(user_attr.is_some(), "USERNAME attribute missing");

        // Check PRIORITY attribute
        use stun_rs::attributes::ice::Priority;
        let prio_attr = msg.get::<Priority>();
        assert!(prio_attr.is_some(), "PRIORITY attribute missing");

        // Check ICE-CONTROLLING (since controlling=true)
        use stun_rs::attributes::ice::IceControlling;
        let ctrl_attr = msg.get::<IceControlling>();
        assert!(ctrl_attr.is_some(), "ICE-CONTROLLING attribute missing");

        // Check FINGERPRINT
        use stun_rs::attributes::stun::Fingerprint;
        let fp_attr = msg.get::<Fingerprint>();
        assert!(fp_attr.is_some(), "FINGERPRINT attribute missing");

        // Check MESSAGE-INTEGRITY
        use stun_rs::attributes::stun::MessageIntegrity;
        let mi_attr = msg.get::<MessageIntegrity>();
        assert!(mi_attr.is_some(), "MESSAGE-INTEGRITY attribute missing");

        // Also verify that a wrong password fails the integrity check
        let wrong_key = stun_rs::HMACKey::new_short_term("wrong-password")
            .map_err(|e| Error::StunError(format!("hmac key: {e}")))?;
        let bad_ctx = stun_rs::DecoderContextBuilder::default()
            .with_key(wrong_key)
            .with_validation()
            .build();
        let bad_decoder = stun_rs::MessageDecoderBuilder::default()
            .with_context(bad_ctx)
            .build();
        assert!(
            bad_decoder.decode(&encoded).is_err(),
            "MESSAGE-INTEGRITY should fail with wrong key"
        );
    }

    /// Verify ICE-CONTROLLED attribute appears when controlling=false.
    #[test]
    fn test_ice_check_controlled_attribute() {
        let encoded = StunClientAdapter::build_ice_check(
            "remote:local",
            "apassword",
            1000,
            false,  // no use_candidate
            false,  // controlled
            0x1234_5678_9ABC_DEF0,
        )
        .map_err(|e| Error::StunError(format!("build_ice_check: {e}")))?;

        let hmac_key = stun_rs::HMACKey::new_short_term("apassword")
            .map_err(|e| Error::StunError(format!("hmac key: {e}")))?;
        let ctx = stun_rs::DecoderContextBuilder::default()
            .with_key(hmac_key)
            .with_validation()
            .build();
        let decoder = stun_rs::MessageDecoderBuilder::default()
            .with_context(ctx)
            .build();

        let (msg, _) = decoder.decode(&encoded)
            .map_err(|e| Error::StunError(format!("decode: {e:?}")))?;

        use stun_rs::attributes::ice::IceControlled;
        let ctrl_attr = msg.get::<IceControlled>();
        assert!(ctrl_attr.is_some(), "ICE-CONTROLLED attribute missing");

        // ICE-CONTROLLING should NOT be present
        use stun_rs::attributes::ice::IceControlling;
        assert!(msg.get::<IceControlling>().is_none(), "ICE-CONTROLLING should not be present");
    }

    /// Ensure that a wrong password fails verification.
    #[test]
    fn test_verify_integrity_wrong_key() {
        let sample_ipv4_response: [u8; 80] = [
            0x01, 0x01, 0x00, 0x3c,
            0x21, 0x12, 0xa4, 0x42,
            0xb7, 0xe7, 0xa7, 0x01,
            0xbc, 0x34, 0xd6, 0x86,
            0xfa, 0x87, 0xdf, 0xae,
            0x80, 0x22, 0x00, 0x0b,
            0x74, 0x65, 0x73, 0x74,
            0x20, 0x76, 0x65, 0x63,
            0x74, 0x6f, 0x72, 0x20,
            0x00, 0x20, 0x00, 0x08,
            0x00, 0x01, 0xa1, 0x47,
            0xe1, 0x12, 0xa6, 0x43,
            0x00, 0x08, 0x00, 0x14,
            0x2b, 0x91, 0xf5, 0x99,
            0xfd, 0x9e, 0x90, 0xc3,
            0x8c, 0x74, 0x89, 0xf9,
            0x2a, 0xf9, 0xba, 0x53,
            0xf0, 0x6b, 0xe7, 0xd7,
            0x80, 0x28, 0x00, 0x04,
            0xc0, 0x7d, 0x4c, 0x96,
        ];

        let result = StunClientAdapter::verify_message_integrity(
            &sample_ipv4_response,
            b"wrong-password",
        );
        assert!(result.is_ok());
        assert!(
            !result.unwrap_or(true),
            "MESSAGE-INTEGRITY should fail with wrong key"
        );
    }
}
