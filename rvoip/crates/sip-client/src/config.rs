use std::net::SocketAddr;
use std::time::Duration;

/// SIP client configuration
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Local address to bind to
    pub local_addr: Option<SocketAddr>,

    /// Username for authentication
    pub username: String,

    /// Domain for SIP URIs
    pub domain: String,

    /// User-Agent header value
    pub user_agent: String,

    /// Registration expiry in seconds
    pub register_expires: u32,

    /// Registration refresh interval (percentage of expiry)
    pub register_refresh: f32,

    /// Transport configuration
    pub transport: TransportConfig,

    /// Media configuration
    pub media: MediaConfig,

    /// Transaction configuration
    pub transaction: TransactionConfig,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            local_addr: None,
            username: "anonymous".to_string(),
            domain: "localhost".to_string(),
            user_agent: format!("RVOIP-SIP-Client/{}", env!("CARGO_PKG_VERSION")),
            register_expires: 3600,
            register_refresh: 0.8,
            transport: TransportConfig::default(),
            media: MediaConfig::default(),
            transaction: TransactionConfig::default(),
        }
    }
}

impl ClientConfig {
    /// Create a new client configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the local address
    pub fn with_local_addr(mut self, addr: SocketAddr) -> Self {
        self.local_addr = Some(addr);
        self
    }

    /// Set the username
    pub fn with_username<S: Into<String>>(mut self, username: S) -> Self {
        self.username = username.into();
        self
    }

    /// Set the domain
    pub fn with_domain<S: Into<String>>(mut self, domain: S) -> Self {
        self.domain = domain.into();
        self
    }

    /// Set the User-Agent header
    pub fn with_user_agent<S: Into<String>>(mut self, user_agent: S) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Set registration expiry in seconds
    pub fn with_register_expires(mut self, expires: u32) -> Self {
        self.register_expires = expires;
        self
    }

    /// Set registration refresh percentage (0.0 - 1.0)
    pub fn with_register_refresh(mut self, refresh: f32) -> Self {
        self.register_refresh = refresh.clamp(0.1, 0.99);
        self
    }

    /// Set transport configuration
    pub fn with_transport(mut self, transport: TransportConfig) -> Self {
        self.transport = transport;
        self
    }

    /// Set media configuration
    pub fn with_media(mut self, media: MediaConfig) -> Self {
        self.media = media;
        self
    }

    /// Set transaction configuration
    pub fn with_transaction(mut self, transaction: TransactionConfig) -> Self {
        self.transaction = transaction;
        self
    }
}

/// Transport configuration
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// UDP buffer size
    pub udp_buffer_size: usize,

    /// Maximum message size
    pub max_message_size: usize,

    /// Connection timeout
    pub connect_timeout: Duration,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            udp_buffer_size: 8192,
            max_message_size: 65536,
            connect_timeout: Duration::from_secs(5),
        }
    }
}

/// Media configuration
#[derive(Debug, Clone)]
pub struct MediaConfig {
    /// Enable RTP
    pub rtp_enabled: bool,

    /// Enable RTCP
    pub rtcp_enabled: bool,

    /// RTCP reporting interval in milliseconds
    pub rtcp_interval: Duration,

    /// Local RTP port range start
    pub rtp_port_min: u16,

    /// Local RTP port range end
    pub rtp_port_max: u16,

    /// Jitter buffer size in milliseconds
    pub jitter_buffer_ms: u32,

    /// Audio sample rate
    pub audio_sample_rate: u32,

    /// Audio packet duration in milliseconds
    pub audio_ptime: u32,

    /// Preferred codecs in order of preference
    pub preferred_codecs: Vec<CodecType>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            rtp_enabled: true,
            rtcp_enabled: true,
            rtcp_interval: Duration::from_secs(5),
            rtp_port_min: 10000,
            rtp_port_max: 20000,
            jitter_buffer_ms: 60,
            audio_sample_rate: 8000,
            audio_ptime: 20,
            preferred_codecs: vec![CodecType::PCMU, CodecType::PCMA],
        }
    }
}

/// Transaction configuration
#[derive(Debug, Clone)]
pub struct TransactionConfig {
    /// Transaction timeout base value
    pub timer_t1: Duration,

    /// Transaction timeout maximum value
    pub timer_t2: Duration,

    /// Transaction final timeout
    pub timer_t4: Duration,

    /// INVITE transaction timeout
    pub timer_b: Duration,

    /// ACK wait timeout
    pub timer_d: Duration,

    /// Non-INVITE transaction timeout
    pub timer_f: Duration,

    /// Maximum event queue size
    pub max_events: usize,
}

impl Default for TransactionConfig {
    fn default() -> Self {
        Self {
            timer_t1: Duration::from_millis(500),
            timer_t2: Duration::from_secs(4),
            timer_t4: Duration::from_secs(5),
            timer_b: Duration::from_secs(32),
            timer_d: Duration::from_secs(32),
            timer_f: Duration::from_secs(32),
            max_events: 100,
        }
    }
}

/// Call configuration
#[derive(Debug, Clone)]
pub struct CallConfig {
    /// Enable audio
    pub audio_enabled: bool,

    /// Enable video (not implemented yet)
    pub video_enabled: bool,

    /// Enable DTMF
    pub dtmf_enabled: bool,

    /// Auto answer incoming calls
    pub auto_answer: bool,

    /// Auto answer delay in milliseconds
    pub auto_answer_delay: Duration,

    /// Call timeout
    pub call_timeout: Duration,

    /// Media configuration overrides
    pub media: Option<MediaConfig>,

    /// Authentication username (if different from client username)
    pub auth_username: Option<String>,

    /// Authentication password
    pub auth_password: Option<String>,

    /// Display name
    pub display_name: Option<String>,
}

impl Default for CallConfig {
    fn default() -> Self {
        Self {
            audio_enabled: true,
            video_enabled: false,
            dtmf_enabled: true,
            auto_answer: false,
            auto_answer_delay: Duration::from_secs(0),
            call_timeout: Duration::from_secs(60),
            media: None,
            auth_username: None,
            auth_password: None,
            display_name: None,
        }
    }
}

impl CallConfig {
    /// Create a new call configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable audio
    pub fn with_audio(mut self, enabled: bool) -> Self {
        self.audio_enabled = enabled;
        self
    }

    /// Enable or disable video
    pub fn with_video(mut self, enabled: bool) -> Self {
        self.video_enabled = enabled;
        self
    }

    /// Enable or disable DTMF
    pub fn with_dtmf(mut self, enabled: bool) -> Self {
        self.dtmf_enabled = enabled;
        self
    }

    /// Enable or disable auto answer
    pub fn with_auto_answer(mut self, auto_answer: bool) -> Self {
        self.auto_answer = auto_answer;
        self
    }

    /// Set auto answer delay
    pub fn with_auto_answer_delay(mut self, delay: Duration) -> Self {
        self.auto_answer_delay = delay;
        self
    }

    /// Set call timeout
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// Set media configuration
    pub fn with_media(mut self, media: MediaConfig) -> Self {
        self.media = Some(media);
        self
    }

    /// Set authentication username
    pub fn with_auth_username<S: Into<String>>(mut self, username: S) -> Self {
        self.auth_username = Some(username.into());
        self
    }

    /// Set authentication password
    pub fn with_auth_password<S: Into<String>>(mut self, password: S) -> Self {
        self.auth_password = Some(password.into());
        self
    }

    /// Set display name
    pub fn with_display_name<S: Into<String>>(mut self, name: S) -> Self {
        self.display_name = Some(name.into());
        self
    }
}

/// Supported audio codecs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecType {
    /// G.711 μ-law
    PCMU,
    /// G.711 A-law
    PCMA,
    /// G.722
    G722,
    /// G.729
    G729,
    /// Opus
    OPUS,
} 