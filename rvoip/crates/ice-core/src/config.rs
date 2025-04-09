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