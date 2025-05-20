//! Client security API
//!
//! This module provides security-related interfaces for the client-side media transport.

use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use tokio::net::UdpSocket;

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::server::security::SocketHandle;
use crate::dtls::{DtlsConfig, DtlsRole};

mod client_security_impl;

// Re-export public implementation
pub use client_security_impl::DefaultClientSecurityContext;

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
}

impl Default for ClientSecurityConfig {
    fn default() -> Self {
        Self {
            security_mode: SecurityMode::DtlsSrtp,
            fingerprint_algorithm: "sha-256".to_string(),
            remote_fingerprint: None,
            remote_fingerprint_algorithm: None,
            validate_fingerprint: true,
            srtp_profiles: vec![
                SrtpProfile::AesCm128HmacSha1_80,
                SrtpProfile::AesGcm128,
            ],
            certificate_path: None,
            private_key_path: None,
        }
    }
}

/// Convert API SrtpProfile to internal DTLS SrtpProtectionProfile
pub(crate) fn convert_to_dtls_profile(profile: SrtpProfile) -> crate::dtls::message::extension::SrtpProtectionProfile {
    match profile {
        SrtpProfile::AesCm128HmacSha1_80 => crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_80,
        SrtpProfile::AesCm128HmacSha1_32 => crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_32,
        SrtpProfile::AesGcm128 => crate::dtls::message::extension::SrtpProtectionProfile::AeadAes128Gcm,
        SrtpProfile::AesGcm256 => crate::dtls::message::extension::SrtpProtectionProfile::AeadAes256Gcm,
    }
}

/// Create a DtlsConfig from API ClientSecurityConfig
pub(crate) fn create_dtls_config(config: &ClientSecurityConfig) -> DtlsConfig {
    // Verify that SRTP profiles are specified
    if config.srtp_profiles.is_empty() {
        panic!("No SRTP profiles specified in client security config");
    }

    // Convert our API profiles to DTLS profiles
    let dtls_profiles: Vec<crate::dtls::message::extension::SrtpProtectionProfile> = config.srtp_profiles.iter()
        .map(|p| convert_to_dtls_profile(*p))
        .collect();
    
    // Create DTLS config with client role
    let mut dtls_config = DtlsConfig::default();
    dtls_config.role = DtlsRole::Client;
    
    // We need to convert the SrtpProtectionProfile values to SrtpCryptoSuite values
    let crypto_suites: Vec<crate::srtp::SrtpCryptoSuite> = dtls_profiles.iter()
        .map(|profile| match profile {
            &crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_80 => 
                crate::srtp::SRTP_AES128_CM_SHA1_80,
            &crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_32 => 
                crate::srtp::SRTP_AES128_CM_SHA1_32,
            &crate::dtls::message::extension::SrtpProtectionProfile::AeadAes128Gcm => 
                crate::srtp::SRTP_AEAD_AES_128_GCM,
            &crate::dtls::message::extension::SrtpProtectionProfile::AeadAes256Gcm => 
                crate::srtp::SRTP_AEAD_AES_256_GCM,
            &crate::dtls::message::extension::SrtpProtectionProfile::Unknown(_) => 
                panic!("Unknown SRTP protection profile specified"), // Don't silently default
        })
        .collect();
    
    // Never use a default - if no crypto suites were mapped, that's an error
    if crypto_suites.is_empty() {
        panic!("Failed to map any SRTP profiles to crypto suites");
    }
    
    // Set the mapped crypto suites
    dtls_config.srtp_profiles = crypto_suites;
    
    // Set appropriate mtu and timeout values
    dtls_config.mtu = 1200;
    dtls_config.max_retransmissions = 5;
    
    dtls_config
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
    
    /// Set the remote address for the security context
    async fn set_remote_address(&self, addr: SocketAddr) -> Result<(), SecurityError>;
    
    /// Set the socket handle to use for security operations
    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError>;
    
    /// Set the remote fingerprint for DTLS verification
    async fn set_remote_fingerprint(&self, fingerprint: &str, algorithm: &str) -> Result<(), SecurityError>;
    
    /// Get security information for SDP exchange
    async fn get_security_info(&self) -> Result<SecurityInfo, SecurityError>;
    
    /// Close the security context and clean up resources
    async fn close(&self) -> Result<(), SecurityError>;
    
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
} 