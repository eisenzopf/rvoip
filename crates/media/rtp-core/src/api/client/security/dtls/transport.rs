//! Retained DTLS transport setup signatures
//!
//! The DTLS connection stack is unavailable in 0.3.5. These public helpers
//! remain source-compatible but return a typed unsupported-feature error before
//! opening a raw receive path or mutating connection state.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use crate::api::client::security::ClientSecurityContext;
use crate::api::common::error::SecurityError;
use crate::api::server::security::SocketHandle;
use crate::dtls::transport::udp::UdpTransport;
use crate::dtls::DtlsConnection;

/// Set up a DTLS transport with the given socket
pub async fn setup_transport(
    _socket: &SocketHandle,
    _connection: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<(), SecurityError> {
    Err(SecurityError::UnsupportedFeature(
        "DTLS transport setup is unavailable in 0.3.5".to_string(),
    ))
}

/// Start a packet handler for the DTLS transport
pub async fn start_packet_handler(
    _socket: &SocketHandle,
    _remote_addr: SocketAddr,
    _context: Arc<dyn ClientSecurityContext>,
) -> Result<(), SecurityError> {
    Err(SecurityError::UnsupportedFeature(
        "DTLS packet handlers are unavailable in 0.3.5".to_string(),
    ))
}

/// Create a UDP transport for DTLS
pub async fn create_udp_transport(
    _socket: Arc<UdpSocket>,
    _mtu: usize,
) -> Result<UdpTransport, SecurityError> {
    Err(SecurityError::UnsupportedFeature(
        "DTLS UDP transport construction is unavailable in 0.3.5".to_string(),
    ))
}
