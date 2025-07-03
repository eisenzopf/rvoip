//! DTLS (Datagram Transport Layer Security) implementation
//!
//! This module provides a DTLS 1.2 implementation for use with SRTP key exchange.
//! It follows RFC 6347 (DTLS) and RFC 5764 (DTLS-SRTP) specifications.

pub mod connection;
pub mod handshake;
pub mod record;
pub mod alert;
pub mod crypto;
pub mod message;
pub mod transport;
pub mod srtp;

// Re-export key public API types
pub use connection::DtlsConnection;
pub use srtp::extractor::DtlsSrtpContext;
pub use crypto::keys::DtlsKeyingMaterial;

/// DTLS protocol version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtlsVersion {
    /// DTLS 1.0 (equivalent to TLS 1.1)
    Dtls10 = 0xFEFF,
    
    /// DTLS 1.2 (equivalent to TLS 1.2)
    Dtls12 = 0xFEFD,
}

/// DTLS connection role
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtlsRole {
    /// DTLS client role
    Client,
    
    /// DTLS server role
    Server,
}

/// DTLS connection configuration
#[derive(Debug, Clone)]
pub struct DtlsConfig {
    /// The DTLS role (client or server)
    pub role: DtlsRole,
    
    /// The DTLS protocol version
    pub version: DtlsVersion,
    
    /// Maximum transmission unit (MTU) size
    pub mtu: usize,
    
    /// Maximum number of retransmissions
    pub max_retransmissions: usize,
    
    /// SRTP profiles to offer/accept
    pub srtp_profiles: Vec<crate::srtp::SrtpCryptoSuite>,
}

impl Default for DtlsConfig {
    fn default() -> Self {
        Self {
            role: DtlsRole::Client,
            version: DtlsVersion::Dtls12,
            mtu: 1200,
            max_retransmissions: 5,
            srtp_profiles: vec![
                crate::srtp::SRTP_AES128_CM_SHA1_80,
                crate::srtp::SRTP_AES128_CM_SHA1_32,
            ],
        }
    }
}

/// Result type for DTLS operations
pub type Result<T> = std::result::Result<T, crate::error::Error>;

/// Creates a new DTLS connection with the given configuration
///
/// # Arguments
/// * `config` - The DTLS connection configuration
///
/// # Returns
/// A new DTLS connection
pub async fn create_connection(config: DtlsConfig) -> Result<DtlsConnection> {
    unimplemented!("DTLS implementation is not yet complete")
} 