//! Common configuration types
//!
//! This module defines configuration types shared between client and server APIs.

use std::net::SocketAddr;
use crate::api::common::frame::MediaFrameType;

/// Security mode for transport
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityMode {
    /// No security (plain RTP)
    None,
    
    /// SRTP with pre-shared keys
    Srtp,
    
    /// DTLS-SRTP (keys negotiated via DTLS)
    DtlsSrtp,
}

impl SecurityMode {
    /// Check if security is enabled
    pub fn is_enabled(&self) -> bool {
        match self {
            SecurityMode::None => false,
            _ => true,
        }
    }
}

impl Default for SecurityMode {
    fn default() -> Self {
        SecurityMode::DtlsSrtp
    }
}

/// Identity validation mechanism
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityValidation {
    /// No validation (use with caution)
    None,
    /// Fingerprint validation (DTLS)
    Fingerprint,
    /// Certificate validation (DTLS)
    Certificate,
    /// Custom validation
    Custom,
}

/// SRTP profiles for negotiation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpProfile {
    /// AES_CM_128_HMAC_SHA1_80 (most common)
    AesCm128HmacSha1_80,
    /// AES_CM_128_HMAC_SHA1_32 (reduced auth tag for bandwidth savings)
    AesCm128HmacSha1_32,
    /// AEAD_AES_128_GCM (more secure, less overhead)
    AesGcm128,
    /// AEAD_AES_256_GCM (highest security)
    AesGcm256,
}

/// Network condition preset for buffer configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPreset {
    /// Minimal latency, good for LAN
    LowLatency,
    
    /// Balanced preset, good for stable broadband
    Balanced,
    
    /// Resilient preset, good for mobile or unstable networks
    Resilient,
    
    /// Maximum protection, for very unstable networks
    HighProtection,
}

/// Base transport configuration shared by client and server
#[derive(Debug, Clone)]
pub struct BaseTransportConfig {
    /// Local address to bind to
    pub local_address: Option<SocketAddr>,
    /// Whether to use RTCP multiplexing (RTP and RTCP on same port)
    pub rtcp_mux: bool,
    /// Media types enabled for this transport
    pub media_types: Vec<MediaFrameType>,
    /// Maximum transmission unit size
    pub mtu: usize,
}

/// Security information for SDP exchange
#[derive(Debug, Clone)]
pub struct SecurityInfo {
    /// Security mode (None, Srtp, DtlsSrtp)
    pub mode: SecurityMode,
    
    /// DTLS fingerprint (for DtlsSrtp)
    pub fingerprint: Option<String>,
    
    /// Fingerprint algorithm (for DtlsSrtp)
    pub fingerprint_algorithm: Option<String>,
    
    /// Crypto suites in order of preference
    pub crypto_suites: Vec<String>,
    
    /// Key parameters (for Srtp)
    pub key_params: Option<String>,
    
    /// Selected SRTP profile
    pub srtp_profile: Option<String>,
}

impl Default for SecurityInfo {
    fn default() -> Self {
        Self {
            mode: SecurityMode::None,
            fingerprint: None,
            fingerprint_algorithm: None,
            crypto_suites: Vec::new(),
            key_params: None,
            srtp_profile: None,
        }
    }
}

/// Predefined security profiles for common use cases
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityProfile {
    /// No security - plain RTP
    Unsecured,
    
    /// Basic SRTP with pre-shared key (for simple deployments)
    SrtpBasic,
    
    /// DTLS-SRTP with self-signed certificates (common for WebRTC)
    DtlsSrtpSelfSigned,
    
    /// DTLS-SRTP with provided certificates (enterprise/production)
    DtlsSrtpCertificate,
    
    /// Custom configuration (use the detailed SecurityConfig)
    Custom,
}

impl Default for SecurityProfile {
    fn default() -> Self {
        // WebRTC-style DTLS-SRTP is a good default for modern systems
        SecurityProfile::DtlsSrtpSelfSigned
    }
}

/// Complete security configuration with reasonable defaults
/// This struct makes it easy to configure security without understanding
/// all the underlying details of DTLS-SRTP, SRTP, etc.
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Security profile (for common configurations)
    pub profile: SecurityProfile,
    
    /// Security mode (None, SRTP, DTLS-SRTP)
    pub mode: SecurityMode,
    
    /// Whether security is required (fail if not available)
    pub required: bool,
    
    /// SRTP profiles in order of preference
    pub srtp_profiles: Vec<SrtpProfile>,
    
    /// Certificate file path (PEM format)
    pub certificate_path: Option<String>,
    
    /// Private key file path (PEM format)
    pub private_key_path: Option<String>,
    
    /// Fingerprint algorithm for DTLS
    pub fingerprint_algorithm: String,
    
    /// Pre-shared key for SRTP (used when mode is Srtp)
    pub srtp_key: Option<Vec<u8>>,
    
    /// Require client certificate validation 
    pub require_client_certificate: bool,
    
    /// Remote fingerprint (if known, e.g. from SDP)
    pub remote_fingerprint: Option<String>,
    
    /// Remote fingerprint algorithm
    pub remote_fingerprint_algorithm: Option<String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            profile: SecurityProfile::default(),
            mode: SecurityMode::DtlsSrtp, 
            required: true,
            srtp_profiles: vec![
                SrtpProfile::AesCm128HmacSha1_80,
                SrtpProfile::AesGcm128,
            ],
            certificate_path: None,
            private_key_path: None,
            fingerprint_algorithm: "sha-256".to_string(),
            srtp_key: None,
            require_client_certificate: false,
            remote_fingerprint: None,
            remote_fingerprint_algorithm: None,
        }
    }
}

impl SecurityConfig {
    /// Create a security configuration from a predefined profile
    pub fn from_profile(profile: SecurityProfile) -> Self {
        match profile {
            SecurityProfile::Unsecured => {
                Self {
                    profile,
                    mode: SecurityMode::None,
                    required: false,
                    srtp_profiles: vec![],
                    certificate_path: None,
                    private_key_path: None,
                    fingerprint_algorithm: "sha-256".to_string(),
                    srtp_key: None,
                    require_client_certificate: false,
                    remote_fingerprint: None,
                    remote_fingerprint_algorithm: None,
                }
            },
            
            SecurityProfile::SrtpBasic => {
                Self {
                    profile,
                    mode: SecurityMode::Srtp,
                    required: true,
                    srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
                    certificate_path: None,
                    private_key_path: None,
                    fingerprint_algorithm: "sha-256".to_string(),
                    // Default key will need to be set by the user
                    srtp_key: None,
                    require_client_certificate: false,
                    remote_fingerprint: None,
                    remote_fingerprint_algorithm: None,
                }
            },
            
            SecurityProfile::DtlsSrtpSelfSigned => {
                Self {
                    profile,
                    mode: SecurityMode::DtlsSrtp,
                    required: true,
                    srtp_profiles: vec![
                        SrtpProfile::AesCm128HmacSha1_80,
                        SrtpProfile::AesGcm128,
                    ],
                    certificate_path: None, // Will use self-signed
                    private_key_path: None, // Will use self-signed
                    fingerprint_algorithm: "sha-256".to_string(),
                    srtp_key: None, // Not needed for DTLS-SRTP
                    require_client_certificate: false,
                    remote_fingerprint: None,
                    remote_fingerprint_algorithm: None,
                }
            },
            
            SecurityProfile::DtlsSrtpCertificate => {
                Self {
                    profile,
                    mode: SecurityMode::DtlsSrtp,
                    required: true,
                    srtp_profiles: vec![
                        SrtpProfile::AesCm128HmacSha1_80,
                        SrtpProfile::AesGcm128,
                    ],
                    // Paths need to be set by user
                    certificate_path: None, 
                    private_key_path: None,
                    fingerprint_algorithm: "sha-256".to_string(),
                    srtp_key: None, // Not needed for DTLS-SRTP
                    require_client_certificate: false, // Optional in most deployments
                    remote_fingerprint: None,
                    remote_fingerprint_algorithm: None,
                }
            },
            
            SecurityProfile::Custom => {
                // Use all defaults for custom profile
                Self::default()
            }
        }
    }
    
    /// Create an unsecured configuration (plain RTP)
    pub fn unsecured() -> Self {
        Self::from_profile(SecurityProfile::Unsecured)
    }
    
    /// Create a basic SRTP configuration with a pre-shared key
    pub fn srtp_with_key(key: Vec<u8>) -> Self {
        let mut config = Self::from_profile(SecurityProfile::SrtpBasic);
        config.srtp_key = Some(key);
        config
    }
    
    /// Create a WebRTC-compatible DTLS-SRTP configuration with self-signed certificates
    pub fn webrtc_compatible() -> Self {
        Self::from_profile(SecurityProfile::DtlsSrtpSelfSigned)
    }
    
    /// Create a DTLS-SRTP configuration with provided certificate files
    pub fn dtls_with_certificate(cert_path: String, key_path: String) -> Self {
        let mut config = Self::from_profile(SecurityProfile::DtlsSrtpCertificate);
        config.certificate_path = Some(cert_path);
        config.private_key_path = Some(key_path);
        config
    }
    
    /// Set the remote party's fingerprint (e.g. from SDP)
    pub fn with_remote_fingerprint(mut self, fingerprint: String, algorithm: Option<String>) -> Self {
        self.remote_fingerprint = Some(fingerprint);
        self.remote_fingerprint_algorithm = algorithm.or_else(|| Some(self.fingerprint_algorithm.clone()));
        self
    }
    
    /// Make security optional (don't fail if unavailable)
    pub fn with_optional_security(mut self) -> Self {
        self.required = false;
        self
    }
} 