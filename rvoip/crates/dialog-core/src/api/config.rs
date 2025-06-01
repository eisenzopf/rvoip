//! Configuration for Dialog-Core API
//!
//! This module provides configuration types for dialog-core API operations,
//! supporting both simple use cases and advanced customization.

use std::net::SocketAddr;
use std::time::Duration;
use serde::{Deserialize, Serialize};

/// Main configuration for dialog operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogConfig {
    /// Local address for SIP communication
    pub local_address: SocketAddr,
    
    /// User agent string to include in SIP messages
    pub user_agent: Option<String>,
    
    /// Default timeout for dialog operations
    pub dialog_timeout: Duration,
    
    /// Maximum number of concurrent dialogs
    pub max_dialogs: Option<usize>,
    
    /// Enable automatic dialog cleanup
    pub auto_cleanup: bool,
    
    /// Cleanup interval for terminated dialogs
    pub cleanup_interval: Duration,
}

impl Default for DialogConfig {
    fn default() -> Self {
        Self {
            local_address: "127.0.0.1:5060".parse().unwrap(),
            user_agent: Some("RVOIP-Dialog/1.0".to_string()),
            dialog_timeout: Duration::from_secs(180), // 3 minutes
            max_dialogs: Some(10000),
            auto_cleanup: true,
            cleanup_interval: Duration::from_secs(30),
        }
    }
}

impl DialogConfig {
    /// Create a new configuration with a specific local address
    pub fn new(local_address: SocketAddr) -> Self {
        Self {
            local_address,
            ..Default::default()
        }
    }
    
    /// Set the user agent string
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }
    
    /// Set the dialog timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.dialog_timeout = timeout;
        self
    }
    
    /// Set the maximum number of dialogs
    pub fn with_max_dialogs(mut self, max: usize) -> Self {
        self.max_dialogs = Some(max);
        self
    }
    
    /// Disable automatic cleanup
    pub fn without_auto_cleanup(mut self) -> Self {
        self.auto_cleanup = false;
        self
    }
    
    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.dialog_timeout.as_secs() == 0 {
            return Err("Dialog timeout must be greater than 0".to_string());
        }
        
        if let Some(max) = self.max_dialogs {
            if max == 0 {
                return Err("Max dialogs must be greater than 0".to_string());
            }
        }
        
        if self.cleanup_interval.as_secs() == 0 {
            return Err("Cleanup interval must be greater than 0".to_string());
        }
        
        Ok(())
    }
}

/// Server-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Base dialog configuration
    pub dialog: DialogConfig,
    
    /// Enable automatic response to OPTIONS requests
    pub auto_options_response: bool,
    
    /// Enable automatic response to REGISTER requests
    pub auto_register_response: bool,
    
    /// Server domain name
    pub domain: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            dialog: DialogConfig::default(),
            auto_options_response: true,
            auto_register_response: false,
            domain: None,
        }
    }
}

impl ServerConfig {
    /// Create a new server configuration with a local address
    pub fn new(local_address: SocketAddr) -> Self {
        Self {
            dialog: DialogConfig::new(local_address),
            ..Default::default()
        }
    }
    
    /// Set the server domain
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }
    
    /// Enable automatic OPTIONS response
    pub fn with_auto_options(mut self) -> Self {
        self.auto_options_response = true;
        self
    }
    
    /// Enable automatic REGISTER response
    pub fn with_auto_register(mut self) -> Self {
        self.auto_register_response = true;
        self
    }
    
    /// Validate the server configuration
    pub fn validate(&self) -> Result<(), String> {
        self.dialog.validate()
    }
}

/// Client-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Base dialog configuration
    pub dialog: DialogConfig,
    
    /// Default from URI for outgoing requests
    pub from_uri: Option<String>,
    
    /// Enable automatic authentication
    pub auto_auth: bool,
    
    /// Default credentials for authentication
    pub credentials: Option<Credentials>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            dialog: DialogConfig::default(),
            from_uri: None,
            auto_auth: false,
            credentials: None,
        }
    }
}

impl ClientConfig {
    /// Create a new client configuration with a local address
    pub fn new(local_address: SocketAddr) -> Self {
        Self {
            dialog: DialogConfig::new(local_address),
            ..Default::default()
        }
    }
    
    /// Set the default from URI
    pub fn with_from_uri(mut self, from_uri: impl Into<String>) -> Self {
        self.from_uri = Some(from_uri.into());
        self
    }
    
    /// Enable automatic authentication with credentials
    pub fn with_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.auto_auth = true;
        self.credentials = Some(Credentials {
            username: username.into(),
            password: password.into(),
            realm: None,
        });
        self
    }
    
    /// Validate the client configuration
    pub fn validate(&self) -> Result<(), String> {
        self.dialog.validate()?;
        
        if self.auto_auth && self.credentials.is_none() {
            return Err("Auto auth enabled but no credentials provided".to_string());
        }
        
        Ok(())
    }
}

/// Authentication credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    /// Username
    pub username: String,
    
    /// Password
    pub password: String,
    
    /// Realm (optional, will be extracted from challenge if not provided)
    pub realm: Option<String>,
}

impl Credentials {
    /// Create new credentials
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            realm: None,
        }
    }
    
    /// Set the realm
    pub fn with_realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = Some(realm.into());
        self
    }
} 