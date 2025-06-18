use std::net::SocketAddr;
use serde::{Deserialize, Serialize};

/// Configuration for the SIP client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Local SIP bind address
    pub local_sip_addr: SocketAddr,
    /// Local media bind address  
    pub local_media_addr: SocketAddr,
    /// User agent string
    pub user_agent: String,
    /// Preferred codec list (in order of preference)
    pub preferred_codecs: Vec<String>,
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
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
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

    /// Set preferred codecs
    pub fn with_codecs(mut self, codecs: Vec<String>) -> Self {
        self.preferred_codecs = codecs;
        self
    }

    /// Set maximum concurrent calls
    pub fn with_max_calls(mut self, max_calls: usize) -> Self {
        self.max_concurrent_calls = max_calls;
        self
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self::new()
    }
}
