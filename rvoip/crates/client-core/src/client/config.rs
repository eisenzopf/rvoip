//! Client configuration structures and presets
//!
//! This module provides comprehensive configuration structures for the VoIP client, including
//! media settings, codec preferences, network parameters, and security options. It offers
//! both fine-grained control and convenient presets for common use cases.
//!
//! # Key Components
//!
//! - **ClientConfig** - Main client configuration with network and session settings
//! - **MediaConfig** - Media-specific settings including codecs and audio processing
//! - **MediaPreset** - Predefined media configuration templates for common scenarios
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────┐
//! │    ClientConfig         │
//! │ ┌─────────────────────┐ │
//! │ │   Network Settings  │ │  • SIP & Media addresses
//! │ │   Session Settings  │ │  • Timeouts & Limits
//! │ │   MediaConfig      ─┼─┼─ • Codec preferences
//! │ └─────────────────────┘ │  • Audio processing
//! └─────────────────────────┘  • Security settings
//! ```
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
//! assert_eq!(config.user_agent, "MyApp/1.0");
//! ```
//!
//! ## Advanced Media Configuration
//!
//! ```rust
//! use rvoip_client_core::client::config::{ClientConfig, MediaConfig};
//! use std::collections::HashMap;
//!
//! let mut custom_attributes = HashMap::new();
//! custom_attributes.insert("a".to_string(), "sendrecv".to_string());
//!
//! let media_config = MediaConfig {
//!     preferred_codecs: vec!["opus".to_string(), "G722".to_string()],
//!     echo_cancellation: true,
//!     noise_suppression: true,
//!     auto_gain_control: true,
//!     max_bandwidth_kbps: Some(128),
//!     require_srtp: true,
//!     srtp_profiles: vec!["AES_CM_128_HMAC_SHA1_80".to_string()],
//!     rtp_port_start: 12000,
//!     rtp_port_end: 15000,
//!     preferred_ptime: Some(20),
//!     custom_sdp_attributes: custom_attributes,
//!     dtmf_enabled: true,
//! };
//!
//! let config = ClientConfig::new().with_media(media_config);
//! assert!(config.media.require_srtp);
//! assert_eq!(config.media.rtp_port_start, 12000);
//! ```
//!
//! ## Using Media Presets
//!
//! ```rust
//! use rvoip_client_core::client::config::{ClientConfig, MediaPreset, MediaConfig};
//!
//! // Secure configuration for enterprise use
//! let secure_config = ClientConfig::new()
//!     .with_media_preset(MediaPreset::Secure)
//!     .with_user_agent("Enterprise-Phone/1.0".to_string());
//!
//! assert!(secure_config.media.require_srtp);
//! assert!(!secure_config.media.srtp_profiles.is_empty());
//!
//! // Low bandwidth configuration for mobile
//! let mobile_config = ClientConfig::new()
//!     .with_media_preset(MediaPreset::LowBandwidth);
//!
//! assert!(mobile_config.media.max_bandwidth_kbps.unwrap() <= 32);
//!
//! // Voice-optimized configuration
//! let voice_config = MediaConfig::from_preset(MediaPreset::VoiceOptimized);
//! assert!(voice_config.echo_cancellation);
//! assert!(voice_config.noise_suppression);
//! ```
//!
//! ## Configuration Validation
//!
//! ```rust
//! use rvoip_client_core::client::config::ClientConfig;
//!
//! let config = ClientConfig::new()
//!     .with_sip_addr("0.0.0.0:5060".parse().unwrap())
//!     .with_media_addr("0.0.0.0:0".parse().unwrap())
//!     .with_max_calls(100);
//!
//! // Validate configuration settings
//! assert!(config.max_concurrent_calls > 0);
//! assert!(config.session_timeout_secs > 0);
//! assert!(config.media.rtp_port_start < config.media.rtp_port_end);
//! assert!(config.enable_audio); // Default is true
//! ```
//!
//! # Common Patterns
//!
//! ## Enterprise VoIP Setup
//!
//! ```rust
//! use rvoip_client_core::client::config::{ClientConfig, MediaPreset};
//!
//! let enterprise_config = ClientConfig::new()
//!     .with_sip_addr("192.168.1.100:5060".parse().unwrap())
//!     .with_media_preset(MediaPreset::Secure)
//!     .with_user_agent("CorporatePhone/2.1".to_string())
//!     .with_max_calls(20);
//!
//! assert_eq!(enterprise_config.max_concurrent_calls, 20);
//! ```
//!
//! ## Residential VoIP Setup
//!
//! ```rust
//! use rvoip_client_core::client::config::{ClientConfig, MediaPreset};
//!
//! let home_config = ClientConfig::new()
//!     .with_media_preset(MediaPreset::VoiceOptimized)
//!     .with_max_calls(3);
//!
//! assert_eq!(home_config.max_concurrent_calls, 3);
//! assert!(home_config.media.echo_cancellation);
//! ```
//!
//! ## Mobile VoIP Setup
//!
//! ```rust
//! use rvoip_client_core::client::config::{ClientConfig, MediaPreset};
//!
//! let mobile_config = ClientConfig::new()
//!     .with_media_preset(MediaPreset::LowBandwidth)
//!     .with_user_agent("MobileVoIP/1.0".to_string())
//!     .with_max_calls(2);
//!
//! assert!(mobile_config.media.max_bandwidth_kbps.is_some());
//! assert!(mobile_config.media.max_bandwidth_kbps.unwrap() <= 32);
//! ```

use std::net::SocketAddr;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Media configuration preferences
/// 
/// Defines comprehensive media-related settings including codec preferences, audio processing
/// options, security requirements, network parameters, and SDP customization. This structure
/// provides fine-grained control over all aspects of media handling in VoIP calls.
/// 
/// # Audio Processing Features
/// 
/// - **Echo Cancellation**: Removes acoustic echo during calls
/// - **Noise Suppression**: Filters background noise from audio
/// - **Auto Gain Control**: Automatically adjusts microphone levels
/// - **DTMF Support**: Dual-tone multi-frequency signaling for dial tones
/// 
/// # Security Features
/// 
/// - **SRTP Encryption**: Secure Real-time Transport Protocol for media encryption
/// - **Profile Selection**: Choose from multiple SRTP encryption profiles
/// - **Mandatory Encryption**: Option to require SRTP for all calls
/// 
/// # Network Configuration
/// 
/// - **Port Range Control**: Specify RTP port ranges for NAT/firewall traversal
/// - **Bandwidth Management**: Set maximum bandwidth limits
/// - **Packetization Time**: Control audio packet timing for latency/quality trade-offs
/// 
/// # Examples
/// 
/// ## Basic Media Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::config::MediaConfig;
/// use std::collections::HashMap;
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
///     custom_sdp_attributes: HashMap::new(),
/// };
/// 
/// assert_eq!(media_config.preferred_codecs[0], "opus");
/// assert!(media_config.echo_cancellation);
/// assert_eq!(media_config.max_bandwidth_kbps, Some(128));
/// ```
/// 
/// ## Secure Media Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::config::MediaConfig;
/// use std::collections::HashMap;
/// 
/// let secure_config = MediaConfig {
///     preferred_codecs: vec!["opus".to_string(), "G722".to_string()],
///     require_srtp: true,
///     srtp_profiles: vec![
///         "AES_CM_128_HMAC_SHA1_80".to_string(),
///         "AES_CM_128_HMAC_SHA1_32".to_string(),
///     ],
///     dtmf_enabled: true,
///     echo_cancellation: true,
///     noise_suppression: true,
///     auto_gain_control: true,
///     max_bandwidth_kbps: Some(256),
///     rtp_port_start: 16384,
///     rtp_port_end: 32767,
///     preferred_ptime: Some(20),
///     custom_sdp_attributes: HashMap::new(),
/// };
/// 
/// assert!(secure_config.require_srtp);
/// assert_eq!(secure_config.srtp_profiles.len(), 2);
/// assert!(secure_config.srtp_profiles.contains(&"AES_CM_128_HMAC_SHA1_80".to_string()));
/// ```
/// 
/// ## Low Bandwidth Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::config::MediaConfig;
/// use std::collections::HashMap;
/// 
/// let low_bandwidth_config = MediaConfig {
///     preferred_codecs: vec!["G729".to_string(), "GSM".to_string()],
///     max_bandwidth_kbps: Some(32),
///     preferred_ptime: Some(30), // Larger packets for efficiency
///     echo_cancellation: true,
///     noise_suppression: true,
///     auto_gain_control: true,
///     dtmf_enabled: true,
///     require_srtp: false,
///     srtp_profiles: vec![],
///     rtp_port_start: 10000,
///     rtp_port_end: 20000,
///     custom_sdp_attributes: HashMap::new(),
/// };
/// 
/// assert_eq!(low_bandwidth_config.max_bandwidth_kbps, Some(32));
/// assert_eq!(low_bandwidth_config.preferred_ptime, Some(30));
/// assert!(low_bandwidth_config.preferred_codecs.contains(&"G729".to_string()));
/// ```
/// 
/// ## Custom SDP Attributes
/// 
/// ```rust
/// use rvoip_client_core::client::config::MediaConfig;
/// use std::collections::HashMap;
/// 
/// let mut custom_attrs = HashMap::new();
/// custom_attrs.insert("a".to_string(), "sendrecv".to_string());
/// custom_attrs.insert("a".to_string(), "rtcp-mux".to_string());
/// 
/// let custom_config = MediaConfig {
///     preferred_codecs: vec!["opus".to_string()],
///     custom_sdp_attributes: custom_attrs,
///     dtmf_enabled: true,
///     echo_cancellation: true,
///     noise_suppression: true,
///     auto_gain_control: true,
///     max_bandwidth_kbps: None,
///     require_srtp: false,
///     srtp_profiles: vec![],
///     rtp_port_start: 10000,
///     rtp_port_end: 20000,
///     preferred_ptime: Some(20),
/// };
/// 
/// assert!(!custom_config.custom_sdp_attributes.is_empty());
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

/// Predefined media configuration presets for common use cases
/// 
/// These presets provide optimized configurations for different scenarios,
/// allowing users to quickly configure appropriate settings without manual tuning.
/// Each preset can be further customized after application.
/// 
/// # Preset Characteristics
/// 
/// | Preset | Primary Use | Codecs | Bandwidth | Security | Audio Processing |
/// |--------|-------------|--------|-----------|----------|------------------|
/// | VoiceOptimized | Phone calls | Opus, PCMU | Standard | None | Full (AEC, NS, AGC) |
/// | MusicOptimized | Music streaming | Opus | High | None | Minimal |
/// | LowBandwidth | Mobile/poor networks | G.729, GSM | Low | None | Full |
/// | Secure | Enterprise | Default | Standard | SRTP | Full |
/// | Legacy | Compatibility | G.711 | Standard | None | Minimal |
/// 
/// # Examples
/// 
/// ## Voice Calling (Recommended for Phone Calls)
/// 
/// ```rust
/// use rvoip_client_core::client::config::{MediaPreset, MediaConfig};
/// 
/// let voice_config = MediaConfig::from_preset(MediaPreset::VoiceOptimized);
/// 
/// // Optimized for voice with full audio processing
/// assert!(voice_config.echo_cancellation);
/// assert!(voice_config.noise_suppression);
/// assert!(voice_config.auto_gain_control);
/// assert_eq!(voice_config.preferred_ptime, Some(20));
/// assert!(voice_config.preferred_codecs.contains(&"opus".to_string()));
/// ```
/// 
/// ## Music Streaming (High Quality Audio)
/// 
/// ```rust
/// use rvoip_client_core::client::config::{MediaPreset, MediaConfig};
/// 
/// let music_config = MediaConfig::from_preset(MediaPreset::MusicOptimized);
/// 
/// // Optimized for music with minimal processing
/// assert!(!music_config.echo_cancellation); // No echo cancellation for music
/// assert!(!music_config.noise_suppression); // Preserve audio fidelity
/// assert!(!music_config.auto_gain_control);  // No gain adjustment
/// assert_eq!(music_config.max_bandwidth_kbps, Some(256)); // Higher bandwidth
/// assert!(music_config.preferred_codecs.contains(&"opus".to_string()));
/// ```
/// 
/// ## Low Bandwidth (Mobile/Constrained Networks)
/// 
/// ```rust
/// use rvoip_client_core::client::config::{MediaPreset, MediaConfig};
/// 
/// let mobile_config = MediaConfig::from_preset(MediaPreset::LowBandwidth);
/// 
/// // Optimized for low bandwidth connections
/// assert_eq!(mobile_config.max_bandwidth_kbps, Some(32));
/// assert_eq!(mobile_config.preferred_ptime, Some(30)); // Larger packets
/// assert!(mobile_config.preferred_codecs.iter().any(|c| c == "G.729"));
/// assert!(mobile_config.echo_cancellation); // Still enabled for quality
/// ```
/// 
/// ## Secure Communications (Enterprise)
/// 
/// ```rust
/// use rvoip_client_core::client::config::{MediaPreset, MediaConfig};
/// 
/// let secure_config = MediaConfig::from_preset(MediaPreset::Secure);
/// 
/// // Requires encryption for all calls
/// assert!(secure_config.require_srtp);
/// assert!(!secure_config.srtp_profiles.is_empty());
/// assert!(secure_config.srtp_profiles.contains(&"AES_CM_128_HMAC_SHA1_80".to_string()));
/// assert!(secure_config.echo_cancellation); // Full audio processing
/// ```
/// 
/// ## Legacy Compatibility (Older Systems)
/// 
/// ```rust
/// use rvoip_client_core::client::config::{MediaPreset, MediaConfig};
/// 
/// let legacy_config = MediaConfig::from_preset(MediaPreset::Legacy);
/// 
/// // Compatible with older SIP systems
/// assert!(legacy_config.preferred_codecs.contains(&"PCMU".to_string()));
/// assert!(legacy_config.preferred_codecs.contains(&"PCMA".to_string()));
/// assert!(!legacy_config.echo_cancellation); // Minimal processing
/// assert!(!legacy_config.require_srtp); // No encryption requirement
/// ```
/// 
/// ## Preset Comparison
/// 
/// ```rust
/// use rvoip_client_core::client::config::{MediaPreset, MediaConfig};
/// 
/// let voice = MediaConfig::from_preset(MediaPreset::VoiceOptimized);
/// let music = MediaConfig::from_preset(MediaPreset::MusicOptimized);
/// let mobile = MediaConfig::from_preset(MediaPreset::LowBandwidth);
/// 
/// // Voice has audio processing, music doesn't
/// assert!(voice.echo_cancellation);
/// assert!(!music.echo_cancellation);
/// 
/// // Mobile has lower bandwidth than music
/// assert!(mobile.max_bandwidth_kbps.unwrap() < music.max_bandwidth_kbps.unwrap());
/// 
/// // All have DTMF enabled by default
/// assert!(voice.dtmf_enabled);
/// assert!(music.dtmf_enabled);
/// assert!(mobile.dtmf_enabled);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPreset {
    /// Optimized for voice calls with full audio processing
    /// 
    /// **Use Case**: Standard phone calls, conferencing, customer service
    /// 
    /// **Features**:
    /// - Opus and PCMU codecs for voice clarity
    /// - Full audio processing (echo cancellation, noise suppression, AGC)
    /// - 20ms packetization for low latency
    /// - Standard bandwidth usage
    /// 
    /// **Best For**: Business phones, softphones, call centers
    VoiceOptimized,
    
    /// Optimized for high-quality music streaming
    /// 
    /// **Use Case**: Music streaming, audio conferencing, broadcast
    /// 
    /// **Features**:
    /// - Opus codec for high fidelity
    /// - Minimal audio processing to preserve quality
    /// - Higher bandwidth allocation (256 kbps)
    /// - No echo cancellation or gain control
    /// 
    /// **Best For**: Music apps, audio streaming, podcast recording
    MusicOptimized,
    
    /// Optimized for constrained bandwidth connections
    /// 
    /// **Use Case**: Mobile networks, satellite links, poor connectivity
    /// 
    /// **Features**:
    /// - Low bitrate codecs (G.729, GSM)
    /// - 32 kbps maximum bandwidth
    /// - 30ms packetization for efficiency
    /// - Audio processing enabled for quality
    /// 
    /// **Best For**: Mobile VoIP, rural networks, international calling
    LowBandwidth,
    
    /// Requires SRTP encryption for secure communications
    /// 
    /// **Use Case**: Enterprise communications, sensitive data, compliance
    /// 
    /// **Features**:
    /// - Mandatory SRTP encryption
    /// - Multiple SRTP profiles supported
    /// - Full audio processing enabled
    /// - Standard codec selection
    /// 
    /// **Best For**: Corporate phones, government, healthcare, legal
    Secure,
    
    /// Basic G.711 compatibility for legacy systems
    /// 
    /// **Use Case**: Interoperability with older PBX systems
    /// 
    /// **Features**:
    /// - G.711 (PCMU/PCMA) codecs only
    /// - Minimal audio processing
    /// - No encryption requirements
    /// - Maximum compatibility
    /// 
    /// **Best For**: Legacy PBX integration, older VoIP systems
    Legacy,
}

impl MediaConfig {
    /// Create a MediaConfig from a predefined preset
    /// 
    /// This convenience method creates a MediaConfig with optimized settings for
    /// specific use cases. The resulting configuration can be further customized
    /// by modifying individual fields after creation.
    /// 
    /// # Arguments
    /// 
    /// * `preset` - The MediaPreset to use as a template
    /// 
    /// # Returns
    /// 
    /// A MediaConfig configured for the specified use case
    /// 
    /// # Examples
    /// 
    /// ## Voice-Optimized Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::{MediaConfig, MediaPreset};
    /// 
    /// let config = MediaConfig::from_preset(MediaPreset::VoiceOptimized);
    /// 
    /// // Verify voice optimization settings
    /// assert!(config.echo_cancellation);
    /// assert!(config.noise_suppression);
    /// assert!(config.auto_gain_control);
    /// assert_eq!(config.preferred_ptime, Some(20));
    /// assert!(config.preferred_codecs.contains(&"opus".to_string()));
    /// ```
    /// 
    /// ## Secure Configuration for Enterprise
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::{MediaConfig, MediaPreset};
    /// 
    /// let mut config = MediaConfig::from_preset(MediaPreset::Secure);
    /// 
    /// // Verify security settings
    /// assert!(config.require_srtp);
    /// assert!(!config.srtp_profiles.is_empty());
    /// 
    /// // Customize for specific enterprise needs
    /// config.preferred_codecs = vec!["opus".to_string(), "G722".to_string()];
    /// config.max_bandwidth_kbps = Some(128);
    /// 
    /// assert_eq!(config.preferred_codecs.len(), 2);
    /// ```
    /// 
    /// ## Mobile-Optimized Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::{MediaConfig, MediaPreset};
    /// 
    /// let config = MediaConfig::from_preset(MediaPreset::LowBandwidth);
    /// 
    /// // Verify mobile optimization
    /// assert_eq!(config.max_bandwidth_kbps, Some(32));
    /// assert_eq!(config.preferred_ptime, Some(30)); // Efficient packetization
    /// assert!(config.preferred_codecs.iter().any(|c| c == "G.729"));
    /// ```
    /// 
    /// ## Customizing After Preset Application
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::{MediaConfig, MediaPreset};
    /// use std::collections::HashMap;
    /// 
    /// let mut config = MediaConfig::from_preset(MediaPreset::VoiceOptimized);
    /// 
    /// // Add custom SDP attributes
    /// let mut custom_attrs = HashMap::new();
    /// custom_attrs.insert("a".to_string(), "rtcp-mux".to_string());
    /// config.custom_sdp_attributes = custom_attrs;
    /// 
    /// // Adjust port range for firewall
    /// config.rtp_port_start = 16384;
    /// config.rtp_port_end = 32767;
    /// 
    /// // Verify customizations
    /// assert!(!config.custom_sdp_attributes.is_empty());
    /// assert_eq!(config.rtp_port_start, 16384);
    /// ```
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

/// Comprehensive configuration for the SIP client
/// 
/// This structure contains all the configuration needed to set up and run a VoIP client,
/// including network settings, media configuration, session parameters, and feature flags.
/// It uses the builder pattern for easy configuration and provides sensible defaults.
/// 
/// # Configuration Categories
/// 
/// ## Network Configuration
/// - **SIP Address**: Local address for SIP signaling
/// - **Media Address**: Local address for RTP media streams
/// - **Domain**: Optional SIP domain for routing
/// 
/// ## Session Management
/// - **Concurrent Calls**: Maximum number of simultaneous calls
/// - **Timeouts**: Session and registration timeout values
/// - **User Agent**: Client identification string
/// 
/// ## Media Settings
/// - **Audio/Video**: Enable/disable media types
/// - **MediaConfig**: Detailed codec and processing settings
/// 
/// # Examples
/// 
/// ## Basic Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::config::ClientConfig;
/// 
/// let config = ClientConfig::new()
///     .with_sip_addr("127.0.0.1:5060".parse().unwrap())
///     .with_user_agent("MyApp/1.0".to_string())
///     .with_max_calls(5);
/// 
/// assert_eq!(config.max_concurrent_calls, 5);
/// assert_eq!(config.user_agent, "MyApp/1.0");
/// assert!(config.enable_audio);
/// ```
/// 
/// ## Enterprise Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::config::{ClientConfig, MediaPreset};
/// 
/// let enterprise_config = ClientConfig::new()
///     .with_sip_addr("192.168.1.100:5060".parse().unwrap())
///     .with_media_addr("192.168.1.100:0".parse().unwrap())
///     .with_user_agent("EnterprisePhone/2.1".to_string())
///     .with_media_preset(MediaPreset::Secure)
///     .with_max_calls(20);
/// 
/// assert_eq!(enterprise_config.max_concurrent_calls, 20);
/// assert!(enterprise_config.media.require_srtp);
/// assert_eq!(enterprise_config.local_sip_addr.ip().to_string(), "192.168.1.100");
/// ```
/// 
/// ## Mobile Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::config::{ClientConfig, MediaPreset};
/// 
/// let mobile_config = ClientConfig::new()
///     .with_media_preset(MediaPreset::LowBandwidth)
///     .with_user_agent("MobileVoIP/1.0".to_string())
///     .with_max_calls(2);
/// 
/// assert_eq!(mobile_config.max_concurrent_calls, 2);
/// assert_eq!(mobile_config.media.max_bandwidth_kbps, Some(32));
/// assert!(mobile_config.media.preferred_codecs.iter().any(|c| c == "G.729"));
/// ```
/// 
/// ## Custom Media Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::config::{ClientConfig, MediaConfig};
/// use std::collections::HashMap;
/// 
/// let custom_media = MediaConfig {
///     preferred_codecs: vec!["opus".to_string(), "G722".to_string()],
///     echo_cancellation: true,
///     noise_suppression: true,
///     auto_gain_control: false, // Disable AGC
///     max_bandwidth_kbps: Some(128),
///     require_srtp: true,
///     srtp_profiles: vec!["AES_CM_128_HMAC_SHA1_80".to_string()],
///     rtp_port_start: 16384,
///     rtp_port_end: 32767,
///     preferred_ptime: Some(20),
///     custom_sdp_attributes: HashMap::new(),
///     dtmf_enabled: true,
/// };
/// 
/// let config = ClientConfig::new()
///     .with_media(custom_media)
///     .with_user_agent("CustomApp/1.0".to_string());
/// 
/// assert!(!config.media.auto_gain_control);
/// assert!(config.media.require_srtp);
/// assert_eq!(config.media.rtp_port_start, 16384);
/// ```
/// 
/// ## Configuration Validation
/// 
/// ```rust
/// use rvoip_client_core::client::config::ClientConfig;
/// 
/// let config = ClientConfig::new()
///     .with_sip_addr("0.0.0.0:5060".parse().unwrap())
///     .with_media_addr("0.0.0.0:0".parse().unwrap());
/// 
/// // Verify default settings
/// assert!(config.max_concurrent_calls > 0);
/// assert!(config.session_timeout_secs > 0);
/// assert!(config.enable_audio);
/// assert!(!config.enable_video); // Default: video disabled
/// assert!(config.domain.is_none()); // Default: no domain
/// 
/// // Verify media defaults
/// assert_eq!(config.media.rtp_port_start, 10000);
/// assert_eq!(config.media.rtp_port_end, 20000);
/// assert!(config.media.rtp_port_start < config.media.rtp_port_end);
/// ```
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
    /// Create a new client configuration with sensible defaults
    /// 
    /// Initializes a ClientConfig with default values suitable for most applications.
    /// The configuration can be customized using the builder pattern methods.
    /// 
    /// # Default Values
    /// 
    /// - **SIP Address**: `127.0.0.1:0` (bind to localhost, OS-assigned port)
    /// - **Media Address**: `127.0.0.1:0` (bind to localhost, OS-assigned port)
    /// - **User Agent**: `rvoip-client-core/0.1.0`
    /// - **Max Calls**: 10 concurrent calls
    /// - **Timeout**: 300 seconds (5 minutes)
    /// - **Audio**: Enabled
    /// - **Video**: Disabled
    /// - **Media**: Default MediaConfig with standard codecs
    /// 
    /// # Returns
    /// 
    /// A new ClientConfig with default settings
    /// 
    /// # Examples
    /// 
    /// ## Basic Usage
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::ClientConfig;
    /// 
    /// let config = ClientConfig::new();
    /// 
    /// // Verify defaults
    /// assert_eq!(config.local_sip_addr.ip().to_string(), "127.0.0.1");
    /// assert_eq!(config.local_sip_addr.port(), 0); // OS-assigned
    /// assert_eq!(config.max_concurrent_calls, 10);
    /// assert_eq!(config.session_timeout_secs, 300);
    /// assert!(config.enable_audio);
    /// assert!(!config.enable_video);
    /// assert!(config.domain.is_none());
    /// ```
    /// 
    /// ## Immediate Customization
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::ClientConfig;
    /// 
    /// let config = ClientConfig::new()
    ///     .with_sip_addr("192.168.1.100:5060".parse().unwrap())
    ///     .with_user_agent("MyApp/1.0".to_string())
    ///     .with_max_calls(5);
    /// 
    /// assert_eq!(config.local_sip_addr.ip().to_string(), "192.168.1.100");
    /// assert_eq!(config.local_sip_addr.port(), 5060);
    /// assert_eq!(config.user_agent, "MyApp/1.0");
    /// assert_eq!(config.max_concurrent_calls, 5);
    /// ```
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

    /// Set the local SIP bind address for signaling
    /// 
    /// Configures the local address and port that the SIP client will bind to
    /// for receiving SIP messages. Use `0.0.0.0` to bind to all interfaces
    /// and port `0` to let the OS assign an available port.
    /// 
    /// # Arguments
    /// 
    /// * `addr` - The socket address to bind SIP signaling to
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::ClientConfig;
    /// use std::net::SocketAddr;
    /// 
    /// // Bind to specific address and port
    /// let config = ClientConfig::new()
    ///     .with_sip_addr("192.168.1.100:5060".parse().unwrap());
    /// 
    /// assert_eq!(config.local_sip_addr.ip().to_string(), "192.168.1.100");
    /// assert_eq!(config.local_sip_addr.port(), 5060);
    /// 
    /// // Bind to all interfaces with OS-assigned port
    /// let config2 = ClientConfig::new()
    ///     .with_sip_addr("0.0.0.0:0".parse().unwrap());
    /// 
    /// assert_eq!(config2.local_sip_addr.ip().to_string(), "0.0.0.0");
    /// assert_eq!(config2.local_sip_addr.port(), 0);
    /// ```
    pub fn with_sip_addr(mut self, addr: SocketAddr) -> Self {
        self.local_sip_addr = addr;
        self
    }

    /// Set the local media bind address for RTP streams
    /// 
    /// Configures the local address that will be used for RTP media streams.
    /// This is typically the same as the SIP address but can be different
    /// for advanced network configurations.
    /// 
    /// # Arguments
    /// 
    /// * `addr` - The socket address to bind RTP media to
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::ClientConfig;
    /// 
    /// // Use same address for SIP and media
    /// let addr = "192.168.1.100:0".parse().unwrap();
    /// let config = ClientConfig::new()
    ///     .with_sip_addr(addr)
    ///     .with_media_addr(addr);
    /// 
    /// assert_eq!(config.local_sip_addr.ip(), config.local_media_addr.ip());
    /// 
    /// // Use different addresses (advanced networking)
    /// let config2 = ClientConfig::new()
    ///     .with_sip_addr("10.0.0.100:5060".parse().unwrap())
    ///     .with_media_addr("192.168.1.100:0".parse().unwrap());
    /// 
    /// assert_ne!(config2.local_sip_addr.ip(), config2.local_media_addr.ip());
    /// ```
    pub fn with_media_addr(mut self, addr: SocketAddr) -> Self {
        self.local_media_addr = addr;
        self
    }

    /// Set the User-Agent string for SIP identification
    /// 
    /// The User-Agent header identifies the client software in SIP messages.
    /// It's helpful for debugging and server-side logging.
    /// 
    /// # Arguments
    /// 
    /// * `user_agent` - String identifying the client application
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::ClientConfig;
    /// 
    /// let config = ClientConfig::new()
    ///     .with_user_agent("MyVoIPApp/2.1.0".to_string());
    /// 
    /// assert_eq!(config.user_agent, "MyVoIPApp/2.1.0");
    /// 
    /// // Enterprise naming convention
    /// let enterprise_config = ClientConfig::new()
    ///     .with_user_agent("CorporatePhone/1.0 (Build 12345)".to_string());
    /// 
    /// assert!(enterprise_config.user_agent.contains("CorporatePhone"));
    /// assert!(enterprise_config.user_agent.contains("Build"));
    /// ```
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = user_agent;
        self
    }

    /// Set preferred audio codecs (convenience method)
    /// 
    /// This is a convenience method that sets the preferred codec list
    /// in the media configuration. Codecs are tried in the order specified.
    /// 
    /// # Arguments
    /// 
    /// * `codecs` - Vector of codec names in order of preference
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::ClientConfig;
    /// 
    /// let config = ClientConfig::new()
    ///     .with_codecs(vec![
    ///         "opus".to_string(),
    ///         "G722".to_string(),
    ///         "PCMU".to_string()
    ///     ]);
    /// 
    /// assert_eq!(config.media.preferred_codecs[0], "opus");
    /// assert_eq!(config.media.preferred_codecs[1], "G722");
    /// assert_eq!(config.media.preferred_codecs[2], "PCMU");
    /// assert_eq!(config.media.preferred_codecs.len(), 3);
    /// ```
    pub fn with_codecs(mut self, codecs: Vec<String>) -> Self {
        self.media.preferred_codecs = codecs;
        self
    }
    
    /// Set complete media configuration
    /// 
    /// Replaces the entire media configuration with a custom MediaConfig.
    /// This provides full control over all media-related settings.
    /// 
    /// # Arguments
    /// 
    /// * `media` - Custom MediaConfig with desired settings
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::{ClientConfig, MediaConfig};
    /// use std::collections::HashMap;
    /// 
    /// let custom_media = MediaConfig {
    ///     preferred_codecs: vec!["opus".to_string()],
    ///     echo_cancellation: false,
    ///     noise_suppression: true,
    ///     auto_gain_control: true,
    ///     max_bandwidth_kbps: Some(64),
    ///     require_srtp: true,
    ///     srtp_profiles: vec!["AES_CM_128_HMAC_SHA1_80".to_string()],
    ///     rtp_port_start: 20000,
    ///     rtp_port_end: 30000,
    ///     preferred_ptime: Some(40),
    ///     custom_sdp_attributes: HashMap::new(),
    ///     dtmf_enabled: true,
    /// };
    /// 
    /// let config = ClientConfig::new().with_media(custom_media);
    /// 
    /// assert!(!config.media.echo_cancellation);
    /// assert!(config.media.require_srtp);
    /// assert_eq!(config.media.max_bandwidth_kbps, Some(64));
    /// assert_eq!(config.media.rtp_port_start, 20000);
    /// ```
    pub fn with_media(mut self, media: MediaConfig) -> Self {
        self.media = media;
        self
    }
    
    /// Set media configuration using a preset
    /// 
    /// Applies a predefined media configuration optimized for specific use cases.
    /// This is a convenience method that replaces the current media config
    /// with one generated from the specified preset.
    /// 
    /// # Arguments
    /// 
    /// * `preset` - MediaPreset to use for configuration
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::{ClientConfig, MediaPreset};
    /// 
    /// // Voice-optimized for phone calls
    /// let voice_config = ClientConfig::new()
    ///     .with_media_preset(MediaPreset::VoiceOptimized);
    /// 
    /// assert!(voice_config.media.echo_cancellation);
    /// assert!(voice_config.media.noise_suppression);
    /// 
    /// // Low bandwidth for mobile
    /// let mobile_config = ClientConfig::new()
    ///     .with_media_preset(MediaPreset::LowBandwidth);
    /// 
    /// assert_eq!(mobile_config.media.max_bandwidth_kbps, Some(32));
    /// 
    /// // Secure for enterprise
    /// let secure_config = ClientConfig::new()
    ///     .with_media_preset(MediaPreset::Secure);
    /// 
    /// assert!(secure_config.media.require_srtp);
    /// ```
    pub fn with_media_preset(mut self, preset: MediaPreset) -> Self {
        self.media = MediaConfig::from_preset(preset);
        self
    }

    /// Set maximum number of concurrent calls
    /// 
    /// Configures the maximum number of calls that can be active simultaneously.
    /// This helps with resource management and prevents overloading the client.
    /// 
    /// # Arguments
    /// 
    /// * `max_calls` - Maximum number of concurrent calls (must be > 0)
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::ClientConfig;
    /// 
    /// // Single-line phone
    /// let single_line = ClientConfig::new().with_max_calls(1);
    /// assert_eq!(single_line.max_concurrent_calls, 1);
    /// 
    /// // Small office setup
    /// let office_phone = ClientConfig::new().with_max_calls(5);
    /// assert_eq!(office_phone.max_concurrent_calls, 5);
    /// 
    /// // Call center agent
    /// let call_center = ClientConfig::new().with_max_calls(20);
    /// assert_eq!(call_center.max_concurrent_calls, 20);
    /// 
    /// // Enterprise server
    /// let enterprise = ClientConfig::new().with_max_calls(100);
    /// assert_eq!(enterprise.max_concurrent_calls, 100);
    /// ```
    pub fn with_max_calls(mut self, max_calls: usize) -> Self {
        self.max_concurrent_calls = max_calls;
        self
    }
    
    /// Get the preferred codecs list (backwards compatibility)
    /// 
    /// Returns a slice of the preferred codec names in order of preference.
    /// This is a convenience method that provides direct access to the
    /// codec list without going through the media configuration.
    /// 
    /// # Returns
    /// 
    /// A slice containing codec names in preference order
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::config::ClientConfig;
    /// 
    /// let config = ClientConfig::new()
    ///     .with_codecs(vec![
    ///         "opus".to_string(),
    ///         "G722".to_string(),
    ///         "PCMU".to_string()
    ///     ]);
    /// 
    /// let codecs = config.preferred_codecs();
    /// assert_eq!(codecs.len(), 3);
    /// assert_eq!(codecs[0], "opus");
    /// assert_eq!(codecs[1], "G722");
    /// assert_eq!(codecs[2], "PCMU");
    /// 
    /// // Check for specific codec support
    /// assert!(codecs.contains(&"opus".to_string()));
    /// assert!(!codecs.contains(&"G729".to_string()));
    /// ```
    pub fn preferred_codecs(&self) -> &[String] {
        &self.media.preferred_codecs
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self::new()
    }
}
