//! Client security API
//!
//! This module provides security-related interfaces for the client-side media transport.

use async_trait::async_trait;
use std::any::Any;
use std::net::SocketAddr;

use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::common::error::SecurityError;
use crate::api::server::security::SocketHandle;
use crate::dtls::{DtlsConfig, DtlsRole};

// Export modules
pub mod default;
pub mod dtls;
pub mod fingerprint;
pub mod packet;
pub mod srtp;

// Re-export public implementation
pub use default::DefaultClientSecurityContext;

/// Client security configuration
#[derive(Debug, Clone)]
pub struct ClientSecurityConfig {
    /// Security mode to use
    pub security_mode: SecurityMode,
    /// DTLS fingerprint algorithm
    pub fingerprint_algorithm: String,
    /// Remote DTLS fingerprint (if known)
    pub remote_fingerprint: Option<String>,
    /// Remote fingerprint algorithm (if known)
    pub remote_fingerprint_algorithm: Option<String>,
    /// Whether to validate remote fingerprint
    pub validate_fingerprint: bool,
    /// SRTP profiles supported (in order of preference)
    pub srtp_profiles: Vec<SrtpProfile>,
    /// Path to certificate file (PEM format)
    pub certificate_path: Option<String>,
    /// Path to private key file (PEM format)
    pub private_key_path: Option<String>,
    /// Pre-shared SRTP key (for SRTP mode)
    pub srtp_key: Option<Vec<u8>>,
}

impl Default for ClientSecurityConfig {
    fn default() -> Self {
        Self {
            security_mode: SecurityMode::DtlsSrtp,
            fingerprint_algorithm: "sha-256".to_string(),
            remote_fingerprint: None,
            remote_fingerprint_algorithm: None,
            validate_fingerprint: true,
            srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
            certificate_path: None,
            private_key_path: None,
            srtp_key: None,
        }
    }
}

impl ClientSecurityConfig {
    /// Plain RTP configuration with no latent DTLS defaults.
    pub fn unsecured() -> Self {
        Self {
            security_mode: SecurityMode::None,
            validate_fingerprint: false,
            srtp_profiles: Vec::new(),
            ..Self::default()
        }
    }
}

/// Convert API SrtpProfile to internal DTLS SrtpProtectionProfile
pub(crate) fn convert_to_dtls_profile(
    profile: SrtpProfile,
) -> Result<crate::dtls::message::extension::SrtpProtectionProfile, SecurityError> {
    match profile {
        SrtpProfile::AesCm128HmacSha1_80 => {
            Ok(crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_80)
        }
        SrtpProfile::AesCm128HmacSha1_32 => {
            Ok(crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_32)
        }
        SrtpProfile::AesGcm128 | SrtpProfile::AesGcm256 => Err(SecurityError::UnsupportedFeature(
            format!("SRTP profile {profile:?} is not implemented"),
        )),
    }
}

/// Create a DtlsConfig from API ClientSecurityConfig
pub(crate) fn create_dtls_config(
    config: &ClientSecurityConfig,
) -> Result<DtlsConfig, SecurityError> {
    // Verify that SRTP profiles are specified
    if config.srtp_profiles.is_empty() {
        return Err(SecurityError::Configuration(
            "No SRTP profiles specified in client security config".to_string(),
        ));
    }

    // Convert our API profiles to DTLS profiles
    let dtls_profiles: Vec<crate::dtls::message::extension::SrtpProtectionProfile> = config
        .srtp_profiles
        .iter()
        .map(|p| convert_to_dtls_profile(*p))
        .collect::<Result<_, _>>()?;

    // Create DTLS config with client role
    let mut dtls_config = DtlsConfig::default();
    dtls_config.role = DtlsRole::Client;

    // We need to convert the SrtpProtectionProfile values to SrtpCryptoSuite values
    let crypto_suites: Vec<crate::srtp::SrtpCryptoSuite> = dtls_profiles
        .into_iter()
        .map(|profile| match profile {
            crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_80 => {
                Ok(crate::srtp::SRTP_AES128_CM_SHA1_80)
            }
            crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_32 => {
                Ok(crate::srtp::SRTP_AES128_CM_SHA1_32)
            }
            unsupported => Err(SecurityError::UnsupportedFeature(format!(
                "DTLS-SRTP profile {unsupported:?} is not implemented"
            ))),
        })
        .collect::<Result<_, _>>()?;

    // Never use a default - if no crypto suites were mapped, that's an error
    if crypto_suites.is_empty() {
        return Err(SecurityError::Configuration(
            "Failed to map any SRTP profiles to crypto suites".to_string(),
        ));
    }

    // Set the mapped crypto suites
    dtls_config.srtp_profiles = crypto_suites;

    // Set appropriate mtu and timeout values
    dtls_config.mtu = 1200;
    dtls_config.max_retransmissions = 5;

    Ok(dtls_config)
}

/// Client security context interface
///
/// This trait defines the interface for client-side security operations,
/// including the DTLS handshake and SRTP key extraction.
#[async_trait]
pub trait ClientSecurityContext: Send + Sync {
    /// Initialize the security context
    async fn initialize(&self) -> Result<(), SecurityError>;

    /// Start the DTLS handshake with the server
    async fn start_handshake(&self) -> Result<(), SecurityError>;

    /// Check if the security handshake is complete
    async fn is_handshake_complete(&self) -> Result<bool, SecurityError>;

    /// Wait for the DTLS handshake to complete
    async fn wait_for_handshake(&self) -> Result<(), SecurityError>;

    /// Set the remote address for the security context
    async fn set_remote_address(&self, addr: SocketAddr) -> Result<(), SecurityError>;

    /// Set the socket handle to use for security operations
    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError>;

    /// Set the remote fingerprint for DTLS verification
    async fn set_remote_fingerprint(
        &self,
        fingerprint: &str,
        algorithm: &str,
    ) -> Result<(), SecurityError>;

    /// Perform a complete handshake in a single call
    /// This combines setting the remote fingerprint, starting handshake, and waiting for completion
    async fn complete_handshake(
        &self,
        remote_addr: SocketAddr,
        remote_fingerprint: &str,
    ) -> Result<(), SecurityError>;

    /// Process a DTLS packet manually
    /// This allows for explicit processing of received DTLS packets
    async fn process_packet(&self, data: &[u8]) -> Result<(), SecurityError>;

    /// Start automatic packet handler to process incoming DTLS packets
    /// This creates a background task that receives packets from the socket
    /// and automatically passes them to process_packet
    async fn start_packet_handler(&self) -> Result<(), SecurityError>;

    /// Get security information for SDP exchange
    async fn get_security_info(&self) -> Result<SecurityInfo, SecurityError>;

    /// Close the security context and clean up resources
    async fn close(&self) -> Result<(), SecurityError>;

    /// Check if the security context is fully initialized and ready to start a handshake
    /// This verifies that all prerequisites (socket, transport, etc.) are set
    async fn is_ready(&self) -> Result<bool, SecurityError>;

    /// Is the client using secure transport?
    fn is_secure(&self) -> bool;

    /// Get basic security information synchronously
    /// (for use during initialization when async isn't available)
    fn get_security_info_sync(&self) -> SecurityInfo;

    /// Get the local fingerprint (client's fingerprint)
    async fn get_fingerprint(&self) -> Result<String, SecurityError>;

    /// Get the local fingerprint algorithm (client's algorithm)
    async fn get_fingerprint_algorithm(&self) -> Result<String, SecurityError>;

    /// Check if transport is set
    async fn has_transport(&self) -> Result<bool, SecurityError>;

    /// Process a DTLS packet received from the server
    async fn process_dtls_packet(&self, data: &[u8]) -> Result<(), SecurityError>;

    /// Get the security configuration
    fn get_config(&self) -> &ClientSecurityConfig;

    /// Allow downcasting for internal implementation details
    fn as_any(&self) -> &dyn Any;
}
