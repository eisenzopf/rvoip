use std::net::SocketAddr;
use std::time::Duration;
use std::net::IpAddr;

use rvoip_ice_core::IceServerConfig as IceServerCoreConfig;
use rvoip_media_core::codec::CodecType as MediaCodecType;
use crate::error::{Error, Result};
use crate::DEFAULT_SIP_PORT;

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

    /// Outbound proxy server address
    pub outbound_proxy: Option<SocketAddr>,

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
    
    /// Maximum number of calls to keep in history
    pub max_call_history: Option<usize>,
    
    /// Whether to retain call history between restarts
    pub persist_call_history: bool,

    /// Optional local IP address to use for media
    pub local_ip: Option<IpAddr>,
    
    /// RTP port range start (default: 10000)
    /// 
    /// This is the beginning of the range to use for allocating RTP ports
    pub rtp_port_range_start: Option<u16>,
    
    /// RTP port range end (default: 20000)
    /// 
    /// This is the end of the range to use for allocating RTP ports
    pub rtp_port_range_end: Option<u16>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            username: "anonymous".to_string(),
            domain: "localhost".to_string(),
            outbound_proxy: None,
            register_expires: 3600,
            register_refresh: 0.8,
            local_addr: None,
            user_agent: format!("RVOIP SIP Client {}", crate::VERSION),
            transport: TransportConfig::default(),
            transaction: TransactionConfig::default(),
            media: MediaConfig::default(),
            max_call_history: Some(100),
            persist_call_history: false,
            local_ip: None,
            rtp_port_range_start: None,
            rtp_port_range_end: None,
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

    /// Set outbound proxy server address
    pub fn with_outbound_proxy(mut self, proxy: Option<SocketAddr>) -> Self {
        self.outbound_proxy = proxy;
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

    /// Set the maximum number of calls to keep in history
    pub fn with_max_call_history(mut self, max: Option<usize>) -> Self {
        self.max_call_history = max;
        self
    }
    
    /// Set whether to persist call history between restarts
    pub fn with_persist_call_history(mut self, persist: bool) -> Self {
        self.persist_call_history = persist;
        self
    }

    /// Set the local IP address to use for media
    pub fn with_local_ip(mut self, local_ip: IpAddr) -> Self {
        self.local_ip = Some(local_ip);
        self
    }
    
    /// Set the RTP port range
    pub fn with_rtp_port_range(mut self, start: u16, end: u16) -> Self {
        self.rtp_port_range_start = Some(start);
        self.rtp_port_range_end = Some(end);
        self
    }
    
    /// Set auto-answer behavior
    pub fn with_auto_answer(mut self, enabled: bool) -> Self {
        self.media.auto_answer = enabled;
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
    
    /// Enable SRTP for secure media
    pub srtp_enabled: bool,
    
    /// Enable DTLS-SRTP for key exchange
    pub dtls_srtp_enabled: bool,
    
    /// Enable ICE for NAT traversal
    pub ice_enabled: bool,
    
    /// ICE servers (STUN/TURN)
    pub ice_servers: Vec<IceServerConfig>,
    
    /// Auto-answer incoming calls
    pub auto_answer: bool,
}

/// ICE server configuration
#[derive(Debug, Clone)]
pub struct IceServerConfig {
    /// Server URL (stun: or turn: protocol)
    pub url: String,
    
    /// Username for TURN server
    pub username: Option<String>,
    
    /// Credential for TURN server
    pub credential: Option<String>,
}

impl From<IceServerConfig> for IceServerCoreConfig {
    fn from(config: IceServerConfig) -> Self {
        Self {
            url: config.url,
            username: config.username,
            credential: config.credential,
        }
    }
}

impl From<IceServerCoreConfig> for IceServerConfig {
    fn from(config: IceServerCoreConfig) -> Self {
        Self {
            url: config.url,
            username: config.username,
            credential: config.credential,
        }
    }
}

/// Default RTP port range start
pub const DEFAULT_RTP_PORT_MIN: u16 = 10000;

/// Default RTP port range end
pub const DEFAULT_RTP_PORT_MAX: u16 = 20000;

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            rtp_enabled: true,
            rtcp_enabled: true,
            rtcp_interval: Duration::from_secs(5),
            rtp_port_min: DEFAULT_RTP_PORT_MIN,
            rtp_port_max: DEFAULT_RTP_PORT_MAX,
            jitter_buffer_ms: 60,
            audio_sample_rate: 8000,
            audio_ptime: 20,
            preferred_codecs: vec![CodecType::Pcmu, CodecType::Pcma],
            srtp_enabled: false,  // Default to disabled for backward compatibility
            dtls_srtp_enabled: false,  // Default to disabled for backward compatibility
            ice_enabled: false,  // Default to disabled for backward compatibility
            ice_servers: vec![
                // Default to Google's public STUN server
                IceServerConfig {
                    url: "stun:stun.l.google.com:19302".to_string(),
                    username: None,
                    credential: None,
                }
            ],
            auto_answer: false,  // Default to not auto-answering
        }
    }
}

impl MediaConfig {
    /// Enable SRTP for secure media
    pub fn with_srtp(mut self, enabled: bool) -> Self {
        self.srtp_enabled = enabled;
        self
    }
    
    /// Enable DTLS-SRTP for key exchange
    pub fn with_dtls_srtp(mut self, enabled: bool) -> Self {
        self.dtls_srtp_enabled = enabled;
        self
    }
    
    /// Enable ICE for NAT traversal
    pub fn with_ice(mut self, enabled: bool) -> Self {
        self.ice_enabled = enabled;
        self
    }
    
    /// Set ICE servers
    pub fn with_ice_servers(mut self, servers: Vec<IceServerConfig>) -> Self {
        self.ice_servers = servers;
        self
    }
    
    /// Add an ICE server
    pub fn add_ice_server(mut self, url: &str, username: Option<String>, credential: Option<String>) -> Self {
        self.ice_servers.push(IceServerConfig {
            url: url.to_string(),
            username,
            credential,
        });
        self
    }
    
    /// Set auto-answer behavior
    pub fn with_auto_answer(mut self, enabled: bool) -> Self {
        self.auto_answer = enabled;
        self
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
    
    /// RTP port range start (default: 10000)
    pub rtp_port_range_start: Option<u16>,
    
    /// RTP port range end (default: 20000)
    pub rtp_port_range_end: Option<u16>,
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
            rtp_port_range_start: None,
            rtp_port_range_end: None,
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
    
    /// Set RTP port range
    pub fn with_rtp_port_range(mut self, start: u16, end: u16) -> Self {
        self.rtp_port_range_start = Some(start);
        self.rtp_port_range_end = Some(end);
        self
    }
    
    /// Check if RTCP is enabled
    pub fn enable_rtcp(&self) -> bool {
        if let Some(media_config) = &self.media {
            media_config.rtcp_enabled
        } else {
            true // Default to enabled
        }
    }
    
    /// Check if ICE is enabled
    pub fn enable_ice(&self) -> bool {
        if let Some(media_config) = &self.media {
            media_config.ice_enabled
        } else {
            true // Default to enabled
        }
    }
}

/// Supported audio codecs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecType {
    /// G.711 μ-law
    Pcmu,
    /// G.711 A-law
    Pcma,
    /// G.722
    G722,
    /// G.729
    G729,
    /// Opus
    Opus,
}

impl From<CodecType> for MediaCodecType {
    fn from(codec: CodecType) -> Self {
        match codec {
            CodecType::Pcmu => MediaCodecType::Pcmu,
            CodecType::Pcma => MediaCodecType::Pcma,
            CodecType::G729 => MediaCodecType::G729,
            CodecType::Opus => MediaCodecType::Opus,
            // If G722 doesn't have a direct equivalent, use a placeholder or default
            _ => MediaCodecType::Pcmu, // Default for now
        }
    }
}

impl From<MediaCodecType> for CodecType {
    fn from(codec: MediaCodecType) -> Self {
        match codec {
            MediaCodecType::Pcmu => CodecType::Pcmu,
            MediaCodecType::Pcma => CodecType::Pcma,
            MediaCodecType::G729 => CodecType::G729,
            MediaCodecType::Opus => CodecType::Opus,
        }
    }
} 