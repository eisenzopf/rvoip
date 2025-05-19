//! Common configuration types
//!
//! This module defines configuration types shared between client and server APIs.

use std::net::SocketAddr;
use crate::api::common::frame::MediaFrameType;

/// Security mode for media transport
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityMode {
    /// No security
    None,
    /// DTLS-SRTP (WebRTC standard)
    DtlsSrtp,
    /// Pre-shared key SRTP (for SIP)
    SrtpPsk,
    /// Custom security mode
    Custom,
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
    /// Security mode
    pub mode: SecurityMode,
    /// DTLS fingerprint (for DtlsSrtp)
    pub fingerprint: Option<String>,
    /// Fingerprint algorithm (for DtlsSrtp)
    pub fingerprint_algorithm: Option<String>,
    /// Crypto suites (string representations for SDP)
    pub crypto_suites: Vec<String>,
    /// Key parameters (for SrtpPsk)
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