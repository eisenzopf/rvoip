//! Common configuration types
//!
//! This module defines configuration types shared between client and server components.

use std::net::SocketAddr;
use crate::api::common::frame::MediaFrameType;

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