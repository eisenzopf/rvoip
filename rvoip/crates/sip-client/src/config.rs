//! Configuration - Simple, clean SIP client configuration
//!
//! This module provides an easy-to-use configuration system for the SIP client,
//! with sensible defaults and support for multiple configuration sources.

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::path::Path;
use serde::{Deserialize, Serialize};

use crate::{Error, Result, DEFAULT_SIP_PORT};

/// Main configuration for the SIP client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// User credentials for SIP registration
    pub credentials: Option<Credentials>,
    /// SIP server settings
    pub server: ServerConfig,
    /// Local network settings
    pub local: LocalConfig,
    /// Media preferences
    pub media: MediaConfig,
    /// User agent string
    pub user_agent: String,
    /// Maximum concurrent calls
    pub max_concurrent_calls: usize,
    /// Call-engine integration settings
    pub call_engine: Option<CallEngineConfig>,
}

/// User credentials for SIP authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub domain: String,
}

/// SIP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Registrar server address (if different from domain)
    pub registrar: Option<String>,
    /// Proxy server address (if different from domain)
    pub proxy: Option<String>,
    /// Registration expiration time in seconds
    pub registration_expires: u32,
}

/// Local network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    /// Local SIP listening address
    pub sip_address: SocketAddr,
    /// Local media (RTP) address
    pub media_address: SocketAddr,
    /// Preferred local IP (for multi-homed systems)
    pub preferred_ip: Option<IpAddr>,
}

/// Media configuration and preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Preferred audio codecs (in priority order)
    pub preferred_codecs: Vec<String>,
    /// Audio device settings
    pub audio: AudioConfig,
}

/// Audio device configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Default microphone volume (0.0 - 1.0)
    pub microphone_volume: f32,
    /// Default speaker volume (0.0 - 1.0)
    pub speaker_volume: f32,
    /// Start with microphone muted
    pub microphone_muted: bool,
    /// Start with speaker muted
    pub speaker_muted: bool,
}

/// Call-engine integration configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEngineConfig {
    /// Call-engine server address
    pub server_address: String,
    /// Agent identification for call center
    pub agent_id: Option<String>,
    /// Default queue to join
    pub default_queue: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            credentials: None,
            server: ServerConfig::default(),
            local: LocalConfig::default(),
            media: MediaConfig::default(),
            user_agent: format!("rvoip-sip-client/{}", crate::VERSION),
            max_concurrent_calls: 5,
            call_engine: None,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            registrar: None,
            proxy: None,
            registration_expires: 3600, // 1 hour
        }
    }
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            sip_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0), // Random port
            media_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0), // Random port
            preferred_ip: None,
        }
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            preferred_codecs: vec![
                "PCMU".to_string(),
                "PCMA".to_string(),
                "opus".to_string(),
            ],
            audio: AudioConfig::default(),
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            microphone_volume: 0.8,
            speaker_volume: 0.8,
            microphone_muted: false,
            speaker_muted: false,
        }
    }
}

impl Config {
    /// Create a new default configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set user credentials
    pub fn with_credentials(mut self, username: &str, password: &str, domain: &str) -> Self {
        self.credentials = Some(Credentials {
            username: username.to_string(),
            password: password.to_string(),
            domain: domain.to_string(),
        });
        self
    }

    /// Set local SIP port (use 0 for random port)
    pub fn with_local_port(mut self, port: u16) -> Self {
        self.local.sip_address.set_port(port);
        self
    }

    /// Set local SIP address
    pub fn with_local_address(mut self, address: SocketAddr) -> Self {
        self.local.sip_address = address;
        self
    }

    /// Set media address
    pub fn with_media_address(mut self, address: SocketAddr) -> Self {
        self.local.media_address = address;
        self
    }

    /// Set user agent string
    pub fn with_user_agent(mut self, user_agent: &str) -> Self {
        self.user_agent = user_agent.to_string();
        self
    }

    /// Set preferred audio codecs
    pub fn with_codecs(mut self, codecs: Vec<String>) -> Self {
        self.media.preferred_codecs = codecs;
        self
    }

    /// Set maximum concurrent calls
    pub fn with_max_calls(mut self, max_calls: usize) -> Self {
        self.max_concurrent_calls = max_calls;
        self
    }

    /// Enable call-engine integration
    pub fn with_call_engine(mut self, server_address: &str) -> Self {
        self.call_engine = Some(CallEngineConfig {
            server_address: server_address.to_string(),
            agent_id: None,
            default_queue: None,
        });
        self
    }

    /// Load configuration from a TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Configuration(format!("Failed to read config file: {}", e)))?;
        
        let config: Config = toml::from_str(&content)
            .map_err(|e| Error::Configuration(format!("Failed to parse config file: {}", e)))?;
        
        Ok(config)
    }

    /// Save configuration to a TOML file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| Error::Configuration(format!("Failed to serialize config: {}", e)))?;
        
        std::fs::write(path, content)
            .map_err(|e| Error::Configuration(format!("Failed to write config file: {}", e)))?;
        
        Ok(())
    }

    /// Create configuration for a call center agent
    pub fn agent(username: &str, domain: &str) -> Self {
        Self::new()
            .with_credentials(username, "agent_password", domain)
            .with_user_agent(&format!("rvoip-agent/{}", crate::VERSION))
    }

    /// Create configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();
        
        // Load credentials from environment
        if let (Ok(username), Ok(password), Ok(domain)) = (
            std::env::var("SIP_USERNAME"),
            std::env::var("SIP_PASSWORD"),
            std::env::var("SIP_DOMAIN"),
        ) {
            config.credentials = Some(Credentials { username, password, domain });
        }
        
        // Load local port
        if let Ok(port) = std::env::var("SIP_LOCAL_PORT") {
            if let Ok(port) = port.parse::<u16>() {
                config.local.sip_address.set_port(port);
            }
        }
        
        // Load user agent
        if let Ok(user_agent) = std::env::var("SIP_USER_AGENT") {
            config.user_agent = user_agent;
        }
        
        Ok(config)
    }

    // === Helper methods for client-core integration ===

    /// Get the local SIP address
    pub fn local_sip_addr(&self) -> SocketAddr {
        self.local.sip_address
    }

    /// Get the local media address
    pub fn local_media_addr(&self) -> SocketAddr {
        self.local.media_address
    }

    /// Get the preferred codecs
    pub fn preferred_codecs(&self) -> &[String] {
        &self.media.preferred_codecs
    }

    /// Get the local URI for this client
    pub fn local_uri(&self) -> String {
        if let Some(ref creds) = self.credentials {
            format!("sip:{}@{}", creds.username, creds.domain)
        } else {
            format!("sip:anonymous@{}", self.local.sip_address.ip())
        }
    }

    /// Get the username (if configured)
    pub fn username(&self) -> Option<&str> {
        self.credentials.as_ref().map(|c| c.username.as_str())
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Check that we have credentials for registration
        if self.credentials.is_none() {
            return Err(Error::Configuration(
                "No credentials configured - cannot register with SIP server".to_string()
            ));
        }
        
        // Check that local addresses are valid
        if self.local.sip_address.port() == 0 && self.local.media_address.port() == 0 {
            // Both random ports is fine
        }
        
        // Check that we have at least one codec
        if self.media.preferred_codecs.is_empty() {
            return Err(Error::Configuration(
                "No preferred codecs configured".to_string()
            ));
        }
        
        Ok(())
    }
} 