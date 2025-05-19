//! Integration module for the media-core library
//!
//! This module provides integration with other components in the rvoip ecosystem,
//! including session-core for signaling, rtp-core for transport, and SDP handling.

// Session-core integration
pub mod session_core;

// RTP-core integration
pub mod rtp_core;

// SDP handling for media negotiation
pub mod sdp;

// Re-export key types
pub use session_core::{MediaManager, MediaManagerConfig};
pub use rtp_core::{RtpManager, RtpManagerConfig};
pub use sdp::{SdpHandler, SdpMediaDescription};

/// A high-level abstraction for media resource allocation and management
///
/// This is the main entry point for session-core integration, providing
/// a facade over the underlying media capabilities.
#[derive(Debug, Clone)]
pub struct MediaFactory {
    /// Configuration for new media sessions
    config: MediaFactoryConfig,
}

/// Configuration for the media factory
#[derive(Debug, Clone)]
pub struct MediaFactoryConfig {
    /// Base port for RTP
    pub rtp_base_port: u16,
    
    /// Maximum number of concurrent sessions
    pub max_sessions: usize,
    
    /// Default codec preferences
    pub codec_preferences: Vec<String>,
    
    /// Whether to enable SRTP by default
    pub srtp_enabled: bool,
}

impl Default for MediaFactoryConfig {
    fn default() -> Self {
        Self {
            rtp_base_port: 10000,
            max_sessions: 100,
            codec_preferences: vec![
                "opus".to_string(),
                "PCMA".to_string(),
                "PCMU".to_string(),
            ],
            srtp_enabled: true,
        }
    }
}

impl MediaFactory {
    /// Create a new media factory
    pub fn new(config: MediaFactoryConfig) -> Self {
        Self { config }
    }
    
    /// Create a new media manager
    pub fn create_media_manager(&self) -> session_core::MediaManager {
        session_core::MediaManager::new(self.config.clone())
    }
    
    /// Create a new SDP negotiator
    pub fn create_sdp_negotiator(&self) -> sdp::SdpNegotiator {
        sdp::SdpNegotiator::new(self.config.codec_preferences.clone())
    }
} 