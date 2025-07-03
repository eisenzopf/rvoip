use std::net::SocketAddr;
use std::time::Duration;
use serde::{Serialize, Deserialize};

/// ICE server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceServerConfig {
    /// Server URL (stun: or turn: protocol)
    pub url: String,
    
    /// Username for TURN server
    pub username: Option<String>,
    
    /// Credential for TURN server
    pub credential: Option<String>,
}

impl IceServerConfig {
    /// Create a new STUN server configuration
    pub fn new_stun(url: &str) -> Self {
        Self {
            url: url.to_string(),
            username: None,
            credential: None,
        }
    }
    
    /// Create a new TURN server configuration
    pub fn new_turn(url: &str, username: &str, credential: &str) -> Self {
        Self {
            url: url.to_string(),
            username: Some(username.to_string()),
            credential: Some(credential.to_string()),
        }
    }
    
    /// Is this a TURN server?
    pub fn is_turn(&self) -> bool {
        self.url.starts_with("turn:") || self.url.starts_with("turns:")
    }
    
    /// Is this a STUN server?
    pub fn is_stun(&self) -> bool {
        self.url.starts_with("stun:") || self.url.starts_with("stuns:")
    }
}

/// ICE configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceConfig {
    /// ICE servers (STUN/TURN)
    pub servers: Vec<IceServerConfig>,
    
    /// ICE connection timeout
    pub timeout: Duration,
    
    /// Whether to use UDP
    pub use_udp: bool,
    
    /// Whether to use TCP
    pub use_tcp: bool,
    
    /// Whether to gather host candidates
    pub gather_host: bool,
    
    /// Whether to gather server reflexive candidates
    pub gather_srflx: bool,
    
    /// Whether to gather relay candidates
    pub gather_relay: bool,
    
    /// Maximum gathering time in milliseconds
    pub max_gathering_time_ms: u64,
    
    /// Connection check interval in milliseconds
    pub check_interval_ms: u64,
    
    /// The binding interface for local candidates (optional)
    pub bind_interface: Option<String>,
    
    /// Override specified binding addresses
    pub bind_addresses: Vec<SocketAddr>,
    
    /// STUN server keep-alive interval in seconds
    pub stun_keepalive_interval: Option<u64>,
    
    /// Aggressive nomination (ICE-LITE approach)
    pub aggressive_nomination: bool,
    
    /// Gathering policy for candidates
    pub gathering_policy: GatheringPolicy,
}

/// Candidate gathering policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatheringPolicy {
    /// Gather all types of candidates
    All,
    
    /// Gather host candidates only
    HostOnly,
    
    /// Gather STUN/TURN candidates only (no host)
    NoHost,
    
    /// Gather relay candidates only
    RelayOnly,
}

impl Default for IceConfig {
    fn default() -> Self {
        Self {
            servers: vec![
                // Default to Google's public STUN server
                IceServerConfig {
                    url: "stun:stun.l.google.com:19302".to_string(),
                    username: None,
                    credential: None,
                }
            ],
            timeout: Duration::from_secs(30),
            use_udp: true,
            use_tcp: true,
            gather_host: true,
            gather_srflx: true,
            gather_relay: true,
            max_gathering_time_ms: 5000,
            check_interval_ms: 50,
            bind_interface: None,
            bind_addresses: Vec::new(),
            stun_keepalive_interval: Some(15),  // 15 seconds
            aggressive_nomination: false,
            gathering_policy: GatheringPolicy::All,
        }
    }
}

/// ICE agent role
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IceRole {
    /// Controlling role
    Controlling,
    
    /// Controlled role
    Controlled,
}

/// ICE component type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IceComponent {
    /// RTP component (ID 1)
    Rtp = 1,
    
    /// RTCP component (ID 2)
    Rtcp = 2,
}

impl IceComponent {
    /// Get the component ID
    pub fn id(&self) -> u32 {
        match self {
            Self::Rtp => 1,
            Self::Rtcp => 2,
        }
    }
    
    /// Create from component ID
    pub fn from_id(id: u32) -> Option<Self> {
        match id {
            1 => Some(Self::Rtp),
            2 => Some(Self::Rtcp),
            _ => None,
        }
    }
}

/// Builder for ICE configuration
pub struct IceConfigBuilder {
    config: IceConfig,
}

impl IceConfigBuilder {
    /// Create a new builder with default config
    pub fn new() -> Self {
        Self {
            config: IceConfig::default(),
        }
    }
    
    /// Add a STUN server
    pub fn add_stun_server(mut self, url: &str) -> Self {
        self.config.servers.push(IceServerConfig::new_stun(url));
        self
    }
    
    /// Add a TURN server
    pub fn add_turn_server(mut self, url: &str, username: &str, credential: &str) -> Self {
        self.config.servers.push(IceServerConfig::new_turn(url, username, credential));
        self
    }
    
    /// Set connection timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }
    
    /// Enable/disable UDP
    pub fn with_udp(mut self, enabled: bool) -> Self {
        self.config.use_udp = enabled;
        self
    }
    
    /// Enable/disable TCP
    pub fn with_tcp(mut self, enabled: bool) -> Self {
        self.config.use_tcp = enabled;
        self
    }
    
    /// Set gathering policy
    pub fn with_gathering_policy(mut self, policy: GatheringPolicy) -> Self {
        self.config.gathering_policy = policy;
        self
    }
    
    /// Set maximum gathering time
    pub fn with_max_gathering_time(mut self, ms: u64) -> Self {
        self.config.max_gathering_time_ms = ms;
        self
    }
    
    /// Enable/disable aggressive nomination
    pub fn with_aggressive_nomination(mut self, enabled: bool) -> Self {
        self.config.aggressive_nomination = enabled;
        self
    }
    
    /// Add a binding address
    pub fn add_bind_address(mut self, addr: SocketAddr) -> Self {
        self.config.bind_addresses.push(addr);
        self
    }
    
    /// Set binding interface
    pub fn with_bind_interface(mut self, interface: &str) -> Self {
        self.config.bind_interface = Some(interface.to_string());
        self
    }
    
    /// Set STUN keepalive interval
    pub fn with_stun_keepalive(mut self, seconds: u64) -> Self {
        self.config.stun_keepalive_interval = Some(seconds);
        self
    }
    
    /// Build the configuration
    pub fn build(self) -> IceConfig {
        self.config
    }
} 