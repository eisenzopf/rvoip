//! Configuration utilities and ICE configuration

use serde::{Serialize, Deserialize};

/// ICE configuration for connectivity establishment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceConfig {
    /// STUN server URLs
    pub stun_servers: Vec<String>,
    /// TURN server configurations
    pub turn_servers: Vec<TurnServerConfig>,
    /// ICE gathering timeout
    pub gathering_timeout: std::time::Duration,
}

/// TURN server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnServerConfig {
    /// TURN server URL
    pub url: String,
    /// Authentication username
    pub username: String,
    /// Authentication credential
    pub credential: String,
}

impl Default for IceConfig {
    fn default() -> Self {
        Self {
            stun_servers: vec!["stun:stun.l.google.com:19302".to_string()],
            turn_servers: vec![],
            gathering_timeout: std::time::Duration::from_secs(5),
        }
    }
} 