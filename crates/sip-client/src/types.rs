//! Core types for the SIP client library

use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

// Re-export commonly used types from underlying crates
pub use rvoip_client_core::CallId;
pub use rvoip_session_core::SessionId;
pub use codec_core::CodecType;
pub use rvoip_audio_core::AudioDirection;

/// Call state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    /// Call is being initiated
    Initiating,
    /// Call is ringing (outgoing)
    Ringing,
    /// Call is ringing (incoming)
    IncomingRinging,
    /// Call is connected
    Connected,
    /// Call is on hold
    OnHold,
    /// Call is being transferred
    Transferring,
    /// Call has ended
    Terminated,
}

/// Represents an active call
#[derive(Debug, Clone)]
pub struct Call {
    /// Unique call identifier
    pub id: CallId,
    /// Current call state
    pub state: Arc<RwLock<CallState>>,
    /// Remote party URI
    pub remote_uri: String,
    /// Local party URI
    pub local_uri: String,
    /// Call start time
    pub start_time: chrono::DateTime<chrono::Utc>,
    /// Call connect time (if connected)
    pub connect_time: Option<chrono::DateTime<chrono::Utc>>,
    /// Selected codec
    pub codec: Option<CodecType>,
    /// Call direction
    pub direction: CallDirection,
}

/// Call direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    /// Outgoing call
    Outgoing,
    /// Incoming call
    Incoming,
}

/// Audio configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Input device name (None for default)
    pub input_device: Option<String>,
    /// Output device name (None for default)
    pub output_device: Option<String>,
    /// Enable echo cancellation
    pub echo_cancellation: bool,
    /// Enable noise suppression
    pub noise_suppression: bool,
    /// Enable automatic gain control
    pub auto_gain_control: bool,
    /// Sample rate (Hz)
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u16,
    /// Frame duration in milliseconds
    pub frame_duration_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device: None,
            output_device: None,
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            sample_rate: 48000,
            channels: 1,
            frame_duration_ms: 20,
        }
    }
}

/// Codec configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecConfig {
    /// Ordered list of codec preferences
    pub priorities: Vec<CodecPriority>,
    /// Allow dynamic codec switching
    pub allow_dynamic_switching: bool,
    /// Preferred packet time (ptime) in milliseconds
    pub preferred_ptime: Option<u32>,
    /// Maximum packet time (maxptime) in milliseconds
    pub max_ptime: Option<u32>,
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            priorities: vec![
                CodecPriority::new("PCMU", 100),
                CodecPriority::new("PCMA", 90),
            ],
            allow_dynamic_switching: false,
            preferred_ptime: Some(20),
            max_ptime: Some(60),
        }
    }
}

/// Codec priority configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecPriority {
    /// Codec name
    pub name: String,
    /// Priority (higher is preferred)
    pub priority: u8,
    /// Codec-specific parameters
    pub parameters: std::collections::HashMap<String, String>,
}

impl CodecPriority {
    /// Create a new codec priority
    pub fn new(name: impl Into<String>, priority: u8) -> Self {
        Self {
            name: name.into(),
            priority,
            parameters: std::collections::HashMap::new(),
        }
    }
    
    /// Add a codec parameter
    pub fn with_parameter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.parameters.insert(key.into(), value.into());
        self
    }
}

/// SIP client configuration
#[derive(Debug, Clone)]
pub struct SipClientConfig {
    /// SIP identity (e.g., "sip:alice@example.com")
    pub sip_identity: String,
    /// SIP server address (e.g., "sip.example.com:5060")
    pub sip_server: Option<String>,
    /// SIP registrar address (if different from server)
    pub sip_registrar: Option<String>,
    /// Local SIP address to bind to
    pub local_address: std::net::SocketAddr,
    /// Audio configuration
    pub audio: AudioConfig,
    /// Codec configuration
    pub codecs: CodecConfig,
    /// User agent string
    pub user_agent: String,
    /// Registration configuration
    pub registration: Option<RegistrationConfig>,
    /// Call timeout
    pub call_timeout: Duration,
    /// Enable automatic call recording
    pub auto_record: bool,
    /// Registration TTL in seconds
    pub registration_ttl: u32,
}

impl Default for SipClientConfig {
    fn default() -> Self {
        Self {
            sip_identity: String::new(),
            sip_server: None,
            sip_registrar: None,
            local_address: "0.0.0.0:5060".parse().unwrap(),
            audio: AudioConfig::default(),
            codecs: CodecConfig::default(),
            user_agent: format!("RVOIP-SipClient/{}", crate::VERSION),
            registration: None,
            call_timeout: Duration::from_secs(30),
            auto_record: false,
            registration_ttl: 3600,
        }
    }
}

/// Registration configuration
#[derive(Debug, Clone)]
pub struct RegistrationConfig {
    /// Registration interval in seconds
    pub expires: u32,
    /// Username for authentication
    pub username: Option<String>,
    /// Password for authentication
    pub password: Option<String>,
    /// Realm for authentication
    pub realm: Option<String>,
}

/// Call statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallStatistics {
    /// Duration of the call
    pub duration: Duration,
    /// Number of packets sent
    pub packets_sent: u64,
    /// Number of packets received
    pub packets_received: u64,
    /// Number of packets lost
    pub packets_lost: u64,
    /// Average jitter in milliseconds
    pub jitter_ms: f64,
    /// Average round-trip time in milliseconds
    pub rtt_ms: f64,
    /// MOS (Mean Opinion Score) estimate
    pub mos_score: f64,
    /// Codec used
    pub codec: String,
}

/// Audio quality metrics
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AudioQualityMetrics {
    /// Current audio level (0.0 to 1.0)
    pub level: f32,
    /// Peak audio level (0.0 to 1.0)
    pub peak_level: f32,
    /// Current MOS score (1.0 to 5.0)
    pub mos: f64,
    /// Packet loss percentage
    pub packet_loss_percent: f64,
    /// Jitter in milliseconds
    pub jitter_ms: f64,
    /// Round-trip time in milliseconds
    pub rtt_ms: f64,
}

/// Audio stream handle for direct frame access
pub struct AudioStreamHandle {
    /// Session ID for the audio stream
    pub session_id: SessionId,
    /// Audio format information
    pub format: rvoip_audio_core::AudioFormat,
}

/// Represents an audio frame
pub type AudioFrame = rvoip_client_core::AudioFrame;