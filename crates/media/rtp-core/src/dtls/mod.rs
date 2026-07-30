//! DTLS (Datagram Transport Layer Security) implementation
//!
//! This module provides a DTLS 1.2 implementation for use with SRTP key exchange.
//! It follows RFC 6347 (DTLS) and RFC 5764 (DTLS-SRTP) specifications.

pub mod alert;
pub mod connection;
pub mod crypto;
pub mod handshake;
pub mod message;
pub mod record;
pub mod srtp;
pub mod transport;

// Re-export key public API types
pub use connection::DtlsConnection;
pub use crypto::keys::DtlsKeyingMaterial;
pub use srtp::extractor::DtlsSrtpContext;

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

impl DtlsConfig {
    /// Validate that every configured SRTP profile can actually be used.
    pub fn validate(&self) -> Result<()> {
        if self.srtp_profiles.is_empty() {
            return Err(crate::error::Error::InvalidParameter(
                "at least one implemented SRTP profile is required".to_string(),
            ));
        }
        for profile in &self.srtp_profiles {
            profile.validate()?;
            if profile != &crate::srtp::SRTP_AES128_CM_SHA1_80
                && profile != &crate::srtp::SRTP_AES128_CM_SHA1_32
            {
                return Err(crate::Error::UnsupportedFeature(format!(
                    "SRTP suite {profile:?} is not implemented for DTLS-SRTP"
                )));
            }
        }
        Ok(())
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
/// A typed unsupported-feature error after validating the configuration. The
/// signature is retained so callers can handle this release's fail-closed
/// behavior without a panic.
pub async fn create_connection(config: DtlsConfig) -> Result<DtlsConnection> {
    config.validate()?;

    Err(crate::error::Error::UnsupportedFeature(
        "DTLS connection construction is not supported in this release".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn construction_fails_without_panicking() {
        let Err(error) = create_connection(DtlsConfig::default()).await else {
            panic!("incomplete DTLS must fail closed");
        };
        assert!(matches!(error, crate::Error::UnsupportedFeature(_)));
    }

    #[tokio::test]
    async fn construction_rejects_unimplemented_gcm_profile_first() {
        let mut config = DtlsConfig::default();
        config.srtp_profiles = vec![crate::srtp::SRTP_AEAD_AES_128_GCM];
        let Err(error) = create_connection(config).await else {
            panic!("AES-GCM must not be negotiated as AES-CM");
        };
        assert!(
            matches!(error, crate::Error::UnsupportedFeature(message) if message.contains("AES-GCM"))
        );
    }

    #[test]
    fn incomplete_constructor_is_test_only() {
        let connection = connection::DtlsConnection::new_for_test(DtlsConfig::default());
        assert_eq!(connection.state(), connection::ConnectionState::New);
    }
}
