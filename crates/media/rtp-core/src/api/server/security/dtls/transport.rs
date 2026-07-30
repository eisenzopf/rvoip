//! Retained DTLS transport helper signatures
//!
//! The DTLS connection stack is unavailable in 0.3.5. Every helper returns a
//! typed unsupported-feature error before binding a raw handler or consuming a
//! datagram.

use std::net::SocketAddr;

use crate::api::common::error::SecurityError;
use crate::api::server::security::SocketHandle;
use crate::dtls::transport::udp::UdpTransport;

/// Create a UDP transport for DTLS
pub async fn create_udp_transport(
    _socket: &SocketHandle,
    _mtu: usize,
) -> Result<UdpTransport, SecurityError> {
    Err(SecurityError::UnsupportedFeature(
        "DTLS UDP transport construction is unavailable in 0.3.5".to_string(),
    ))
}

/// Start a packet handler for DTLS
pub async fn start_packet_handler(
    _socket: &SocketHandle,
    _handler: impl Fn(Vec<u8>, SocketAddr) -> Result<(), SecurityError> + Send + Sync + 'static,
) -> Result<(), SecurityError> {
    Err(SecurityError::UnsupportedFeature(
        "DTLS packet handlers are unavailable in 0.3.5".to_string(),
    ))
}

/// Capture an initial packet from a client
pub async fn capture_initial_packet(
    _socket: &SocketHandle,
    _timeout_secs: u64,
) -> Result<Option<(Vec<u8>, SocketAddr)>, SecurityError> {
    Err(SecurityError::UnsupportedFeature(
        "DTLS initial-packet capture is unavailable in 0.3.5".to_string(),
    ))
}

/// Start a UDP transport
pub async fn start_udp_transport(_transport: &mut UdpTransport) -> Result<(), SecurityError> {
    Err(SecurityError::UnsupportedFeature(
        "DTLS UDP transport startup is unavailable in 0.3.5".to_string(),
    ))
}
