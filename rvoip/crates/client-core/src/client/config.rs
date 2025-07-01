//! Client configuration structures and presets
//!
//! This module provides configuration structures for the VoIP client, including
//! media settings, codec preferences, and network parameters.
//!
//! # Key Components
//!
//! - **ClientConfig** - Main client configuration
//! - **MediaConfig** - Media-specific settings and preferences
//! - **MediaPreset** - Predefined media configuration templates
//!
//! # Usage Examples
//!
//! ## Basic Client Configuration
//!
//! ```rust
//! use rvoip_client_core::client::config::{ClientConfig, MediaPreset};
//! use std::net::SocketAddr;
//!
//! let config = ClientConfig::new()
//!     .with_sip_addr("127.0.0.1:5060".parse().unwrap())
//!     .with_media_addr("127.0.0.1:0".parse().unwrap())
//!     .with_user_agent("MyApp/1.0".to_string())
//!     .with_max_calls(5);
//!
//! assert_eq!(config.max_concurrent_calls, 5);
//! ```
//!
//! ## Using Media Presets
//!
//! ```rust
//! use rvoip_client_core::client::config::{ClientConfig, MediaPreset};
//!
//! let secure_config = ClientConfig::new()
//!     .with_media_preset(MediaPreset::Secure);
//!
//! assert!(secure_config.media.require_srtp);
//! ```

use std::net::SocketAddr;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Media configuration preferences
/// 
/// Defines media-related settings including codec preferences, audio processing
/// options, security requirements, and network parameters.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::client::config::MediaConfig;
/// 
/// let media_config = MediaConfig {
///     preferred_codecs: vec!["opus".to_string(), "PCMU".to_string()],
///     dtmf_enabled: true,
///     echo_cancellation: true,
///     noise_suppression: true,
///     auto_gain_control: true,
///     max_bandwidth_kbps: Some(128),
///     require_srtp: false,
///     srtp_profiles: vec![],
///     rtp_port_start: 10000,
///     rtp_port_end: 20000,
///     preferred_ptime: Some(20),
///     custom_sdp_attributes: std::collections::HashMap::new(),
/// };
/// 
/// assert_eq!(media_config.preferred_codecs[0], "opus");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Preferred codecs in order of preference
    pub preferred_codecs: Vec<String>,
    
    /// Whether DTMF (Dual-Tone Multi-Frequency) signaling is enabled
    pub dtmf_enabled: bool,
    /// Whether echo cancellation audio processing is enabled
    pub echo_cancellation: bool,
    /// Whether noise suppression audio processing is enabled
    pub noise_suppression: bool,
    /// Whether automatic gain control audio processing is enabled
    pub auto_gain_control: bool,
    
    /// Maximum bandwidth in kilobits per second (None for unlimited)
    pub max_bandwidth_kbps: Option<u32>,
    
    /// Whether SRTP (Secure RTP) encryption is required
    pub require_srtp: bool,
    /// List of supported SRTP encryption profiles
    pub srtp_profiles: Vec<String>,
    
    /// Starting port number for RTP media streams
    pub rtp_port_start: u16,
    /// Ending port number for RTP media streams
    pub rtp_port_end: u16,
    
    /// Preferred packetization time in milliseconds
    pub preferred_ptime: Option<u8>,
    
    /// Additional custom SDP (Session Description Protocol) attributes
    pub custom_sdp_attributes: HashMap<String, String>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            preferred_codecs: vec!["opus".to_string(), "PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            max_bandwidth_kbps: None,
            require_srtp: false,
            srtp_profiles: vec![],
            rtp_port_start: 10000,
            rtp_port_end: 20000,
            preferred_ptime: Some(20),
            custom_sdp_attributes: HashMap::new(),
        }
    }
}

/// Media configuration presets
#[derive(Debug, Clone, Copy)]
pub enum MediaPreset {
    /// Optimized for voice calls (opus, echo cancellation, noise suppression)
    VoiceOptimized,
    /// Optimized for music (opus, no echo cancellation)
    MusicOptimized,
    /// Optimized for low bandwidth connections
    LowBandwidth,
    /// Requires SRTP encryption
    Secure,
    /// Basic G.711 compatibility mode
    Legacy,
}

impl MediaConfig {
    /// Create config from preset
    pub fn from_preset(preset: MediaPreset) -> Self {
        match preset {
            MediaPreset::VoiceOptimized => Self {
                preferred_codecs: vec!["opus".to_string(), "PCMU".to_string()],
                echo_cancellation: true,
                noise_suppression: true,
                auto_gain_control: true,
                preferred_ptime: Some(20),
                ..Default::default()
            },
            MediaPreset::MusicOptimized => Self {
                preferred_codecs: vec!["opus".to_string()],
                echo_cancellation: false,
                noise_suppression: false,
                auto_gain_control: false,
                max_bandwidth_kbps: Some(256),
                ..Default::default()
            },
            MediaPreset::LowBandwidth => Self {
                preferred_codecs: vec!["G.729".to_string(), "GSM".to_string(), "PCMU".to_string()],
                max_bandwidth_kbps: Some(32),
                preferred_ptime: Some(30),
                ..Default::default()
            },
            MediaPreset::Secure => Self {
                require_srtp: true,
                srtp_profiles: vec![
                    "AES_CM_128_HMAC_SHA1_80".to_string(),
                    "AES_CM_128_HMAC_SHA1_32".to_string(),
                ],
                ..Default::default()
            },
            MediaPreset::Legacy => Self {
                preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
                dtmf_enabled: true,
                echo_cancellation: false,
                noise_suppression: false,
                auto_gain_control: false,
                ..Default::default()
            },
        }
    }
}

/// Configuration for the SIP client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Local SIP bind address
    pub local_sip_addr: SocketAddr,
    /// Local media bind address  
    pub local_media_addr: SocketAddr,
    /// User agent string
    pub user_agent: String,
    /// Media configuration
    pub media: MediaConfig,
    /// Maximum number of concurrent calls
    pub max_concurrent_calls: usize,
    /// Session timeout in seconds
    pub session_timeout_secs: u64,
    /// Enable audio processing
    pub enable_audio: bool,
    /// Enable video processing (future)
    pub enable_video: bool,
    /// SIP domain (optional)
    pub domain: Option<String>,
}

impl ClientConfig {
    /// Create a new client configuration with defaults
    pub fn new() -> Self {
        Self {
            local_sip_addr: "127.0.0.1:0".parse().unwrap(),
            local_media_addr: "127.0.0.1:0".parse().unwrap(),
            user_agent: "rvoip-client-core/0.1.0".to_string(),
            media: MediaConfig::default(),
            max_concurrent_calls: 10,
            session_timeout_secs: 300,
            enable_audio: true,
            enable_video: false,
            domain: None,
        }
    }

    /// Set SIP bind address
    pub fn with_sip_addr(mut self, addr: SocketAddr) -> Self {
        self.local_sip_addr = addr;
        self
    }

    /// Set media bind address
    pub fn with_media_addr(mut self, addr: SocketAddr) -> Self {
        self.local_media_addr = addr;
        self
    }

    /// Set user agent string
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = user_agent;
        self
    }

    /// Set preferred codecs (convenience method)
    pub fn with_codecs(mut self, codecs: Vec<String>) -> Self {
        self.media.preferred_codecs = codecs;
        self
    }
    
    /// Set media configuration
    pub fn with_media(mut self, media: MediaConfig) -> Self {
        self.media = media;
        self
    }
    
    /// Set media preset
    pub fn with_media_preset(mut self, preset: MediaPreset) -> Self {
        self.media = MediaConfig::from_preset(preset);
        self
    }

    /// Set maximum concurrent calls
    pub fn with_max_calls(mut self, max_calls: usize) -> Self {
        self.max_concurrent_calls = max_calls;
        self
    }
    
    /// Get preferred codecs (backwards compatibility)
    pub fn preferred_codecs(&self) -> &[String] {
        &self.media.preferred_codecs
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self::new()
    }
}
