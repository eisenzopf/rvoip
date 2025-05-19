//! Security API
//!
//! This module provides a simplified interface for securing RTP/RTCP communications
//! using DTLS-SRTP with sensible defaults and presets.

use std::sync::Arc;
use thiserror::Error;

/// Error types for security operations
#[derive(Error, Debug)]
pub enum SecurityError {
    /// Failed to initialize security
    #[error("Failed to initialize security: {0}")]
    InitError(String),
    
    /// Error during DTLS handshake
    #[error("DTLS handshake error: {0}")]
    HandshakeError(String),
    
    /// Error during SRTP operations
    #[error("SRTP error: {0}")]
    SrtpError(String),
    
    /// Certificate error
    #[error("Certificate error: {0}")]
    CertificateError(String),
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
}

/// Security mode for media transport
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityMode {
    /// No security (plain RTP)
    None,
    
    /// DTLS-SRTP (recommended)
    DtlsSrtp,
    
    /// SRTP with pre-shared keys
    SrtpWithPsk,
}

/// SRTP protection profile
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpProfile {
    /// AES-CM with 128-bit keys and HMAC-SHA1 authentication (80-bit tag)
    AesCm128HmacSha1_80,
    
    /// AES-CM with 128-bit keys and HMAC-SHA1 authentication (32-bit tag)
    AesCm128HmacSha1_32,
    
    /// AES-GCM with 128-bit keys
    AesGcm128,
    
    /// AES-GCM with 256-bit keys
    AesGcm256,
}

/// Security configuration for media transport
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Security mode to use
    pub mode: SecurityMode,
    
    /// SRTP protection profiles in order of preference
    pub srtp_profiles: Vec<SrtpProfile>,
    
    /// Whether to require secure transport
    pub require_secure: bool,
    
    /// DTLS setup role (true = active/client, false = passive/server)
    pub dtls_client: bool,
    
    /// Pre-shared SRTP key material (only used in SrtpWithPsk mode)
    pub psk_material: Option<Vec<u8>>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            mode: SecurityMode::DtlsSrtp,
            srtp_profiles: vec![
                SrtpProfile::AesGcm128,
                SrtpProfile::AesCm128HmacSha1_80,
            ],
            require_secure: true,
            dtls_client: true,
            psk_material: None,
        }
    }
}

/// Builder for SecurityConfig
pub struct SecurityConfigBuilder {
    config: SecurityConfig,
}

impl SecurityConfigBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self {
            config: SecurityConfig::default(),
        }
    }
    
    /// Create the WebRTC profile (optimized for WebRTC compatibility)
    pub fn webrtc() -> Self {
        let mut config = SecurityConfig::default();
        config.srtp_profiles = vec![
            SrtpProfile::AesGcm128, 
            SrtpProfile::AesCm128HmacSha1_80,
        ];
        Self { config }
    }
    
    /// Create the SIP profile (optimized for SIP compatibility)
    pub fn sip() -> Self {
        let mut config = SecurityConfig::default();
        config.srtp_profiles = vec![
            SrtpProfile::AesCm128HmacSha1_80,
            SrtpProfile::AesCm128HmacSha1_32,
        ];
        Self { config }
    }
    
    /// Set the security mode
    pub fn mode(mut self, mode: SecurityMode) -> Self {
        self.config.mode = mode;
        self
    }
    
    /// Set the SRTP protection profiles in order of preference
    pub fn srtp_profiles(mut self, profiles: Vec<SrtpProfile>) -> Self {
        self.config.srtp_profiles = profiles;
        self
    }
    
    /// Set whether to require secure transport
    pub fn require_secure(mut self, require: bool) -> Self {
        self.config.require_secure = require;
        self
    }
    
    /// Set the DTLS setup role (true = client, false = server)
    pub fn dtls_client(mut self, client: bool) -> Self {
        self.config.dtls_client = client;
        self
    }
    
    /// Set pre-shared key material for SRTP (only used in SrtpWithPsk mode)
    pub fn psk_material(mut self, material: Vec<u8>) -> Self {
        self.config.psk_material = Some(material);
        self
    }
    
    /// Build the configuration
    pub fn build(self) -> Result<SecurityConfig, SecurityError> {
        // Validate configuration
        if self.config.mode == SecurityMode::SrtpWithPsk && self.config.psk_material.is_none() {
            return Err(SecurityError::ConfigurationError(
                "PSK material must be provided when using SrtpWithPsk mode".to_string()
            ));
        }
        
        Ok(self.config)
    }
}

/// Information about the secure context for SDP
#[derive(Debug, Clone)]
pub struct SecurityInfo {
    /// DTLS fingerprint (e.g., for SDP a=fingerprint)
    pub fingerprint: Option<String>,
    
    /// Fingerprint hash algorithm (e.g., "sha-256")
    pub fingerprint_algorithm: Option<String>,
    
    /// DTLS setup role as string (e.g., "active", "passive")
    pub setup_role: String,
    
    /// Negotiated SRTP profile
    pub srtp_profile: Option<SrtpProfile>,
}

/// Secure media context for DTLS-SRTP
///
/// This trait provides an interface for securing media transport with DTLS-SRTP.
pub trait SecureMediaContext: Send + Sync {
    /// Get security information for SDP
    fn get_security_info(&self) -> SecurityInfo;
    
    /// Start the DTLS handshake
    async fn start_handshake(&self) -> Result<(), SecurityError>;
    
    /// Check if the context is secure (handshake completed)
    fn is_secure(&self) -> bool;
    
    /// Set remote fingerprint from SDP
    fn set_remote_fingerprint(&mut self, fingerprint: &str, algorithm: &str) 
        -> Result<(), SecurityError>;
}

/// Factory for creating SecureMediaContext instances
pub struct SecurityFactory;

impl SecurityFactory {
    /// Create a new SecureMediaContext
    pub async fn create_context(
        config: SecurityConfig,
    ) -> Result<Arc<dyn SecureMediaContext>, SecurityError> {
        // This is a placeholder that will be implemented to create the actual security context
        // based on the internal DTLS/SRTP implementation
        todo!("Implement context creation using internal components")
    }
} 